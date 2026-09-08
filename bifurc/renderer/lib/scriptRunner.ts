/**
 * Script execution sandbox for pre/post request scripts.
 * Scripts run in the main process via vm.createContext (sandboxed - no fs/net/require).
 *
 * Pre-script context:
 *   lp.request.url, lp.request.headers, lp.request.body  (mutable - changes apply to this send only)
 *   lp.environment.get/set/unset
 *
 * Post-script context:
 *   lp.response.status, lp.response.headers, lp.response.body (read-only)
 *   lp.environment.get/set/unset
 */

import { Environment } from "@/types";

export interface ScriptRequest {
  method: string;
  url: string;
  headers: Record<string, string>;
  body: string; // plain text
}

export interface ScriptResponse {
  status: number;
  headers: Record<string, string>;
  body: string; // plain text
}

export interface ScriptEnvProxy {
  get(key: string): string | undefined;
  set(key: string, value: string): void;
  unset(key: string): void;
}

export interface RunPostScriptResult {
  envVars: Record<string, string>;
  error?: string;
}

export interface RunPreScriptResult {
  /** Possibly-mutated request to use for the actual send (ephemeral - not saved) */
  req: ScriptRequest;
  /** Updated environment variables (to be used for the rest of this send cycle) */
  envVars: Record<string, string>;
  error?: string;
}

// -- Helpers ------------------------------------------------------------------

function envToMap(env: Environment | null): Record<string, string> {
  const map: Record<string, string> = {};
  if (!env) return map;
  for (const v of env.variables) map[v.key] = v.value;
  return map;
}

// Scripts run via IPC in the main-process vm sandbox (scriptExecutor.ts)

// -- Public API ----------------------------------------------------------------

/**
 * Run the pre-script for a request via the main-process vm sandbox.
 * Returns a (possibly mutated) copy of the request - the original is never touched.
 */
export async function runPreScript(
  script: string,
  req: ScriptRequest,
  env: Environment | null,
): Promise<RunPreScriptResult> {
  const vars = envToMap(env);
  const fallback = { ...req, headers: { ...req.headers } };
  try {
    const result = await window.api.executeScript({
      script,
      context: "pre",
      request: req,
      envVars: vars,
    });
    if (result.error) {
      return { req: result.request ?? fallback, envVars: result.envVars ?? vars, error: result.error };
    }
    return { req: result.request ?? fallback, envVars: result.envVars ?? vars };
  } catch (e) {
    return { req: fallback, envVars: vars, error: e instanceof Error ? e.message : String(e) };
  }
}

/**
 * Run the post-script for a request via the main-process vm sandbox.
 * The response is read-only; only environment variables may be mutated.
 */
export async function runPostScript(
  script: string,
  response: ScriptResponse,
  env: Environment | null,
): Promise<RunPostScriptResult> {
  const vars = envToMap(env);
  try {
    const result = await window.api.executeScript({
      script,
      context: "post",
      response,
      envVars: vars,
    });
    if (result.error) {
      return { envVars: result.envVars ?? vars, error: result.error };
    }
    return { envVars: result.envVars ?? vars };
  } catch (e) {
    return { envVars: vars, error: e instanceof Error ? e.message : String(e) };
  }
}
