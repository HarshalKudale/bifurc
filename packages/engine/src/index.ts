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
 * Electron's per-user `userData` directory.
 *
 * ## Provisional surface (removed at P6)
 *
 * During P2–P5 the shell's own registration layer (`src/ipc/**`, 17 files — it was 54 before P3
 * moved `src/ipc/importExport/**` into this package) deep-imports
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

import { setDataRoot, dataDir, isDataRootSet } from "./workspace/paths";
import { loadSettings, type AppSettings } from "./workspace/appSettings";
import { startServer, isRunning, getPort, getServerError } from "./proxy/server";
// The **edge router** — `plan/15` §3.3, work item 3. Host-level: one listener for the reverse half
// (the `*.localhost` mappings and the home page) and, later, the GUI.
//
// `bindEdgeInstance` is `plan/17` §5 work item 7: the edge is per-engine state, so it is *named* to the
// module-level binding here rather than being found through it — which is what lets `config.save` re-bind it
// when `settings.port` changes.
import { bindEdgeInstance, createEdgeRouter, type EdgeRouterInstance } from "./reverseproxy/edge";
// `DEFAULT_HOST_DOMAIN` was imported here for the edge's `hostDomain`, removed 2026-09-26.
import { loadConfig } from "./workspace/config";
import { bus, wireLogEventsToBus } from "./kernel/eventBus";
import { preflight, bootstrapWorkspaces, type StartupCheck } from "./lifecycle/startup";
import { shutdownEngine } from "./lifecycle/shutdown";
import { startBlobSweeper } from "./workspace/blob/sweep";
import { commandRegistry, type CommandRegistry } from "./kernel/registry";
import { EventLog } from "./transport/eventLog";

// ─────────────────────────────────────────────────────────────────────────────
// Public API
// ─────────────────────────────────────────────────────────────────────────────

export interface EngineOptions {
  /**
   * Absolute path to the data root. Required, and the engine never invents one — `store/paths.ts`
   * throws `DataRootNotInitialisedError` rather than falling back to the cwd, because a silent
   * fallback would write the user's workspaces somewhere surprising. The Electron shell passes
   * Electron's per-user `userData` directory; a CLI passes `--data-dir`.
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
  /**
   * The **forward proxy**'s port — the per-workspace half (`plan/15` §3.4) — or `null` when it is not
   * running.
   *
   * **It is not `settings.port`, and that is work item 3.** `settings.port` is the *edge's* port (see
   * `edgePort`); the forward proxy's port is resolved from D16's base with the edge's port reserved, so
   * the two never collide — which on Windows they otherwise would, since the proxy base and the
   * settings port are both `80`.
   */
  proxyPort: number | null;
  /** Whatever the proxy reported on a failed bind, if anything. */
  proxyError: string | null;
  /**
   * The **edge router**'s port — the host-level half — or `null` when it is not running.
   *
   * This is `settings.port`, which is the migration `plan/15` §6.2 promises: before the split that one
   * socket served both halves, so a mapping's URL was `*.localhost:<settings.port>`. Handing the edge
   * the same port is what keeps every existing mapping working with no URL edited.
   */
  edgePort: number | null;
  /**
   * Whatever the edge reported on a failed bind, if anything.
   *
   * Separate from `proxyError` rather than folded into it, because the two failures mean different
   * things: a proxy that failed to bind costs one workspace its forward path, and an edge that failed
   * to bind costs **every** mapping on the host — the reverse path is not one of N.
   */
  edgeError: string | null;
  /** The **effective** settings after bootstrap — `activeWorkspaceId` may have been repaired. */
  settings: AppSettings;
  /** Workspace ids whose auto-sync poller was started by this `start()`. */
  autoSyncStarted: string[];
}

export interface Engine {
  /**
   * Resolve the data root, run preflight, bootstrap workspaces, then bind the edge and the proxy
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
  /**
   * The engine's **event log** — one `seq` authority and one bounded ring buffer for the whole
   * process, created here rather than per transport (P4 work item 3).
   *
   * The scope is the point. `plan/05`'s own `hello` sketch presents a `lastSeq` with no session id to
   * disambiguate it, so the number is only meaningful if it belongs to the engine: a reconnect is a
   * *new session*, and a per-session counter would give the same number two different meanings across
   * one. So this is created once, handed to every transport `createEngine()`'s caller builds, and
   * closed by `stop()` — after which no transport may use it.
   *
   * Retention is **lazy**: nothing is retained, and no bus listener is attached, until something
   * subscribes to an event name. An engine that never uses events pays nothing for this.
   */
  readonly log: EventLog;
}

/**
 * Poll until a fire-and-forget server reports ready, or reports an error, or the deadline passes.
 *
 * Polling rather than an event because neither `startServer()` nor `edge.start()` exposes its
 * `net.Server` — the only readiness signal they publish is the predicate passed here. 10 ms is well
 * below the cost of a socket bind, so this adds no measurable latency in practice; the deadline
 * exists so a hung bind cannot hang `start()` forever.
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
  /**
   * The one edge router for this engine. `null` before `start()`.
   *
   * Per engine **instance**, like the log wiring and the sweeper, and for the same reason: it holds a
   * listener and must not outlive the object that created it.
   */
  let edge: EdgeRouterInstance | null = null;
  /**
   * P3 work item 1 — the blob TTL sweep. Owned by the lifecycle rather than by a caller because
   * an unswept blob root only grows: in Docker (P9) that is a volume that fills up forever, and
   * the whole point of the sweep is that nobody has to remember it exists.
   *
   * The timer is unref'd (`startBlobSweeper()`), so this is housekeeping, not a reason for the
   * process to stay alive.
   */
  let stopBlobSweeper: (() => void) | null = null;

  /**
   * P6 finding 1 — the log-events wiring, per engine instance for the same reason the sweeper is.
   *
   * `wireLogEventsToBus()` subscribes to the process-wide `logEmitter`, so an engine that is created
   * and then discarded without this would leave three listeners attached to a singleton it no longer
   * owns — and the next engine in the same process would find them already there. Wired in
   * `doStart()` (the proxy, the only emitter, starts there) and detached in `stop()`.
   */
  let stopLogWiring: (() => void) | null = null;

  /**
   * P4 work item 3 — the event log, one per engine instance.
   *
   * Created eagerly rather than on first use because it is free until something subscribes (retention
   * is lazy), and because a lazily-created shared object is a race: two transports asking for it at
   * once would each get their own, which is exactly the split-brain the engine-scoped counter exists
   * to prevent. Created **before** `start()` so a transport built from this engine can subscribe even
   * if the proxy never comes up.
   */
  const log = new EventLog({ bus });

  async function doStart(): Promise<EngineStatus> {
    // Must come first: every store module resolves through the data root.
    setDataRoot(opts.dataDir);

    // After `setDataRoot()` — the sweep resolves its root through it. It deliberately does not
    // create the directory, so a fresh install sweeps nothing until the first `blob.put`.
    stopBlobSweeper = startBlobSweeper();

    settings = loadSettings();

    // Advisory only — see `EngineOptions.onPreflightWarning`.
    const checks = await preflight({
      dataDir: dataDir(),
      ports: [
        // Named for the edge rather than the proxy because `settings.port` **is** the edge's port as
        // of work item 3. The forward proxy's port is resolved after this, from D16's base, so there
        // is nothing to check here for it.
        { name: "edge", port: settings.port },
        { name: "webhook", port: settings.webhookPort ?? 9101 },
      ],
    });
    for (const check of checks) if (!check.ok) warn(check);

    const boot = await bootstrapWorkspaces(settings);
    settings = boot.settings;
    autoSyncStarted = boot.autoSyncStarted;

    // P6 finding 1. The three log events reach the bus only if something wires them, and the proxy
    // started two lines below is the only thing that emits them — so this must happen before
    // `startServer()`, or a bind error's `server.error` would be the one event that got away.
    //
    // Wired here and **not only** in the shell's `registerIpcHandlers()` so that P7's web UI, P8's CLI
    // and P9's Docker image inherit it from the lifecycle instead of each re-solving it. That is the
    // whole reason the function lives in the engine rather than in the shell.
    stopLogWiring = wireLogEventsToBus(bus);

    /**
     * ## The edge first, then the proxy — `plan/15` §3.3, work item 3
     *
     * The order is load-bearing in both directions.
     *
     * **The edge takes `settings.port`**, which is the migration §6.2 promises: that port is where
     * every existing `*.localhost` mapping already points, so the reverse half keeps answering exactly
     * where it did before the split and no mapping is edited.
     *
     * **The forward proxy is then resolved with the edge's port reserved.** D16's rule is *base →
     * scan upward*, and on Windows the proxy base and the settings port are both `80` — so without the
     * reservation the scan would hand the proxy the very port the edge holds, and the failure would be
     * an `EADDRINUSE` that looked like a busy machine.
     *
     * `resolveConfig` reads `loadConfig()` — the **active** workspace's config — and that is the right
     * answer *for this shape*: `createEngine()` is the single-workspace facade, so the active workspace
     * is the only one there is. A multi-workspace host resolves it from its `runtimes` map instead
     * (`runtime/host.ts`), because there the answer differs per workspace.
     */
    edge = createEdgeRouter({
      resolveConfig: () => loadConfig(),
      // No `hostDomain`: the edge stopped taking its own name on 2026-09-26, when the web GUI's global
      // mapping went. `EdgeRouterOptions` carries the reasoning — the only branch that asked the question was
      // the one this removal deleted.
    });
    /**
     * ## `plan/17` §5 work item 7 — the edge is *named* to the module-level binding
     *
     * The forward proxy needs no equivalent call here, and the asymmetry is the design: `startServer()` acts
     * on `proxy/server.ts`'s **lazy default instance**, which *is* this engine's proxy, so a command can reach
     * it without being told. The edge is per-engine state created on the line above, so the only way a command
     * can find it is for this composition to say so.
     *
     * **Bound before `start()`, not after.** A `config.save` arriving during the boot window would otherwise
     * find no edge and skip its re-bind silently; binding first makes the worst case a `start()` on a listener
     * that has not bound yet — the same fire-and-forget call the next line already makes.
     *
     * `stop()` unbinds it. That is not tidiness: the binding is **process-global** module state, so a leaked one
     * would leave the next consumer in this process re-binding a dead edge — the leak `runtime/host.ts`
     * documents for the other two facades, and which `facadeBinding.integration.test.ts`'s teardown records the
     * rule against.
     */
    bindEdgeInstance(edge);
    edge.start(settings.port);
    /**
     * ## The forward proxy's port is a **setting**, not a scan result — `plan/17` §1
     *
     * This used to be `findFreePortFrom(proxyBasePort(), "proxy", new Set([settings.port]))`: D16's
     * base-and-scan, with the edge's port reserved so the scan could not hand the proxy the port the edge
     * holds (on Windows both were `80`, so without the reservation the failure was an `EADDRINUSE` that read
     * like a busy machine). `plan/17` §1 removed the scan for this port — *"it binds `8080` and **fails if
     * occupied**, naming the port, so the user enters a working one instead of the app silently
     * incrementing"* — because a browser's proxy setting names it, so a port that moves on its own is a broken
     * browser rather than a degraded mode. There is nothing left to reserve and nothing to scan for.
     *
     * **A busy port is not fatal here**, and that is the engine's oldest bind contract rather than an
     * oversight: `startServer()` hands off to an async `listen()` whose failure lands in the instance's
     * `lastError`, which `server.status` reports. `runtime/host.ts` step 3b states the same reasoning for the
     * deployable host, and adds the notice that makes the failure impossible to miss.
     */
    startServer(settings.forwardProxyPort);

    // `startServer()` is fire-and-forget — it hands off to an async `listen()` and returns
    // immediately, so `isRunning()` is still false on the very next line. Resolving `start()` before
    // the sockets are actually accepting would make the one promise a caller has to await tell them
    // nothing, so wait for readiness here.
    //
    // A bind failure is *not* thrown: the proxy already reports it through `bus` and
    // `status().proxyError`, and the shell has always tolerated a busy port. Waiting on the error
    // as well as on success keeps `start()` bounded either way.
    await waitUntilReady(isRunning, getServerError);
    // The edge is fire-and-forget too, and `status().edgePort` is asserted by the smoke test
    // immediately after `start()` resolves — so it is waited on the same way. There is no
    // `getEdgeError` predicate to pass: a failed bind is reported through `status().edgeError`, and
    // the deadline bounds the wait either way.
    await waitUntilReady(() => edge?.isRunning() === true, () => null);

    return status();
  }

  function status(): EngineStatus {
    const proxyUp = isRunning();
    return {
      running: proxyUp,
      dataDir: isDataRootSet() ? dataDir() : opts.dataDir,
      proxyPort: proxyUp ? getPort() : null,
      proxyError: getServerError(),
      // Follows `proxyPort`'s rule rather than a new one: a port is reported only while the listener
      // is up, so a stopped engine reports `null` rather than a stale number. `edgeError` stays
      // readable either way — a stopped edge's last failure is still the reason it is not serving.
      edgePort: edge !== null && edge.isRunning() ? edge.getPort() : null,
      edgeError: edge?.getError() ?? null,
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
      // Stopped here rather than inside `shutdownEngine()`: that function is memoised and shared
      // with the shell's `before-quit`, so it can only ever run once — but the sweeper is
      // per-engine-instance and its lifetime is this object's, not the process's.
      stopBlobSweeper?.();
      stopBlobSweeper = null;
      // Same reasoning again: the log wiring is per-instance, so it is detached here rather than in
      // the memoised `shutdownEngine()`. Left attached, it would keep forwarding into a bus whose
      // engine has stopped, and would double up if another engine were ever created in this process.
      stopLogWiring?.();
      stopLogWiring = null;
      // Same reasoning as the sweeper: the log is per-instance, so it is closed here rather than in
      // the memoised shutdown. Closing it detaches the retention listeners, which is what stops the
      // engine numbering events after it has been told to stop.
      log.close();
      // The edge is per-instance state like the sweeper and the log wiring, so it is torn down here
      // rather than in the memoised `shutdownEngine()`. Stopped **before** `shutdownEngine()` so a
      // request arriving mid-teardown finds a closed edge rather than a live one in front of a
      // stopped proxy — the ordering `runtime/host.ts` gives the same reason.
      edge?.stop();
      /**
       * **…and the binding goes with it.** `bindEdgeInstance()` is process-global module state, so an engine
       * that bound and did not unbind would leave the next consumer in this process re-binding *this*
       * engine's stopped edge. One engine per process is the design, so the CLI never pays for it — the test
       * worker does, and it is the same leak `runtime/host.ts` documents for the proxy and the webhook.
       */
      bindEdgeInstance(null);
      edge = null;
      await shutdownEngine();
    },

    status,
    registry: commandRegistry,
    bus,
    log,
  };
}

// ─────────────────────────────────────────────────────────────────────────────
// P4 — the transport layer
// ─────────────────────────────────────────────────────────────────────────────

/**
 * Deliberately above the provisional divider: the transport is the engine's **permanent** client
 * surface, not an interim deep-import shim. `Transport` is what P5's typed client, P7's web UI, P8's
 * CLI and P9's Docker image all speak, so it must not be filed under "removed at P6".
 *
 * `createInProcessTransport(registry)` composes the two things `createEngine()` already returns,
 * which is why the factory takes a registry rather than reaching for the `commandRegistry` singleton:
 * a transport that hard-coded the singleton could not be pointed at a test's own registry, and the
 * conformance suite needs exactly that.
 *
 * `createWsServer` / `createWsTransport` are the same contract over a socket. The server binds
 * **loopback by default and refuses a non-loopback bind without both an explicit opt-in and TLS**
 * (work item 5), so exporting it does not export a remotely-reachable surface — see `transport/ws.ts`.
 *
 * `createSocketServer` / `createSocketTransport` / `defaultSocketPath` are the same contract over a
 * unix domain socket or a Windows named pipe — D3's decided transport for the desktop shell (P6). Its
 * access control is the socket file's `0600` mode, asserted before `createSocketServer()` resolves, so
 * it too exports no surface another local user can reach. See `transport/socket.ts`.
 *
 * See `packages/engine/src/transport/types.ts` for the two decisions that matter — that `request()`
 * resolves with a handler's value *uninspected* (so `{ok:false}` stays data, keeping `window.api`
 * byte-identical), and that bus event names are bridged to wire names in one validated place.
 */
export { createInProcessTransport, type InProcessTransportOptions } from "./transport/inProcess";
export {
  DEFAULT_MAX_AGE_MS,
  DEFAULT_MAX_EVENTS,
  EventLog,
  type EventLogOptions,
  type LoggedEvent,
  type ReplayOutcome,
} from "./transport/eventLog";
export type { EngineEvent, Transport, TransportKind } from "./transport/types";
export {
  BUS_EVENTS_NOT_ON_THE_WIRE,
  BUS_NAME_BY_WIRE_NAME,
  WIRE_NAME_BY_BUS_NAME,
  assertBridgeIsTotal,
  wireNameFor,
} from "./transport/types";
export { createAuthenticatedTransport } from "./transport/auth/authenticated";
export {
  CLOSE_CODE_POLICY,
  createWsServer,
  createWsTransport,
  DEFAULT_WS_HOST,
  type WsServer,
  type WsServerAuthOptions,
  type WsServerOptions,
  type WsTransportOptions,
} from "./transport/ws";
export {
  createSocketServer,
  createSocketTransport,
  defaultSocketPath,
  type SocketServer,
  type SocketServerAuthOptions,
  type SocketServerOptions,
  type SocketTransportOptions,
} from "./transport/socket";

// ─────────────────────────────────────────────────────────────────────────────
// Provisional surface — removed at P6 (see the header)
// ─────────────────────────────────────────────────────────────────────────────

export * as paths from "./workspace/paths";
export * as types from "./workspace/types";
export * as appSettings from "./workspace/appSettings";
export * as config from "./workspace/config";
export * as workspaceFs from "./workspace/workspaceFs";
export * as gitStore from "./workspace/gitStore";
export * as subscription from "./workspace/entityCount";
export * as randomNames from "./utils/randomNames";
export * as randomizer from "./utils/randomizer";
export * as eventBus from "./kernel/eventBus";
export * as proxy from "./proxy/server";
export * as webhookServer from "./webhooks/webhookServer";
// P14 WI 3 — the workspace **host**: the composition a deployable unit runs. It owns one listener per port
// for the whole host — the edge, the forward proxy, the webhook receiver and the RPC endpoint — plus the
// signal handling.
//
// **`runtime` (the `WorkspaceRuntime` namespace) and `hostRegistry` (the `runtime.json` namespace) were both
// exported here until `plan/17`, and both are gone.** `WorkspaceRuntime` composed exactly two things — a
// forward proxy and a webhook receiver — and `plan/17` §1 moved both to the host, so it owned no listener and
// had no subject left. It was **deleted rather than hollowed** (owner decision, 2026-09-25). `hostRegistry`
// remembered *which port each workspace landed on*, which stopped being a question the moment the ports
// stopped varying by workspace. `plan/17` §5 work items 1 and 9.
export * as host from "./runtime/host";
// P14 WI 3 — the engine's **whole** command surface in one call, for a consumer with no shell.
// `createEngine()` registers nothing by design (P3: "the consumer declares what it is willing to serve"),
// and until a headless host existed the only declaration was the Electron shell's `registerIpcHandlers()`.
export { registerAllEngineCommands, registerAllEngineKinds } from "./registerAll";
// `plan/17` §5 work item 2 — what is left of D16's allocator: **finding** ports, not allocating them per
// workspace. The per-workspace machinery (`allocatePorts`, `PortUnavailableError`,
// `PortRangeExhaustedError`, `AllocatedPorts` and its provenance flags) was deleted along with the model that
// needed it. One finder remains — `findFreePortFrom`, for the single companion port (Electron's path).
export * as ports from "./runtime/portAllocator";
export * as sync from "./gitops/syncManager";
export * as applications from "./applications/processSpawner";
export * as commands from "./kernel/registry";
export * as startup from "./lifecycle/startup";
export * as shutdown from "./lifecycle/shutdown";
export * as blob from "./workspace/blob/store";
export * as blobSweep from "./workspace/blob/sweep";
export * as importExport from "./importexport/registry";
export * as importExportCommands from "./importexport/commands";
