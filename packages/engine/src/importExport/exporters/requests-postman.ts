import { loadConfig, SavedRequest, Folder } from "../../store/config";
import { readAllEntities } from "../../store/workspaceFs";
import type { ExportResult } from "../types";
import { exportRequestsToPostman } from "../formats/postman";

export async function run(wsId: string): Promise<ExportResult> {
  try {
    const cfg = loadConfig();
    const requests = readAllEntities<SavedRequest>(wsId, "requests").filter((r) => r.workspaceId === wsId);
    const folders = cfg.requestFolders.filter((f) => f.workspaceId === wsId);
    const ws = cfg.workspaces.find((w) => w.id === wsId);
    const json = exportRequestsToPostman(requests, folders, ws?.name ?? "Bifurc Requests");
    const content = json;
    return { ok: true, content, suggestedName: "requests-export.json" };
  } catch (err) {
    return { ok: false, error: String(err) };
  }
}
