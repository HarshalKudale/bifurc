import * as fs from "fs";
import {
  loadConfig, saveConfig, generateId, SavedRequest, Folder,
} from "@/store/config";
import { writeEntity, upsertNameEntry } from "@/store/workspaceFs";
import { reloadConfig } from "@/proxy/server";
import type { PreflightResult, ImportResult, CollisionStrategy } from "@/ipc/importExport/types";

interface InsomniaResource {
  _id: string;
  _type: string;
  name?: string;
  parentId?: string;
  method?: string;
  url?: string;
  headers?: Array<{ name: string; value: string }>;
  body?: { mimeType?: string; text?: string };
  created?: number;
}

interface InsomniaExport {
  __export_format?: number;
  resources?: InsomniaResource[];
}

export function preflight(_wsId: string, filePath: string): PreflightResult {
  try {
    const data = JSON.parse(fs.readFileSync(filePath, "utf-8")) as InsomniaExport;
    if (!data.resources || !Array.isArray(data.resources)) {
      return { ok: false, error: "Not a valid Insomnia export file" };
    }
    const count = data.resources.filter((r) => r._type === "request").length;
    return { ok: true, filePath, itemCount: count, collisionIds: [] };
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
    const data = JSON.parse(fs.readFileSync(filePath, "utf-8")) as InsomniaExport;
    if (!data.resources) return { ok: false, error: "Not a valid Insomnia export file" };

    const cfg = loadConfig();
    const resources = data.resources;

    // Map Insomnia IDs to LP folder IDs
    const insoFolderToLpId = new Map<string, string>();
    for (const res of resources) {
      if (res._type === "request_group" || res._type === "workspace") {
        if (res._type === "request_group") {
          const lpId = generateId();
          insoFolderToLpId.set(res._id, lpId);
          const parentId = res.parentId ? (insoFolderToLpId.get(res.parentId) ?? null) : null;
          const newFolder: Folder = {
            id: lpId,
            name: res.name ?? "Folder",
            parentId,
            createdAt: res.created ?? Date.now(),
            workspaceId: wsId,
          };
          cfg.requestFolders = [...cfg.requestFolders, newFolder];
        }
      }
    }

    let imported = 0;
    for (const res of resources) {
      if (res._type !== "request") continue;

      const headers: Record<string, string> = {};
      for (const h of (res.headers ?? [])) {
        if (h.name) headers[h.name] = h.value ?? "";
      }

      const folderId = res.parentId ? (insoFolderToLpId.get(res.parentId) ?? null) : null;
      const folderName = folderId ? (cfg.requestFolders.find((f) => f.id === folderId)?.name ?? null) : null;

      const id = generateId();
      const name = res.name ?? `${res.method ?? "GET"} ${res.url ?? ""}`;
      const newReq: SavedRequest = {
        id,
        name,
        method: (res.method ?? "GET").toUpperCase(),
        url: res.url ?? "",
        headers,
        body: res.body?.text ?? "",
        createdAt: res.created ?? Date.now(),
        folderId,
        workspaceId: wsId,
      };
      writeEntity(wsId, "requests", id, newReq, folderName);
      upsertNameEntry(wsId, "requests", id, { name, method: newReq.method, url: newReq.url });
      imported++;
    }

    saveConfig(cfg);
    reloadConfig();
    return { ok: true, imported };
  } catch (err) {
    return { ok: false, error: String(err) };
  }
}
