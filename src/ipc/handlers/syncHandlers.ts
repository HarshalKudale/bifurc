import { ipcMain, dialog } from "electron";
import * as fs from "fs";
import type {
  SyncSetRemoteParams, SyncDisconnectParams, SyncPushParams, SyncPullParams,
  SyncGetStateParams, SyncSetAutoSyncParams, SyncGetEntityStatusParams,
  GitDiffParams, GitDiscardParams, GitSyncParams, GitHistoryParams,
  EntityPublishParams, EntityRestoreParams, FolderPublishParams,
  AuditDiffParams, HistoryListParams, HistoryDiffParams, AuditExportResult,
} from "@bifurc/protocol";
import {
  setRemote, disconnect, syncPush, syncPull, getSyncState, setAutoSync,
  getSyncConfig, getRemoteHead,
} from "@bifurc/engine/sync/syncManager";
import { startAutoSync, stopAutoSync, updateLastKnownHead } from "@bifurc/engine/sync/autoSync";
import { getFileDiff, discardChanges, syncChanges, getFileHistory } from "@bifurc/engine/sync/gitOps";
import { getWorkspaceSyncStatus, invalidateCache } from "@bifurc/engine/sync/statusTracker";
import { loadConfig, saveConfig } from "@bifurc/engine/store/config";
import { reloadConfig } from "@bifurc/engine/proxy/server";
import {
  queryLog, getEntityAtCommit, getCommitChangedFiles, QueryLogOptions, AuditEntity,
} from "@bifurc/engine/store/gitStore";
import { bus, emitEntityStatus } from "@bifurc/engine/eventBus";
import { commandRegistry } from "@bifurc/engine/commands/registry";
import { call, writeArtifact } from "@/ipc/fileOpsClient";

// P2 work item 7 — these are the second batch of commands wired through the CommandRegistry,
// following coreHandlers.ts's proof of concept. Every command here is already a clean 1:1 with
// its legacy channel (no entityCrudFactory involvement), which is exactly the "next slice"
// `plan/03-phase-2-engine-extraction.md`'s work item 7 status note calls for. `audit:export` was
// SPLIT (see `packages/protocol/src/commands/audit.ts`) and was **converted in P3 work item 3**:
// the engine now renders `{format}` -> artifact and this file owns the Electron save dialog and the
// file write, per `File_Ops_Protocol.md` §4.
const ctx = { bus };

commandRegistry.register("sync.setRemote", async ({ workspaceId, remote, branch }: SyncSetRemoteParams) => {
  const result = await setRemote(workspaceId, remote, branch);
  if (result.ok) reloadConfig();
  return result;
});

commandRegistry.register("sync.disconnect", ({ workspaceId }: SyncDisconnectParams) => disconnect(workspaceId));

commandRegistry.register("sync.push", ({ workspaceId }: SyncPushParams) => syncPush(workspaceId));

commandRegistry.register("sync.pull", async ({ workspaceId }: SyncPullParams) => {
  const result = await syncPull(workspaceId);
  if (result.ok && result.updated) {
    reloadConfig();
    invalidateCache(workspaceId);
    emitEntityStatus(workspaceId);
  }
  return result;
});

commandRegistry.register("sync.getState", ({ workspaceId }: SyncGetStateParams) => getSyncState(workspaceId));

commandRegistry.register("sync.setAutoSync", async ({ workspaceId, enabled }: SyncSetAutoSyncParams) => {
  const result = await setAutoSync(workspaceId, enabled);
  if (enabled) startAutoSync(workspaceId);
  else stopAutoSync(workspaceId);
  return result;
});

commandRegistry.register("sync.getEntityStatus", ({ workspaceId }: SyncGetEntityStatusParams) => getWorkspaceSyncStatus(workspaceId));

commandRegistry.register("git.diff", ({ workspaceId, relPath }: GitDiffParams) => getFileDiff(workspaceId, relPath));

commandRegistry.register("git.discard", async ({ workspaceId, relPath }: GitDiscardParams) => {
  const result = await discardChanges(workspaceId, relPath);
  emitEntityStatus(workspaceId);
  reloadConfig();
  return result;
});

commandRegistry.register("git.sync", async ({ workspaceId, paths, message }: GitSyncParams) => {
  const result = await syncChanges(workspaceId, paths, message);
  emitEntityStatus(workspaceId);
  try {
    const sha = await getRemoteHead(workspaceId);
    if (sha) updateLastKnownHead(workspaceId, sha);
  } catch { }
  return result;
});

commandRegistry.register("git.history", ({ workspaceId, relPath, limit, offset }: GitHistoryParams) =>
  getFileHistory(workspaceId, relPath, { limit, offset }));

commandRegistry.register("entity.publish", async ({ workspaceId, paths, message }: EntityPublishParams) => {
  const result = await syncChanges(workspaceId, paths, message);
  emitEntityStatus(workspaceId);
  try {
    const sha = await getRemoteHead(workspaceId);
    if (sha) updateLastKnownHead(workspaceId, sha);
  } catch { }
  return result;
});

commandRegistry.register("folder.publish", async ({ workspaceId, kind, folderName }: FolderPublishParams) => {
  const sanitizeDirName = (name: string) => name.replace(/[<>:"/\\|?*]+/g, "-");
  const folderPath = folderName ? `${kind}/${sanitizeDirName(folderName)}/` : `${kind}/`;
  const result = await syncChanges(workspaceId, [folderPath]);
  emitEntityStatus(workspaceId);
  try {
    const sha = await getRemoteHead(workspaceId);
    if (sha) updateLastKnownHead(workspaceId, sha);
  } catch { }
  return result;
});

commandRegistry.register("entity.restore", async ({ workspaceId, relPath }: EntityRestoreParams) => {
  const result = await discardChanges(workspaceId, relPath);
  emitEntityStatus(workspaceId);
  reloadConfig();
  return result;
});

// `audit.list` is deliberately NOT wired through the registry here: `AuditListParams` is
// `.strict()` in the frozen `packages/protocol` package and does not declare `fromTs`/`toTs`,
// but `AuditLogPanel.tsx` actively sends both when a date-range filter is set
// (`renderer/types/ipc.ts`'s `AuditListOptions` has them; `queryLog`'s `QueryLogOptions` accepts
// them). Routing this through `commandRegistry.invoke()` as written would make every
// date-filtered audit-log query throw a validation error — a real regression, not a
// registration-only change. This is a genuine gap in the frozen v1 protocol schema, worth a
// `protocol-changes.md` entry before it is converted, not something to route around silently.

commandRegistry.register("audit.diff", async ({ commitHash, entityId, workspaceId }: AuditDiffParams) => {
  // Resolve the actual file path from the commit's changed files list
  const changed = await getCommitChangedFiles(commitHash, workspaceId);
  const relPath = changed.find((f) => f.includes(entityId)) ?? changed[0] ?? "";
  if (!relPath) return { before: null, after: null };
  const after = await getEntityAtCommit(commitHash, workspaceId, relPath);
  const before = await getEntityAtCommit(`${commitHash}~1`, workspaceId, relPath);
  return { before, after };
});

commandRegistry.register("history.list", async (opts: HistoryListParams) => {
  const wsId = opts.workspaceId ?? loadConfig().activeWorkspaceId;
  const normalized = opts.filePath.replace(/\\/g, "/");
  return queryLog({ workspaceId: wsId, filePath: normalized, limit: opts.limit ?? 100, offset: opts.offset ?? 0 });
});

commandRegistry.register("history.diff", async ({ commitHash, filePath, workspaceId }: HistoryDiffParams) => {
  const normalized = filePath.replace(/\\/g, "/");
  const after = await getEntityAtCommit(commitHash, workspaceId, normalized);
  const before = await getEntityAtCommit(`${commitHash}~1`, workspaceId, normalized);
  return { before, after };
});

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

