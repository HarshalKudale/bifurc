import { ipcMain } from "electron";
import * as fs from "fs";
import * as path from "path";
import type {
  FolderAddParams, FolderRenameParams, FolderMoveParams, FolderDeleteParams, FolderKindValue,
} from "@bifurc/protocol";
import { Folder } from "@bifurc/engine/store/config";
import { loadConfig, saveConfig } from "@bifurc/engine/store/config";
import { generateId } from "@bifurc/engine/store/config";
import { readIndex, writeIndex, wsDir as workspaceDir, sanitizeDirName, deleteEntityDir } from "@bifurc/engine/store/workspaceFs";
import { bus } from "@bifurc/engine/eventBus";
import { commandRegistry } from "@/commands/registry";

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

// P2 work item 7 — folder.* is a clean 1:1 with its legacy channel (`FolderKind` in
// `packages/protocol/src/commands/folder.ts` is the same 11-value enum as the local `FolderKind`
// above), so all four commands convert without any schema gap.
const ctx = { bus };

commandRegistry.register("folder.add", async (params: FolderAddParams) => {
  const kind = params.kind as FolderKind;
  const cfg = loadConfig() as any;
  const wsId = params.workspaceId ?? cfg.activeWorkspaceId;
  const newFolder: Folder = { name: params.name, parentId: params.parentId, id: generateId(), createdAt: Date.now(), workspaceId: wsId };

  const conf = FOLDER_CONFIG[kind];
  cfg[conf.configKey] = cfg[conf.configKey] ?? [];
  cfg[conf.configKey].push(newFolder);
  saveConfig(cfg);

  const folderDir = path.join(workspaceDir(wsId), conf.fsKind, sanitizeDirName(newFolder.name));
  fs.mkdirSync(folderDir, { recursive: true });

  const idx = readIndex(wsId, conf.fsKind as any);
  idx.folders.push(newFolder);
  writeIndex(wsId, conf.fsKind as any, idx);
  bus.emitTyped("entity.changed", { wsId, kind, id: newFolder.id, action: "created" });
  return newFolder;
});

commandRegistry.register("folder.rename", ({ kind, id, name }: FolderRenameParams) => {
  const cfg = loadConfig() as any;
  const conf = FOLDER_CONFIG[kind as FolderKind];
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

commandRegistry.register("folder.move", ({ kind, id, parentId }: FolderMoveParams) => {
  const cfg = loadConfig() as any;
  const conf = FOLDER_CONFIG[kind as FolderKind];
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

commandRegistry.register("folder.delete", async ({ kind, id }: FolderDeleteParams) => {
  const cfg = loadConfig() as any;
  const conf = FOLDER_CONFIG[kind as FolderKind];

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

export function registerFolderHandlers() {
  ipcMain.handle("folder:add", (_e, kind: FolderKind, folder: Omit<Folder, "id" | "createdAt">) =>
    commandRegistry.invoke("folder.add", { kind: kind as FolderKindValue, ...folder }, ctx));

  ipcMain.handle("folder:rename", (_e, kind: FolderKind, id: string, name: string) =>
    commandRegistry.invoke("folder.rename", { kind: kind as FolderKindValue, id, name }, ctx));

  ipcMain.handle("folder:move", (_e, kind: FolderKind, id: string, parentId: string | null) =>
    commandRegistry.invoke("folder.move", { kind: kind as FolderKindValue, id, parentId }, ctx));

  ipcMain.handle("folder:delete", (_e, kind: FolderKind, id: string) =>
    commandRegistry.invoke("folder.delete", { kind: kind as FolderKindValue, id }, ctx));
}

