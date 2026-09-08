import * as fs from "fs";
import {
  loadConfig, saveConfig, generateId, SavedWsConnection, Folder,
} from "@/store/config";
import { writeEntity, upsertNameEntry, readAllEntities } from "@/store/workspaceFs";
import { reloadConfig } from "@/proxy/server";
import type { PreflightResult, ImportResult, CollisionStrategy } from "@/ipc/importExport/types";

function parse(filePath: string): { wsConnections: SavedWsConnection[]; folders: Folder[] } {
  const data = JSON.parse(fs.readFileSync(filePath, "utf-8"));
  if (data?.schema !== "lp-websockets-v1" || !Array.isArray(data.wsConnections)) {
    throw new Error("Not a valid lp-websockets-v1 file");
  }
  return { wsConnections: data.wsConnections, folders: data.folders ?? [] };
}

export function preflight(wsId: string, filePath: string): PreflightResult {
  try {
    const { wsConnections } = parse(filePath);
    const existingIds = new Set(readAllEntities<SavedWsConnection>(wsId, "sockets").map((c) => c.id));
    const collisionIds = wsConnections.filter((c) => existingIds.has(c.id)).map((c) => c.id);
    return { ok: true, filePath, itemCount: wsConnections.length, collisionIds };
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
    const { wsConnections, folders } = parse(filePath);
    const cfg = loadConfig();
    const existingIds = new Set(
      readAllEntities<SavedWsConnection>(wsId, "sockets").map((c) => c.id),
    );

    const folderIdMap = new Map<string, string>();
    for (const f of folders) {
      const newId = strategy === "new" ? generateId() : f.id;
      folderIdMap.set(f.id, newId);
      if (!cfg.wsFolders.some((x) => x.id === newId)) {
        cfg.wsFolders = [...cfg.wsFolders, { ...f, id: newId, workspaceId: wsId }];
      }
    }

    let imported = 0;
    let skipped = 0;

    for (const conn of wsConnections) {
      const isCollision = existingIds.has(conn.id);
      if (isCollision && strategy === "keep") { skipped++; continue; }
      const newId = strategy === "new" ? generateId() : conn.id;
      const folderId = conn.folderId ? (folderIdMap.get(conn.folderId) ?? null) : null;
      const folderName = folderId ? (cfg.wsFolders.find((f) => f.id === folderId)?.name ?? null) : null;
      const newConn: SavedWsConnection = { ...conn, id: newId, folderId, workspaceId: wsId };
      writeEntity(wsId, "sockets", newId, newConn, folderName);
      upsertNameEntry(wsId, "sockets", newId, { name: newConn.name, url: newConn.url });
      imported++;
    }

    saveConfig(cfg);
    reloadConfig();
    return { ok: true, imported, skipped };
  } catch (err) {
    return { ok: false, error: String(err) };
  }
}
