import * as fs from "fs";
import {
  loadConfig, saveConfig, generateId, Environment,
} from "@/store/config";
import { writeFlatEntity, readAllEntities } from "@/store/workspaceFs";
import { reloadConfig } from "@/proxy/server";
import type { PreflightResult, ImportResult, CollisionStrategy } from "@/ipc/importExport/types";

function parse(filePath: string): Environment[] {
  const data = JSON.parse(fs.readFileSync(filePath, "utf-8"));
  if (data?.schema !== "lp-environments-v1" || !Array.isArray(data.environments)) {
    throw new Error("Not a valid lp-environments-v1 file");
  }
  return data.environments as Environment[];
}

export function preflight(wsId: string, filePath: string): PreflightResult {
  try {
    const envs = parse(filePath);
    const existingIds = new Set(readAllEntities<Environment>(wsId, "environments").map((e) => e.id));
    const collisionIds = envs.filter((e) => existingIds.has(e.id)).map((e) => e.id);
    return { ok: true, filePath, itemCount: envs.length, collisionIds };
  } catch (err) {
    return { ok: false, error: String(err) };
  }
}

export async function run(
  wsId: string,
  filePath: string,
  strategy: CollisionStrategy,
): Promise<ImportResult> {
  try {
    const envs = parse(filePath);
    const cfg = loadConfig();
    const existingIds = new Set(cfg.environments.filter((e) => e.workspaceId === wsId).map((e) => e.id));

    let imported = 0;
    let skipped = 0;

    for (const env of envs) {
      const isCollision = existingIds.has(env.id);
      if (isCollision && strategy === "keep") { skipped++; continue; }

      const newId = strategy === "new" ? generateId() : env.id;
      const newEnv: Environment = { ...env, id: newId, workspaceId: wsId };

      const existIdx = cfg.environments.findIndex((e) => e.id === newId);
      if (existIdx !== -1) cfg.environments[existIdx] = newEnv;
      else cfg.environments = [...cfg.environments, newEnv];

      writeFlatEntity(wsId, "environments", newId, newEnv);
      imported++;
    }

    saveConfig(cfg);
    reloadConfig();
    return { ok: true, imported, skipped };
  } catch (err) {
    return { ok: false, error: String(err) };
  }
}
