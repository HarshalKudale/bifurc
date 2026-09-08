import * as fs from "fs";
import { loadConfig, SavedRequest } from "@/store/config";
import { readAllEntities } from "@/store/workspaceFs";
import type { ExportResult } from "@/ipc/importExport/types";

function toCurl(req: SavedRequest): string {
  const parts: string[] = [`curl -X ${req.method}`];

  for (const [key, value] of Object.entries(req.headers ?? {})) {
    parts.push(`  -H ${JSON.stringify(`${key}: ${value}`)}`);
  }

  if (req.body?.trim()) {
    parts.push(`  -d ${JSON.stringify(req.body)}`);
  }

  parts.push(`  ${JSON.stringify(req.url)}`);
  return parts.join(" \\\n");
}

export async function run(wsId: string, filePath: string): Promise<ExportResult> {
  try {
    const requests = readAllEntities<SavedRequest>(wsId, "requests").filter((r) => r.workspaceId === wsId);

    const lines: string[] = [];
    for (const req of requests) {
      lines.push(`# ${req.name}`);
      lines.push(toCurl(req));
      lines.push("");
    }

    fs.writeFileSync(filePath, lines.join("\n"), "utf-8");
    return { ok: true, filePath };
  } catch (err) {
    return { ok: false, error: String(err) };
  }
}
