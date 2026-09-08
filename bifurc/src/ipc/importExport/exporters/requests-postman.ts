import * as fs from "fs";
import { loadConfig, SavedRequest, Folder } from "@/store/config";
import { readAllEntities } from "@/store/workspaceFs";
import type { ExportResult } from "@/ipc/importExport/types";
import { exportRequestsToPostman } from "@/ipc/importExport/formats/postman";

export async function run(wsId: string, filePath: string): Promise<ExportResult> {
  try {
    const cfg = loadConfig();
    const requests = readAllEntities<SavedRequest>(wsId, "requests").filter((r) => r.workspaceId === wsId);
    const folders = cfg.requestFolders.filter((f) => f.workspaceId === wsId);
    const ws = cfg.workspaces.find((w) => w.id === wsId);
    const json = exportRequestsToPostman(requests, folders, ws?.name ?? "Bifurc Requests");
    fs.writeFileSync(filePath, json, "utf-8");
    return { ok: true, filePath };
  } catch (err) {
    return { ok: false, error: String(err) };
  }
}
