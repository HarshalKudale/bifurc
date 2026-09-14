import * as fs from "fs";
import {
  loadConfig, saveConfig, generateId, SavedRequest, Folder,
} from "@/store/config";
import {
  writeEntity, upsertNameEntry, readAllEntities,
} from "@/store/workspaceFs";
import { reloadConfig } from "@/proxy/server";
import { parsePostmanRequests } from "@/ipc/importExport/formats/postman";
import type { PreflightResult, ImportResult, CollisionStrategy } from "@/ipc/importExport/types";

export function preflight(wsId: string, filePath: string): PreflightResult {
  try {
    const raw = fs.readFileSync(filePath, "utf-8");
    const parsed = parsePostmanRequests(raw);
    const existing = new Set(
      readAllEntities<SavedRequest>(wsId, "requests").map((r) => r.id),
    );
    // Since Postman import generates new IDs, no ID collisions possible
    return {
      ok: true,
      filePath,
      itemCount: parsed.requests.length,
      collisionIds: [],
    };
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
    const parsed = parsePostmanRequests(raw);
    const cfg = loadConfig();

    // Build a name->folderId map as we create folders
    const folderNameToId = new Map<string, string>();

    for (const f of parsed.folders) {
      const id = generateId();
      const parentId = f.parentId ? (folderNameToId.get(f.parentId) ?? null) : null;
      const newFolder: Folder = { id, name: f.name, parentId, createdAt: Date.now(), workspaceId: wsId };
      cfg.requestFolders = [...(cfg.requestFolders ?? []), newFolder];
      folderNameToId.set(f.name, id);
    }

    let imported = 0;
    for (const req of parsed.requests) {
      const id = generateId();
      const folderId = req.folderId ? (folderNameToId.get(req.folderId) ?? null) : null;
      const folderName = folderId ? (cfg.requestFolders.find((f) => f.id === folderId)?.name ?? null) : null;
      const newReq: SavedRequest = {
        id,
        name: req.name,
        method: req.method,
        url: req.url,
        headers: req.headers,
        body: req.body,
        ...(req.preScript ? { preScript: req.preScript } : {}),
        ...(req.postScript ? { postScript: req.postScript } : {}),
        createdAt: Date.now(),
        folderId,
        workspaceId: wsId,
      };
      writeEntity(wsId, "requests", id, newReq, folderName);
      upsertNameEntry(wsId, "requests", id, { name: newReq.name, method: newReq.method, url: newReq.url });
      imported++;
    }

    saveConfig(cfg);
    reloadConfig();

    return { ok: true, imported };
  } catch (err) {
    return { ok: false, error: String(err) };
  }
}
