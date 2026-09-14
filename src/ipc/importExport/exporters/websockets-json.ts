import * as fs from "fs";
import { loadConfig, SavedWsConnection } from "@/store/config";
import { readAllEntities } from "@/store/workspaceFs";
import type { ExportResult } from "@/ipc/importExport/types";

export async function run(wsId: string, filePath: string): Promise<ExportResult> {
  try {
    const cfg = loadConfig();
    const wsConnections = readAllEntities<SavedWsConnection>(wsId, "sockets").filter((c) => c.workspaceId === wsId);
    const folders = cfg.wsFolders.filter((f) => f.workspaceId === wsId);
    const ws = cfg.workspaces.find((w) => w.id === wsId);
    const payload = { schema: "lp-websockets-v1", name: ws?.name ?? "Bifurc", wsConnections, folders };
    fs.writeFileSync(filePath, JSON.stringify(payload, null, 2), "utf-8");
    return { ok: true, filePath };
  } catch (err) {
    return { ok: false, error: String(err) };
  }
}
