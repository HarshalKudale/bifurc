import { loadConfig, Environment } from "../../store/config";
import type { ExportResult } from "../types";

function envToPostman(env: Environment): object {
  return {
    id: env.id,
    name: env.name,
    values: env.variables.map((v) => ({
      key: v.key,
      value: v.value,
      enabled: true,
      type: "default",
    })),
    _postman_variable_scope: "environment",
  };
}

export async function run(wsId: string): Promise<ExportResult> {
  try {
    const cfg = loadConfig();
    const environments = cfg.environments.filter((e) => e.workspaceId === wsId);
    if (environments.length === 0) {
      return { ok: false, error: "No environments found in this workspace" };
    }
    const content =
      environments.length === 1
        ? JSON.stringify(envToPostman(environments[0]), null, 2)
        : JSON.stringify(
            { schema: "lp-postman-environments-v1", environments: environments.map(envToPostman) },
            null,
            2,
          );
    return { ok: true, content, suggestedName: "environments-export.json" };
  } catch (err) {
    return { ok: false, error: String(err) };
  }
}
