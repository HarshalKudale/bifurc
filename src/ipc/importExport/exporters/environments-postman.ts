import * as fs from "fs";
import { loadConfig, Environment } from "@/store/config";
import type { ExportResult } from "@/ipc/importExport/types";

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

export async function run(wsId: string, filePath: string): Promise<ExportResult> {
  try {
    const cfg = loadConfig();
    const environments = cfg.environments.filter((e) => e.workspaceId === wsId);
    if (environments.length === 0) {
      return { ok: false, error: "No environments found in this workspace" };
    }
    if (environments.length === 1) {
      fs.writeFileSync(filePath, JSON.stringify(envToPostman(environments[0]), null, 2), "utf-8");
    } else {
      // Multiple environments: export as array wrapped in LP container
      const payload = {
        schema: "lp-postman-environments-v1",
        environments: environments.map(envToPostman),
      };
      fs.writeFileSync(filePath, JSON.stringify(payload, null, 2), "utf-8");
    }
    return { ok: true, filePath };
  } catch (err) {
    return { ok: false, error: String(err) };
  }
}
