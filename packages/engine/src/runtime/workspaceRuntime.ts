/**
 * P14 work item 3 — `WorkspaceRuntime`, the unit a host deploys.
 *
 * ## What this is, and what it deliberately is not
 *
 * D10/D11 make the **workspace** the deployable unit, not the engine: "N workspaces per host, and
 * `WorkspaceRuntime` is the unit (one-per-container = N=1)." So this composes the two servers WI 2
 * de-singletonized — the proxy and the webhook receiver — into one object that owns a workspace's
 * listening state on a host, and nothing else:
 *
 * - **Not the engine.** The engine stays a library (`plan/14` D10). A runtime does not resolve the data
 *   root, does not bootstrap workspaces, does not own the event log or the command registry. Those are
 *   the *host's* or the *library caller's* concerns, and reproducing them here would give a host two
 *   competing lifecycle authorities.
 * - **Not the companion server.** `plan/14`'s WI 3 criterion is that a workspace host starts with no
 *   companion; `legacyCompanion.ts` is app-level by design (WI 2) and a host passes
 *   `EngineOptions.startCompanion: false` for it. A runtime never touches it.
 * - **Not the RPC endpoint.** The host exposes one endpoint for all its runtimes (D13); a transport is
 *   per host, not per runtime, which is also why `createSocketServer()` takes a registry rather than a
 *   runtime.
 *
 * ## The two things a runtime *does* own
 *
 * The proxy and the webhook receiver, each with its resolved port. Both are created with
 * `ownsCA: false` when the runtime is part of a multi-runtime host — see `createProxyServer()`'s header
 * for why the TLS CA cannot be unloaded by a member.
 *
 * ## Ports are passed in, not chosen here
 *
 * `proxyPort` / `webhookPort` are required, and that separation is the point: allocation is D16's rule
 * (per-environment base, scan upward, resolved by a **real** `listen()`), and it needs host-wide
 * knowledge of which ports its other runtimes already took. A runtime that invented its own port would
 * have to guess at that and would reintroduce the TOCTOU race `plan/14` §4 warns about.
 */

import { createProxyServer, type ProxyServerInstance } from "../proxy/server";
import { createWebhookServer, type WebhookServerInstance } from "../proxy/webhookServer";

export interface WorkspaceRuntimeOptions {
  /** The workspace this runtime serves. Required — a runtime with no workspace has no meaning (D10). */
  workspaceId: string;
  /**
   * The port the proxy binds. **Required**, because the host allocates it (D16) and knows what its
   * other runtimes hold. See the header.
   */
  proxyPort: number;
  /**
   * The port the webhook receiver binds. **Required**, for the same reason, and because the second
   * workspace's webhook must skip a port the first took (`plan/14` §2.3: base `9101`, so B wants
   * `9102` and skips to `9103`).
   */
  webhookPort: number;
  /**
   * Whether this runtime owns the process-global TLS CA lifecycle. Defaults to `false`, which is the
   * **host** case: the CA is one mkcert root for the whole machine, so a runtime must not unload it on
   * `stop()`. A single-runtime caller that wants the pre-P14 behaviour passes `true`.
   */
  ownsCA?: boolean;
}

/**
 * The status a runtime reports. Mirrors `EngineStatus`'s proxy half, deliberately narrowed: a runtime
 * has no data root and no bootstrap state, and reporting fields it does not own would invite a caller
 * to read them.
 */
export interface WorkspaceRuntimeStatus {
  workspaceId: string;
  /** True once the proxy is listening. */
  running: boolean;
  /** The port the proxy actually bound, or `null` when it is not running. */
  proxyPort: number | null;
  /** Whatever the proxy reported on a failed bind, if anything. */
  proxyError: string | null;
  /** True once the webhook receiver is listening. */
  webhookRunning: boolean;
  /** The port the webhook receiver actually bound, or `null`. */
  webhookPort: number | null;
  /** Whatever the webhook receiver reported on a failed bind, if anything. */
  webhookError: string | null;
}

export interface WorkspaceRuntime {
  readonly workspaceId: string;
  /** The proxy server this runtime owns — exposed so a host can re-point readers (WI 4). */
  readonly proxy: ProxyServerInstance;
  /** The webhook receiver this runtime owns. */
  readonly webhook: WebhookServerInstance;
  /** Bind both servers, then resolve once each reports ready or failed. */
  start(): Promise<WorkspaceRuntimeStatus>;
  /** Stop both. Idempotent, and safe to call on a runtime that never started. */
  stop(): Promise<void>;
  /** A snapshot of live state. Safe to call before `start()`. */
  status(): WorkspaceRuntimeStatus;
}

/**
 * Poll until a fire-and-forget server reports ready or failed, or the deadline passes.
 *
 * The same predicate contract `index.ts`'s `waitUntilReady()` uses, and for the same reason: neither
 * server exposes its `net.Server`, so `isRunning()` / `getError()` are the only readiness signals they
 * publish. Duplicated rather than imported because `index.ts` does not export its copy and this module
 * must not depend on the engine's lifecycle module — see the header.
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

/**
 * Create one workspace runtime. `start()` binds both servers; `stop()` releases both.
 */
export function createWorkspaceRuntime(opts: WorkspaceRuntimeOptions): WorkspaceRuntime {
  const { workspaceId, proxyPort, webhookPort } = opts;
  const ownsCA = opts.ownsCA ?? false;

  if (!workspaceId) {
    throw new Error("createWorkspaceRuntime needs a workspaceId — a runtime serves exactly one workspace.");
  }

  const proxy = createProxyServer({ workspaceId, ownsCA });
  const webhook = createWebhookServer({ workspaceId });

  /**
   * `start()` runs once. A second call returns the same status rather than re-binding — the same
   * idempotence `Engine.start()` promises, and the reason a supervisor can call it on a poll.
   */
  let startPromise: Promise<WorkspaceRuntimeStatus> | null = null;
  let stopped = false;

  function status(): WorkspaceRuntimeStatus {
    const proxyUp = proxy.isRunning();
    const webhookUp = webhook.isRunning();
    return {
      workspaceId,
      running: proxyUp,
      proxyPort: proxyUp ? proxy.getPort() : null,
      proxyError: proxy.getError(),
      webhookRunning: webhookUp,
      webhookPort: webhookUp ? webhook.getPort() : null,
      webhookError: webhook.getError(),
    };
  }

  async function doStart(): Promise<WorkspaceRuntimeStatus> {
    proxy.start(proxyPort);
    webhook.start(webhookPort);

    // Both `start()`s are fire-and-forget (they hand off to an async `listen()` and return), so
    // resolving before the sockets accept would make this promise tell a caller nothing. A bind
    // failure is *not* thrown — the servers report it through `getError()` and `status()`, which is
    // the pre-P14 contract (the shell has always tolerated a busy port).
    //
    // Waited on **together**: the two binds are independent, so a busy webhook port must not delay the
    // proxy's readiness report by the other's deadline.
    await Promise.all([
      waitUntilReady(() => proxy.isRunning(), () => proxy.getError()),
      waitUntilReady(() => webhook.isRunning(), () => webhook.getError()),
    ]);

    return status();
  }

  return {
    workspaceId,
    proxy,
    webhook,
    start(): Promise<WorkspaceRuntimeStatus> {
      if (stopped) {
        return Promise.reject(
          new Error(
            `WorkspaceRuntime(${workspaceId}) has already been stopped and cannot be restarted — ` +
              "create a new runtime instead.",
          ),
        );
      }
      if (!startPromise) startPromise = doStart();
      return startPromise;
    },
    async stop(): Promise<void> {
      stopped = true;
      // Both are null-guarded internally, so a runtime that never started tears down cleanly.
      proxy.stop();
      webhook.stop();
    },
    status,
  };
}
