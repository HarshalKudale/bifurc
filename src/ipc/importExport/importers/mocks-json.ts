import * as fs from "fs";
import {
  loadConfig, saveConfig, generateId, MockRule, Folder,
} from "@/store/config";
import {
  writeEntity, upsertNameEntry, readEnabledSet, writeEnabledSet, bootstrapEnabledSet, readAllEntities,
} from "@/store/workspaceFs";
import { reloadConfig } from "@/proxy/server";
import type { PreflightResult, ImportResult, CollisionStrategy } from "@/ipc/importExport/types";

function syncEnabled(wsId: string, id: string, enabled: boolean): void {
  let set = readEnabledSet(wsId, "mocks");
  if (!set) set = bootstrapEnabledSet(wsId, "mocks");
  if (enabled) set.add(id); else set.delete(id);
  writeEnabledSet(wsId, "mocks", set);
}

function parse(filePath: string): { mocks: MockRule[]; folders: Folder[] } {
  const data = JSON.parse(fs.readFileSync(filePath, "utf-8"));
  if (data?.schema !== "lp-mocks-v1" || !Array.isArray(data.mocks)) {
    throw new Error("Not a valid lp-mocks-v1 file");
  }
  return { mocks: data.mocks as MockRule[], folders: (data.folders ?? []) as Folder[] };
}

export function preflight(wsId: string, filePath: string): PreflightResult {
  try {
    const { mocks } = parse(filePath);
    const existingIds = new Set(readAllEntities<MockRule>(wsId, "mocks").map((m) => m.id));
    const collisionIds = mocks.filter((m) => existingIds.has(m.id)).map((m) => m.id);
    return { ok: true, filePath, itemCount: mocks.length, collisionIds };
  } catch (err) {
    return { ok: false, error: String(err) };
  }
}

export async function run(
  wsId: string,
  filePath: string,
  strategy: CollisionStrategy,
): Promise<ImportResult> {
  try {
    const { mocks, folders } = parse(filePath);
    const cfg = loadConfig();
    const existingIds = new Set(cfg.mocks.filter((m) => m.workspaceId === wsId).map((m) => m.id));

    const folderIdMap = new Map<string, string>();
    for (const f of folders) {
      const newId = strategy === "new" ? generateId() : f.id;
      folderIdMap.set(f.id, newId);
      const exists = cfg.mockFolders.some((x) => x.id === newId);
      if (!exists) {
        cfg.mockFolders = [...cfg.mockFolders, { ...f, id: newId, workspaceId: wsId }];
      }
    }

    let imported = 0;
    let skipped = 0;

    for (const m of mocks) {
      const newId = strategy === "new" ? generateId() : m.id;
      const isCollision = existingIds.has(m.id);

      if (isCollision && strategy === "keep") { skipped++; continue; }

      const folderId = m.folderId ? (folderIdMap.get(m.folderId) ?? null) : null;
      const folderName = folderId ? (cfg.mockFolders.find((f) => f.id === folderId)?.name ?? null) : null;
      const newMock: MockRule = { ...m, id: newId, folderId, workspaceId: wsId };

      const existIdx = cfg.mocks.findIndex((x) => x.id === newId);
      if (existIdx !== -1) cfg.mocks[existIdx] = newMock;
      else cfg.mocks = [...cfg.mocks, newMock];

      writeEntity(wsId, "mocks", newId, newMock, folderName);
      syncEnabled(wsId, newId, newMock.enabled);
      upsertNameEntry(wsId, "mocks", newId, { name: newMock.name, method: newMock.method, url: newMock.urlPattern });
      imported++;
    }

    saveConfig(cfg);
    reloadConfig();
    return { ok: true, imported, skipped };
  } catch (err) {
    return { ok: false, error: String(err) };
  }
}
