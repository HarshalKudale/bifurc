import { ipcMain, BrowserWindow } from "electron";
import { loadConfig, saveConfig, generateId, AppConfig } from "@/store/config";
import { 
  writeEntity, deleteEntityFile, writeFlatEntity, deleteFlatEntityFile,
  upsertNameEntry, removeNameEntry, 
  addPendingDeletion, findEntityRelPath,
  readEnabledSet, writeEnabledSet, bootstrapEnabledSet
} from "@/store/workspaceFs";
import { getGit } from "@/store/gitStore";
import { reloadConfig } from "@/proxy/server";
import { invalidateCache, getWorkspaceSyncStatus } from "@/sync/statusTracker";

export async function isGitTracked(wsId: string, relPath: string): Promise<boolean> {
  try {
    const result = await getGit(wsId).raw(["ls-files", "--error-unmatch", "--", relPath]);
    return result.trim().length > 0;
  } catch {
    return false;
  }
}

export function syncEnabledSet(wsId: string, kind: string, id: string, enabled: boolean): void {
  let set = readEnabledSet(wsId, kind);
  if (!set) set = bootstrapEnabledSet(wsId, kind);
  if (enabled) set.add(id); else set.delete(id);
  writeEnabledSet(wsId, kind, set);
}

export function notifyRendererRefresh(): void {
  const windows = BrowserWindow.getAllWindows();
  for (const w of windows) {
    if (!w.isDestroyed()) w.webContents.send("companion:refresh");
  }
}

export function broadcastEntityStatus(wsId: string): void {
  getWorkspaceSyncStatus(wsId).then((status) => {
    BrowserWindow.getAllWindows().forEach((w) => {
      if (!w.isDestroyed()) w.webContents.send("sync:entityStatus", { wsId, status });
    });
  }).catch(() => { });
}

export interface CrudFactoryOpts<T> {
  kind: string;
  configKey: keyof AppConfig;
  folderConfigKey?: keyof AppConfig;
  ipcPrefix?: string;
  ipcAdd?: string;
  ipcUpdate?: string;
  ipcDelete?: string;
  isFlat?: boolean; // If true, uses writeFlatEntity/deleteFlatEntityFile instead of writeEntity/deleteEntityFile
  hasEnabledState?: boolean;
  validate?: (entity: Partial<T>) => void;
  onAddConflict?: (cfg: AppConfig, newEntity: T) => void;
  getNameEntry?: (entity: T) => any; // Return object for upsertNameEntry/addPendingDeletion
}

export function registerEntityCrudHandlers<T extends { id: string; workspaceId?: string; folderId?: string | null; enabled?: boolean; createdAt?: number }>(opts: CrudFactoryOpts<T>) {
  ipcMain.handle(opts.ipcAdd || `${opts.ipcPrefix}:add`, async (_e, entity: Omit<T, "id" | "createdAt">) => {
    if (opts.validate) opts.validate(entity as Partial<T>);

    const cfg = loadConfig() as any;
    const wsId = (entity as any).workspaceId ?? cfg.activeWorkspaceId;
    
    const newEntity = { ...entity, id: generateId(), createdAt: Date.now(), workspaceId: wsId } as unknown as T;
    
    if (opts.hasEnabledState && newEntity.enabled === undefined) {
      newEntity.enabled = true;
    }

    cfg[opts.configKey] = cfg[opts.configKey] ?? [];
    
    if (opts.onAddConflict) {
      opts.onAddConflict(cfg, newEntity);
    }

    if (opts.ipcPrefix === "rule" || opts.ipcPrefix === "mock") {
      cfg[opts.configKey].unshift(newEntity);
    } else {
      cfg[opts.configKey].push(newEntity);
    }
    
    saveConfig(cfg);
    reloadConfig();

    if (opts.isFlat) {
      writeFlatEntity(wsId, opts.kind, newEntity.id, newEntity);
    } else {
      let folderName = null;
      if (opts.folderConfigKey && newEntity.folderId) {
        const folders = cfg[opts.folderConfigKey] ?? [];
        folderName = folders.find((f: any) => f.id === newEntity.folderId)?.name ?? null;
      }
      writeEntity(wsId, opts.kind, newEntity.id, newEntity, folderName);
    }

    if (opts.hasEnabledState) {
      syncEnabledSet(wsId, opts.kind, newEntity.id, newEntity.enabled ?? false);
    }

    if (opts.getNameEntry) {
      upsertNameEntry(wsId, opts.kind, newEntity.id, opts.getNameEntry(newEntity));
    }

    broadcastEntityStatus(wsId);
    
    if (["mock", "request"].includes(opts.ipcPrefix ?? "")) {
      notifyRendererRefresh();
    }
    
    return newEntity;
  });

  ipcMain.handle(opts.ipcUpdate || `${opts.ipcPrefix}:update`, async (_e, entity: T) => {
    const cfg = loadConfig() as any;
    const wsId = entity.workspaceId ?? cfg.activeWorkspaceId;
    
    cfg[opts.configKey] = cfg[opts.configKey] ?? [];
    const idx = cfg[opts.configKey].findIndex((e: any) => e.id === entity.id);
    if (idx !== -1) cfg[opts.configKey][idx] = entity;
    
    saveConfig(cfg);
    reloadConfig();

    if (opts.isFlat) {
      writeFlatEntity(wsId, opts.kind, entity.id, entity);
    } else {
      let folderName = null;
      if (opts.folderConfigKey && entity.folderId) {
        const folders = cfg[opts.folderConfigKey] ?? [];
        folderName = folders.find((f: any) => f.id === entity.folderId)?.name ?? null;
      }
      writeEntity(wsId, opts.kind, entity.id, entity, folderName);
    }

    if (opts.getNameEntry) {
      upsertNameEntry(wsId, opts.kind, entity.id, opts.getNameEntry(entity));
    }

    broadcastEntityStatus(wsId);
    return { ok: true };
  });

  ipcMain.handle(opts.ipcDelete || `${opts.ipcPrefix}:delete`, async (_e, id: string) => {
    const cfg = loadConfig() as any;
    const wsId = cfg.activeWorkspaceId;
    
    const arr = cfg[opts.configKey] ?? [];
    const entity = arr.find((e: any) => e.id === id);
    
    if (entity) {
      cfg[opts.configKey] = arr.filter((e: any) => e.id !== id);
      
      if (opts.ipcPrefix === "mapping") {
        cfg.proxyRules = (cfg.proxyRules ?? []).filter((r: any) => (r.targetType ?? "mapping") !== "mapping" || r.targetMappingId !== id);
      }

      saveConfig(cfg);
      reloadConfig();

      if (opts.isFlat) {
        deleteFlatEntityFile(wsId, opts.kind, id);
      } else {
        const relPath = findEntityRelPath(wsId, opts.kind, id);
        const tracked = relPath ? await isGitTracked(wsId, relPath) : false;
        
        deleteEntityFile(wsId, opts.kind, id);
        
        if (tracked && opts.getNameEntry) {
          const entry = opts.getNameEntry(entity);
          addPendingDeletion(wsId, opts.kind, { id, folderId: entity.folderId ?? null, ...entry });
        } else if (opts.getNameEntry) {
          removeNameEntry(wsId, opts.kind, id);
        }
      }

      if (opts.hasEnabledState) {
        syncEnabledSet(wsId, opts.kind, id, false);
      }

      invalidateCache(wsId);
      broadcastEntityStatus(wsId);
    }
    
    return { ok: true };
  });
}
