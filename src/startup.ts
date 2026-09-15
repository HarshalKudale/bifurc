/**
 * P2 work item 6 — headless startup checks.
 *
 * The engine cannot depend on a dialog to report a fatal startup condition (see
 * `plan/03-phase-2-engine-extraction.md` work item 6) — a CLI or Docker entrypoint has no dialog
 * to show. `preflight()` returns a structured list of checks instead; each caller renders
 * failures however fits: the Electron shell in a dialog, the CLI on stderr, the Docker
 * entrypoint as a non-zero exit. One check, three presentations.
 *
 * This module is intentionally Electron-free — it imports only Node builtins plus
 * `@/store/gitStore`, `@/applications/portUtils`, and `mkcert`, none of which touch Electron.
 */
import * as fs from "fs";
import * as path from "path";
import { randomBytes } from "crypto";
import { createCA } from "mkcert";
import { checkGitInstalled } from "@/store/gitStore";
import { checkPortInUse } from "@/applications/portUtils";

export type StartupCheckCode =
  | "git-missing"
  | "data-dir-unwritable"
  | "port-in-use"
  | "mkcert-unusable";

export interface StartupCheck {
  ok: boolean;
  /** Present only when `ok` is false. */
  code?: StartupCheckCode;
  message?: string;
  hint?: string;
}

export interface PreflightPort {
  /** Human-readable label used in the check's message, e.g. "proxy", "webhook", "companion". */
  name: string;
  port: number;
}

export interface PreflightOptions {
  /** The resolved data directory — see `@/store/paths`. Must already exist or be creatable. */
  dataDir: string;
  /** Ports the engine is about to bind. Checked but never fatal — the server already reports a
   * bind failure via `bus.emit("server.error", …)` when it actually tries to listen. */
  ports?: PreflightPort[];
}

async function checkGit(): Promise<StartupCheck> {
  const ok = await checkGitInstalled();
  if (ok) return { ok: true };
  return {
    ok: false,
    code: "git-missing",
    message: "Git is required for config versioning and sync, but was not found.",
    hint: "Install Git from https://git-scm.com and restart.",
  };
}

function checkDataDirWritable(dataDir: string): StartupCheck {
  const probe = path.join(dataDir, `.preflight-${randomBytes(4).toString("hex")}.tmp`);
  try {
    fs.mkdirSync(dataDir, { recursive: true });
    fs.writeFileSync(probe, "");
    fs.rmSync(probe, { force: true });
    return { ok: true };
  } catch (e) {
    return {
      ok: false,
      code: "data-dir-unwritable",
      message: `Data directory "${dataDir}" is not writable: ${e instanceof Error ? e.message : String(e)}`,
      hint: "Check filesystem permissions, or point --data-dir / BIFURC_DATA_DIR at a writable location.",
    };
  }
}

async function checkPortAvailable(p: PreflightPort): Promise<StartupCheck> {
  const info = await checkPortInUse(p.port);
  if (!info.inUse) return { ok: true };
  return {
    ok: false,
    code: "port-in-use",
    message: `Port ${p.port} (${p.name}) is already in use${info.pid ? ` by PID ${info.pid}` : ""}.`,
    hint: "Free the port, or change the configured port in settings.",
  };
}

/** `mkcert` (`createCA`) is pure JS (`node-forge`) — this only fails if the dependency itself is
 * broken or missing, e.g. a stripped-down container image. It does not generate a real CA on
 * every startup; that only happens on demand via `certManager.ts#generateCA`. */
function checkMkcertUsable(): StartupCheck {
  if (typeof createCA === "function") return { ok: true };
  return {
    ok: false,
    code: "mkcert-unusable",
    message: "The mkcert module is unavailable — TLS interception cannot generate a CA.",
    hint: "Reinstall dependencies; TLS-off proxying is unaffected.",
  };
}

/**
 * Runs all headless startup checks. Never throws — every check catches its own failure mode and
 * reports it as `{ ok: false, ... }` instead.
 */
export async function preflight(opts: PreflightOptions): Promise<StartupCheck[]> {
  const checks: StartupCheck[] = [];
  checks.push(await checkGit());
  checks.push(checkDataDirWritable(opts.dataDir));
  for (const p of opts.ports ?? []) checks.push(await checkPortAvailable(p));
  checks.push(checkMkcertUsable());
  return checks;
}
