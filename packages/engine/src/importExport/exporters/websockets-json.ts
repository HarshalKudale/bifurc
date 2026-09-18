import { loadConfig, SavedWsConnection } from "../../store/config";
import { readAllEntities } from "../../store/workspaceFs";
import type { ExportResult } from "../types";

export async function run(wsId: string): Promise<ExportResult> {
  try {
    const cfg = loadConfig();
    const wsConnections = readAllEntities<SavedWsConnection>(wsId, "sockets").filter((c) => c.workspaceId === wsId);
    const folders = cfg.wsFolders.filter((f) => f.workspaceId === wsId);
    const ws = cfg.workspaces.find((w) => w.id === wsId);
    const payload = { schema: "lp-websockets-v1", name: ws?.name ?? "Bifurc", wsConnections, folders };
    const content = JSON.stringify(payload, null, 2);
    return { ok: true, content, suggestedName: "websockets-export.json" };
  } catch (err) {
    return { ok: false, error: String(err) };
  }
}
