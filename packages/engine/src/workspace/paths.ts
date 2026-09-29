/**
 * P2 work item 4 — data directory resolution.
 *
 * `appSettings.ts` and `workspaceFs.ts` now consume `dataDir()` as their ultimate fallback (after
 * the win32 `LOCALAPPDATA` special case and the `*Override` test hooks, both left untouched —
 * see `store/workspaceFs.ts#dataRoot` and `store/appSettings.ts#settingsPath`). Neither
 * module imports `electron` any more.
 *
 * `src/main.ts` calls `setDataRoot()` with Electron's `userData` path once, at the very top of
 * `app.whenReady()`, before any store module is touched — this preserves the exact directory
 * Electron previously resolved directly, so existing installs see no path change. The two
 * `*Override` hooks are deliberately left as-is (not folded into this module): they are proven
 * by the full integration suite and folding them in was a distinct, riskier change this pass
 * did not attempt.
 *
 * This module also gives the CLI (P8) and Docker (P9) entrypoints the resolution order they
 * need, exactly as specified —
 *
 *   1. `--data-dir <path>` CLI flag (see `resolveDataDir(argv)`)
 *   2. `BIFURC_DATA_DIR` environment variable
 *   3. Platform default: `%LOCALAPPDATA%\Bifurc` (Windows), `~/Library/Application Support/Bifurc`
 *      (macOS), `$XDG_CONFIG_HOME/bifurc` or `~/.config/bifurc` (Linux)
 *
 * Non-Electron callers (CLI/Docker, once they exist) are expected to call
 * `setDataRoot(resolveDataDir(argv))` once at startup, exactly like the shell does.
 */
import * as os from "os";
import * as path from "path";
import * as fs from "fs";

let dataRoot: string | null = null;

/** Loud by design (work item 4) — a silent fallback to cwd is how data ends up in a container's
 * ephemeral layer. Callers must call `setDataRoot()` before touching any store module. */
export class DataRootNotInitialisedError extends Error {
  constructor() {
    super(
      "Data root not initialised — call setDataRoot() at startup before using any store module. " +
      "See plan/03-phase-2-engine-extraction.md work item 4.",
    );
    this.name = "DataRootNotInitialisedError";
  }
}

/** Sets the data root and ensures the directory exists (per work item 4: "must create the
 * directory and its subdirectories if missing"). */
export function setDataRoot(dir: string): void {
  const resolved = path.resolve(dir);
  fs.mkdirSync(resolved, { recursive: true });
  dataRoot = resolved;
}

/** For tests only — returns to the uninitialised state so each test can assert the loud-error
 * behaviour independently. */
export function resetDataRootForTests(): void {
  dataRoot = null;
}

export function dataDir(): string {
  if (!dataRoot) throw new DataRootNotInitialisedError();
  return dataRoot;
}

export function isDataRootSet(): boolean {
  return dataRoot !== null;
}

/** Platform default per work item 4's resolution order, step 3. */
export function platformDefaultDataDir(): string {
  if (process.platform === "win32") {
    const base = process.env.LOCALAPPDATA || path.join(os.homedir(), "AppData", "Local");
    return path.join(base, "Bifurc");
  }
  if (process.platform === "darwin") {
    return path.join(os.homedir(), "Library", "Application Support", "Bifurc");
  }
  const xdg = process.env.XDG_CONFIG_HOME || path.join(os.homedir(), ".config");
  return path.join(xdg, "bifurc");
}

/**
 * Resolves the data directory per the documented order, without side effects — does not call
 * `setDataRoot()`. The CLI/Docker entrypoint (P8/P9) is expected to call
 * `setDataRoot(resolveDataDir(argv))` once at startup.
 */
export function resolveDataDir(argv: readonly string[] = process.argv): string {
  const flagIndex = argv.indexOf("--data-dir");
  if (flagIndex !== -1 && argv[flagIndex + 1]) {
    return argv[flagIndex + 1];
  }
  const eq = argv.find((a) => a.startsWith("--data-dir="));
  if (eq) return eq.slice("--data-dir=".length);

  if (process.env.BIFURC_DATA_DIR) return process.env.BIFURC_DATA_DIR;

  return platformDefaultDataDir();
}
