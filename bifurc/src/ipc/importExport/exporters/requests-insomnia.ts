import * as fs from "fs";
import { loadConfig, SavedRequest, Folder } from "@/store/config";
import { readAllEntities } from "@/store/workspaceFs";
import type { ExportResult } from "@/ipc/importExport/types";

function mkId(prefix: string): string {
  return `${prefix}_${Date.now().toString(36)}${Math.random().toString(36).slice(2, 7)}`;
}

export async function run(wsId: string, filePath: string): Promise<ExportResult> {
  try {
    const cfg = loadConfig();
    const requests = readAllEntities<SavedRequest>(wsId, "requests").filter((r) => r.workspaceId === wsId);
    const folders = cfg.requestFolders.filter((f) => f.workspaceId === wsId);
    const ws = cfg.workspaces.find((w) => w.id === wsId);

    const workspaceId = mkId("wrk");
    const resources: object[] = [
      {
        _id: workspaceId,
        _type: "workspace",
        name: ws?.name ?? "Bifurc",
        created: Date.now(),
        modified: Date.now(),
      },
    ];

    // Map LP folder IDs to Insomnia folder resource IDs
    const folderIdMap = new Map<string, string>();
    for (const folder of folders) {
      const insoId = mkId("fld");
      folderIdMap.set(folder.id, insoId);
      resources.push({
        _id: insoId,
        _type: "request_group",
        parentId: folder.parentId ? (folderIdMap.get(folder.parentId) ?? workspaceId) : workspaceId,
        name: folder.name,
        created: folder.createdAt,
        modified: folder.createdAt,
      });
    }

    for (const req of requests) {
      const headers = Object.entries(req.headers ?? {}).map(([name, value]) => ({ name, value }));
      resources.push({
        _id: mkId("req"),
        _type: "request",
        parentId: req.folderId ? (folderIdMap.get(req.folderId) ?? workspaceId) : workspaceId,
        name: req.name,
        method: req.method,
        url: req.url,
        headers,
        body: req.body?.trim() ? { mimeType: "text/plain", text: req.body } : {},
        created: req.createdAt,
        modified: req.createdAt,
      });
    }

    const output = {
      __export_format: 4,
      __export_date: new Date().toISOString(),
      __export_source: "bifurc",
      resources,
    };

    fs.writeFileSync(filePath, JSON.stringify(output, null, 2), "utf-8");
    return { ok: true, filePath };
  } catch (err) {
    return { ok: false, error: String(err) };
  }
}
