import * as fs from "fs";
import {
  loadConfig, saveConfig, generateId, SavedRequest, Folder,
} from "@/store/config";
import { writeEntity, upsertNameEntry } from "@/store/workspaceFs";
import { reloadConfig } from "@/proxy/server";
import type { PreflightResult, ImportResult, CollisionStrategy } from "@/ipc/importExport/types";

interface HarHeader { name: string; value: string }
interface HarPostData { mimeType?: string; text?: string }
interface HarRequest {
  method: string;
  url: string;
  headers?: HarHeader[];
  postData?: HarPostData;
}
interface HarEntry { request?: HarRequest; startedDateTime?: string }
interface HarLog { comment?: string; entries?: HarEntry[] }
interface Har { log?: HarLog }

export function preflight(_wsId: string, filePath: string): PreflightResult {
  try {
    const data = JSON.parse(fs.readFileSync(filePath, "utf-8")) as Har;
    if (!data.log) return { ok: false, error: "Not a valid HAR file" };
    const count = (data.log.entries ?? []).filter((e) => e.request).length;
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
    const data = JSON.parse(fs.readFileSync(filePath, "utf-8")) as Har;
    if (!data.log) return { ok: false, error: "Not a valid HAR file" };

    const cfg = loadConfig();
    const comment = data.log.comment;
    let folderId: string | null = null;

    if (comment) {
      const id = generateId();
      const newFolder: Folder = { id, name: comment, parentId: null, createdAt: Date.now(), workspaceId: wsId };
      cfg.requestFolders = [...cfg.requestFolders, newFolder];
      folderId = id;
    }

    let imported = 0;
    const SKIP = new Set(["host", "content-length", "connection"]);

    for (const entry of (data.log.entries ?? [])) {
      const req = entry.request;
      if (!req?.url || !req.method) continue;

      const headers: Record<string, string> = {};
      for (const h of (req.headers ?? [])) {
        if (!SKIP.has(h.name.toLowerCase())) headers[h.name] = h.value;
      }

      const id = generateId();
      const name = `${req.method} ${req.url}`;
      const newReq: SavedRequest = {
        id,
        name,
        method: req.method.toUpperCase(),
        url: req.url,
        headers,
        body: req.postData?.text ?? "",
        createdAt: entry.startedDateTime ? new Date(entry.startedDateTime).getTime() : Date.now(),
        folderId,
        workspaceId: wsId,
      };
      writeEntity(wsId, "requests", id, newReq, comment ?? null);
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
