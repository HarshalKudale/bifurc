import * as fs from "fs";
import { loadConfig } from "@/store/config";
import type { ExportResult } from "@/ipc/importExport/types";

function escapeValue(val: string): string {
  if (/[\s"'\\#\r\n]/.test(val)) return `"${val.replace(/"/g, '\\"')}"`;
  return val;
}

export async function run(wsId: string, filePath: string): Promise<ExportResult> {
  try {
    const cfg = loadConfig();
    const environments = cfg.environments.filter((e) => e.workspaceId === wsId);
    if (environments.length === 0) {
      return { ok: false, error: "No environments found in this workspace" };
    }

    const lines: string[] = [];
    for (const env of environments) {
      if (environments.length > 1) lines.push(`# ${env.name}`, "");
      for (const v of env.variables) {
        if (v.key) lines.push(`${v.key}=${escapeValue(v.value ?? "")}`);
      }
      if (environments.length > 1) lines.push("");
    }

    fs.writeFileSync(filePath, lines.join("\n") + "\n", "utf-8");
    return { ok: true, filePath };
  } catch (err) {
    return { ok: false, error: String(err) };
  }
}
