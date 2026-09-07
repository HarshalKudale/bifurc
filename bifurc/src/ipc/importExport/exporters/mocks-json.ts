import * as fs from "fs";
import { loadConfig } from "@/store/config";
import type { ExportResult } from "@/ipc/importExport/types";

export async function run(wsId: string, filePath: string): Promise<ExportResult> {
  try {
    const cfg = loadConfig();
    const mocks = cfg.mocks.filter((m) => m.workspaceId === wsId);
    const folders = cfg.mockFolders.filter((f) => f.workspaceId === wsId);
    const ws = cfg.workspaces.find((w) => w.id === wsId);
    const payload = { schema: "lp-mocks-v1", name: ws?.name ?? "Bifurc", mocks, folders };
    fs.writeFileSync(filePath, JSON.stringify(payload, null, 2), "utf-8");
    return { ok: true, filePath };
  } catch (err) {
    return { ok: false, error: String(err) };
  }
}
