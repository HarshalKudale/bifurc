/**
 * P2 work item 6 — headless engine startup.
 *
 * Two responsibilities, both of which used to live in the Electron shell (`src/main.ts`) and
 * belong to the engine instead:
 *
 * 1. `preflight()` — read-only readiness checks. The engine cannot depend on a dialog to report a
 *    fatal startup condition (see `plan/03-phase-2-engine-extraction.md` work item 6) — a CLI or
 *    Docker entrypoint has no dialog to show. It returns a structured list of checks instead; each
 *    caller renders failures however fits: the Electron shell in a dialog, the CLI on stderr, the
 *    Docker entrypoint as a non-zero exit. One check, three presentations.
 * 2. `bootstrapWorkspaces()` — the mutating half of startup: create each known workspace's
 *    directories and git repo, start auto-sync where configured, and repair or create the active
 *    workspace. The shell used to do this inline inside `app.whenReady()`; a headless engine needs
 *    exactly the same sequence with no Electron involved.
 *
 * This module is intentionally Electron-free — it imports only Node builtins plus the store/sync
 * modules and `mkcert`, none of which touch Electron. Keep it that way: it is on the
 * `packages/engine` side of the seam.
 */
import * as fs from "fs";
import * as path from "path";
import { randomBytes } from "crypto";
import { createCA } from "mkcert";
import { checkGitInstalled, initWorkspaceRepo } from "@bifurc/engine/store/gitStore";
import { checkPortInUse } from "@bifurc/engine/applications/portUtils";
import { saveSettings, type AppSettings } from "@bifurc/engine/store/appSettings";
import { initWorkspaceDir, wsDir } from "@bifurc/engine/store/workspaceFs";
import { generateId } from "@bifurc/engine/store/config";
import { getSyncConfig } from "@bifurc/engine/sync/syncManager";
import { setAutoSyncReloadFn, startAutoSync } from "@bifurc/engine/sync/autoSync";
import { reloadConfig } from "@bifurc/engine/proxy/server";

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

// ── Workspace bootstrap ───────────────────────────────────────────────────────

export interface WorkspaceBootstrapResult {
  /** The settings actually in effect afterwards — `activeWorkspaceId` may have been repaired. */
  settings: AppSettings;
  /** Workspace ids whose auto-sync poller was started. */
  autoSyncStarted: string[];
  /** True when no workspace directory on disk was usable and a fresh one had to be created. */
  createdDefaultWorkspace: boolean;
}

/** A workspace is only usable if its directory is actually on disk — a settings entry alone is
 * not enough (the user may have deleted or moved the data directory). */
function workspaceDirExists(wsId: string): boolean {
  try {
    return fs.statSync(wsDir(wsId)).isDirectory();
  } catch {
    return false;
  }
}

/**
 * Bring the engine's on-disk workspace state up to match `settings`.
 *
 * Extracted verbatim from `src/main.ts`'s `app.whenReady()` handler (P2 work item 6) so that the
 * Electron shell, the CLI and the Docker entrypoint all run the *same* startup sequence. Must be
 * called **after** `setDataRoot()` — every store module resolves through it.
 *
 * Idempotent in the sense that matters: running it twice leaves the same directories and repos,
 * starts no duplicate pollers (`startAutoSync` keys off `wsId`), and only rewrites settings when
 * `activeWorkspaceId` genuinely needed repairing.
 *
 * @returns the effective settings (never mutate the argument in place — the caller keeps its own
 * reference) plus a record of what was started, so a caller can log it.
 */
export async function bootstrapWorkspaces(settings: AppSettings): Promise<WorkspaceBootstrapResult> {
  // The engine owns the "refresh live proxy routing after a pull" wiring; the shell never needed
  // to know about it. Doing it here keeps that dependency engine-internal.
  setAutoSyncReloadFn(reloadConfig);

  let next = settings;
  const autoSyncStarted: string[] = [];

  // Init dirs/repos for all known workspaces first.
  for (const ws of settings.workspaces) {
    initWorkspaceDir(ws.id, ws.name);
    await initWorkspaceRepo(ws.id);
    const syncCfg = getSyncConfig(ws.id);
    if (syncCfg?.autoSync) {
      startAutoSync(ws.id);
      autoSyncStarted.push(ws.id);
    }
  }

  // Validate the active workspace — fall back to any usable one, or create a fresh default.
  //
  // NOTE (preserved quirk, do not "fix" during the package move): the loop above calls
  // `initWorkspaceDir()` for *every* workspace listed in settings, which creates the directory if
  // it is missing. So by the time we get here, every id in `settings.workspaces` has a directory on
  // disk, and this branch can only ever fire when `activeWorkspaceId` is **not** one of them —
  // i.e. settings were edited or a workspace was dropped from the list while still active. The
  // "its dir is missing" reading of this code is unreachable for a listed workspace. Changing that
  // would be a behaviour change, which P2 must not make.
  if (!workspaceDirExists(next.activeWorkspaceId)) {
    const valid = next.workspaces.find((w) => workspaceDirExists(w.id));
    if (valid) {
      next = { ...next, activeWorkspaceId: valid.id };
      saveSettings(next);
    } else {
      // No valid workspace on disk — create a fresh empty one.
      const id = generateId();
      const name = "Workspace 1";
      next = { ...next, workspaces: [{ id, name, activeEnvironmentId: null }], activeWorkspaceId: id };
      saveSettings(next);
      initWorkspaceDir(id, name);
      await initWorkspaceRepo(id);
      return { settings: next, autoSyncStarted, createdDefaultWorkspace: true };
    }
  }

  return { settings: next, autoSyncStarted, createdDefaultWorkspace: false };
}
