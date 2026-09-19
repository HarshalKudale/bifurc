import { ipcMain, dialog } from "electron";
import type { AuditExportResult } from "@bifurc/protocol";
import { loadConfig } from "@bifurc/engine/store/config";
import { queryLog, QueryLogOptions, type AuditEntity } from "@bifurc/engine/store/gitStore";
import { bus } from "@bifurc/engine/eventBus";
import { commandRegistry } from "@bifurc/engine/commands/registry";
import { call, writeArtifact } from "@/ipc/fileOpsClient";

// ## Channels only — the registry half now lives in the engine
//
// The 17 `sync.*` / `git.*` / `entity.publish` / `folder.publish` / `entity.restore` / `audit.diff` /
// `history.*` commands used to be registered at **module scope** in this file, which is the `plan/07`
// §5c defect: a containerised engine (P9) answered `UNKNOWN_COMMAND` for all 17. Step 3e slice 6
// (2026-09-19) moved them into `packages/engine/src/sync/syncCommands.ts` behind
// `registerSyncCommands()`, called from `src/ipc/handlers.ts`.
//
// Every channel below **delegates** through `commandRegistry.invoke()` rather than duplicating the
// body — unlike `runnerHandlers.ts`, where three of the five are self-contained copies. That
// distinction decides what breaks: a delegating channel is only ever as alive as the registry behind
// it, so a suite that registers this handler group by hand **must** also call
// `registerSyncCommands(commandRegistry)`, or the channel exists with nothing behind it. The failure
// is `No handler registered` at *invocation* time, so nothing catches it at build time.
// `tests/integration/gitRemote.integration.test.ts` and `gitSync.integration.test.ts` both do this,
// and both import `registerSyncHandlers` statically at module scope — so before the move the registry
// was populated as an import side effect and neither needed an explicit call.
//
// The file's own P2 note is still accurate: every one of the 17 is a clean 1:1 with its legacy
// channel, with no `entityCrudFactory` involvement.
//
// P2 work item 7 — the bodies themselves (the conditional `reloadConfig`s, the silent `catch {}`
// around `updateLastKnownHead`, the pathspec prefix in `folder.publish` and its **local**
// `sanitizeDirName` that is deliberately not the engine's function of the same name) are documented in
// the engine module. Two dead imports went with the move: `fs` and `saveConfig` were already unused
// here, and `getSyncConfig` was imported and never referenced.
const ctx = { bus };

export function registerSyncHandlers() {
  // ── Sync ──────────────────────────────────────────────────────────────────
  ipcMain.handle("sync:setRemote", (_e, workspaceId: string, remote: string, branch: string) =>
    commandRegistry.invoke("sync.setRemote", { workspaceId, remote, branch }, ctx));

  ipcMain.handle("sync:disconnect", (_e, workspaceId: string) =>
    commandRegistry.invoke("sync.disconnect", { workspaceId }, ctx));

  ipcMain.handle("sync:push", (_e, workspaceId: string) =>
    commandRegistry.invoke("sync.push", { workspaceId }, ctx));

  ipcMain.handle("sync:pull", (_e, workspaceId: string) =>
    commandRegistry.invoke("sync.pull", { workspaceId }, ctx));

  ipcMain.handle("sync:getState", (_e, workspaceId: string) =>
    commandRegistry.invoke("sync.getState", { workspaceId }, ctx));

  ipcMain.handle("sync:setAutoSync", (_e, workspaceId: string, enabled: boolean) =>
    commandRegistry.invoke("sync.setAutoSync", { workspaceId, enabled }, ctx));

  ipcMain.handle("sync:getEntityStatus", (_e, workspaceId: string) =>
    commandRegistry.invoke("sync.getEntityStatus", { workspaceId }, ctx));

  // ── Generic Git Operations (file-centric) ───────────────────────────────────

  ipcMain.handle("git:diff", (_e, workspaceId: string, relPath: string) =>
    commandRegistry.invoke("git.diff", { workspaceId, relPath }, ctx));

  ipcMain.handle("git:discard", (_e, workspaceId: string, relPath: string) =>
    commandRegistry.invoke("git.discard", { workspaceId, relPath }, ctx));

  ipcMain.handle("git:sync", (_e, workspaceId: string, paths: string[], message?: string) =>
    commandRegistry.invoke("git.sync", { workspaceId, paths, message }, ctx));

  ipcMain.handle("git:history", (_e, workspaceId: string, relPath: string, opts?: { limit?: number; offset?: number }) =>
    commandRegistry.invoke("git.history", { workspaceId, relPath, ...opts }, ctx));

  // Backward-compatible entity / folder handlers delegating to generic git ops
  ipcMain.handle("entity:publish", (_e, workspaceId: string, paths: string[], message?: string) =>
    commandRegistry.invoke("entity.publish", { workspaceId, paths, message }, ctx));

  ipcMain.handle("folder:publish", (_e, workspaceId: string, kind: string, folderName: string | null) =>
    commandRegistry.invoke("folder.publish", { workspaceId, kind, folderName }, ctx));

  ipcMain.handle("entity:restore", (_e, workspaceId: string, relPath: string) =>
    commandRegistry.invoke("entity.restore", { workspaceId, relPath }, ctx));

  // ── Audit Log ──────────────────────────────────────────────────────────────

  // `audit:list` is the one channel in this file that does **not** go through the registry: it calls
  // `queryLog` directly. `audit.list` *is* registered — step 3d widened `AuditListParams` with
  // `filePath` / `fromTs` / `toTs` and moved the command into
  // `packages/engine/src/store/auditCommands.ts` — but this body is deliberately **not** re-pointed at
  // it. `plan/07` §5a: the `register*Commands()` modules run from `registerIpcHandlers()`, so a legacy
  // body that delegates acquires a hidden ordering dependency, and two suites register handler groups
  // without it. Re-pointing would break them at invocation time, invisible to the compiler.
  //
  // (The comment that used to sit here said `AuditListParams` was `.strict()` and lacked
  // `fromTs`/`toTs`, so routing it would have broken every date-filtered query. That was true until
  // 2026-09-19 and is now stale — the schema was widened precisely because
  // `renderer/panels/AuditLogPanel.tsx:61–62` sends both. The body stays for the §5a reason above,
  // not because the schema blocks it.)
  ipcMain.handle("audit:list", async (_e, opts: Omit<QueryLogOptions, "workspaceId"> & { workspaceId?: string } = {}) => {
    if (!opts.workspaceId) {
      const cfg = loadConfig();
      opts = { ...opts, workspaceId: cfg.activeWorkspaceId };
    }
    return queryLog(opts as QueryLogOptions);
  });

  ipcMain.handle("audit:diff", (_e, commitHash: string, _entity: AuditEntity, entityId: string, workspaceId: string) =>
    commandRegistry.invoke("audit.diff", { commitHash, entity: _entity, entityId, workspaceId }, ctx));

  ipcMain.handle("history:list", (_e, opts: { workspaceId?: string; filePath: string; limit?: number; offset?: number }) =>
    commandRegistry.invoke("history.list", opts, ctx));

  ipcMain.handle("history:diff", (_e, commitHash: string, filePath: string, workspaceId: string) =>
    commandRegistry.invoke("history.diff", { commitHash, filePath, workspaceId }, ctx));

  ipcMain.handle("audit:export", async (_e, format: "json" | "csv") => {
    // Dialog FIRST. The pre-P3 handler queried the whole log and *then* asked for a path, so
    // cancelling still paid for a `git log` over every commit the workspace has. "Fail fast on
    // cancel" (`File_Ops_Protocol.md` §3.2) is the whole difference here, because `limit: 0` means
    // the entire history.
    const { filePath, canceled } = await dialog.showSaveDialog({
      title: "Export Audit Log",
      defaultPath: `audit-log.${format}`,
      filters: [{ name: format.toUpperCase(), extensions: [format] }],
    });
    if (canceled || !filePath) return { ok: false };

    const artifact = await call<AuditExportResult>("audit.export", { format });
    if (!artifact.ok) return { ok: false, error: artifact.error };

    try {
      writeArtifact(artifact, filePath);
    } catch (err: any) {
      return { ok: false, error: err?.message };
    }
    return { ok: true };
  });
}
