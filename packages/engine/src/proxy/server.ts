import * as net from "net";
import { EngineError } from "@bifurc/protocol";
import { isLoopbackHost } from "../utils/net";
import { loadConfig, loadWorkspaceDispatchSlice, AppConfig, ProxyRule, type WorkspaceDispatchSlice } from "../workspace/config";
import { readAllEntities } from "../workspace/workspaceFs";
import { HOP_BY_HOP } from "../utils/http";
import { sendHtml } from "../utils/sendHtml";
import { tcpTunnel, passthroughToUpstream, passthroughToUpstreamHttps } from "./proxyHandler";
import { loadCA, unloadCA, isCALoaded, clearCertCache } from "../certificates/tlsCert";
import { interceptTls } from "./tlsIntercept";
import { handleProxyRouting } from "./routingUtils";
import { RequestLogEntry, logEmitter, emitLog, emitLogChunk } from "../kernel/telemetry";
export { RequestLogEntry, logEmitter, emitLogChunk } from "../kernel/telemetry";

// `plan/15` work item 3 took this module's **reverse half**, and its imports went with it: the home
// page and the mapping/mock dispatch — `buildHomePage`, `buildNotMappedPage`, `matchMock`, `serveMock`,
// `isFullyMocked`, `mergeMockWithUpstream`, `resolveMockOnlyResponse`, `serveResolvedResponse`,
// `proxyToUpstream`, `fetchUpstreamResponse` — are `reverseproxy/edge.ts`'s now. The forward half never used
// any of them.
//
// `http` and `https` were **already** unused here before the split and are gone with the rest. Worth
// naming because it is not a one-off: `tsc` does not report an unused import, so a namespace imported
// for a branch that later moves stays imported indefinitely, and the next reader has no way to tell
// which of a dozen imports a module actually needs.


import { mkId } from "../utils/id";
import { activeEnv, EnabledSets, loadEnabledSets, workspaceCfg } from "../workspace/dispatch";

/**
 * The proxy server, as an instance (P14 work item 2, step 2).
 *
 * ## Why a factory, and why the module-level facade stays beside it
 *
 * Every mutable this file used to hold at module scope (`server`, `currentPort`, `currentConfig`,
 * `fullRules`, `lastError`, `enabledSets`) is instance state below, so a host can run one proxy per
 * workspace. `plan/14` §6 says to rehearse the shape on the webhook receiver first (done, 2026-09-21),
 * and this is the second application of what that taught: `createProxyServer()` returns an instance, and
 * the old module functions become a **facade over one lazily-created default instance**, kept until WI 7
 * flips the desktop onto the runtime (§9 rollback). Nothing on `window.api` moves.
 *
 * ## `workspaceId` is read, and the fallback is the desktop's path
 *
 * WI 4 severed `activeWorkspaceId` from the proxy: the instance reads **its own** `workspaceId` for rules and
 * enabled sets, and falls back to `cfg.activeWorkspaceId` only when none was supplied (see
 * `servedWorkspaceId()`). That fallback is not a leftover — it is exactly how the module facade and the
 * desktop keep working, because they construct one proxy and mean "whatever the user has selected". The two
 * behaviours coexist without a flag: a host member always passes an id, so it never reaches the fallback.
 *
 * ## The TLS CA is process-global, and `ownsCA` is how a host states who unloads it
 *
 * `tlsCert.ts` holds the loaded CA in module state (`caCertPem` / `caKeyPem` / `certCache`), and both
 * `loadCA()` and `unloadCA()` operate on that single copy. So a per-instance proxy that calls
 * `unloadCA()` in its `stop()` **breaks every other running proxy's TLS interception** — the same class of
 * hazard `plan/14` §6 names for `store/paths.ts` and `processSpawner.ts`.
 *
 * The CA genuinely *should* be shared — it is one mkcert root that is installed once into the OS trust
 * store, and N workspaces on one host all sign leaf certificates from it. What was wrong was that every
 * instance assumed it *owned* that lifecycle. So:
 *
 * - **`ownsCA: true` (the default, and what the module-level facade uses)** — this instance loaded the CA
 *   and unloads it on `stop()`. Exactly the pre-P14 behaviour, so the desktop is unchanged.
 * - **`ownsCA: false`** — a member of a multi-runtime host. It still `loadCA()`s its own configured CA
 *   (idempotent: same paths, same result, and `clearCertCache()` is harmless there), but **never
 *   `unloadCA()`s**, because the shared CA outlives it. The host that composed the group owns the unload.
 *
 * Do **not** "fix" the hazard by deleting the `unloadCA()` call outright — that leaks the CA past the stop
 * it was written to bound, and the desktop's single-proxy lifecycle depends on it.
 *
 * ## The reverse half is gone — `plan/15` work item 3 (2026-09-23)
 *
 * This module used to serve **two** jobs on one socket: the reverse proxy (a mapped `Host` forwarded to
 * its target) and the forward proxy (a browser configured to use this port). They are now two servers,
 * and only the forward half is here — `reverseproxy/edge.ts` holds the reverse half, on a **host-level** port.
 *
 * The reason is §1's second problem: sharing one socket meant a mock could not be applied to
 * forward-proxied traffic without also affecting reverse-proxied traffic, which is exactly what the
 * split buys. The reason the *reverse* half is the one that moved is §3.1's routing rule — a port
 * belongs to the thing that has to be addressed once, and a mapping is a hostname, which carries no
 * workspace in it.
 *
 * Two consequences are worth knowing before reading `dispatch()`:
 *
 * - **The home page moved with it.** It is tempting to read §3.4's *"loses only the reverse branch"* as
 *   leaving the `host === "localhost"` page here. It cannot: `buildHomePage()` builds every mapping's
 *   link from the port the page is served on, and after the split the mappings are no longer on this
 *   port — so a home page left behind would render links to a port that refuses them.
 * - **`dispatchConfig()` is public because of it.** The edge routes the same workspace's mappings and
 *   asks this instance for them rather than composing a second copy of the same config.
 */

// The six mutables this file used to hold here (`server`, `currentPort`, `currentConfig`, `fullRules`,
// `lastError`, `enabledSets`) are **gone**, not merely shadowed — they are instance state inside
// `createProxyServer()` below. Leaving dead module-level copies behind is the exact hazard this work
// item's own trap entry records, and `tsc --noEmit` will not catch it once nothing reads them.

/** The instance a factory call returns. Mirrors the module's old surface, one per runtime. */
export interface ProxyServerInstance {
  readonly workspaceId: string | null;
  start(port: number): void;
  stop(): void;
  isRunning(): boolean;
  getPort(): number;
  getError(): string | null;
  reloadConfig(): void;
  /**
   * The config this instance dispatches with — its own workspace's entities, composed by
   * `workspaceCfg()`. (`plan/15` work item 3.)
   *
   * ## Why the edge reads its config from here instead of loading its own
   *
   * The edge router serves the **active** workspace's mappings, and this instance already holds that
   * workspace's four dispatch inputs — `currentConfig`, `fullRules`, `enabledSets` and `wsSlice` —
   * loaded at `start()` and refreshed by `reloadConfig()`. Composing them is exactly what `dispatch()`
   * does on every request, so the edge asks for the same object rather than loading the same files a
   * second time.
   *
   * That is not a convenience, and it is not only about the I/O. Two readers of one config is how the
   * two halves drift, and this file's own history is the record of the cost: `store/serverUtils.ts`'s
   * `workspaceCfg()` exists because a proxy once read its entities through `loadConfig()` (the *active*
   * workspace) and its enabled sets through its *own* workspace, cross-filtered the two, and served
   * nothing — silently, with every test green. A second composition inside `edge.ts` would be a second
   * chance at exactly that defect, and every later change to either composition would have to be made
   * twice to avoid it.
   *
   * A **method**, not a property, because it must be re-read per call: it falls back to `loadConfig()`
   * before `start()`, and everything behind it changes on `reloadConfig()`.
   */
  dispatchConfig(): AppConfig;
}

/** Options for `createProxyServer()`. */
export interface ProxyServerOptions {
  /**
   * The workspace this proxy serves (P14 WI 4). Omitted → the active workspace, which is what the desktop and
   * the module-level facade mean. Always supplied by a host member. See `servedWorkspaceId()`.
   */
  workspaceId?: string | null;
  /**
   * Whether this instance owns the process-global TLS CA lifecycle, i.e. should `unloadCA()` on
   * `stop()`. Defaults to `true` — the pre-P14 single-proxy behaviour, so the desktop is unchanged.
   *
   * A host running several proxies passes `false` for all of them and unloads once itself, because the
   * CA is one mkcert root shared by every workspace on the machine. See the header.
   */
  ownsCA?: boolean;
  /**
   * The interface the proxy binds. Defaults to loopback — the literal `server.listen(port, "127.0.0.1")`
   * that used to be hardcoded here, so the desktop is unchanged (`plan/15` work item 2).
   *
   * **A host-local value, deliberately not an `AppConfig` field.** `AppConfig` is the *per-workspace,
   * git-tracked* config and it **syncs** (`store/gitStore.ts`) — a bind host is a property of the machine,
   * so storing it there would carry one machine's interface name to another and back. It is the same rule
   * that keeps resolved ports in the host-local `runtime.json` rather than in `workspace.json`
   * (`plan/14` §4). The deployable host takes it as an option, from its CLI.
   *
   * **The exposure is larger than the RPC bind's, and the plan says so** (`plan/15` §9): this port serves a
   * proxy that forwards into whatever the machine can reach *and* performs TLS interception, so a wide bind
   * is a route into a private network rather than a control channel.
   *
   * **Which is why it is loopback, and only loopback.** `plan/15` work item 2 gave it its own opt-in on the
   * argument that one flag covering both listeners would mean accepting the larger exposure in order to get
   * the smaller; the 2026-09-25 decision that the engine is never deployed remotely removed the exposure
   * itself, and the opt-in with it. `bindHost` stays a parameter because a test may want to *prove* the
   * refusal, and because the constant should have one definition rather than two.
   */
  bindHost?: string;
}

/**
 * The address a proxy binds when its caller names none — the pre-`plan/15` literal, in one place, and
 * since 2026-09-25 the only address it will accept.
 */
export const DEFAULT_PROXY_BIND_HOST = "127.0.0.1";

/**
 * Create one proxy server. Every mutable the module used to hold lives here instead, so two callers get
 * two independent servers, two ports and two error states.
 */
export function createProxyServer(opts: ProxyServerOptions = {}): ProxyServerInstance {
  const workspaceId = opts.workspaceId ?? null;
  const ownsCA = opts.ownsCA ?? true;
  const bindHost = opts.bindHost ?? DEFAULT_PROXY_BIND_HOST;

  /**
   * ## The bind guard — now unconditional
   *
   * `plan/15` work item 2 made this an opt-in (`allowNonLoopback`). As of **2026-09-25 there is no
   * opt-in**, because there is no deployment in which a wide bind is wanted: the engine is never
   * deployed remotely. Encoding the rule means the engine cannot be talked into publishing this port by
   * a config typo — the process refuses rather than exposing one. It is deliberately *not* left to the
   * caller to check, and it is deliberately **not** duplicated in `runtime/host.ts`: the module that
   * binds owns the rule about binding.
   *
   * **It throws from the factory rather than from `start()`.** This is a *configuration* error, not a
   * bind failure: a caller that named `0.0.0.0` has nothing to retry, so there is no value in waiting
   * until the first workspace is being placed — and by then the host has bootstrapped a data root it did
   * not need to touch.
   */
  if (!isLoopbackHost(bindHost)) {
    throw new EngineError(
      "ENGINE_ERROR",
      `Refusing to bind the proxy to ${bindHost}: this listener binds loopback only. A forward proxy ` +
        `forwards into whatever this machine can reach and performs TLS interception, so a wide bind is ` +
        `a route into a private network; the engine is never deployed remotely, so there is no opt-in ` +
        `for it and no deployment that would need one. Bind ${DEFAULT_PROXY_BIND_HOST}.`,
    );
  }

  let server: net.Server | null = null;
  let currentPort = 0;
  let currentConfig: AppConfig;
  let fullRules: ProxyRule[] = [];
  let lastError: string | null = null;
  let enabledSets: EnabledSets = { mocks: new Set(), mappings: new Set(), rules: new Set() };

  /**
   * This instance's own workspace-scoped entities — mocks, mappings, environments. (P14 WI 4's gap, closed
   * 2026-09-22.)
   *
   * `currentConfig` cannot supply them: it comes from `loadConfig()`, which is **active-workspace-only**, so
   * for a host member it holds the *active* workspace's mocks and mappings while `fullRules` and
   * `enabledSets` hold the instance's. `workspaceCfg()` used to cross-filter the two, which matched nothing
   * and left the instance serving nothing at all — silently. Set by `start()` / `reloadConfig()`; see
   * `loadWorkspaceDispatchSlice()` for the measurement.
   */
  let wsSlice: WorkspaceDispatchSlice | null = null;

  function stop(): void {
    if (server) {
      server.close();
      server = null;
      // See the header: this is process-global TLS state. Correct for a proxy that owns the CA (the
      // desktop's single one, and the module-level facade); a host running N proxies passes
      // `ownsCA: false` so one runtime's stop cannot unload the CA the others are signing with.
      if (ownsCA) {
        unloadCA();
        clearCertCache();
      }
      console.log(`[server:${currentPort}] stopped`);
    }
  }

  function start(port: number): void {
    if (server) stop();
    currentConfig = loadConfig();
    const ws = servedWorkspaceId();
    fullRules = readAllEntities<ProxyRule>(ws, "rules");
    enabledSets = loadEnabledSets(ws);
    wsSlice = loadWorkspaceDispatchSlice(ws);
    currentPort = port;
    lastError = null;

    if (currentConfig.tlsEnabled && currentConfig.tlsCaCertPath && currentConfig.tlsCaKeyPath) {
      // Idempotent for a shared CA: the paths come from the same config, so re-loading yields the same
      // PEMs. Not gated on `ownsCA` — a member of a host still needs *its* configured CA loaded before
      // it can intercept, it just does not get to unload it.
      loadCA(currentConfig.tlsCaCertPath, currentConfig.tlsCaKeyPath);
    } else if (ownsCA) {
      // Only an owner may clear the CA for a config that has TLS off. A non-owner seeing TLS off means
      // *its own* workspace is not intercepting; unloading would silently break a sibling that is.
      unloadCA();
    }

    server = net.createServer((socket) => {
      let buf = Buffer.alloc(0);
      let dispatched = false;

      socket.on("data", (chunk) => {
        if (dispatched) return;
        buf = Buffer.concat([buf, chunk]);
        const sep = buf.indexOf("\r\n\r\n");
        if (sep === -1) return;
        dispatched = true;

        const headerSection = buf.slice(0, sep).toString("utf-8");
        const bodyBuf = buf.slice(sep + 4);
        const lines = headerSection.split("\r\n");
        const parts = lines[0].split(" ");
        const method = parts[0] ?? "GET";
        const rawTarget = parts[1] ?? "/";

        if (method === "CONNECT") {
          const lastColon = rawTarget.lastIndexOf(":");
          const host = rawTarget.slice(0, lastColon);
          const tlsPort = parseInt(rawTarget.slice(lastColon + 1) || "443", 10);
          if (currentConfig.tlsEnabled && isCALoaded()) {
            interceptTls(socket, host, tlsPort, (tlsSocket, decMethod, decTarget, decHeaders, decBody, hostname, hPort) => {
              dispatchHttps(tlsSocket, decMethod, decTarget, decHeaders, decBody, hostname, hPort);
            });
          } else {
            tcpTunnel(socket, host, tlsPort, null, "HTTP/1.1 200 Connection Established\r\n\r\n");
          }
        } else {
          dispatch(socket, method, rawTarget, lines.slice(1), bodyBuf);
        }
      });

      socket.on("error", () => socket.destroy());
    });

    server.on("error", (err: NodeJS.ErrnoException) => {
      if (err.code === "EACCES") {
        lastError = `Permission denied — run as Administrator to bind port ${port}`;
        console.error(`[server:${port}]`, lastError);
      } else if (err.code === "EADDRINUSE") {
        lastError = `Port ${port} is already in use`;
        console.error(`[server:${port}]`, lastError);
      } else {
        lastError = err.message;
        console.error(`[server:${port}] error:`, err.message);
      }
      logEmitter.emit("server-error", lastError);
    });

    server.listen(port, bindHost, () => {
      lastError = null;
      console.log(`[server:${port}] listening on ${bindHost}:${port}`);
    });
  }

  function reloadConfig(): void {
    currentConfig = loadConfig();
    const ws = servedWorkspaceId();
    fullRules = readAllEntities<ProxyRule>(ws, "rules");
    enabledSets = loadEnabledSets(ws);
    wsSlice = loadWorkspaceDispatchSlice(ws);
  }

  /**
   * The workspace this instance serves — **its own**, per P14 WI 4.
   *
   * The module-level facade and the desktop construct without a `workspaceId`, and for them "the active
   * workspace" is the right answer: there is exactly one proxy and it is showing what the user is looking at.
   * A host member is constructed *with* an id and must serve that workspace's rules regardless of which one
   * the UI happens to have selected — otherwise two runtimes on one host serve identical rule sets and the
   * acceptance criterion ("each serving its own rules") fails while every unit test stays green.
   *
   * The fallback is deliberately read **per call**, not captured in `start()`: the facade's behaviour is that
   * switching workspace in the UI changes what the single proxy serves, and P14's WI 7 keeps that until the
   * desktop adopts the runtime. Capturing it once would silently freeze the desktop on whichever workspace
   * was active at startup.
   *
   * Returns `string`, not `string | null`: `activeWorkspaceId` is declared non-nullable (`store/config.ts:27`)
   * and every config shape has one, so the fallback always yields a real id. The `null` branch exists only
   * for a caller that constructed an instance before any config was loadable — and resolving *that* to the
   * empty string, as the module did, would read a workspace called `""` rather than fail. Keeping the return
   * type honest is what let the compiler catch this: the first version returned `string | null` and `tsup`'s
   * dts leg rejected the four call sites.
   */
  function servedWorkspaceId(): string {
    return workspaceId ?? (currentConfig ?? loadConfig()).activeWorkspaceId;
  }

  /**
   * The config this instance dispatches with — the four inputs `start()` / `reloadConfig()` load,
   * composed by `workspaceCfg()`. Public because the edge router routes the **same** workspace's
   * mappings and must not compose a second copy; see `ProxyServerInstance.dispatchConfig`.
   *
   * Both dispatch paths below call this, and both always did compute this same expression. The
   * extraction is not new behaviour — it is the removal of a duplicated one, which is what made the
   * edge's need for it cheap to satisfy.
   */
  function dispatchConfig(): AppConfig {
    return workspaceCfg(
      currentConfig ?? loadConfig(),
      fullRules,
      enabledSets,
      wsSlice ?? loadWorkspaceDispatchSlice(servedWorkspaceId()),
    );
  }

  // ── HTTP dispatch (instance-scoped: reads this instance's config/rules) ──

  /**
   * ## This is the **forward** half — `plan/15` work item 3
   *
   * The reverse half is no longer here. A request whose `Host` is a mapped domain, and the home page
   * that listed those mappings, are served by `reverseproxy/edge.ts` on the host-level **edge** port. What
   * remains is what a browser sends when it has been *configured to use this port as its proxy*: an
   * absolute-form target for `http://`, and `CONNECT` for the https it wants tunnelled — the latter
   * handled by the connection handler, not here.
   *
   * That is why this function still parses a relative request at all: it does not serve one, it
   * **refuses** one, and the refusal names where to go instead.
   */
  function dispatch(
    socket: net.Socket,
    method: string,
    rawTarget: string,
    headerLines: string[],
    bodyBuf: Buffer,
  ): void {
    const cfg = dispatchConfig();
    const t0 = Date.now();
    const id = mkId();

    // Parse headers, strip hop-by-hop
    const headers: Record<string, string> = {};
    for (const line of headerLines) {
      const idx = line.indexOf(":");
      if (idx === -1) continue;
      const key = line.slice(0, idx).trim().toLowerCase();
      if (!HOP_BY_HOP.has(key)) headers[key] = line.slice(idx + 1).trim();
    }

    const hostHeader = (headers["host"] ?? "").toLowerCase();
    const host = hostHeader.split(":")[0];

    let reqPath = "/";
    if (rawTarget.startsWith("http://") || rawTarget.startsWith("https://")) {
      try { const u = new URL(rawTarget); reqPath = u.pathname + u.search; } catch { reqPath = "/"; }
    } else {
      reqPath = rawTarget || "/";
    }

    const url = rawTarget.startsWith("http") ? rawTarget : `http://${hostHeader}${reqPath}`;
    const reqBodyB64 = bodyBuf.length > 0 ? bodyBuf.toString("base64") : "";

    const baseEntry: Omit<RequestLogEntry, "status" | "via" | "target" | "durationMs" | "resHeaders" | "resBody" | "resStatus"> = {
      id, ts: t0, method, url, host,
      reqHeaders: headers,
      reqBody: reqBodyB64,
      workspaceId: cfg.activeWorkspaceId,
    };

    // ── the forward-proxy path — the only HTTP path this port serves ─────────
    //
    // Still numbered 3, and the numbering is kept on purpose: the branches that were 1 and 2 are the
    // ones that moved, so a reader looking for them finds a pointer here rather than a gap.
    if (rawTarget.startsWith("http://") || rawTarget.startsWith("https://")) {
      // 3a. Proxy routing (mocks, rules)
      const env = activeEnv(cfg);
      if (handleProxyRouting(socket, cfg, env, method, url, rawTarget, reqPath, headers, bodyBuf, reqBodyB64, baseEntry, t0)) {
        return;
      }

      // 3c. Passthrough
      passthroughToUpstream(socket, method, rawTarget, reqPath, headers, bodyBuf, (status, dur, resH, resB) => {
        emitLog({ ...baseEntry, status, via: "proxy", target: host, durationMs: dur, resHeaders: resH, resBody: resB, resStatus: status });
      }, baseEntry.id);
      return;
    }

    /**
     * ## 4. anything else → refused, and the refusal says where to go
     *
     * This branch used to read `400 Not a localhost domain`, and a relative request for an unmapped
     * host was one of several things that reached it. It is now reached by **every** relative request,
     * because the mapped ones belong to the edge — `*.localhost` included, which this port answered
     * until work item 3.
     *
     * **It names the edge, not a port number, and that is deliberate.** Naming the port would mean
     * threading the edge's port into this module, which is the one direction of coupling the split
     * exists to remove: the edge knows nothing about this server, and a forward proxy that knew the
     * edge's port would be the start of the two knowing about each other. The port is discoverable
     * where it is actually useful — in the app, and in the host's startup line for the edge.
     */
    sendHtml(socket, 400,
      "<h1>400 Bad Request</h1>" +
      "<p>This is Bifurc's <strong>forward-proxy</strong> port, and this request did not arrive as a " +
      "proxy request.</p>" +
      "<p>Domain mappings (<code>*.localhost</code>) and the home page are served by the " +
      "<strong>edge</strong> — the port Bifurc shows as your mapping port.</p>");
    emitLog({ ...baseEntry, status: 400, via: "error", target: null, durationMs: Date.now() - t0, resHeaders: {}, resBody: "", resStatus: 400 });
  }

  // ── HTTPS dispatch (TLS intercepted) ────────────────────────────────────

  function dispatchHttps(
    socket: import("tls").TLSSocket,
    method: string,
    rawTarget: string,  // relative path, e.g. "/api/users"
    headerLines: string[],
    bodyBuf: Buffer,
    hostname: string,
    port: number,
  ): void {
    const cfg = dispatchConfig();
    const t0 = Date.now();
    const id = mkId();

    // Parse headers, strip hop-by-hop
    const headers: Record<string, string> = {};
    for (const line of headerLines) {
      const idx = line.indexOf(":");
      if (idx === -1) continue;
      const key = line.slice(0, idx).trim().toLowerCase();
      if (!HOP_BY_HOP.has(key)) headers[key] = line.slice(idx + 1).trim();
    }

    const reqPath = rawTarget || "/";
    const portSuffix = port === 443 ? "" : `:${port}`;
    const url = `https://${hostname}${portSuffix}${reqPath}`;
    const reqBodyB64 = bodyBuf.length > 0 ? bodyBuf.toString("base64") : "";

    const baseEntry: Omit<RequestLogEntry, "status" | "via" | "target" | "durationMs" | "resHeaders" | "resBody" | "resStatus"> = {
      id, ts: t0, method, url, host: hostname,
      reqHeaders: headers,
      reqBody: reqBodyB64,
      workspaceId: cfg.activeWorkspaceId,
    };

    // Proxy routing (mocks, rules)
    const env = activeEnv(cfg);
    if (handleProxyRouting(socket, cfg, env, method, url, url, reqPath, headers, bodyBuf, reqBodyB64, baseEntry, t0)) {
      return;
    }

    // 3. Passthrough — forward to real upstream over HTTPS
    passthroughToUpstreamHttps(socket, method, hostname, port, reqPath, headers, bodyBuf, (status, dur, resH, resB) => {
      emitLog({ ...baseEntry, status, via: "proxy", target: hostname, durationMs: dur, resHeaders: resH, resBody: resB, resStatus: status });
    }, baseEntry.id);
  }

  return {
    workspaceId,
    start,
    stop,
    isRunning() {
      return server !== null && server.listening;
    },
    getPort() {
      return currentPort;
    },
    getError() {
      return lastError;
    },
    reloadConfig,
    // Exposed for the edge router, which routes the same workspace's mappings and must not compose a
    // second copy of them. See the interface's docblock.
    dispatchConfig,
  };
}

// ── The module-level facade ───────────────────────────────────────────────────
//
// One lazily-created default instance backing the pre-P14 module functions, the same shape the webhook
// receiver took. Lazy rather than eager so importing this module has no side effect.
//
// **Not "removed in WI 7", which is what this header used to say.** WI 7's desktop half landed 2026-09-22
// and kept the facade *deliberately*, and `Engine_Module_Plan.md` §4.1 later confirmed the reason: the facade
// is what lets a call mean "the active workspace's server, wherever it lives". What changed is that it can now
// be **bound** to an instance someone else owns — see `bindProxyInstance()`.
//
// **The fan-out no longer lands here.** This header used to enumerate the importers of `reloadConfig()`
// (17 × `importExport/importers/*`, `startup.ts`, `store/{configCommands,coreCommands,entityCrudCommands}.ts`,
// `sync/syncCommands.ts`, and a `transport/legacyCompanion.ts` that has since been deleted). Those 29 call
// sites across 21 files now go through `runtime/instances.ts`'s `notifyConfigChanged()`, and **no module
// outside `runtime/` imports this function for the fan-out any more** (`Engine_Module_Plan.md` §10 criterion 4).
// `reloadConfig` remains a legitimate *proxy* API — `tests/integration/facadeBinding.integration.test.ts`
// exercises it as one, and `runtime/instances.ts` calls it — but it is no longer the engine's config seam.

let defaultInstance: ProxyServerInstance | null = null;

/**
 * P14 WI 7 — the instance every module function acts on, when someone has adopted one.
 *
 * ## Why an explicit binding, and not a registry lookup
 *
 * The obvious shape — `getRuntime(loadConfig().activeWorkspaceId)?.proxy` — is **wrong here**, and WI 4 is
 * why. `workspace.setActive` deliberately no longer calls `reloadConfig()`, so on the desktop the active
 * pointer and the running proxies are **independent by design**: the port is owned by the workspace, not by
 * which one is selected. Resolving by the active id would, after a switch, look up a runtime for the newly
 * active workspace, find none, and fall back to the lazy default below — whose `isRunning()` is `false`
 * while the real proxy is listening on a bound port. That is the WI 1 bug shape ("the failure succeeds")
 * arriving through a new door.
 *
 * So the binding names no workspace: whoever adopted an instance says so, and every module function follows
 * it. `src/main.ts` binds its one `WorkspaceRuntime`'s proxy here; `createEngine()` and every test bind
 * nothing and get the lazy default, unchanged.
 *
 * **Why this module does not import `runtime/`:** `runtime/host.ts` imports *this* module as a value (it is
 * the composition root, and it binds each listener here). A value import in the other direction would close
 * that loop, so a binding — a parameter — is the only edge this module may have to the runtime layer.
 *
 * (This note used to cite `runtimeRegistry.ts` and `runtime/workspaceRuntime.ts`. **Both are deleted** —
 * `runtime/host.ts` replaced them, as its own header records. The acyclicity requirement is unchanged; only
 * the files satisfying it moved.)
 */
let boundInstance: ProxyServerInstance | null = null;

/**
 * Adopt `instance` as the instance every module function acts on, or `null` to return to the lazy default.
 *
 * Idempotent and last-write-wins. `src/main.ts` calls it once at startup; a test calls it with `null` to get
 * a clean module back. Nothing else should call it — a second binder would silently steal the first one's
 * server, which is why the parameter is an instance rather than a workspace id.
 */
export function bindProxyInstance(instance: ProxyServerInstance | null): void {
  boundInstance = instance;
}

/** The instance the module functions act on. Creates the lazy default only when nothing is bound. */
function defaultServer(): ProxyServerInstance {
  if (boundInstance) return boundInstance;
  if (!defaultInstance) defaultInstance = createProxyServer({ workspaceId: null });
  return defaultInstance;
}

/**
 * The instance to *read* from, without creating one.
 *
 * `isRunning()` / `getPort()` / `getServerError()` / `stopServer()` must not have the side effect of
 * creating an instance: they are reached from `status()` paths and from readiness polls, and
 * `createEngine()`'s `waitUntilReady()` polls `isRunning()` in a loop. `defaultServer()` would create the
 * lazy default on the first poll of a never-started engine, which would be a behaviour change for every
 * caller that only ever asked. This preserves the pre-WI-7 property exactly — the old bodies were
 * `defaultInstance !== null && defaultInstance.isRunning()` and `defaultInstance?.…`.
 */
function existingServer(): ProxyServerInstance | null {
  return boundInstance ?? defaultInstance;
}

export function startServer(port: number): void {
  defaultServer().start(port);
}

export function stopServer(): void {
  existingServer()?.stop();
}

export function isRunning(): boolean {
  return existingServer()?.isRunning() ?? false;
}

export function getPort(): number {
  return existingServer()?.getPort() ?? 0;
}

export function getServerError(): string | null {
  return existingServer()?.getError() ?? null;
}

export function reloadConfig(): void {
  defaultServer().reloadConfig();
}

// ── Replay a captured request, return {status, headers, body (base64)} ────
//
// Re-exported from `utils/replay.ts`, where the implementation moved in engine-layout phase 7: both
// this module's routing path and the `request.replay` command (`rest/commands.ts`) need it, and §5.1
// forbids two feature modules importing each other, so the shared behaviour was extracted down.
// `routingUtils.ts` still imports it from here, unchanged.

export { replayRequest } from "../utils/replay";
