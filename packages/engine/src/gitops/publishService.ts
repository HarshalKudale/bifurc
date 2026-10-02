import * as fs from "fs";
import * as path from "path";
import * as os from "os";
import { getGit } from "../workspace/gitStore";
import { getSyncConfig } from "./syncManager";
import { wsDir } from "../workspace/workspaceFs";
import { unquoteGitPath } from "./statusTracker";
import { withIndexLockRetry } from "./gitLock";

/**
 * Revert has exactly **one** implementation, and it is `gitOps.discardChanges()`.
 *
 * A second one used to live here — `restoreEntity()`, which did `git checkout HEAD -- <path>` and,
 * when that failed, unlinked the untracked file. It was never called (`entity.restore` has always
 * routed to `discardChanges`), and that was lucky rather than tidy: it skipped the three things
 * `discardChanges` does and this one did not — clearing the pending-deletion record, re-syncing
 * `names.json`, and invalidating the status cache. Any caller that had reached for the closer-looking
 * name would have left a reverted entity still marked deleted and still missing from the sidebar.
 *
 * Deleted 2026-09-28 while unifying publish/sync/revert/delete across entity types; `TESTING.md`'s
 * dead-code note had already recorded it as unused.
 */
export interface PublishOptions {
  wsId: string;
  /** Relative paths (files or directories) to git add and commit */
  paths: string[];
  /** Optional override for commit message */
  message?: string;
}

/**
 * What a publish actually did.
 *
 * ## `commit` is the proof; `ok` is only a claim
 *
 * Publish and Sync are the same two git commands — `git add` then `git commit` — so both produce a
 * commit, and the **hash** is the one thing that says a git operation really happened. `{ok: true}`
 * could not: `publishEntities()` returns it when it staged nothing, which is correct for an
 * already-committed file (and test-pinned: "does not create an empty commit when nothing changed") but
 * was indistinguishable at the UI from a commit. That is what *"clicking publish does nothing"* was —
 * correct behaviour, reported as nothing.
 *
 * | result | means |
 * | --- | --- |
 * | `{ok: true, commit: "<sha>"}` | a commit was created; this is its id |
 * | `{ok: true, commit: null}` | nothing was staged — the entity was already committed |
 * | `{ok: false, error}` | a step failed; the repo may still have changed, see below |
 *
 * ## The key is present only when the operation is commit-producing
 *
 * A revert is `git checkout HEAD -- <path>` and creates no commit, so it carries no `commit` key at
 * all — rather than `commit: null`, which would read as "a commit was expected and did not happen".
 * The two are different statements, and only the caller knows which operation it asked for.
 *
 * ## `{ok: false}` does not mean "nothing happened"
 *
 * `publishEntities()` stages *before* it commits and pushes, so a failure at the push step leaves a
 * real commit behind. Callers must refresh their view on failure too — see
 * `useEntityGitActions.write()`.
 */
export interface PublishResult {
  ok: boolean;
  error?: string;
  /** The created commit's id, or `null` when there was nothing to commit. */
  commit?: string | null;
}

function deviceName(): string {
  return os.hostname() || "unknown";
}

/**
 * Derive a display name from a workspace-relative file path by reading the JSON name field.
 *
 * **`label` before `domain`**, and the order is the point: a mapping is the one kind whose card is
 * titled by `label` and identified by `domain`, so a commit subject built from `domain` alone read
 * `create mapping app.localhost` for a card the user knows as "My App". The History sidebar and the
 * Audit Log both show this string, so the two disagreed about the entity's name for exactly one kind.
 * `label` is absent on every other kind, so the extra link in the chain changes nothing for them.
 *
 * `getNameEntry` for `mappings` (`store/kinds/crudKinds.ts`) already prefers the label — this is that
 * same rule, in the second place that has to agree with it.
 */
function readEntityName(wsId: string, relPath: string): string | null {
  try {
    const abs = path.join(wsDir(wsId), relPath);
    const raw = fs.readFileSync(abs, "utf-8");
    const parsed = JSON.parse(raw);
    return parsed.name || parsed.label || parsed.domain || parsed.urlPattern || null;
  } catch {
    return null;
  }
}

function entityKindFromPath(relPath: string): string {
  const parts = relPath.split(/[/\\]/);
  return parts[0] ?? "entity";
}

function singularKind(kind: string): string {
  const map: Record<string, string> = {
    requests: "request",
    mocks: "mock",
    sockets: "socket",
    mappings: "mapping",
    rules: "rule",
    environments: "environment",
  };
  return map[kind] ?? kind;
}

function entityIdFromPath(relPath: string): string | null {
  const base = relPath.split(/[/\\]/).pop();
  if (!base || !base.endsWith(".json")) return null;
  return base.slice(0, -5);
}

/**
 * The requested paths git will not stage, with the reason for each. Empty when every path is either
 * tracked (so "nothing staged" really does mean "unchanged") or already staged.
 *
 * **This exists to keep two very different outcomes apart.** `git add` staging nothing for an unchanged
 * file is correct and must stay a silent success; `git add` staging nothing for a file the caller named
 * and the status map calls *untracked* means the two disagree, and only one of them can be right. The
 * caller needs the second reported, not folded into the first — that fold is what made
 * *"clicking publish does nothing"* look like *"this entity is already committed"*.
 *
 * A **folder** pathspec (trailing separator) is skipped: there is no single file to name, and a folder
 * with nothing to stage is an ordinary no-op. `folder.publish` passes one.
 */
async function findUnstageablePaths(wsId: string, paths: string[]): Promise<string[]> {
  const g = getGit(wsId);
  const problems: string[] = [];

  for (const raw of paths) {
    if (raw.endsWith("/") || raw.endsWith("\\")) continue;
    const rel = raw.replace(/\\/g, "/");

    let tracked = false;
    try {
      tracked = (await g.raw(["ls-files", "--error-unmatch", "--", rel])).trim().length > 0;
    } catch {
      tracked = false;
    }
    // Tracked and unchanged is the one honest "nothing to publish".
    if (tracked) continue;

    if (fs.existsSync(path.join(wsDir(wsId), rel))) {
      // The two causes worth naming, in the order they are worth checking. A stale `index.lock` is the
      // one that produces this exact pair — `git status` reads happily while every `git add` fails —
      // and it is left behind by any git process that was killed mid-write, which includes the app
      // being closed during a push.
      problems.push(
        `${rel} exists but git will not stage it — check for a stale .git/index.lock (a killed git process leaves one), or a .gitignore rule`,
      );
    } else {
      problems.push(`${rel} does not exist in this workspace`);
    }
  }

  return problems;
}

/** Publish (git add → commit → push) one or more file/folder paths */
export async function publishEntities(opts: PublishOptions): Promise<PublishResult> {
  try {
    const g = getGit(opts.wsId);
    const actor = deviceName();

    // Check pre-staging status to know which files were new/modified/deleted.
    // NOTE: `--porcelain` without `-z` C-quotes paths containing spaces, so the keys must be
    // unquoted — otherwise `preStatusMap.get(singlePath)` misses for e.g. `mocks/My Folder/x.json`
    // and the create/update/delete classification below silently degrades to "update".
    const preStatus = await g.raw(["status", "--porcelain", "-uall"]).catch(() => "");
    const preStatusMap = new Map<string, string>();
    for (const line of preStatus.split("\n")) {
      if (!line.trim()) continue;
      const xy = line.slice(0, 2);
      const filePath = unquoteGitPath(line.slice(3).trim());
      preStatusMap.set(filePath, xy);
    }

    // Stage all specified paths (handles new/modified files and deletions).
    //
    // ⚠ **The `catch` collects rather than ignores.** It used to be a bare `catch {}` with the comment
    // "path may not exist (already deleted) — ignore", which silently swallowed *every* failure: a
    // locked index, a permission error, a pathspec that matches nothing. The consequence was that
    // "git refused to stage this" and "there was nothing to stage" became the same outcome — a
    // `{ok: true, commit: null}` — and the UI told the user the entity was "already committed" while
    // its own status map still said `new`. Collected here, reported below.
    const stageFailures: string[] = [];
    for (const p of opts.paths) {
      try {
        // Retried, because a lock held by a *live* git process is transient — see `gitLock.ts`. The
        // one left by a killed process is removed there rather than reported as a broken workspace.
        await withIndexLockRetry(opts.wsId, () => g.raw(["add", "-A", "--", p]));
      } catch (e) {
        stageFailures.push(`${p}: ${(e instanceof Error ? e.message : String(e)).split("\n")[0]}`);
      }
    }

    // Check if anything is staged
    const status = await g.status();
    const hasStagedChanges =
      status.staged.length > 0 ||
      status.created.length > 0 ||
      status.deleted.length > 0 ||
      status.renamed.length > 0;

    if (!hasStagedChanges) {
      // Nothing staged. That is a legitimate no-op **only** if git already holds every path the caller
      // named — an unchanged, tracked file. Anything else is a refusal, and reporting it as "nothing to
      // publish" is how a broken publish came to look like a no-op.
      const problems = [...stageFailures, ...(await findUnstageablePaths(opts.wsId, opts.paths))];
      if (problems.length > 0) {
        return { ok: false, error: `git staged nothing — ${problems.join("; ")}` };
      }
      return { ok: true, commit: null };
    }

    // Build commit message
    let message = opts.message;
    let body: string | undefined;

    if (!message) {
      if (opts.paths.length === 1) {
        const p = opts.paths[0];
        const isDir = p.endsWith("/") || p.endsWith("\\");

        if (isDir) {
          // Folder/bundled publish — use a structured message for each staged file
          // so queryLog can parse them. We collect all staged entity paths.
          // Dedupe: simple-git reports a newly-added file in BOTH `staged` and `created`,
          // so concatenating without a Set double-counts every new file. That made the
          // `allStagedPaths.length === 1` branch below unreachable for a folder holding a
          // single new entity — it fell through to the bundled form and recorded a
          // *create* as `update mock folder "X"`, losing the per-entity linkage.
          const allStagedPaths = Array.from(
            new Set([...status.staged, ...status.created, ...status.deleted]),
          ).filter((f) => f.endsWith(".json") && !["index.json", "enabled.json", "names.json"].some((s) => f.endsWith(s)));

          if (allStagedPaths.length === 1) {
            // Single entity in folder publish — treat as single entity publish
            const singlePath = allStagedPaths[0];
            const kind = singularKind(entityKindFromPath(singlePath));
            const entityId = entityIdFromPath(singlePath);
            const preXy = preStatusMap.get(singlePath) ?? "";
            const isDeleted = status.deleted.includes(singlePath) || preXy[0] === "D" || preXy[1] === "D";
            const isNew = preXy === "??" || preXy === "A " || preXy === "AM";
            const action = isDeleted ? "delete" : isNew ? "create" : "update";
            const name = isDeleted ? (entityId ?? "unknown") : (readEntityName(opts.wsId, singlePath) ?? entityId ?? "unknown");
            message = `${action} ${kind} ${name}`;
            body = entityId ? `entity-id: ${entityId}\nworkspace-id: ${opts.wsId}\nactor: ${actor}` : undefined;
          } else {
            // Multiple entities — bundled commit; list entity IDs so history can still link them
            const parts = p.replace(/[/\\]+$/, "").split(/[/\\]/);
            const kindPart = parts[0] ?? "folder";
            const folderPart = parts[1];
            const entityIds = allStagedPaths.map((f) => entityIdFromPath(f)).filter(Boolean);
            const subjectKind = singularKind(kindPart);
            message = folderPart
              ? `update ${subjectKind} folder "${folderPart}"`
              : `update ${subjectKind} changes`;
            // Body lists all entity IDs for history linkage
            if (entityIds.length > 0) {
              body = `workspace-id: ${opts.wsId}\nactor: ${actor}\nentity-ids: ${entityIds.join(",")}`;
            }
          }
        } else {
          // Single file publish
          const kind = singularKind(entityKindFromPath(p));
          const entityId = entityIdFromPath(p);
          const preXy = preStatusMap.get(p) ?? "";
          const isDeleted = status.deleted.includes(p) || preXy[0] === "D" || preXy[1] === "D";
          const isNew = preXy === "??" || preXy === "A " || preXy === "AM";
          const action = isDeleted ? "delete" : isNew ? "create" : "update";
          const name = isDeleted ? (readEntityName(opts.wsId, p) ?? entityId ?? "unknown") : (readEntityName(opts.wsId, p) ?? entityId ?? "unknown");
          message = `${action} ${kind} ${name}`;
          body = entityId ? `entity-id: ${entityId}\nworkspace-id: ${opts.wsId}\nactor: ${actor}` : undefined;
        }
      } else {
        // Multiple paths
        message = `update ${opts.paths.length} files`;
        body = `workspace-id: ${opts.wsId}\nactor: ${actor}`;
      }
    }

    const fullMessage = body ? `${message}\n\n${body}` : message;
    // `simple-git`'s `commit()` resolves to the commit it created. `|| null` guards the one shape it
    // can take when git reports nothing was written — a success with no id must not be reported as an
    // id, because the id is what the caller is checking. Retried on an index lock: the commit takes it
    // too, so a collision here is the same failure one step later.
    const committed = await withIndexLockRetry(opts.wsId, () => g.commit(fullMessage));

    // Push if remote is configured
    const config = getSyncConfig(opts.wsId);
    if (config?.remote) {
      await g.push("origin", `HEAD:${config.branch}`);
    }

    return { ok: true, commit: committed.commit || null };
  } catch (e) {
    return { ok: false, error: e instanceof Error ? e.message : String(e) };
  }
}
