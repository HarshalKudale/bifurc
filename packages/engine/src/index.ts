/**
 * `@bifurc/engine` — package entry point.
 *
 * ## The public API
 *
 * `createEngine(opts) -> { start(), stop(), registry, bus, status() }` — the engine's own entry
 * point, for a bare Node script, the CLI (P8) and the Docker image (P9). Everything below the
 * `Public API` divider is the **provisional** surface and is removed once P6 replaces the shell's
 * in-process registration layer with the RPC client.
 *
 * The engine deliberately does **not** invent a data directory. `opts.dataDir` is required, and
 * `store/paths.ts` throws `DataRootNotInitialisedError` rather than falling back to the cwd — a
 * silent fallback would write the user's workspaces somewhere surprising. The Electron shell passes
 * `app.getPath("userData")`.
 *
 * ## Provisional surface (removed at P6)
 *
 * During P2–P5 the shell's own registration layer (`src/ipc/**`, 54 files) deep-imports
 * `@bifurc/engine/store/config` and friends directly, and so does `src/main.ts`. Those consumers are
 * replaced wholesale in P6 by the RPC client, at which point this barrel drops to the public API
 * alone. The namespaces below exist so that interim deep-import surface is explicit and obviously
 * temporary.
 *
 * They are **namespaces** rather than flattened star-exports for two reasons:
 *
 *  1. `store/config.ts` re-exports from `store/workspaceFs.ts` and `store/types.ts`, and
 *     `WorkspaceSyncConfig` / `WorkspaceSyncMeta` are declared in both `appSettings.ts` and
 *     `types.ts`. Flattening with `export *` would make those names ambiguous. (`sync/types.ts`
 *     adds a third potential clash, for the same reason.)
 *  2. Namespacing makes the interim surface obviously provisional, so nobody starts depending on
 *     `import { loadConfig } from "@bifurc/engine"` before P6 freezes the real API.
 */

import { setDataRoot, dataDir, isDataRootSet } from "./store/paths";
import { loadSettings, type AppSettings } from "./store/appSettings";
import { startServer, isRunning, getPort, getServerError } from "./proxy/server";
import { startCompanionServer, getCompanionPort, isCompanionRunning } from "./companion/companionServer";
import { bus } from "./eventBus";
import { preflight, bootstrapWorkspaces, type StartupCheck } from "./startup";
import { shutdownEngine } from "./shutdown";
import { commandRegistry, type CommandRegistry } from "./commands/registry";

// ─────────────────────────────────────────────────────────────────────────────
// Public API
// ─────────────────────────────────────────────────────────────────────────────

export interface EngineOptions {
  /**
   * Absolute path to the data root. Required, and the engine never invents one — `store/paths.ts`
   * throws `DataRootNotInitialisedError` rather than falling back to the cwd, because a silent
   * fallback would write the user's workspaces somewhere surprising. The Electron shell passes
   * `app.getPath("userData")`; a CLI passes `--data-dir`.
   *
   * ⚠️ **On Windows this is not authoritative, and that is a known defect.** `workspaceFs.dataRoot()`
   * and `appSettings.settingsPath()` check `%LOCALAPPDATA%` **before** consulting the data root, so
   * on Windows they resolve to `%LOCALAPPDATA%\Bifurc\…` regardless of what was passed here —
   * i.e. to the user's real data directory. On macOS and Linux the branch does not exist and this
   * option works as described.
   *
   * The shortcut is pre-existing (the store modules' own fallback is exactly
   * `platformDefaultDataDir()`) and is deliberately left alone, because changing the resolution
   * order would move existing Windows users' data — a user-visible change, which P2 must not make.
   * `e2e/fixtures/electronApp.ts` already works around it by overriding `LOCALAPPDATA` to a temp
   * directory; a caller that needs a non-default root on Windows must do the same, or set the store
   * override. Reconciling this is a product decision, recorded in `plan/03` work item 8.
   */
  dataDir: string;
  /**
   * Called once per **non-fatal** preflight finding (`git-missing`, `data-dir-unwritable`,
   * `port-in-use`, `mkcert-unusable`). Defaults to `console.warn`.
   *
   * Preflight is deliberately advisory: the shell has always tolerated a busy port (the server
   * reports its own bind failure on the bus) and a missing `mkcert`, and turning those into
   * startup failures would be a user-visible behaviour change. Whether any of them should become
   * *blocking* is an open product decision, recorded in `plan/03`.
   */
  onPreflightWarning?: (check: StartupCheck) => void;
}

export interface EngineStatus {
  /** True once `start()` has completed and the proxy is listening. */
  running: boolean;
  dataDir: string;
  /** The port the proxy actually bound, or `null` when it is not running. */
  proxyPort: number | null;
  /** Whatever the proxy reported on a failed bind, if anything. */
  proxyError: string | null;
  companionPort: number | null;
  /** The **effective** settings after bootstrap — `activeWorkspaceId` may have been repaired. */
  settings: AppSettings;
  /** Workspace ids whose auto-sync poller was started by this `start()`. */
  autoSyncStarted: string[];
}

export interface Engine {
  /**
   * Resolve the data root, run preflight, bootstrap workspaces, then bind the proxy and companion
   * sockets. Safe to call twice — the second call returns the same status without re-binding.
   *
   * @throws if the engine has already been stopped (see `stop()`).
   */
  start(): Promise<EngineStatus>;
  /**
   * Tear everything down. Idempotent, and delegates to the memoised `shutdownEngine()`, which is
   * also what the shell's `before-quit` handler calls — the shell quitting, a supervisor restarting
   * a crashed engine and a SIGTERM handler may all ask for this, in any order.
   *
   * **The engine is single-use per process.** Because `shutdownEngine()` memoises, a `start()`
   * after `stop()` cannot work; it throws rather than appearing to succeed.
   */
  stop(): Promise<void>;
  /** A snapshot of the live state. Safe to call before `start()`. */
  status(): EngineStatus;
  /** The command registry the transport layers dispatch through (~112 commands). */
  readonly registry: CommandRegistry;
  /** The engine's event bus. */
  readonly bus: typeof bus;
}

/**
 * Poll until a fire-and-forget server reports ready, or reports an error, or the deadline passes.
 *
 * Polling rather than an event because neither `startServer()` nor `startCompanionServer()` exposes
 * its `net.Server`/`WebSocketServer` — the only readiness signal they publish is the predicate
 * passed here. 10 ms is well below the cost of a socket bind, so this adds no measurable latency in
 * practice; the deadline exists so a hung bind cannot hang `start()` forever.
 */
async function waitUntilReady(
  isReady: () => boolean,
  getError: () => string | null,
  timeoutMs = 5000,
): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (isReady() || getError()) return;
    await new Promise((resolve) => setTimeout(resolve, 10));
  }
}

export function createEngine(opts: EngineOptions): Engine {
  const warn =
    opts.onPreflightWarning ??
    ((check: StartupCheck) => console.warn(`[engine] ${check.code}: ${check.message}`));

  let settings: AppSettings | null = null;
  let autoSyncStarted: string[] = [];
  let startPromise: Promise<EngineStatus> | null = null;
  let stopped = false;

  async function doStart(): Promise<EngineStatus> {
    // Must come first: every store module resolves through the data root.
    setDataRoot(opts.dataDir);

    settings = loadSettings();

    // Advisory only — see `EngineOptions.onPreflightWarning`.
    const checks = await preflight({
      dataDir: dataDir(),
      ports: [
        { name: "proxy", port: settings.port },
        { name: "webhook", port: settings.webhookPort ?? 9101 },
        { name: "companion", port: settings.companionPort ?? 9271 },
      ],
    });
    for (const check of checks) if (!check.ok) warn(check);

    const boot = await bootstrapWorkspaces(settings);
    settings = boot.settings;
    autoSyncStarted = boot.autoSyncStarted;

    startServer(settings.port);
    startCompanionServer(settings.companionPort ?? 9271);

    // `startServer()` and `startCompanionServer()` are fire-and-forget — both hand off to an async
    // `listen()` and return immediately, so `isRunning()` is still false on the very next line.
    // Resolving `start()` before the sockets are actually accepting would make the one promise a
    // caller has to await tell them nothing, so wait for readiness here.
    //
    // A bind failure is *not* thrown: the proxy already reports it through `bus` and
    // `status().proxyError`, and the shell has always tolerated a busy port. Waiting on the error
    // as well as on success keeps `start()` bounded either way.
    await waitUntilReady(isRunning, getServerError);
    await waitUntilReady(isCompanionRunning, () => null);

    return status();
  }

  function status(): EngineStatus {
    const proxyUp = isRunning();
    const companionUp = isCompanionRunning();
    return {
      running: proxyUp,
      dataDir: isDataRootSet() ? dataDir() : opts.dataDir,
      proxyPort: proxyUp ? getPort() : null,
      proxyError: getServerError(),
      companionPort: companionUp ? getCompanionPort() : null,
      settings: settings ?? loadSettingsIfPossible(),
      autoSyncStarted,
    };
  }

  function loadSettingsIfPossible(): AppSettings {
    // `status()` is documented as safe before `start()`. Reading settings needs the data root, so
    // fall back to defaults rather than throwing at the caller for merely *asking* for status.
    try {
      setDataRoot(opts.dataDir);
      return loadSettings();
    } catch {
      return { ...({} as AppSettings) };
    }
  }

  return {
    start(): Promise<EngineStatus> {
      if (stopped) {
        return Promise.reject(
          new Error(
            "createEngine(): this engine has already been stopped. The engine is single-use per " +
              "process — `shutdownEngine()` is memoised by design, so a restart cannot work. " +
              "Create a new engine instead.",
          ),
        );
      }
      // Re-entrancy guard: two concurrent start() calls must not both bind.
      startPromise ??= doStart();
      return startPromise;
    },

    async stop(): Promise<void> {
      if (stopped) return;
      stopped = true;
      await shutdownEngine();
    },

    status,
    registry: commandRegistry,
    bus,
  };
}

// ─────────────────────────────────────────────────────────────────────────────
// Provisional surface — removed at P6 (see the header)
// ─────────────────────────────────────────────────────────────────────────────

export * as paths from "./store/paths";
export * as types from "./store/types";
export * as appSettings from "./store/appSettings";
export * as config from "./store/config";
export * as workspaceFs from "./store/workspaceFs";
export * as gitStore from "./store/gitStore";
export * as subscription from "./subscription/entityCount";
export * as randomNames from "./lib/randomNames";
export * as randomizer from "./lib/randomizer";
export * as eventBus from "./eventBus";
export * as proxy from "./proxy/server";
export * as sync from "./sync/syncManager";
export * as applications from "./applications/processSpawner";
export * as companion from "./companion/companionServer";
export * as commands from "./commands/registry";
export * as startup from "./startup";
export * as shutdown from "./shutdown";
