import {
  loadConfig, saveConfig, generateId, Environment,
} from "../../store/config";
import type { EnvVariable } from "../../store/types";
import { writeFlatEntity, readAllEntities } from "../../store/workspaceFs";
import { reloadConfig } from "../../proxy/server";
import type { PreflightResult, ImportResult, CollisionStrategy, ImportSource } from "../types";

interface PostmanEnvValue {
  key: string;
  value: string;
  enabled?: boolean;
  type?: string;
}

interface PostmanEnvironment {
  id?: string;
  name?: string;
  values?: PostmanEnvValue[];
}

function parseFile(content: string): PostmanEnvironment[] {
  const data = JSON.parse(content);
  // Single Postman environment
  if (data._postman_variable_scope === "environment" || data.values) {
    return [data];
  }
  // Multi-environment LP wrapper
  if (data.schema === "lp-postman-environments-v1" && Array.isArray(data.environments)) {
    return data.environments;
  }
  throw new Error("Not a valid Postman environment file");
}

export function preflight(wsId: string, source: ImportSource): PreflightResult {
  try {
    const parsed = parseFile(source.content);
    return { ok: true, itemCount: parsed.length, collisionIds: [] };
  } catch (err) {
    return { ok: false, error: String(err) };
  }
}

export async function run(
  wsId: string,
  source: ImportSource,
  _strategy: CollisionStrategy,
): Promise<ImportResult> {
  try {
    const parsed = parseFile(source.content);
    const cfg = loadConfig();

    let imported = 0;
    for (const pmEnv of parsed) {
      const id = generateId();
      const variables: EnvVariable[] = (pmEnv.values ?? [])
        .filter((v) => v.enabled !== false && v.key)
        .map((v) => ({ id: generateId(), key: v.key, value: v.value ?? "" }));

      const newEnv: Environment = {
        id,
        name: pmEnv.name ?? "Imported Environment",
        variables,
        createdAt: Date.now(),
        workspaceId: wsId,
      };

      cfg.environments = [...cfg.environments, newEnv];
      writeFlatEntity(wsId, "environments", id, newEnv);
      imported++;
    }

    saveConfig(cfg);
    reloadConfig();
    return { ok: true, imported };
  } catch (err) {
    return { ok: false, error: String(err) };
  }
}
