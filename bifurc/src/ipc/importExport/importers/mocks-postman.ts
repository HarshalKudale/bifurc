import * as fs from "fs";
import {
  loadConfig, saveConfig, generateId, MockRule, Folder,
} from "@/store/config";
import {
  writeEntity, upsertNameEntry, readEnabledSet, writeEnabledSet, bootstrapEnabledSet,
} from "@/store/workspaceFs";
import { reloadConfig } from "@/proxy/server";
import { parsePostmanMocks } from "@/ipc/importExport/formats/postman";
import type { PreflightResult, ImportResult, CollisionStrategy } from "@/ipc/importExport/types";

function syncEnabled(wsId: string, id: string, enabled: boolean): void {
  let set = readEnabledSet(wsId, "mocks");
  if (!set) set = bootstrapEnabledSet(wsId, "mocks");
  if (enabled) set.add(id); else set.delete(id);
  writeEnabledSet(wsId, "mocks", set);
}

export function preflight(wsId: string, filePath: string): PreflightResult {
  try {
    const raw = fs.readFileSync(filePath, "utf-8");
    const parsed = parsePostmanMocks(raw);
    return { ok: true, filePath, itemCount: parsed.mocks.length, collisionIds: [] };
  } catch (err) {
    return { ok: false, error: String(err) };
  }
}

export async function run(
  wsId: string,
  filePath: string,
  _strategy: CollisionStrategy,
): Promise<ImportResult> {
  try {
    const raw = fs.readFileSync(filePath, "utf-8");
    const parsed = parsePostmanMocks(raw);
    const cfg = loadConfig();

    const folderNameToId = new Map<string, string>();

    for (const f of parsed.folders) {
      const id = generateId();
      const parentId = f.parentId ? (folderNameToId.get(f.parentId) ?? null) : null;
      const newFolder: Folder = { id, name: f.name, parentId, createdAt: Date.now(), workspaceId: wsId };
      cfg.mockFolders = [...(cfg.mockFolders ?? []), newFolder];
      folderNameToId.set(f.name, id);
    }

    let imported = 0;
    for (const m of parsed.mocks) {
      const id = generateId();
      const folderId = m.folderId ? (folderNameToId.get(m.folderId) ?? null) : null;
      const folderName = folderId ? (cfg.mockFolders.find((f) => f.id === folderId)?.name ?? null) : null;
      const newMock: MockRule = {
        id,
        name: m.name,
        method: m.method,
        urlPattern: m.urlPattern,
        useRegex: m.useRegex,
        enabled: m.enabled,
        capturedHeaders: m.capturedHeaders,
        capturedBody: m.capturedBody,
        responseStatus: m.responseStatus,
        responseHeaders: m.responseHeaders,
        responseBody: m.responseBody,
        createdAt: Date.now(),
        folderId,
        workspaceId: wsId,
      };
      cfg.mocks = [...(cfg.mocks ?? []), newMock];
      writeEntity(wsId, "mocks", id, newMock, folderName);
      syncEnabled(wsId, id, newMock.enabled);
      upsertNameEntry(wsId, "mocks", id, { name: newMock.name, method: newMock.method, url: newMock.urlPattern });
      imported++;
    }

    saveConfig(cfg);
    reloadConfig();

    return { ok: true, imported };
  } catch (err) {
    return { ok: false, error: String(err) };
  }
}
