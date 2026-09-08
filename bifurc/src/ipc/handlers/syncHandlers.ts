import { ipcMain, dialog, BrowserWindow } from "electron";
import * as fs from "fs";
import {
  setRemote, disconnect, syncPush, syncPull, getSyncState, setAutoSync,
  getSyncConfig, getRemoteHead,
} from "@/sync/syncManager";
import { startAutoSync, stopAutoSync, updateLastKnownHead } from "@/sync/autoSync";
import { getFileDiff, discardChanges, syncChanges, getFileHistory } from "@/sync/gitOps";
import { getWorkspaceSyncStatus, invalidateCache } from "@/sync/statusTracker";
import { loadConfig, saveConfig } from "@/store/config";
import { reloadConfig } from "@/proxy/server";
import {
  queryLog, getEntityAtCommit, getCommitChangedFiles, QueryLogOptions, AuditEntity,
} from "@/store/gitStore";

function broadcastEntityStatus(wsId: string): void {
  getWorkspaceSyncStatus(wsId).then((status) => {
    BrowserWindow.getAllWindows().forEach((w) => {
      if (!w.isDestroyed()) w.webContents.send("sync:entityStatus", { wsId, status });
    });
  }).catch(() => { });
}

export function registerSyncHandlers() {
  // ── Sync ──────────────────────────────────────────────────────────────────
  ipcMain.handle("sync:setRemote", async (_e, wsId: string, remote: string, branch: string) => {
    const result = await setRemote(wsId, remote, branch);
    if (result.ok) reloadConfig();
    return result;
  });

  ipcMain.handle("sync:disconnect", (_e, wsId: string) => disconnect(wsId));

  ipcMain.handle("sync:push", (_e, wsId: string) => syncPush(wsId));

  ipcMain.handle("sync:pull", async (_e, wsId: string) => {
    const result = await syncPull(wsId);
    if (result.ok && result.updated) {
      reloadConfig();
      invalidateCache(wsId);
      broadcastEntityStatus(wsId);
    }
    return result;
  });

  ipcMain.handle("sync:getState", (_e, wsId: string) => getSyncState(wsId));

  ipcMain.handle("sync:setAutoSync", async (_e, wsId: string, enabled: boolean) => {
    const result = await setAutoSync(wsId, enabled);
    if (enabled) startAutoSync(wsId);
    else stopAutoSync(wsId);
    return result;
  });

  ipcMain.handle("sync:getEntityStatus", (_e, wsId: string) => getWorkspaceSyncStatus(wsId));

  // ── Generic Git Operations (file-centric) ───────────────────────────────────

  ipcMain.handle("git:diff", (_e, wsId: string, relPath: string) => {
    return getFileDiff(wsId, relPath);
  });

  ipcMain.handle("git:discard", async (_e, wsId: string, relPath: string) => {
    const result = await discardChanges(wsId, relPath);
    broadcastEntityStatus(wsId);
    reloadConfig();
    return result;
  });

  ipcMain.handle("git:sync", async (_e, wsId: string, paths: string[], message?: string) => {
    const result = await syncChanges(wsId, paths, message);
    broadcastEntityStatus(wsId);
    try {
      const sha = await getRemoteHead(wsId);
      if (sha) updateLastKnownHead(wsId, sha);
    } catch { }
    return result;
  });

  ipcMain.handle("git:history", (_e, wsId: string, relPath: string, opts?: { limit?: number; offset?: number }) => {
    return getFileHistory(wsId, relPath, opts);
  });

  // Backward-compatible entity / folder handlers delegating to generic git ops
  ipcMain.handle("entity:publish", async (_e, wsId: string, paths: string[], message?: string) => {
    const result = await syncChanges(wsId, paths, message);
    broadcastEntityStatus(wsId);
    try {
      const sha = await getRemoteHead(wsId);
      if (sha) updateLastKnownHead(wsId, sha);
    } catch { }
    return result;
  });

  ipcMain.handle("folder:publish", async (_e, wsId: string, kind: string, folderName: string | null) => {
    const sanitizeDirName = (name: string) => name.replace(/[<>:"/\\|?*]+/g, "-");
    const folderPath = folderName ? `${kind}/${sanitizeDirName(folderName)}/` : `${kind}/`;
    const result = await syncChanges(wsId, [folderPath]);
    broadcastEntityStatus(wsId);
    try {
      const sha = await getRemoteHead(wsId);
      if (sha) updateLastKnownHead(wsId, sha);
    } catch { }
    return result;
  });

  ipcMain.handle("entity:restore", async (_e, wsId: string, relPath: string) => {
    const result = await discardChanges(wsId, relPath);
    broadcastEntityStatus(wsId);
    reloadConfig();
    return result;
  });

  // ── Audit Log ──────────────────────────────────────────────────────────────

  ipcMain.handle("audit:list", async (_e, opts: Omit<QueryLogOptions, "workspaceId"> & { workspaceId?: string } = {}) => {
    if (!opts.workspaceId) {
      const cfg = loadConfig();
      opts = { ...opts, workspaceId: cfg.activeWorkspaceId };
    }
    return queryLog(opts as QueryLogOptions);
  });

  ipcMain.handle("audit:diff", async (_e, commitHash: string, _entity: AuditEntity, entityId: string, workspaceId: string) => {
    // Resolve the actual file path from the commit's changed files list
    const changed = await getCommitChangedFiles(commitHash, workspaceId);
    const relPath = changed.find((f) => f.includes(entityId)) ?? changed[0] ?? "";
    if (!relPath) return { before: null, after: null };
    const after = await getEntityAtCommit(commitHash, workspaceId, relPath);
    const before = await getEntityAtCommit(`${commitHash}~1`, workspaceId, relPath);
    return { before, after };
  });

  ipcMain.handle("history:list", async (_e, opts: { workspaceId?: string; filePath: string; limit?: number; offset?: number }) => {
    const wsId = opts.workspaceId ?? loadConfig().activeWorkspaceId;
    return queryLog({ workspaceId: wsId, filePath: opts.filePath, limit: opts.limit ?? 100, offset: opts.offset ?? 0 });
  });

  ipcMain.handle("history:diff", async (_e, commitHash: string, filePath: string, workspaceId: string) => {
    const after = await getEntityAtCommit(commitHash, workspaceId, filePath);
    const before = await getEntityAtCommit(`${commitHash}~1`, workspaceId, filePath);
    return { before, after };
  });

  ipcMain.handle("audit:export", async (_e, format: "json" | "csv") => {
    const cfg = loadConfig();
    const { entries } = await queryLog({ workspaceId: cfg.activeWorkspaceId, limit: 0 });
    const { filePath, canceled } = await dialog.showSaveDialog({
      title: "Export Audit Log",
      defaultPath: `audit-log.${format}`,
      filters: [{ name: format.toUpperCase(), extensions: [format] }],
    });
    if (canceled || !filePath) return { ok: false };

    if (format === "json") {
      fs.writeFileSync(filePath, JSON.stringify(entries, null, 2), "utf-8");
    } else {
      const header = "commitHash,ts,action,entity,entityId,entityName,workspaceId,actor";
      const rows = entries.map((e) =>
        [
          e.commitHash, e.ts, e.action, e.entity, e.entityId,
          `"${e.entityName.replace(/"/g, '""')}"`,
          e.workspaceId, e.actor,
        ].join(",")
      );
      fs.writeFileSync(filePath, [header, ...rows].join("\n"), "utf-8");
    }
    return { ok: true };
  });
}
