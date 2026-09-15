/**
 * P2 work item 4 — data directory resolution.
 *
 * New, additive infrastructure only. `appSettings.ts` and `workspaceFs.ts` are NOT yet wired to
 * consume this as their primary path — that wiring, plus the decision on whether the existing
 * `setDataRootOverride()` / `setSettingsPathOverride()` test hooks remain separate or fold into
 * this module, is flagged in `plan/03-phase-2-engine-extraction.md` as the part of this item most
 * likely to silently break e2e data-dir isolation. That wiring needs the 11 e2e specs to verify
 * it (`plan/13-checklist.md` P6 item "Verify e2e data-dir isolation still holds"), which cannot
 * run in this sandbox (no desktop session — see `plan/baseline.md` "Environment caveats"). It is
 * intentionally deferred rather than merged in unverified.
 *
 * What IS delivered here: the resolution order the CLI (P8) and Docker (P9) need, exactly as
 * specified —
 *
 *   1. `--data-dir <path>` CLI flag (see `resolveDataDir(argv)`)
 *   2. `BIFURC_DATA_DIR` environment variable
 *   3. Platform default: `%LOCALAPPDATA%\Bifurc` (Windows), `~/Library/Application Support/Bifurc`
 *      (macOS), `$XDG_CONFIG_HOME/bifurc` or `~/.config/bifurc` (Linux)
 *
 * The Electron shell will call `setDataRoot(app.getPath("userData"))` — option 1 in spirit,
 * preserving current behaviour exactly, once it is wired up (tracked, not done in this pass).
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
