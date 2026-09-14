import { ipcMain, BrowserWindow } from "electron";
import * as fs from "fs";
import * as path from "path";
import { Folder } from "@/store/config";
import { loadConfig, saveConfig } from "@/store/config";
import { generateId } from "@/store/config";
import { readIndex, writeIndex, wsDir as workspaceDir, sanitizeDirName, deleteEntityDir } from "@/store/workspaceFs";

function notifyRendererRefresh(): void {
  const windows = BrowserWindow.getAllWindows();
  for (const w of windows) {
    if (!w.isDestroyed()) w.webContents.send("companion:refresh");
  }
}

type FolderKind = "mock" | "request" | "ws" | "webhook" | "rule" | "graphqlRequest" | "graphqlMock" | "grpcRequest" | "grpcMock" | "soapRequest" | "soapMock";

const FOLDER_CONFIG: Record<FolderKind, { configKey: string; entityConfigKey?: string; fsKind: string }> = {
  mock: { configKey: "mockFolders", entityConfigKey: "mocks", fsKind: "mocks" },
  ws: { configKey: "wsFolders", entityConfigKey: "wsConnections", fsKind: "sockets" },
  webhook: { configKey: "webhookFolders", entityConfigKey: "webhooks", fsKind: "webhooks" },
  rule: { configKey: "ruleFolders", entityConfigKey: "proxyRules", fsKind: "rules" },
  graphqlRequest: { configKey: "graphqlRequestFolders", entityConfigKey: "graphqlRequests", fsKind: "graphqlRequests" },
  graphqlMock: { configKey: "graphqlMockFolders", entityConfigKey: "graphqlMocks", fsKind: "graphqlMocks" },
  grpcRequest: { configKey: "grpcRequestFolders", entityConfigKey: "grpcRequests", fsKind: "grpcRequests" },
  grpcMock: { configKey: "grpcMockFolders", entityConfigKey: "grpcMocks", fsKind: "grpcMocks" },
  soapRequest: { configKey: "soapRequestFolders", entityConfigKey: "soapRequests", fsKind: "soapRequests" },
  soapMock: { configKey: "soapMockFolders", entityConfigKey: "soapMocks", fsKind: "soapMocks" },
  request: { configKey: "requestFolders", entityConfigKey: "requests", fsKind: "requests" },
};

export function registerFolderHandlers() {
  ipcMain.handle("folder:add", async (_e, kind: FolderKind, folder: Omit<Folder, "id" | "createdAt">) => {
    const cfg = loadConfig() as any;
    const wsId = folder.workspaceId ?? cfg.activeWorkspaceId;
    const newFolder: Folder = { ...folder, id: generateId(), createdAt: Date.now(), workspaceId: wsId };
    
    const conf = FOLDER_CONFIG[kind];
    cfg[conf.configKey] = cfg[conf.configKey] ?? [];
    cfg[conf.configKey].push(newFolder);
    saveConfig(cfg);

    const folderDir = path.join(workspaceDir(wsId), conf.fsKind, sanitizeDirName(newFolder.name));
    fs.mkdirSync(folderDir, { recursive: true });

    const idx = readIndex(wsId, conf.fsKind as any);
    idx.folders.push(newFolder);
    writeIndex(wsId, conf.fsKind as any, idx);
    notifyRendererRefresh();
    return newFolder;
  });

  ipcMain.handle("folder:rename", (_e, kind: FolderKind, id: string, name: string) => {
    const cfg = loadConfig() as any;
    const conf = FOLDER_CONFIG[kind];
    const arr: Folder[] = cfg[conf.configKey] ?? [];
    
    const f = arr.find((x: Folder) => x.id === id);
    const oldName = f?.name;
    if (f) f.name = name;
    cfg[conf.configKey] = arr;
    saveConfig(cfg);

    if (f && oldName) {
      const base = path.join(workspaceDir(f.workspaceId), conf.fsKind);
      const oldDir = path.join(base, sanitizeDirName(oldName));
      const newDir = path.join(base, sanitizeDirName(name));
      if (fs.existsSync(oldDir) && oldDir !== newDir) {
        try { fs.renameSync(oldDir, newDir); }
        catch (renameErr) {
          try {
            fs.cpSync(oldDir, newDir, { recursive: true });
            fs.rmSync(oldDir, { recursive: true, force: true });
          } catch (copyErr) { console.warn(`[folder:rename] Error:`, (copyErr as Error).message); }
        }
      }
      const idx = readIndex(f.workspaceId, conf.fsKind as any);
      const fi = idx.folders.find((x: Folder) => x.id === id);
      if (fi) fi.name = name;
      writeIndex(f.workspaceId, conf.fsKind as any, idx);
    }
    return { ok: true };
  });

  ipcMain.handle("folder:move", (_e, kind: FolderKind, id: string, parentId: string | null) => {
    const cfg = loadConfig() as any;
    const conf = FOLDER_CONFIG[kind];
    const arr: Folder[] = cfg[conf.configKey] ?? [];
    
    const f = arr.find((x: Folder) => x.id === id);
    if (f) f.parentId = parentId;
    cfg[conf.configKey] = arr;

    if (f) {
      const idx = readIndex(f.workspaceId, conf.fsKind as any);
      const fi = idx.folders.find((x: Folder) => x.id === id);
      if (fi) fi.parentId = parentId;
      writeIndex(f.workspaceId, conf.fsKind as any, idx);
    }
    saveConfig(cfg);
    return { ok: true };
  });

  ipcMain.handle("folder:delete", async (_e, kind: FolderKind, id: string) => {
    const cfg = loadConfig() as any;
    const conf = FOLDER_CONFIG[kind];
    
    const arr: Folder[] = cfg[conf.configKey] ?? [];
    const folder = arr.find((f: Folder) => f.id === id);
    
    let affectedEntityIds: string[] = [];
    if (conf.entityConfigKey) {
      const entities: any[] = cfg[conf.entityConfigKey] ?? [];
      affectedEntityIds = entities.filter(e => e.folderId === id).map(e => e.id);
      cfg[conf.entityConfigKey] = entities.filter(e => e.folderId !== id);
    }
    cfg[conf.configKey] = arr.filter((f: Folder) => f.id !== id);
    saveConfig(cfg);

    if (folder) {
      const wsId = folder.workspaceId;
      deleteEntityDir(wsId, conf.fsKind as any, folder.name);
      
      if (kind === "request") {
        const runnerDir = path.join(workspaceDir(wsId), "requests", ".runs", id);
        if (fs.existsSync(runnerDir)) fs.rmSync(runnerDir, { recursive: true, force: true });
      }

      const idx = readIndex(wsId, conf.fsKind as any);
      idx.folders = idx.folders.filter((f: any) => f.id !== id);
      const deletedSet = new Set(affectedEntityIds);
      idx.order = (idx.order ?? []).filter((eid: string) => !deletedSet.has(eid));
      writeIndex(wsId, conf.fsKind as any, idx);
    }
    return { ok: true };
  });
}
