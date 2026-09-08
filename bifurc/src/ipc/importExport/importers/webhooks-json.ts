import * as fs from "fs";
import {
  loadConfig, saveConfig, generateId, SavedWebhook, Folder,
} from "@/store/config";
import { writeEntity, upsertNameEntry, readAllEntities } from "@/store/workspaceFs";
import { reloadConfig } from "@/proxy/server";
import type { PreflightResult, ImportResult, CollisionStrategy } from "@/ipc/importExport/types";

function parse(filePath: string): { webhooks: SavedWebhook[]; folders: Folder[] } {
  const data = JSON.parse(fs.readFileSync(filePath, "utf-8"));
  if (data?.schema !== "lp-webhooks-v1" || !Array.isArray(data.webhooks)) {
    throw new Error("Not a valid lp-webhooks-v1 file");
  }
  return { webhooks: data.webhooks, folders: data.folders ?? [] };
}

export function preflight(wsId: string, filePath: string): PreflightResult {
  try {
    const { webhooks } = parse(filePath);
    const existingIds = new Set(readAllEntities<SavedWebhook>(wsId, "webhooks").map((h) => h.id));
    const collisionIds = webhooks.filter((h) => existingIds.has(h.id)).map((h) => h.id);
    return { ok: true, filePath, itemCount: webhooks.length, collisionIds };
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
    const { webhooks, folders } = parse(filePath);
    const cfg = loadConfig();
    const existingIds = new Set(
      readAllEntities<SavedWebhook>(wsId, "webhooks").map((h) => h.id),
    );

    const folderIdMap = new Map<string, string>();
    for (const f of folders) {
      const newId = strategy === "new" ? generateId() : f.id;
      folderIdMap.set(f.id, newId);
      if (!cfg.webhookFolders.some((x) => x.id === newId)) {
        cfg.webhookFolders = [...cfg.webhookFolders, { ...f, id: newId, workspaceId: wsId }];
      }
    }

    let imported = 0;
    let skipped = 0;

    for (const hook of webhooks) {
      const isCollision = existingIds.has(hook.id);
      if (isCollision && strategy === "keep") { skipped++; continue; }
      const newId = strategy === "new" ? generateId() : hook.id;
      const folderId = hook.folderId ? (folderIdMap.get(hook.folderId) ?? null) : null;
      const folderName = folderId ? (cfg.webhookFolders.find((f) => f.id === folderId)?.name ?? null) : null;
      const newHook: SavedWebhook = { ...hook, id: newId, folderId, workspaceId: wsId };
      writeEntity(wsId, "webhooks", newId, newHook, folderName);
      upsertNameEntry(wsId, "webhooks", newId, { name: newHook.name, urlSuffix: newHook.urlSuffix });
      imported++;
    }

    saveConfig(cfg);
    reloadConfig();
    return { ok: true, imported, skipped };
  } catch (err) {
    return { ok: false, error: String(err) };
  }
}
