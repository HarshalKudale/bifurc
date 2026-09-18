import { loadConfig } from "../../store/config";
import type { ExportResult } from "../types";

export async function run(wsId: string): Promise<ExportResult> {
  try {
    const cfg = loadConfig();
    const environments = cfg.environments.filter((e) => e.workspaceId === wsId);
    const ws = cfg.workspaces.find((w) => w.id === wsId);
    const payload = { schema: "lp-environments-v1", name: ws?.name ?? "Bifurc", environments };
    const content = JSON.stringify(payload, null, 2);
    return { ok: true, content, suggestedName: "environments-export.json" };
  } catch (err) {
    return { ok: false, error: String(err) };
  }
}
