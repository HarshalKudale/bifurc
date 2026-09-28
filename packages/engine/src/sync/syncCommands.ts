/**
 * The 17 `sync.*` / `git.*` / `entity.publish` / `folder.publish` / `entity.restore` / `audit.diff` /
 * `history.*` commands, moved out of `src/ipc/handlers/syncHandlers.ts` (P6 step 3e, slice 6).
 *
 * ## Why they moved (§5c)
 *
 * They were registered at **module scope** in the shell file, so a containerised engine (P9) answered
 * `UNKNOWN_COMMAND` for all 17. The file's channels are thin `commandRegistry.invoke()` delegations, so
 * they were only ever as alive as the registry behind them.
 *
 * ## Nothing had to travel, except one helper that must NOT be collapsed
 *
 * Every dependency was already in this package (`sync/syncManager`, `sync/autoSync`, `sync/gitOps`,
 * `sync/statusTracker`, `store/config`, `store/gitStore`, `proxy/server`, `eventBus`) — the shell file
 * was a pure adapter, as in slice 5. The one exception is a **local helper**, and it is the reason this
 * header exists.
 *
 * `folder.publish` declares its own `sanitizeDirName`. This package **already exports a function of that
 * exact name** from `store/workspaceFs.ts`, which `store/folderCommands.ts` imports — so the obvious
 * move is to collapse the two. **They are not the same function:**
 *
 * | | `store/workspaceFs.sanitizeDirName` | the local one here |
 * | --- | --- | --- |
 * | pattern | `[<>:"/\\|?*\x00-\x1f]` | `[<>:"/\\|?*]+` |
 * | replacement | `""` — stripped | `"-"` |
 * | fallback | `\|\| "unnamed"` | none |
 *
 * So `"a:b"` becomes `"ab"` under the engine's and `"a-b"` under this one, and a name of only illegal
 * characters becomes `"unnamed"` versus `"-"`. Collapsing would silently change the git path a folder
 * publish writes to — for names with a colon, a control character, or no legal characters at all.
 * **Kept local and verbatim** — during a *move*, which is what this file was.
 *
 * ### Resolved 2026-09-28: the two are now one function, and it is the engine's
 *
 * The note above is preserved because the reasoning was right for a move and the general rule it
 * teaches still holds ("grep the helper's *body*, not its name"). What it could not decide is which of
 * the two is *correct* — and only one can be. A folder is created by `writeEntity()` →
 * `workspaceFs.sanitizeDirName`, so the **storage** sanitiser is the only one whose output can name a
 * directory that exists. The local `"-"` variant built a pathspec for a directory that was never
 * created: a folder called `a:b` lives at `ab/` and was being published as `a-b/`, which matches
 * nothing — so "Publish All" on that folder silently did nothing while the UI reported changes in it
 * forever. `folder.publish` now imports the storage sanitiser, and `syncCommands.ts` no longer declares
 * a second one. The two sites are no longer "different things": a pathspec prefix must equal the
 * directory it points at.
 *
 * ## Three bodies that look like one, and are deliberately left as three
 *
 * `git.sync`, `entity.publish` and `folder.publish` share a shape — `syncChanges` →
 * `emitEntityStatus` → a best-effort `updateLastKnownHead`. They are **not** identical (only
 * `folder.publish` builds a pathspec prefix), and they are kept as three registrations rather than
 * factored into one helper: factoring is a behaviour-affecting refactor, and this is a move. The
 * same reasoning keeps `git.discard` and `entity.restore` as two commands with one body — the shell's
 * own comment calls the `entity.*` / `folder.*` names "backward-compatible ... delegating to generic
 * git ops", so both names are part of the contract the renderer already calls.
 *
 * The silent `catch {}` around `updateLastKnownHead` is also preserved: reading the remote head is a
 * bookkeeping step, and a failure there must not turn a successful publish into a failed one. It is
 * `catch { }` with no binding and no log, which is deliberate rather than sloppy — the publish has
 * already happened and there is nothing actionable to report.
 *
 * ## Conditionals that are load-bearing
 *
 *  - `sync.setRemote` reloads the config **only when `result.ok`**.
 *  - `sync.pull` reloads, invalidates the status cache and emits an entity status **only when
 *    `result.ok && result.updated`** — a pull that changed nothing must not churn the renderer's
 *    entity list, and the `updated` half is the part that makes that true.
 *  - `sync.setAutoSync` starts or stops the poller from the **requested** `enabled`, not from
 *    `result.ok`. Preserved: the renderer's toggle must leave the poller matching what the user
 *    asked for even if persisting the setting failed, which is the shell's behaviour.
 *
 * ## `history.*` normalisation, and `audit.diff`'s resolved empty
 *
 * `history.list` and `history.diff` both normalise `\` to `/` before touching git, because the
 * renderer sends Windows separators and the store is keyed by posix ones. `history.list` additionally
 * defaults `limit ?? 100` / `offset ?? 0` and falls back to the active workspace when `workspaceId` is
 * absent; `history.diff` does neither, and that asymmetry is the shell's.
 *
 * `audit.diff` resolves a relative path by **substring match** against the commit's changed files,
 * falling back to the first entry, and returns `{before: null, after: null}` when the commit changed
 * nothing. That is a resolved value, so it is an envelope **success** — not a throw, and not an error
 * the renderer has to handle.
 *
 * ## What is deliberately NOT here
 *
 * `audit.list` is not one of the 17 and does not belong here: step 3d widened `AuditListParams` and
 * moved it into `store/auditCommands.ts`. The shell's `audit:list` channel still calls `queryLog`
 * directly rather than through the registry, and per `plan/07` §5a it is **not** re-pointed — the
 * `register*Commands()` modules run from `registerIpcHandlers()`, and two suites register handler
 * groups without it, so a delegating legacy body acquires a hidden ordering dependency.
 *
 * None of the 17 takes a `CommandContext`: every body reaches the module-level `emitEntityStatus`
 * directly, which is why there is no `ctx` parameter here.
 */
import type {
    SyncSetRemoteParams, SyncDisconnectParams, SyncPushParams, SyncPullParams,
    SyncGetStateParams, SyncSetAutoSyncParams, SyncGetEntityStatusParams,
    GitDiffParams, GitDiscardParams, GitSyncParams, GitHistoryParams,
    EntityPublishParams, EntityRestoreParams, FolderPublishParams,
    AuditDiffParams, HistoryListParams, HistoryDiffParams,
} from "@bifurc/protocol";
import {
    setRemote, disconnect, syncPush, syncPull, getSyncState, setAutoSync, getRemoteHead,
} from "./syncManager";
import { startAutoSync, stopAutoSync, updateLastKnownHead } from "./autoSync";
import { getFileDiff, discardChanges, syncChanges, getFileHistory } from "./gitOps";
import { getWorkspaceSyncStatus, invalidateCache } from "./statusTracker";
import { getWorkspaceGitState } from "./gitState";
import { loadConfig } from "../store/config";
import { sanitizeDirName } from "../store/workspaceFs";
import { reloadConfig } from "../proxy/server";
import { queryLog, getEntityAtCommit, getCommitChangedFiles } from "../store/gitStore";
import { emitEntityStatus } from "../eventBus";
import type { CommandRegistry } from "../commands/registry";

export function registerSyncCommands(registry: CommandRegistry): void {
    registry.register("sync.setRemote", async ({ workspaceId, remote, branch }: SyncSetRemoteParams) => {
        const result = await setRemote(workspaceId, remote, branch);
        if (result.ok) reloadConfig();
        return result;
    });

    registry.register("sync.disconnect", ({ workspaceId }: SyncDisconnectParams) => disconnect(workspaceId));

    registry.register("sync.push", ({ workspaceId }: SyncPushParams) => syncPush(workspaceId));

    registry.register("sync.pull", async ({ workspaceId }: SyncPullParams) => {
        const result = await syncPull(workspaceId);
        if (result.ok && result.updated) {
            reloadConfig();
            invalidateCache(workspaceId);
            emitEntityStatus(workspaceId);
        }
        return result;
    });

    registry.register("sync.getState", ({ workspaceId }: SyncGetStateParams) => getSyncState(workspaceId));

    registry.register("sync.setAutoSync", async ({ workspaceId, enabled }: SyncSetAutoSyncParams) => {
        const result = await setAutoSync(workspaceId, enabled);
        if (enabled) startAutoSync(workspaceId);
        else stopAutoSync(workspaceId);
        return result;
    });

    registry.register("sync.getEntityStatus", async ({ workspaceId }: SyncGetEntityStatusParams) => {
        // Both axes, read together. The UI cannot decide what to offer an entity from the file map
        // alone — a committed-but-unpushed entity is `clean` there yet still needs a Sync. See
        // `sync/gitState.ts`.
        const [files, git] = await Promise.all([
            getWorkspaceSyncStatus(workspaceId),
            getWorkspaceGitState(workspaceId),
        ]);
        return { files, git };
    });

    registry.register("git.diff", ({ workspaceId, relPath }: GitDiffParams) => getFileDiff(workspaceId, relPath));

    registry.register("git.discard", async ({ workspaceId, relPath }: GitDiscardParams) => {
        const result = await discardChanges(workspaceId, relPath);
        emitEntityStatus(workspaceId);
        reloadConfig();
        return result;
    });

    registry.register("git.sync", async ({ workspaceId, paths, message }: GitSyncParams) => {
        const result = await syncChanges(workspaceId, paths, message);
        emitEntityStatus(workspaceId);
        try {
            const sha = await getRemoteHead(workspaceId);
            if (sha) updateLastKnownHead(workspaceId, sha);
        } catch { }
        return result;
    });

    registry.register("git.history", ({ workspaceId, relPath, limit, offset }: GitHistoryParams) =>
        getFileHistory(workspaceId, relPath, { limit, offset }));

    registry.register("entity.publish", async ({ workspaceId, paths, message }: EntityPublishParams) => {
        const result = await syncChanges(workspaceId, paths, message);
        emitEntityStatus(workspaceId);
        try {
            const sha = await getRemoteHead(workspaceId);
            if (sha) updateLastKnownHead(workspaceId, sha);
        } catch { }
        return result;
    });

    registry.register("folder.publish", async ({ workspaceId, kind, folderName }: FolderPublishParams) => {
        /**
         * **The storage sanitiser, not the local one this used to carry.**
         *
         * The folder being published was created by `writeEntity()` → `workspaceFs.sanitizeDirName`,
         * which **strips** illegal characters and falls back to `"unnamed"`. The local variant here
         * **replaced** them with `"-"`, so the pathspec it built could only ever name a directory that
         * does not exist: a folder called `a:b` is on disk as `ab/` and this asked git to publish
         * `a-b/`, which matches nothing. The UI would report changes in that folder forever and
         * "Publish All" would do nothing — a *group* publish that silently no-ops.
         *
         * The header's table below is kept as the record of why the two were separated during the P6
         * move (a move must not change behaviour). They are now one function, because only one of them
         * can be right: a pathspec has to match what the storage layer wrote.
         */
        const folderPath = folderName ? `${kind}/${sanitizeDirName(folderName)}/` : `${kind}/`;
        const result = await syncChanges(workspaceId, [folderPath]);
        emitEntityStatus(workspaceId);
        try {
            const sha = await getRemoteHead(workspaceId);
            if (sha) updateLastKnownHead(workspaceId, sha);
        } catch { }
        return result;
    });

    registry.register("entity.restore", async ({ workspaceId, relPath }: EntityRestoreParams) => {
        const result = await discardChanges(workspaceId, relPath);
        emitEntityStatus(workspaceId);
        reloadConfig();
        return result;
    });

    registry.register("audit.diff", async ({ commitHash, entityId, workspaceId }: AuditDiffParams) => {
        // Resolve the actual file path from the commit's changed files list
        const changed = await getCommitChangedFiles(commitHash, workspaceId);
        const relPath = changed.find((f) => f.includes(entityId)) ?? changed[0] ?? "";
        if (!relPath) return { before: null, after: null };
        const after = await getEntityAtCommit(commitHash, workspaceId, relPath);
        const before = await getEntityAtCommit(`${commitHash}~1`, workspaceId, relPath);
        return { before, after };
    });

    registry.register("history.list", async (opts: HistoryListParams) => {
        const wsId = opts.workspaceId ?? loadConfig().activeWorkspaceId;
        const normalized = opts.filePath.replace(/\\/g, "/");
        return queryLog({ workspaceId: wsId, filePath: normalized, limit: opts.limit ?? 100, offset: opts.offset ?? 0 });
    });

    registry.register("history.diff", async ({ commitHash, filePath, workspaceId }: HistoryDiffParams) => {
        const normalized = filePath.replace(/\\/g, "/");
        const after = await getEntityAtCommit(commitHash, workspaceId, normalized);
        const before = await getEntityAtCommit(`${commitHash}~1`, workspaceId, normalized);
        return { before, after };
    });
}
