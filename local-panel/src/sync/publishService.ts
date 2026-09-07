import * as fs from "fs";
import * as path from "path";
import * as os from "os";
import { getGit } from "@/store/gitStore";
import { getSyncConfig } from "@/sync/syncManager";
import { wsDir } from "@/store/workspaceFs";

export interface PublishOptions {
  wsId: string;
  /** Relative paths (files or directories) to git add and commit */
  paths: string[];
  /** Optional override for commit message */
  message?: string;
}

function deviceName(): string {
  return os.hostname() || "unknown";
}

/** Derive a display name from a workspace-relative file path by reading the JSON name field */
function readEntityName(wsId: string, relPath: string): string | null {
  try {
    const abs = path.join(wsDir(wsId), relPath);
    const raw = fs.readFileSync(abs, "utf-8");
    const parsed = JSON.parse(raw);
    return parsed.name || parsed.domain || parsed.urlPattern || null;
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

/** Publish (git add → commit → push) one or more file/folder paths */
export async function publishEntities(opts: PublishOptions): Promise<{ ok: boolean; error?: string }> {
  try {
    const g = getGit(opts.wsId);
    const actor = deviceName();

    // Check pre-staging status to know which files were new/modified/deleted
    const preStatus = await g.raw(["status", "--porcelain", "-uall"]).catch(() => "");
    const preStatusMap = new Map<string, string>();
    for (const line of preStatus.split("\n")) {
      if (!line.trim()) continue;
      const xy = line.slice(0, 2);
      const filePath = line.slice(3).trim();
      preStatusMap.set(filePath, xy);
    }

    // Stage all specified paths (handles new/modified files and deletions)
    for (const p of opts.paths) {
      try {
        await g.raw(["add", "-A", "--", p]);
      } catch {
        // Path may not exist (already deleted) — ignore
      }
    }

    // Check if anything is staged
    const status = await g.status();
    const hasStagedChanges =
      status.staged.length > 0 ||
      status.created.length > 0 ||
      status.deleted.length > 0 ||
      status.renamed.length > 0;

    if (!hasStagedChanges) return { ok: true };

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
          const allStagedPaths = [
            ...status.staged,
            ...status.created,
            ...status.deleted,
          ].filter((f) => f.endsWith(".json") && !["index.json", "enabled.json", "names.json"].some((s) => f.endsWith(s)));

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
    await g.commit(fullMessage);

    // Push if remote is configured
    const config = getSyncConfig(opts.wsId);
    if (config?.remote) {
      await g.push("origin", `HEAD:${config.branch}`);
    }

    return { ok: true };
  } catch (e) {
    return { ok: false, error: e instanceof Error ? e.message : String(e) };
  }
}

/** Restore an entity to its last committed state (git checkout -- path) */
export async function restoreEntity(wsId: string, relPath: string): Promise<{ ok: boolean; error?: string }> {
  try {
    const g = getGit(wsId);
    try {
      await g.raw(["checkout", "HEAD", "--", relPath]);
      return { ok: true };
    } catch (checkoutErr) {
      // If the file is untracked (never committed to HEAD), remove the untracked file
      const fullPath = path.join(wsDir(wsId), relPath);
      if (fs.existsSync(fullPath)) {
        fs.unlinkSync(fullPath);
        return { ok: true };
      }
      throw checkoutErr;
    }
  } catch (e) {
    return { ok: false, error: e instanceof Error ? e.message : String(e) };
  }
}
