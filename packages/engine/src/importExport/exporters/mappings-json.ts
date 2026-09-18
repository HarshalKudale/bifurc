import { loadConfig } from "../../store/config";
import type { ExportResult } from "../types";

export async function run(wsId: string): Promise<ExportResult> {
  try {
    const cfg = loadConfig();
    const mappings = cfg.mappings.filter((m) => m.workspaceId === wsId);
    const ws = cfg.workspaces.find((w) => w.id === wsId);
    const payload = { schema: "lp-mappings-v1", name: ws?.name ?? "Bifurc", mappings };
    const content = JSON.stringify(payload, null, 2);
    return { ok: true, content, suggestedName: "mappings-export.json" };
  } catch (err) {
    return { ok: false, error: String(err) };
  }
}
