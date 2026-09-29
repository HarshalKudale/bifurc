/**
 * P14 work item 3 — the **workspace host**: the composition a deployable unit actually runs.
 *
 * ## What changed, in one paragraph
 *
 * `plan/14` D10/D11 move the deployable unit from the engine to the **workspace**. An `Engine` owns a data
 * root, bootstraps workspaces and binds one proxy; a **host** owns N `WorkspaceRuntime`s, binds *one* RPC
 * endpoint for all of them (D13). It binds no app-level singleton server of its own — see the criterion
 * note below. The engine is unchanged and
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
 * 4. One `createSocketServer()` (or `createWsServer()`) over the engine's registry, with the **identity**
 *    half of `transport/auth/` — the `hello` handshake, per-session scopes and the replay decision. **Not a
 *    token:** it was deleted on 2026-09-25 with the remote deployment it guarded, and the loopback bind plus
 *    `transport/ws.ts`'s `Origin` check are what stand in its place. The step that refused a published bind
 *    went with it, for the same reason — there is no longer a published bind to refuse (step -1 below).
 * 5. `SIGTERM` / `SIGINT`: stop every runtime, close the RPC endpoint, remove the socket, exit.
 *
 * ## Two things it deliberately does not do
 *
 * - **It never starts an app-level singleton server.** That is `plan/14` WI 3's acceptance criterion, and
 *   today the singleton in question is the **edge router** (`plan/15` §3.3): it is host-level and
 *   module-level by design, so N per-workspace runtimes reaching it would have N owners of one listener.
 *   The host composes runtimes, which are per-workspace and own exactly their own two ports.
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
// The host's own name: the value an engine that was not told one answers as. Taken from `lib/net` rather than
// from `../transport/ws` so that "what is this hostname?" does not read as a transport question. Same
// reasoning `hostArgs.ts` records.
//
// `DEFAULT_WS_HOST` and `isLoopbackHost` were imported here too, for the step -1 bind guard that read them;
// both went with that guard on 2026-09-25 — see `doStart`'s note and `transport/ws.ts`'s header.
import { DEFAULT_HOST_DOMAIN } from "../utils/net";
import { createWsServer, type WsServer } from "../transport/ws";
import { commandRegistry, type CommandRegistry } from "../kernel/registry";
import { registerAllEngineCommands } from "../registerAll";
import { bus as defaultBus, type EngineEventBus } from "../kernel/eventBus";
import { loadSettings, appDataDir, type AppSettings } from "../workspace/appSettings";
import { bootstrapWorkspaces } from "../lifecycle/startup";
import { setDataRoot, dataDir } from "../workspace/paths";
import { EventLog } from "../transport/eventLog";
// `DEFAULT_EDGE_BIND_HOST` is imported rather than defaulted to a literal here: the host needs the same
// value `createEdgeRouter()` will apply, to report the edge's address, and a second copy of `"127.0.0.1"`
// in this module is a rule with two homes.
//
// `bindEdgeInstance` is `plan/17` §5 work item 7 — the same treatment the forward proxy and the webhook
// already get below, one port over: the host *names* its edge to the module-level binding so `config.save`
// can re-bind it when `settings.port` changes.
import { bindEdgeInstance, createEdgeRouter, DEFAULT_EDGE_BIND_HOST, type EdgeRouterInstance } from "../reverseproxy/edge";
import { createProxyServer, bindProxyInstance, type ProxyServerInstance } from "../proxy/server";
import {
  createWebhookServer,
  bindWebhookInstance,
  type WebhookServerInstance,
} from "../webhooks/webhookServer";
// The **single**-port finder and the bound on its scan. The companion is the one listener whose port is
// *found* rather than bound (`plan/17` §1): the host scans upward from `settings.companionPort`, skipping the
// ports it has already bound, so the port a client is told to connect to is one that is really free.
import { findFreePortFrom, nextFreePortAbove, SCAN_LIMIT } from "./portAllocator";

/**
 * How the RPC endpoint is exposed. D13: **one** endpoint per host, not one per workspace.
 *
 * ## Three fields were removed on 2026-09-25: `host`, `allowNonLoopback`, `tls`
 *
 * `plan/15` work item 1 added all three so that a deployable host could be **published** — the bind host to
 * name the interface, the opt-in to acknowledge it, and the TLS pair because `plan/05` requires TLS on an RPC
 * channel beyond loopback. The programme has since decided the engine is never deployed remotely, so
 * `transport/ws.ts` accepts loopback and nothing else, and the three fields became a way to pass through a
 * value with exactly one legal setting. Their removal is what makes the image's loopback guarantee structural
 * rather than a property of the command line an operator happened to use.
 */
export type HostTransport =
  | {
      kind: "socket";
      /** Defaults to `defaultSocketPath()`. Windows: a named pipe; POSIX: a `0600` unix socket. */
      path?: string;
    }
  | {
      kind: "ws";
      /** Defaults to `0` — an ephemeral port, so two hosts on one machine cannot collide. */
      port?: number;
    };

export interface WorkspaceHostOptions {
  /** The data root. Required, and never invented — see `store/paths.ts`. */
  dataDir: string;
  /** Overrides the workspace list. When absent it comes from `app.json`, after bootstrap repair. */
  workspaceIds?: string[];
  /** How to expose RPC. Defaults to `{ kind: "socket" }` — D3's decided desktop transport. */
  transport?: HostTransport;
  /**
   * The port the **edge router** binds — `plan/15` §3.1's host tier, work item 3. Omitted → the port in
   * `app.json` (`settings.port`, default `80`), which is the port every existing `*.localhost` mapping
   * already points at.
   *
   * **The default is the settings port rather than a new constant, and that is the migration.** Before
   * the split, `settings.port` was the single socket that served both halves, so a mapping's URL was
   * `*.localhost:<settings.port>`. Handing the edge that same port is what keeps every existing mapping
   * working — §6.2's promise — with no edit to any of them. `BIFURC_PROXY_BASE_PORT` is deliberately not
   * consulted here: it moves the *forward* proxy's base, which is a different question with a different
   * answer.
   */
  edgePort?: number;
  /** Reported to clients in `hello`. Defaults to the `@bifurc/protocol` version, as the transports do. */
  engineVersion?: string;
  bus?: EngineEventBus;
  registry?: CommandRegistry;
  /** Notified once per workspace whose ports had to be moved off the base. **Not** an error — see below. */
  onNotice?: (message: string, workspaceId?: string) => void;
}

export interface WorkspaceHostStatus {
  dataDir: string;
  /**
   * The hostname this engine answers as — `localhost`, and not configurable.
   *
   * **It was `plan/15`'s `--domain`, and it stopped being an option on 2026-09-24** (`hostArgs.ts`'s header
   * records the removal). The engine is no longer deployable in the cloud, so every deployment is on the
   * machine the client is on and the name is a fact rather than a choice; a setting that can hold one value
   * is not a setting.
   *
   * Reported rather than left to the caller's own copy of the constant, because a log line that said
   * something else than the value `hello` advertises would be a log line about a different host.
   */
  domain: string;
  /** The RPC endpoint's address: the socket path, or the `ws://` URL. */
  rpcAddress: string;
  /**
   * Where the edge router is serving — `http://<host>:<port>` — or `null` when it is not running.
   *
   * Reported once for the whole host rather than per workspace, because the edge *is* one listener for the
   * whole host (`plan/15` §3.1).
   */
  edgeAddress: string | null;
  /** The port the edge actually bound, or `null`. */
  edgePort: number | null;
  /**
   * Whatever the edge reported on a failed bind, if anything.
   *
   * Surfaced rather than thrown for the same reason the proxy's is: a busy port has always been tolerated
   * by this engine, and a host that refused to start would turn one taken port into an outage for every
   * workspace. The difference in *consequence* is worth stating, though — a host whose edge is down is
   * serving nothing on the reverse path, so the CLI prints this and does not treat it as routine.
   */
  edgeError: string | null;
  /**
   * The **forward proxy** — host-level since `plan/17` §1, one listener for every workspace.
   *
   * It has no `running` field of its own because it is bound at start and its failure mode is the *bind*:
   * `forwardProxyError` non-null is the thing a caller acts on, and a `running: false` beside it would be a
   * second way to say the same thing that could disagree.
   */
  forwardProxyPort: number | null;
  /** Whatever the forward proxy reported on a failed bind, if anything. */
  forwardProxyError: string | null;
  /**
   * The **webhook receiver** — host-level, and **off by default** (`plan/17` §1).
   *
   * `null` is therefore the ordinary value: the receiver is a *feature*, not infrastructure, and it is the
   * user who starts it. A non-null port means it is listening.
   */
  webhookPort: number | null;
  /** Whatever the webhook receiver reported on a failed bind, if anything. */
  webhookError: string | null;
  /**
   * The **companion** — the RPC endpoint over WebSocket that the browser extension connects to.
   *
   * `null` when this host runs no companion, which is the default: a `--ws-port` host has one, and a
   * socket-transport host does not.
   */
  companionPort: number | null;
  /**
   * The workspaces this host serves, in start order.
   *
   * **Config scope only — no port is per workspace any more.** This field replaced a `runtimes:
   * WorkspaceRuntimeStatus[]` array whose `proxyPort` / `webhookPort` were the whole reason it existed; with
   * one listener per port for the whole host (`plan/17` §1), those numbers were the same for every entry, so
   * the array was N copies of one fact. What is genuinely per workspace is which **config** is in force, and
   * that is what is listed.
   */
  workspaces: { workspaceId: string; name: string; active: boolean }[];
}

export interface WorkspaceHost {
  start(): Promise<WorkspaceHostStatus>;
  /** Stop every listener and close the RPC endpoint. Idempotent. */
  stop(): Promise<void>;
  status(): WorkspaceHostStatus;
  /**
   * The one forward proxy for this host, or `null` before `start()`.
   *
   * Exposed because a test asserting the split needs the proxy's own `isRunning()` / `getPort()` rather than
   * the host's summary of them — the same reason `rpc` and `edge` are.
   */
  readonly forwardProxy: ProxyServerInstance | null;
  /** The one webhook receiver for this host, or `null` before `start()`. */
  readonly webhook: WebhookServerInstance | null;
  /** The one RPC endpoint for this host. `null` before `start()`. */
  readonly rpc: SocketServer | WsServer | null;
  /**
   * The one edge router for this host. `null` before `start()`.
   *
   * Exposed for the same reason `rpc` is — a caller that has to ask "is it up?" should not have to
   * re-derive it from `status()` — and because a test asserting the split needs the edge's own
   * `isRunning()` / `getPort()` rather than the host's summary of them.
   */
  readonly edge: EdgeRouterInstance | null;
}

/**
 * Poll until a fire-and-forget server reports ready, or reports an error, or the deadline passes.
 *
 * ## Why this exists here, and what it restores
 *
 * `forwardProxy.start()` and `edge.start()` each hand off to an **async** `listen()` and return, so the
 * only readiness signal either publishes is the predicate passed here — neither exposes its `net.Server`.
 * (The same helper lives in `index.ts`, unexported, for the same two listeners; duplicated rather than
 * shared because this module must not depend on the engine's lifecycle module — `host.ts`'s header records
 * that a host must never call `createEngine()`.)
 *
 * **This was lost when `WorkspaceRuntime` was deleted** (`plan/17` §5 work item 1, 2026-09-25), and the
 * loss was a real behaviour change rather than a tidy-up. `WorkspaceRuntime.doStart()` awaited readiness of
 * both its servers before resolving, and `host.ts` read `st.proxyPort` off that resolved value — so every
 * caller of `host.start()` was entitled to a **bound** listener. Without the wait, `start()` resolves while
 * the proxy is still binding, and `status()` reads the port through `isRunning()` (i.e. `server.listening`),
 * so it reports `forwardProxyPort: null` for a proxy that comes up milliseconds later. Measured, not
 * deduced: it is what the host suite caught the moment the class was gone.
 *
 * The deadline exists so a hung bind cannot hang `start()` forever; 10 ms is well below the cost of a
 * socket bind, so a successful one returns on the first or second poll.
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

export function createWorkspaceHost(opts: WorkspaceHostOptions): WorkspaceHost {
  const bus = opts.bus ?? defaultBus;
  const registry = opts.registry ?? commandRegistry;
  const notice = opts.onNotice ?? ((message: string) => console.log(`[host] ${message}`));
  /**
   * The host's own name — `localhost`, composed here rather than taken from an option.
   *
   * One local rather than the constant repeated at each of its readers (`status()`, the RPC's `auth` and the
   * edge), on this module's own rule: a value with two homes is a value that can disagree with itself. It was
   * `plan/15`'s `--domain` until 2026-09-24 — `hostArgs.ts`'s header records why the flag went, and
   * `lib/net.ts#DEFAULT_HOST_DOMAIN` is the single source now.
   */
  const domain = DEFAULT_HOST_DOMAIN;

  let forwardProxy: ProxyServerInstance | null = null;
  let webhook: WebhookServerInstance | null = null;
  let rpc: SocketServer | WsServer | null = null;
  let edge: EdgeRouterInstance | null = null;
  /** The edge's URL. Set when the edge is created, and read only while it is running. */
  let edgeUrl = "";
  /** The resolved companion port — `plan/17` §5 work item 4. */
  let companionPort: number | null = null;
  /** What `status()` lists as served. Config scope only — see `WorkspaceHostStatus.workspaces`. */
  let served: { workspaceId: string; name: string; active: boolean }[] = [];
  let rpcAddress = "";
  let startPromise: Promise<WorkspaceHostStatus> | null = null;
  let stopped = false;

  function status(): WorkspaceHostStatus {
    return {
      dataDir: dataDir(),
      // The resolved name, not `opts.domain` — see `WorkspaceHostStatus.domain` for why the two can differ.
      domain,
      rpcAddress,
      // Every listener follows one rule rather than five: a **port** is reported only while the server is
      // answering on it, so a caller cannot read a port for something that is down. The **error** is
      // reported whenever there is one — that is the case where the port matters most and is least
      // available.
      edgeAddress: edge !== null && edge.isRunning() ? edgeUrl : null,
      edgePort: edge !== null && edge.isRunning() ? edge.getPort() : null,
      edgeError: edge?.getError() ?? null,
      forwardProxyPort: forwardProxy !== null && forwardProxy.isRunning() ? forwardProxy.getPort() : null,
      forwardProxyError: forwardProxy?.getError() ?? null,
      // `null` is the ordinary value here: the webhook is off by default (`plan/17` §1).
      webhookPort: webhook !== null && webhook.isRunning() ? webhook.getPort() : null,
      webhookError: webhook?.getError() ?? null,
      companionPort,
      workspaces: served,
    };
  }

  /** The log is per host, not per runtime: `seq` is only meaningful across a reconnect if one authority
   * issues it (`transport/eventLog.ts`), and D13 puts one RPC endpoint in front of all the runtimes. */
  const log = new EventLog({ bus });

  async function doStart(): Promise<WorkspaceHostStatus> {
    /**
     * ## Step -1 was the bind policy, and there is no longer a policy to state
     *
     * `plan/10` work item 5 required the host to *"refuse to start with a clear message"* if the RPC port was
     * bound to anything other than loopback, and `plan/15` work item 1 carried that forward as the second half
     * of the bind opt-in: a wide bind also demanded a token the operator had **seen**, because
     * `ensureEngineToken()` would otherwise generate one, write it into the data root and report it once on
     * stdout — a credential nobody chose and no client was given.
     *
     * Both halves are gone as of **2026-09-25**, and the guard went with its subject. A wide bind is no longer
     * possible at all: `createWsServer`, `createProxyServer` and `createEdgeRouter` each refuse a non-loopback
     * address unconditionally from their own factories, and the CLI can no longer express one. So there is no
     * configuration left for a host to fail closed *on*, and a guard that cannot fire is one the next reader
     * deletes — worse than absent, because it would look like the rule is still enforced here rather than one
     * layer down, where it actually is.
     *
     * **What this costs, stated plainly:** the refusal now happens inside the transport, so it is no longer
     * ordered before the data root is touched. That is a real loss of the "a startup guard should leave no
     * trace" property, and it is accepted because nothing the CLI can be given reaches that throw — see
     * `hostArgs.ts`'s `REMOVED_FLAGS`. A programmatic caller could not do it either: `host`, `allowNonLoopback`
     * and `tls` are no longer fields of `HostTransport`.
     */
    const transport = opts.transport ?? { kind: "socket" };

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
       * `%LOCALAPPDATA%` regardless of `setDataRoot()` (recorded in `certificates/certManager.ts`), so a
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

    // ── 3. the token step, deleted 2026-09-25 ───────────────────────────────
    /**
     * **There is no token, and this is a tombstone rather than a placeholder.**
     *
     * What stood here was `ensureEngineToken()`: generate a token, write it to `<dataDir>/engine.token`,
     * report the path once on stdout, and hand the value to the RPC endpoint's wrapper. It was the second
     * half of `plan/10` work item 5's fail-closed rule — a non-loopback bind refused to start on a token
     * nobody had *seen* — and it went in the same order as the rest of the remote-deployment work: the bind
     * half first (step -1 above), then the token itself, because the RPC surface can no longer be reached
     * from anywhere but this machine.
     *
     * **What replaced it is not a secret, and that is the point.** A token guards a boundary; the boundary
     * is now the loopback bind plus an `Origin` check in `transport/ws.ts`, and neither is a credential. The
     * bind answers *"can this packet arrive?"*; the `Origin` check answers *"is the page that sent it served
     * from this machine?"* — a question the token could not be *asked*, because a web page runs on the same
     * machine as the engine and its connection to `127.0.0.1` is a genuine loopback connection.
     * `lib/net.ts`'s `isAllowedOrigin` header works through both halves of that argument.
     *
     * **`plan/17` §5 work item 9 is a different removal that also happened to live in this step.** The
     * host-local `runtime.json` registry remembered *which port each workspace landed on* — D16's stickiness
     * rule, so a workspace that took 81 because 80 was busy kept 81 and a browser configured for it did not
     * break. With one host-level listener per port there is nothing per workspace to remember: the forward
     * proxy's port is a **setting** the user owns and sees, and the companion pair is re-scanned from its
     * base at every start.
     */

    // ── 3b. the forward proxy — host-level, one listener for every workspace ─
    /**
     * ## `plan/17` §5 work item 3: this used to be one proxy *per workspace*
     *
     * `plan/14` D10/D11 made the workspace the deployable unit, so the host allocated each workspace a port
     * (D16's base-and-scan) and composed a `WorkspaceRuntime` around it. `plan/17` §1 replaces that model: a
     * workspace is a **project**, so switching workspaces changes the **context** — which rules, mocks and
     * webhook endpoints are in force — while the ports stay put. There is now **one** forward proxy for the
     * whole host, and `WorkspaceRuntime` is **deleted** rather than hollowed (owner decision, 2026-09-25):
     * with no listener of its own it had no subject left.
     *
     * ## `workspaceId: null` is the whole of "follow the active workspace"
     *
     * `createProxyServer()` reads its own `workspaceId` and falls back to `loadConfig().activeWorkspaceId`
     * **per call** when none was supplied. That fallback is exactly what a host-level proxy means — *"which
     * workspace am I looking at?"* — and it is why this passes nothing.
     *
     * ## Why it binds rather than scans, and what "fails" means
     *
     * `plan/17` §1: *"the forward proxy does NOT scan. It binds `8080` and **fails if occupied**, naming the
     * port, so the user enters a working one instead of the app silently incrementing."* A browser's proxy
     * setting names this port, so a silently moved one is a broken browser — D16's scan-upward rule was for
     * a port **nobody addressed**, which this one is not.
     *
     * "Fails" here means **the bind fails**, not "the host refuses to start". That distinction is the
     * engine's oldest bind contract: a busy port has always been tolerated and reported through `getError()`,
     * and the edge below records the same reasoning. A host that exited because another program held 8080
     * would turn one taken port into an outage for every workspace — strictly worse than a proxy that is down
     * and says so. What `plan/17` forbids is **silence**, and the notice in step 5 is not silent.
     */
    forwardProxy = createProxyServer({
      // No `workspaceId`: the forward proxy serves whichever workspace is active. See the block above.
      workspaceId: null,
      // **No bind host, and no opt-in to acknowledge one** (both removed 2026-09-25). `plan/15` work item 2
      // let an operator name the interface here and acknowledge it with a second flag; `createProxyServer()`
      // now refuses any non-loopback address unconditionally, so the pair became a way to pass through a
      // value with exactly one legal setting. The refusal still throws **from the factory** — deliberately,
      // because it is a configuration error with nothing to retry — which is why this call site needs no
      // guard of its own. See `proxy/server.ts`'s `DEFAULT_PROXY_BIND_HOST` docblock.
    });
    const forwardProxyPortWanted = settings.forwardProxyPort;
    const proxyListener = forwardProxy;
    proxyListener.start(forwardProxyPortWanted);

    /**
     * **Wait for the bind, so `start()` means what it says.** See `waitUntilReady`'s docblock: `start()`
     * returns before `listen()` completes, and `status()` reads the port through `isRunning()`.
     *
     * A **bind failure is not thrown** — `getError()` ends the wait, and the failure is reported by the
     * notice in step 5 and by `status().forwardProxyError`. That is the engine's oldest bind contract: a busy
     * port has always been tolerated and reported, never fatal. A host that exited because another program
     * held 8080 would turn one taken port into an outage for every workspace.
     */
    await waitUntilReady(() => proxyListener.isRunning(), () => proxyListener.getError());

    // ── 3c. the webhook receiver — host-level, and OFF by default ────────────
    /**
     * `plan/17` §1: *"the webhook does NOT scan either. It is a **feature, not infrastructure**: it is **off
     * by default**, the user starts it, and the user owns the port."* So this is **created and not started**
     * — the `webhookServer.start` command is what binds it, exactly as the desktop has always done
     * (`WebhooksPanel` → `window.api.startWebhookServer()`).
     *
     * Creating it here rather than leaving it lazy is what makes `webhookServer.status` answerable before any
     * tab has opened, and it is what the binding below needs: the facade must point at *this* instance, not
     * at a lazily-created default that would become a second, competing authority.
     *
     * **Known gap, stated rather than implied.** The receiver's *active* set — the suffixes it will accept a
     * POST for — is registered by the UI when a webhook tab opens, and it is not keyed by workspace. So
     * switching workspace does not by itself retire the previous workspace's active webhooks. `plan/17` §1
     * lists *"which webhook endpoints are in force"* among the things a switch changes; this host makes the
     * **port** host-level but not yet the **active set**, because `createWebhookServer()`'s active maps are
     * not workspace-keyed. Recorded here so it is a known limit rather than a surprise.
     */
    webhook = createWebhookServer({ workspaceId: null });

    /**
     * ## Binding the facade is what makes a command reach these two
     *
     * `proxy/server.ts` and `webhooks/webhookServer.ts` each keep a module-level **facade** over a lazily created
     * default instance, and the `server.*` / `webhookServer.*` commands go through it. `bindProxyInstance()` /
     * `bindWebhookInstance()` point that facade at an instance the caller owns — the desktop has always used
     * them, and this host now uses the same mechanism.
     *
     * **This replaces `runtimeRegistry.ts`.** That module existed because a host had N runtimes and a command
     * had to resolve *which* one a `workspaceId` named, so it carried a three-way union (`runtime` /
     * `facade` / `absent`) plus a `runtimeServerOps.ts` to drive it. With one listener per port there is
     * nothing to resolve — the facade **is** the host's server — so the union, the registry and its scope
     * plumbing are all deleted, and the commands become direct facade calls. That is the honest reading of
     * `plan/17` §3: the routing *collapses*.
     */
    bindProxyInstance(forwardProxy);
    bindWebhookInstance(webhook);

    // ── 3d. the edge router — one listener for the whole host (`plan/15` §3.3) ─
    /**
     * ## Why it is not allocated
     *
     * Its port comes from `settings.port` — the port every existing `*.localhost` mapping already points at —
     * so it is **chosen**, not discovered, and it is bound exactly as chosen. A silently moved edge would be
     * a proxy whose mappings answer somewhere other than where the user typed them, which is a worse failure
     * than not starting. `plan/17` §1 keeps this rule unchanged: *"the edge is bound, not scanned."*
     *
     * **A failed edge bind is reported, not thrown.** A bind failure has always been tolerated here and
     * reported through `getError()`, because the shell has always tolerated a busy port. The consequence is
     * larger for the edge than for the proxy — a host whose edge is down serves nothing on the reverse path
     * — so it gets its own notice below, stated as a failure rather than as routine.
     *
     * `resolveConfig` is read **per request**, and that is what makes `workspace.setActive` need no
     * invalidation step *on the edge*: the next request re-reads which workspace is active and asks the
     * forward proxy for the config it dispatches with.
     *
     * **The forward proxy is deliberately not per-request.** It caches its config at `start()` and swaps it
     * on `reloadConfig()` — `plan/17` §5 work item 3: *"loads the active workspace's rules and mocks **at
     * start** and **swaps its in-memory config when the active workspace changes** — not per-request
     * resolution."* So the edge follows the pointer for free, while the proxy is re-pointed by
     * `workspace.setActive` calling `reloadConfig()` (`store/coreCommands.ts`).
     */
    // **The bind host is a constant, not an option** (the option was removed 2026-09-25). `plan/15` work
    // item 3 let an operator name the interface here and acknowledge it with a second flag;
    // `createEdgeRouter()` now refuses any non-loopback address unconditionally, so there is one legal value
    // and passing it through would be a field with a single setting. It is still read as a local rather than
    // inlined, because `edgeUrl` below is built from the same value and the host reports it.
    const edgeBindHost = DEFAULT_EDGE_BIND_HOST;
    const edgePortWanted = opts.edgePort ?? settings.port;
    edge = createEdgeRouter({
      bindHost: edgeBindHost,
      // No `hostDomain` — the edge stopped taking its own name on 2026-09-26, with the web GUI's global
      // mapping (`EdgeRouterOptions` carries the reasoning). `domain` is still composed and still reaches
      // clients as `hello`'s `hostDomain`; that is the transport's field, and a different thing.
      resolveConfig: () => {
        const activeId = loadSettings().activeWorkspaceId;
        /**
         * `null` when the active workspace is **not served by this host** — `--workspace` filtered it out.
         * Returning another workspace's config here would be a wrong-target answer, and returning a
         * synthetic empty config would render "No mappings yet" for a workspace that has plenty. `null` lets
         * the edge say what is actually true.
         *
         * Read from `served`, which step 4 fills: before it runs this is an empty list and every request
         * answers `null`, which is correct — the host is not serving anything yet.
         */
        if (!served.some((s) => s.workspaceId === activeId)) return null;
        return forwardProxy === null ? null : forwardProxy.dispatchConfig();
      },
    });
    edgeUrl = `http://${edgeBindHost}:${edgePortWanted}`;
    const edgeListener = edge;
    /**
     * ## `plan/17` §5 work item 7 — the edge joins the two facades bound in step 3b
     *
     * `bindProxyInstance()` / `bindWebhookInstance()` above are what let the `server.*` / `webhookServer.*`
     * commands reach this host's listeners. The edge had no such handle, so its port was the one runtime port a
     * `config.save` could not move — the gap work item 7 closes. `bindEdgeInstance()` is the same mechanism,
     * and the same obligation comes with it: `stop()` unbinds all three.
     *
     * **Bound before `start()`.** The engine's own composition does the same, for the same reason: a save
     * arriving in the boot window should find the instance rather than silently skip the re-bind.
     */
    bindEdgeInstance(edgeListener);
    edgeListener.start(edgePortWanted);

    // The same contract as the forward proxy above, and the same loss when `WorkspaceRuntime` went: the edge
    // binds asynchronously, so without this `status().edgePort` would be `null` for a listener that is up
    // microseconds later. `index.ts` waits on its own edge for exactly this reason.
    await waitUntilReady(() => edgeListener.isRunning(), () => edgeListener.getError());

    // ── 4. what this host serves, as config ─────────────────────────────────
    /**
     * **This used to be a loop that allocated a port and started a `WorkspaceRuntime` per workspace.** It is
     * now a list, because there is nothing left to start: both listeners are above, and what remains per
     * workspace is *which config is in force* — `plan/17` §1, *a workspace is a project, not a listener*.
     *
     * `served` is also the guard `resolveConfig` reads, so a `--workspace` filter that excludes the active
     * workspace makes the edge answer `null` rather than serving a workspace this host was not asked to
     * serve. That is the surviving half of the check `runtimeRegistry.ts`'s `absent` case used to provide,
     * and it survives because the *question* survives: "is the active workspace one of ours?"
     */
    served = settings.workspaces.map((ws) => ({
      workspaceId: ws.id,
      name: ws.name,
      active: ws.id === settings.activeWorkspaceId,
    }));

    // No `runtime.json` write any more — see step 3. There is no resolved port to remember: the forward
    // proxy's port is the setting the user chose, and the companion pair is re-scanned from its base at
    // every start (`plan/17` §5 work item 9).

    /**
     * The edge's own line, and it is **not** routine when it failed.
     *
     * Before `plan/17` this comment said the edge was special because it was "not one of N" while the
     * per-workspace proxies were. There is no N any more — the host owns one listener per port — so the
     * reason is now about *consequence* rather than about arithmetic: the edge is the reverse path itself,
     * and a host whose edge is down is answering for no mapping at all. So the failure is stated as a
     * failure, naming what it costs, and `status().edgeError` carries it to the CLI and to `server.status`.
     *
     * Read through a local because `edge` is declared `| null` for `status()`'s sake; `stop()` and this
     * block are the only readers that know it cannot be null by now.
     */
    const edgeRouter = edge;
    if (edgeRouter === null) throw new Error("[host] internal: the edge was not created");
    if (edgeRouter.getError() !== null) {
      notice(`edge on ${edgeUrl} FAILED to bind: ${edgeRouter.getError()} — no *.localhost mapping will resolve`);
    } else {
      notice(`edge: ${edgeUrl} — reverse proxy and home page for the active workspace`);
    }

    /**
     * The forward proxy's line — a **failure** when it failed, and the same shape of argument as the edge's.
     *
     * `plan/17` §1 refuses to scan this port precisely so that the user always knows what their browser must
     * be pointed at, which means a proxy that is not on that port is a *broken browser setting* rather than a
     * degraded mode. It still does not stop the host — see step 3b for why — so the notice carries the whole
     * weight of telling the user.
     *
     * The next free port is **named**, because `plan/17`'s instruction to the user is *"enter a working one
     * instead of the app silently incrementing"*, and that instruction is only followable if a working port is
     * on screen. `nextFreePortAbove()` exists for exactly this and nothing else — it never picks the port.
     */
    const proxyErr = forwardProxy.getError();
    if (proxyErr !== null) {
      const nextFree = await nextFreePortAbove(forwardProxyPortWanted + 1);
      notice(
        `forward proxy FAILED to bind ${forwardProxyPortWanted}: ${proxyErr} — your browser's proxy setting ` +
          `points at this port, so it has to be one that works. ` +
          (nextFree === null
            ? `No free port was found above it — free ${forwardProxyPortWanted}, or choose another in Settings.`
            : `Port ${nextFree} is free: set it in Settings → Ports, or free ${forwardProxyPortWanted}.`),
      );
    } else {
      notice(
        `forward proxy: ${forwardProxyPortWanted} — point your browser's proxy setting at this port`,
      );
    }

    // ── 5. the one RPC endpoint, for every workspace (D13) ───────────────────
    // `transport` is resolved at step -1, because the bind policy has to be decided before the data root
    // is touched. See the guard there.
    const auth = {
      engineVersion: opts.engineVersion ?? "0.0.0",
      // The host's own name, so a client can derive a mapping's suffix instead of hardcoding it (`plan/15`
      // §4.3). Read from the resolved local above rather than re-derived here from the transport or the bind
      // address: §4.1's rule is that the *host* decides this and the client does not guess, and an engine
      // bound to `0.0.0.0` is still reached by **name** — and since 2026-09-24 that name is `localhost`,
      // because the engine is no longer deployable in the cloud.
      hostDomain: domain,
      // Reported so an operator can see *why* a host refused a session, which over a plain socket is otherwise
      // unattributable: `socket.ts` has no peer address to record, and a refused client is disconnected with
      // nothing but the `UNAUTHORIZED` reply to go on. The `hello` is included because a client that is not
      // sending the handshake at all and a protocol mismatch are the same code and different problems.
      //
      // **`token` was the first field of this object** until 2026-09-25, and it is what this notice's
      // `UNAUTHORIZED` case used to be about. It went with the token itself. What can still reach this is a
      // client that is not speaking the handshake first — and since this client always does, that now means
      // something else is driving the endpoint (`transport/auth/authenticated.ts`).
      onAuthFailure: (error: { code: string; message: string }, hello: { clientName?: string }) =>
        notice(`rpc session refused (${error.code}) from "${hello.clientName ?? "unnamed"}": ${error.message}`),
    };

    if (transport.kind === "socket") {
      const path = transport.path ?? defaultSocketPath();
      rpc = await createSocketServer({ path, registry, bus, log, auth });
      rpcAddress = path;
    } else {
      /**
       * ## The companion's port is **found**, not bound — `plan/17` §5 work item 4
       *
       * `plan/17` §1: *"Electron has no GUI port, so it checks only the companion's `n` — requiring `n + 1`
       * to be free there would refuse a port for a neighbour it is never going to use."* So when the transport
       * carries **no** explicit port, the host scans upward from `settings.companionPort` for a single free
       * port.
       *
       * **`0` was the one value a companion must never take:** it means "the OS picks", and a client — the
       * extension — has to be *told* where to connect, so a port that moves cannot be found. This is
       * `plan/15` §3.1's own rule (*a port belongs to the thing that has to be addressed once*) applied to the
       * second thing that has an address.
       *
       * `reserved` is what keeps the scan off the ports this host already bound. Without it the finder could
       * hand back `80` (the edge) or `8080` (the forward proxy) as the companion's port, and the failure would
       * surface as a `listen()` that fails *after* the host had reported the port it "resolved" — the same
       * TOCTOU shape `portAllocator.ts`'s header warns about, one layer up.
       */
      const reserved = new Set<number>([edgePortWanted, forwardProxyPortWanted]);
      if (transport.port === undefined || transport.port === 0) {
        const found = await findFreePortFrom(settings.companionPort, reserved);
        if (found === null) {
          /**
           * `plan/17` §1's search **"never fails"** on a real machine — a free port is eventually found. This
           * is the bound on "eventually": `SCAN_LIMIT` ports above the base. Throwing names the base rather
           * than reporting a port that could not be bound.
           */
          throw new Error(
            `No free companion port within ${SCAN_LIMIT} of ${settings.companionPort} — ` +
              `set a different base in Settings → Ports.`,
          );
        }
        companionPort = found;
      } else {
        // An explicit port is the operator's choice and is honoured as-is — the same rule the edge follows,
        // and for the same reason: a companion that moved would be one no client could find.
        companionPort = transport.port;
      }
      notice(`companion: ${companionPort}`);

      rpc = await createWsServer({
        registry,
        bus,
        log,
        auth,
        // **No `host`, no `allowNonLoopback`, no `tls`** — all three were passed here until 2026-09-25.
        // `ws.ts` defaults the bind to `DEFAULT_WS_HOST` and refuses anything that is not a loopback
        // spelling, so the address is not a caller's choice and there is nothing to acknowledge; the TLS
        // pair existed only to make a non-loopback channel safe, and there is no non-loopback channel.
        // Omitting all three is the whole of "this listener is on this machine".
        port: companionPort,
      });
      rpcAddress = rpc.url;
    }
    notice(`rpc endpoint: ${rpcAddress}`);

    /**
     * **The `engine token: <path>` line was here**, and there is nothing left to report — the token step
     * above is a tombstone.
     *
     * Worth recording what was lost with it, because the line was doing real work: it printed the *path*
     * and never the value, on the reasoning that a log is read by more people than a `0600` file is, and
     * that the path is what an operator actually needs. The client that needed it most was the browser,
     * which cannot read a file at all. That client now authenticates with an `Origin` it already has, so the
     * loss is affordable rather than merely accepted.
     */

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
      // cannot get a request in against a listener that is mid-teardown.
      if (rpc !== null) {
        try {
          await rpc.close();
        } catch {
          // Best-effort — a socket file that another process already removed must not block the shutdown.
        }
      }
      // The edge **before** the proxy it routes to. A request arriving mid-teardown should find either a
      // working mapping or a closed edge, never a live edge in front of a stopped proxy — the same ordering
      // argument as the RPC endpoint above, approached from the other end.
      //
      // This order is *reversed* from the pre-`plan/17` version, and the change is the point: there used to
      // be N per-workspace proxies, so the edge was stopped last because it outlived its routing targets.
      // With one proxy, "stop the thing that routes, then the thing it routes to" is the only order that
      // never exposes a live front end to a dead back end.
      edge?.stop();
      // Then the two listeners. Both are null-guarded internally, so a host that never reached them tears
      // down cleanly.
      forwardProxy?.stop();
      webhook?.stop();
      /**
       * **The facade binding is undone here, because `start()` made it.**
       *
       * `bindProxyInstance()` / `bindWebhookInstance()` are **process-global** — they set module-level state in
       * `proxy/server.ts` and `webhooks/webhookServer.ts`, and every module function in those files acts on
       * whatever is bound. So a host that binds and does not unbind leaves the *next* consumer in the same
       * process talking to **this** host's stopped instances: `startServer()` would resurrect them rather than
       * build a fresh one, and `server.status` would keep reporting a dead instance's last error forever.
       *
       * That is not a hypothetical in this tree. `facadeBinding.integration.test.ts`'s own teardown records the
       * rule — *"Unbind before releasing the overrides: a leak here would redirect every later integration test
       * in this worker"* — and `tests/integration/settingsMutations.integration.test.ts` drives the module
       * facade directly, in the same project. One host per process is the design, so the leak is invisible to
       * the CLI; it is the **test worker** and any future second lifecycle that pays for it.
       *
       * The earlier draft of this comment argued the opposite — that a stopped instance answers `isRunning()`
       * with `false` and a truthful error, so leaving it bound was the more informative choice. That is true
       * and it is not worth the global state: "informative" here means one command can describe a host that
       * another part of the process has already torn down, which is exactly the *"second, competing
       * authority"* this file's header warns about — just pointing the other way.
       *
       * `null` restores the lazy default, which is the state a process that never ran a host is in.
       */
      bindProxyInstance(null);
      bindWebhookInstance(null);
      // The edge's binding is released with the other two, and for the same reason — see the block above. It
      // was added by `plan/17` §5 work item 7, and the obligation came with the mechanism rather than later.
      bindEdgeInstance(null);
      // The **served list** is what has to be forgotten, and it is read by `status()` and by the edge's
      // `resolveConfig`. `clearRuntimes()` used to be called here as well, because a registry of N runtimes
      // could hold workspaces this host no longer served; there is no N and no registry now (`plan/17` §5 work
      // item 9), so this list is the whole of the stale state.
      served = [];
      // The host owns the log (one authority for `seq`), so it closes it.
      log.close();
    },

    status,
    get forwardProxy(): ProxyServerInstance | null {
      return forwardProxy;
    },
    get webhook(): WebhookServerInstance | null {
      return webhook;
    },
    get rpc(): SocketServer | WsServer | null {
      return rpc;
    },
    get edge(): EdgeRouterInstance | null {
      return edge;
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
