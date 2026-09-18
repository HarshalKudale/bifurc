import { loadConfig } from "../../store/config";
import type { ExportResult } from "../types";
import { exportMocksToPostman } from "../formats/postman";

export async function run(wsId: string): Promise<ExportResult> {
  try {
    const cfg = loadConfig();
    const mocks = cfg.mocks.filter((m) => m.workspaceId === wsId);
    const folders = cfg.mockFolders.filter((f) => f.workspaceId === wsId);
    const ws = cfg.workspaces.find((w) => w.id === wsId);
    const json = exportMocksToPostman(mocks, folders, ws?.name ?? "Bifurc Mocks");
    const content = json;
    return { ok: true, content, suggestedName: "mocks-export.json" };
  } catch (err) {
    return { ok: false, error: String(err) };
  }
}
