import * as fs from "fs";
import { loadConfig } from "@/store/config";
import type { ExportResult } from "@/ipc/importExport/types";

export async function run(wsId: string, filePath: string): Promise<ExportResult> {
  try {
    const cfg = loadConfig();
    const environments = cfg.environments.filter((e) => e.workspaceId === wsId);
    const ws = cfg.workspaces.find((w) => w.id === wsId);
    const payload = { schema: "lp-environments-v1", name: ws?.name ?? "Bifurc", environments };
    fs.writeFileSync(filePath, JSON.stringify(payload, null, 2), "utf-8");
    return { ok: true, filePath };
  } catch (err) {
    return { ok: false, error: String(err) };
  }
}
