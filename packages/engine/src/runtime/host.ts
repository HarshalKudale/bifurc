/**
 * P14 work item 3 — the **workspace host**: the composition a deployable unit actually runs.
 *
 * ## What changed, in one paragraph
 *
 * `plan/14` D10/D11 move the deployable unit from the engine to the **workspace**. An `Engine` owns a data
 * root, bootstraps workspaces and binds one proxy; a **host** owns N `WorkspaceRuntime`s, binds *one* RPC
 * endpoint for all of them (D13) and never binds the companion server at all. The engine is unchanged and
 * stays a library — this file is a new *entry point* that composes the pieces WI 2 and WI 3 built, which is
 * why it introduces no new abstraction and owns almost no state.
 *
 * ## Why this lives in `packages/engine` rather than a new package
 *
 * `plan/14` WI 3 flags it as the phase's highest-risk decision and recommends this answer: `tsup` runs
 * `bundle: false` specifically so `store/paths.ts` and `applications/processSpawner.ts` stay **one module
 * instance per process**, and a second package could silently duplicate the data root. A host in its own
 * package would be a second copy of `_dataRootOverride` waiting to happen. So it is a module here, which
 * also means it shares the engine's single `dataDir` by construction rather than by discipline.
 *
 * ## What the host does, in order
 *
 * 1. `setDataRoot(opts.dataDir)` — **first**, because every store module resolves through it.
 * 2. `loadSettings()` for the workspace list, then `bootstrapWorkspaces()` — the same sequence
 *    `Engine.start()` runs, reusing the engine's own functions rather than reimplementing them (the
 *    alternative is two bootstrap paths that drift).
 * 3. For each workspace, `allocatePorts()` (D16, honouring the registry's remembered ports and overriding
 *    with its `preferred*` values) then `createWorkspaceRuntime(...)`, started **one at a time** — the
 *    allocation is a real bind, so two concurrent allocators could both see the same port free.
 * 4. One `createSocketServer()` (or `createWsServer()`) over the engine's registry, with auth from
 *    `transport/auth/` — `ensureEngineToken()` supplies the token and `ENGINE_TOKEN_ENV_VAR` lets a
 *    container mount its own.
 * 5. `SIGTERM` / `SIGINT`: stop every runtime, close the RPC endpoint, remove the socket, exit.
 *
 * ## Two things it deliberately does not do
 *
 * - **It never starts the companion server.** That is `plan/14` WI 3's acceptance criterion, and the
 *   reason `EngineOptions.startCompanion` exists. A host that bound the companion would make N
 *   per-workspace runtimes fight over one app-level port — the companion is a module-level singleton by
 *   design, so the host simply does not reach it.
 * - **It never calls `createEngine()`.** The engine's lifecycle owns the proxy through a facade over one
 *   default instance (`proxy/server.ts`), so an engine *and* N runtimes in one process would have two
 *   authorities over the same servers. The host uses the factories directly and leaves the engine's
 *   lifecycle to the desktop.
 *
 * ## `SIGTERM` is why the returned handle exists
 *
 * `createWorkspaceHost()` does **not** install signal handlers itself — `runWorkspaceHost()` does. The
 * distinction is deliberate: a caller embedding a host in a larger process owns its own signals, and a
 * library that re-pointed `SIGTERM` behind its back would break that caller's shutdown. The handler is a
 * five-line wrapper, so the composition stays testable without a signal ever being raised.
 */
import { createSocketServer, defaultSocketPath, type SocketServer } from "../transport/socket";
import { createWsServer, DEFAULT_WS_HOST, type WsServer } from "../transport/ws";
import { commandRegistry, type CommandRegistry } from "../commands/registry";
import { registerAllEngineCommands } from "../registerAll";
import { bus as defaultBus, type EngineEventBus } from "../eventBus";
import { ensureEngineToken } from "../transport/auth/token";
import { loadSettings, appDataDir, type AppSettings } from "../store/appSettings";
import { bootstrapWorkspaces } from "../startup";
import { setDataRoot, dataDir } from "../store/paths";
import { EventLog } from "../transport/eventLog";
import { createWorkspaceRuntime, type WorkspaceRuntime, type WorkspaceRuntimeStatus } from "./workspaceRuntime";
import { clearRuntimes, registerRuntime } from "./runtimeRegistry";
import { allocatePorts, PortUnavailableError, PortRangeExhaustedError } from "./portAllocator";
import {
  loadRuntimeRegistry,
  saveRuntimeRegistry,
  entryFor,
  recordResolvedPorts,
  type RuntimeRegistry,
} from "./hostRegistry";

/** How the RPC endpoint is exposed. D13: **one** endpoint per host, not one per workspace. */
export type HostTransport =
  | {
      kind: "socket";
      /** Defaults to `defaultSocketPath()`. Windows: a named pipe; POSIX: a `0600` unix socket. */
      path?: string;
    }
  | {
      kind: "ws";
      /** Defaults to `DEFAULT_WS_HOST` (loopback). A non-loopback bind additionally needs `tls` +
       * `allowNonLoopback` — `ws.ts`'s own guards, which this host does not weaken or bypass. */
      host?: string;
      port?: number;
    };

export interface WorkspaceHostOptions {
  /** The data root. Required, and never invented — see `store/paths.ts`. */
  dataDir: string;
  /** Overrides the workspace list. When absent it comes from `app.json`, after bootstrap repair. */
  workspaceIds?: string[];
  /** How to expose RPC. Defaults to `{ kind: "socket" }` — D3's decided desktop transport. */
  transport?: HostTransport;
  /** An explicit token, for a caller that already has one. Defaults to `ensureEngineToken(dataDir)`,
   * which honours `BIFURC_ENGINE_TOKEN` and otherwise generates and writes a fresh one. */
  token?: string;
  /** Reported to clients in `hello`. Defaults to the `@bifurc/protocol` version, as the transports do. */
  engineVersion?: string;
  bus?: EngineEventBus;
  registry?: CommandRegistry;
  /** Notified once per workspace whose ports had to be moved off the base. **Not** an error — see below. */
  onNotice?: (message: string, workspaceId?: string) => void;
}

export interface WorkspaceHostStatus {
  dataDir: string;
  /** The RPC endpoint's address: the socket path, or the `ws://` URL. */
  rpcAddress: string;
  /** The workspace runtimes the host owns, in start order. */
  runtimes: WorkspaceRuntimeStatus[];
  /**
   * Workspaces in the settings list that the host did **not** start, with the reason. A workspace whose
   * override port is taken lands here rather than stopping the host — see `startAll()`.
   */
  skipped: { workspaceId: string; reason: string }[];
}

export interface WorkspaceHost {
  start(): Promise<WorkspaceHostStatus>;
  /** Stop every runtime and close the RPC endpoint. Idempotent. */
  stop(): Promise<void>;
  status(): WorkspaceHostStatus;
  /** The runtimes, keyed by workspace id — so a caller can reach one workspace's proxy (WI 4, WI 7). */
  readonly runtimes: ReadonlyMap<string, WorkspaceRuntime>;
  /** The one RPC endpoint for this host. `null` before `start()`. */
  readonly rpc: SocketServer | WsServer | null;
}

export function createWorkspaceHost(opts: WorkspaceHostOptions): WorkspaceHost {
  const bus = opts.bus ?? defaultBus;
  const registry = opts.registry ?? commandRegistry;
  const notice = opts.onNotice ?? ((message: string) => console.log(`[host] ${message}`));

  const runtimes = new Map<string, WorkspaceRuntime>();
  let rpc: SocketServer | WsServer | null = null;
  let skipped: { workspaceId: string; reason: string }[] = [];
  let rpcAddress = "";
  let startPromise: Promise<WorkspaceHostStatus> | null = null;
  let stopped = false;

  function status(): WorkspaceHostStatus {
    return {
      dataDir: dataDir(),
      rpcAddress,
      runtimes: [...runtimes.values()].map((r) => r.status()),
      skipped,
    };
  }

  /** The log is per host, not per runtime: `seq` is only meaningful across a reconnect if one authority
   * issues it (`transport/eventLog.ts`), and D13 puts one RPC endpoint in front of all the runtimes. */
  const log = new EventLog({ bus });

  async function doStart(): Promise<WorkspaceHostStatus> {
    // ── 0. register the command surface, before anything can be asked to serve it ─
    /**
     * **The step this file's first integration run proved was missing.**
     *
     * `createEngine()` registers nothing by design — P3's *"the consumer declares what it is willing to
     * serve"* — and for the desktop that consumer is the shell's `registerIpcHandlers()`. A host has no
     * shell, so without this the RPC endpoint it just bound answers `UNKNOWN_COMMAND` for `config.get`: a
     * command the protocol declares and `@bifurc/client` advertises. It is the §5c bug class one layer up,
     * and it was invisible until a non-shell consumer existed.
     *
     * `opts.registry` is honoured as-is when a caller supplies one — a caller with its own registry has
     * already decided what goes in it, and registering into it would double-register and throw.
     */
    if (opts.registry === undefined) registerAllEngineCommands(registry);

    // ── 1. the data root, first ──────────────────────────────────────────────
    setDataRoot(opts.dataDir);

    // ── 2. settings, then the engine's own bootstrap ─────────────────────────
    // `bootstrapWorkspaces()` is reused rather than reimplemented: it creates each workspace's dirs and repo,
    // starts auto-sync and repairs a stale `activeWorkspaceId`. A host with its own copy of that sequence is
    // how the desktop and the container end up with different on-disk states.
    let settings: AppSettings = loadSettings();
    if (opts.workspaceIds !== undefined) {
      /**
       * **A requested workspace that is not in the settings file is named, not silently dropped.**
       *
       * This is the "accepted and ignored" failure again, one layer out: `--workspace X` where `X` does not
       * exist filtered the list to *empty*, the host came up serving nothing, and it exited 0. Measured on
       * 2026-09-21 while verifying the CLI artifact — the cause was that `--data-dir` had **not** redirected
       * the settings file, so the requested id was absent from a list the operator never saw. Without this
       * notice, the only symptom is `serving 0 workspace(s)`, which looks like an empty data root rather than
       * a settings file read from somewhere else.
       *
       * The directory is included because **that is the diagnostic**: on Windows `settingsPath()` prefers
       * `%LOCALAPPDATA%` regardless of `setDataRoot()` (recorded in `proxy/certManager.ts`), so a
       * `--data-dir` run reads the user's real settings. Naming the location turns that into one readable
       * line. Not fatal — the other requested workspaces may still be serveable, which is the same
       * "skip loudly, do not refuse to start" rule D16 applies to a taken port override.
       */
      const known = new Set(settings.workspaces.map((w) => w.id));
      for (const id of opts.workspaceIds) {
        if (!known.has(id)) {
          notice(
            `requested workspace "${id}" is not in the settings (looked in ${appDataDir()}) — ` +
              `it will not be served`,
          );
        }
      }
      settings = { ...settings, workspaces: settings.workspaces.filter((w) => opts.workspaceIds!.includes(w.id)) };
    }
    const boot = await bootstrapWorkspaces(settings);
    settings = boot.settings;

    // ── 3. the runtime registry, and the token ───────────────────────────────
    const runtimeRegistry: RuntimeRegistry = loadRuntimeRegistry();
    const token = opts.token ?? (await ensureEngineToken(dataDir())).token;

    // ── 4. one runtime per workspace, allocated and started in sequence ──────
    const reserved = new Set<number>();
    skipped = [];
    for (const ws of settings.workspaces) {
      const entry = entryFor(runtimeRegistry, ws.id);
      let ports;
      try {
        ports = await allocatePorts({
          // D16's three sources, in order. The override is the user's explicit choice — the fail-loudly path.
          preferredProxyPort: entry.preferredProxyPort ?? null,
          preferredWebhookPort: entry.preferredWebhookPort ?? null,
          /**
           * **D16's stickiness**, passed as its own input rather than as a preference — and that distinction
           * is the whole point. A workspace that landed on 81 because 80 was busy **keeps 81** once 80 frees
           * up, because a stable proxy URL is worth more to the browser-proxy workflow than reclaiming the
           * base.
           *
           * The remembered port is a *record of what happened*, not a request, so `allocatePorts` treats it
           * **softly**: it is taken when free and skipped when not. Passing it as a `preferred*` value would
           * turn a port some other process has since taken into a hard startup failure — the "refused to
           * start" mode D16 forbids. Until 2026-09-22 the allocator had no soft input, so the only way to
           * pass this was as a preference, and this call passed nothing instead; that is what `plan/14` §8
           * criterion 6b records.
           */
          rememberedProxyPort: entry.resolvedProxyPort ?? null,
          rememberedWebhookPort: entry.resolvedWebhookPort ?? null,
          reserved,
        });
      } catch (err) {
        /**
         * A workspace the host cannot place is **skipped, not fatal** — with one exception.
         *
         * `PortUnavailableError` means a *user's override* was taken. D16 says that fails loudly, so it does:
         * loudly, by name, and for that workspace. It does not stop the other workspaces from serving, because
         * a host that refuses to start at all turns one workspace's bad setting into an outage for every
         * other one. `PortRangeExhaustedError` is different in kind — the whole range is gone, so no
         * workspace after this one can be placed either, and it is re-thrown.
         */
        if (err instanceof PortRangeExhaustedError) throw err;
        const reason = err instanceof PortUnavailableError ? err.message : String(err);
        skipped.push({ workspaceId: ws.id, reason });
        notice(`workspace "${ws.name}" (${ws.id}) was not started: ${reason}`, ws.id);
        continue;
      }

      reserved.add(ports.proxyPort);
      reserved.add(ports.webhookPort);

      const runtime = createWorkspaceRuntime({
        workspaceId: ws.id,
        proxyPort: ports.proxyPort,
        webhookPort: ports.webhookPort,
        // The CA is one mkcert root for the whole machine and outlives any single runtime — so no member of a
        // multi-workspace host unloads it. See `createProxyServer()`'s header.
        ownsCA: false,
      });
      runtimes.set(ws.id, runtime);
      /**
       * Register **before** `start()`, deliberately. A command that arrives while this runtime is still
       * binding must resolve to *this* instance rather than fall through to the facade — otherwise the
       * window between `runtimes.set` and `start()` is one where `server.status` would report the active
       * workspace's server instead of this one, which is the exact class of wrong-target answer WI 7
       * removes. The registry is a lookup, not a health check: `isRunning()` is how a caller asks whether
       * it has come up.
       */
      registerRuntime(runtime);

      const st = await runtime.start();
      recordResolvedPorts(runtimeRegistry, ws.id, {
        proxyPort: st.proxyPort,
        webhookPort: st.webhookPort,
      });

      /**
       * Landing on `base + 1` is **the designed path, not a degradation** — `plan/14` §4 is explicit that it
       * must not be warned about as if something failed. So this is informational and names the workspace;
       * the LOUD case (an override that could not be honoured) already happened above, where it became a
       * `skipped` entry.
       */
      notice(
        `workspace "${ws.name}" is serving: proxy ${ports.proxyPort}, webhook ${ports.webhookPort}`,
        ws.id,
      );
    }

    // One write for the whole start, after every runtime has reported — so a crash mid-start cannot record a
    // port for a workspace that never bound.
    saveRuntimeRegistry(runtimeRegistry);

    // ── 5. the one RPC endpoint, for every runtime (D13) ─────────────────────
    const transport = opts.transport ?? { kind: "socket" };
    const auth = {
      token,
      engineVersion: opts.engineVersion ?? "0.0.0",
      // Reported so an operator can see *why* a host refused a session, which over a plain socket is otherwise
      // unattributable: `socket.ts` has no peer address to record, and a refused client is disconnected with
      // nothing but the `UNAUTHORIZED` reply to go on. The `hello` is included because a token mismatch and a
      // protocol mismatch are the same code and different problems.
      onAuthFailure: (error: { code: string; message: string }, hello: { clientName?: string }) =>
        notice(`rpc session refused (${error.code}) from "${hello.clientName ?? "unnamed"}": ${error.message}`),
    };

    if (transport.kind === "socket") {
      const path = transport.path ?? defaultSocketPath();
      rpc = await createSocketServer({ path, registry, bus, log, auth });
      rpcAddress = path;
    } else {
      rpc = await createWsServer({
        registry,
        bus,
        log,
        auth,
        host: transport.host ?? DEFAULT_WS_HOST,
        port: transport.port ?? 0,
      });
      rpcAddress = rpc.url;
    }
    notice(`rpc endpoint: ${rpcAddress}`);

    return status();
  }

  return {
    start(): Promise<WorkspaceHostStatus> {
      if (stopped) {
        return Promise.reject(
          new Error("createWorkspaceHost(): this host has already been stopped. Create a new one."),
        );
      }
      startPromise ??= doStart();
      return startPromise;
    },

    async stop(): Promise<void> {
      if (stopped) return;
      stopped = true;
      // The endpoint first: closing it stops new sessions before the servers behind it go away, so a client
      // cannot get a request in against a runtime that is mid-teardown.
      if (rpc !== null) {
        try {
          await rpc.close();
        } catch {
          // Best-effort — a socket file that another process already removed must not block the shutdown.
        }
      }
      // Then every runtime. Sequential rather than `Promise.all`: each `stop()` closes its servers, and a
      // burst of concurrent closes buys nothing while making an error in one harder to attribute.
      for (const runtime of runtimes.values()) {
        await runtime.stop();
      }
      runtimes.clear();
      // And the command layer's lookup, or a stopped host would leave `server.status` answering for
      // workspaces it no longer serves — a stale answer of the same kind `currentPort` surviving `stop()`
      // produces one layer down (`proxy/server.ts`'s recorded trap).
      clearRuntimes();
      // The host owns the log (one authority for `seq`), so it closes it.
      log.close();
    },

    status,
    runtimes,
    get rpc(): SocketServer | WsServer | null {
      return rpc;
    },
  };
}

/**
 * Create a host and wire `SIGTERM` / `SIGINT` to its teardown, then resolve once it is serving.
 *
 * Separate from `createWorkspaceHost()` on purpose — see the header's last section. A container's `ENTRYPOINT`
 * wants exactly this; a test or an embedding caller wants the composition without its signals re-pointed.
 *
 * `stop()` is called on the signal, then the process exits with a code that says which signal arrived
 * (`128 + signo`, the shell convention). The exit is explicit rather than left to the event loop draining:
 * a leaked handle anywhere in the engine would otherwise keep a stopped container alive, and "the container
 * will not die" is a worse failure than an early exit.
 */
export async function runWorkspaceHost(
  opts: WorkspaceHostOptions,
): Promise<WorkspaceHost> {
  const host = createWorkspaceHost(opts);
  let shuttingDown = false;

  const shutdown = (signal: NodeJS.Signals): void => {
    if (shuttingDown) return;
    shuttingDown = true;
    void host
      .stop()
      .catch((err) => console.error(`[host] shutdown failed: ${String(err)}`))
      .finally(() => process.exit(signal === "SIGINT" ? 130 : 143));
  };

  process.on("SIGTERM", () => shutdown("SIGTERM"));
  process.on("SIGINT", () => shutdown("SIGINT"));

  await host.start();
  return host;
}
