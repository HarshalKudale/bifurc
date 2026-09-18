import { loadConfig } from "../../store/config";
import type { ExportResult } from "../types";

export async function run(wsId: string): Promise<ExportResult> {
  try {
    const cfg = loadConfig();
    const mocks = cfg.mocks.filter((m) => m.workspaceId === wsId);
    const folders = cfg.mockFolders.filter((f) => f.workspaceId === wsId);
    const ws = cfg.workspaces.find((w) => w.id === wsId);
    const payload = { schema: "lp-mocks-v1", name: ws?.name ?? "Bifurc", mocks, folders };
    const content = JSON.stringify(payload, null, 2);
    return { ok: true, content, suggestedName: "mocks-export.json" };
  } catch (err) {
    return { ok: false, error: String(err) };
  }
}
