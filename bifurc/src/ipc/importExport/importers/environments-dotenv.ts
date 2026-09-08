import * as fs from "fs";
import {
  loadConfig, saveConfig, generateId, Environment,
} from "@/store/config";
import type { EnvVariable } from "@/store/types";
import { writeFlatEntity } from "@/store/workspaceFs";
import { reloadConfig } from "@/proxy/server";
import type { PreflightResult, ImportResult, CollisionStrategy } from "@/ipc/importExport/types";

function parseDotenv(text: string): Record<string, string> {
  const vars: Record<string, string> = {};
  for (const line of text.split(/\r?\n/)) {
    const trimmed = line.trim();
    if (!trimmed || trimmed.startsWith("#")) continue;
    const eqIdx = trimmed.indexOf("=");
    if (eqIdx === -1) continue;
    const key = trimmed.slice(0, eqIdx).trim();
    let value = trimmed.slice(eqIdx + 1).trim();
    // Strip surrounding quotes
    if ((value.startsWith('"') && value.endsWith('"')) ||
        (value.startsWith("'") && value.endsWith("'"))) {
      value = value.slice(1, -1).replace(/\\"/g, '"').replace(/\\'/g, "'");
    }
    if (key) vars[key] = value;
  }
  return vars;
}

export function preflight(_wsId: string, filePath: string): PreflightResult {
  try {
    const text = fs.readFileSync(filePath, "utf-8");
    const vars = parseDotenv(text);
    return { ok: true, filePath, itemCount: Object.keys(vars).length, collisionIds: [] };
  } catch (err) {
    return { ok: false, error: String(err) };
  }
}

export async function run(
  wsId: string,
  filePath: string,
  _strategy: CollisionStrategy,
): Promise<ImportResult> {
  try {
    const text = fs.readFileSync(filePath, "utf-8");
    const vars = parseDotenv(text);
    const cfg = loadConfig();

    const variables: EnvVariable[] = Object.entries(vars).map(([key, value]) => ({
      id: generateId(),
      key,
      value,
    }));

    const name = filePath.split(/[/\\]/).pop()?.replace(/\.env.*$/, "") ?? "Imported";
    const id = generateId();
    const newEnv: Environment = { id, name, variables, createdAt: Date.now(), workspaceId: wsId };

    cfg.environments = [...cfg.environments, newEnv];
    writeFlatEntity(wsId, "environments", id, newEnv);
    saveConfig(cfg);
    reloadConfig();

    return { ok: true, imported: 1 };
  } catch (err) {
    return { ok: false, error: String(err) };
  }
}
