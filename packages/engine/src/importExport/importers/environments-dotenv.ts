import {
  loadConfig, saveConfig, generateId, Environment,
} from "../../store/config";
import type { EnvVariable } from "../../store/types";
import { writeFlatEntity } from "../../store/workspaceFs";
import { reloadConfig } from "../../proxy/server";
import type { PreflightResult, ImportResult, CollisionStrategy, ImportSource } from "../types";

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

// ── The one place a display name used to come from a path ────────────────────
//
// `File_Ops_Protocol.md` §8 singles this out: under a remote engine the path does not
// exist locally, so the name would either be garbage or leak a fragment of the user's
// own filesystem into an entity name. The name now comes from the `filename` the client
// picked, which is the only side that knows it.
//
// `.env` and `.env.local` both reduce to "env", and `myapp.env` keeps `myapp` — i.e. the
// behaviour for real-world filenames is unchanged. The `|| "Imported"` does fix a latent
// bug: a file named exactly `.env` used to yield an EMPTY name, because `"".replace(...)`
// is `""` and `??` only fires on nullish.
function dotenvEnvName(filename: string): string {
  const base = filename.split(/[/\\]/).pop() ?? filename;
  const stripped = base.replace(/\.env.*$/, "");
  return stripped || "Imported";
}

export function preflight(_wsId: string, source: ImportSource): PreflightResult {
  try {
    const vars = parseDotenv(source.content);
    return { ok: true, itemCount: Object.keys(vars).length, collisionIds: [] };
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
    const vars = parseDotenv(source.content);
    const cfg = loadConfig();

    const variables: EnvVariable[] = Object.entries(vars).map(([key, value]) => ({
      id: generateId(),
      key,
      value,
    }));

    const name = dotenvEnvName(source.filename);
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
