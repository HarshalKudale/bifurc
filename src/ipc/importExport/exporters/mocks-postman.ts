import * as fs from "fs";
import { loadConfig } from "@/store/config";
import type { ExportResult } from "@/ipc/importExport/types";
import { exportMocksToPostman } from "@/ipc/importExport/formats/postman";

export async function run(wsId: string, filePath: string): Promise<ExportResult> {
  try {
    const cfg = loadConfig();
    const mocks = cfg.mocks.filter((m) => m.workspaceId === wsId);
    const folders = cfg.mockFolders.filter((f) => f.workspaceId === wsId);
    const ws = cfg.workspaces.find((w) => w.id === wsId);
    const json = exportMocksToPostman(mocks, folders, ws?.name ?? "Bifurc Mocks");
    fs.writeFileSync(filePath, json, "utf-8");
    return { ok: true, filePath };
  } catch (err) {
    return { ok: false, error: String(err) };
  }
}
