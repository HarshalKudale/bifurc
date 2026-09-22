import * as net from "net";
import * as http from "http";
import * as https from "https";
import { loadConfig, loadWorkspaceDispatchSlice, AppConfig, Environment, ProxyRule, type WorkspaceDispatchSlice } from "../store/config";
import { readEnabledSet, bootstrapEnabledSet, readAllEntities } from "../store/workspaceFs";
import { HOP_BY_HOP } from "./constants";
import { sendHtml, buildHomePage, buildNotMappedPage } from "./pages";
import { matchMock, serveMock, isFullyMocked, mergeMockWithUpstream, resolveMockOnlyResponse, serveResolvedResponse } from "./mockHandler";
import { matchGraphQLMock, matchSoapMock, serveProtocolMock, GraphQLMockDef, SoapMockDef } from "./protocolMockHandler";
import { tcpTunnel, proxyToUpstream, passthroughToUpstream, passthroughToUpstreamHttps, matchProxyRule, proxyWithScripts, fetchUpstreamResponse } from "./proxyHandler";
import { loadCA, unloadCA, isCALoaded, clearCertCache } from "./tlsCert";
import { interceptTls } from "./tlsIntercept";
import { decompressBody, stripContentEncoding } from "./decompressUtils";
import { handleProxyRouting } from "./routingUtils";
import { RequestLogEntry, logEmitter, emitLog, emitLogChunk } from "./logEmitter";
export { RequestLogEntry, logEmitter, emitLogChunk } from "./logEmitter";

import { mkId, activeEnv, EnabledSets, loadEnabledSets, workspaceCfg } from "./serverUtils";

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
}

/**
 * Create one proxy server. Every mutable the module used to hold lives here instead, so two callers get
 * two independent servers, two ports and two error states.
 */
export function createProxyServer(opts: ProxyServerOptions = {}): ProxyServerInstance {
  const workspaceId = opts.workspaceId ?? null;
  const ownsCA = opts.ownsCA ?? true;

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

    server.listen(port, "127.0.0.1", () => {
      lastError = null;
      console.log(`[server:${port}] listening on 127.0.0.1:${port}`);
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

  // ── HTTP dispatch (instance-scoped: reads this instance's config/rules) ──

  function dispatch(
    socket: net.Socket,
    method: string,
    rawTarget: string,
    headerLines: string[],
    bodyBuf: Buffer,
  ): void {
    const cfg = workspaceCfg(
      currentConfig ?? loadConfig(),
      fullRules,
      enabledSets,
      wsSlice ?? loadWorkspaceDispatchSlice(servedWorkspaceId()),
    );
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
    };

    // 1. localhost base → home page (no log)
    if (host === "localhost") {
      sendHtml(socket, 200, buildHomePage(cfg, currentPort));
      return;
    }

    // 2. *.localhost → mapping lookup (RFC 6761) — no mock matching here, these are internal
    if (host.endsWith(".localhost")) {
      const mapping = cfg.mappings.find((m) => m.enabled && m.domain.toLowerCase() === host);
      const env = activeEnv(cfg);
      const mock = matchMock(cfg.mocks, method, url, env);
      if (mock) {
        if (isFullyMocked(mock)) {
          serveMock(socket, mock, env);
          const resolved = resolveMockOnlyResponse(mock, env);
          const slice = resolved.body.length <= 512 * 1024 ? resolved.body : resolved.body.slice(0, 512 * 1024);
          emitLog({ ...baseEntry, status: resolved.status, via: "mock", target: `mock:${mock.id}`, durationMs: Date.now() - t0 + resolved.delayMs, resHeaders: resolved.headers, resBody: slice.toString("base64"), resStatus: resolved.status });
          return;
        }

        if (!mapping) {
          sendHtml(socket, 502, `<h1>502 Bad Gateway</h1><p>No downstream mapping exists for <code>${host}</code>, so this partial mock cannot fetch live headers.</p>`);
          emitLog({ ...baseEntry, status: 502, via: "error", target: null, durationMs: Date.now() - t0, resHeaders: {}, resBody: "", resStatus: 502 });
          return;
        }

        fetchUpstreamResponse(method, mapping.target, reqPath, headers, bodyBuf).then((upstream) => {
          const merged = mergeMockWithUpstream(mock, upstream, env);
          serveResolvedResponse(socket, merged);
          const slice = merged.body.length <= 512 * 1024 ? merged.body : merged.body.slice(0, 512 * 1024);
          emitLog({ ...baseEntry, status: merged.status, via: "mock", target: `mock:${mock.id}`, durationMs: upstream.durationMs + merged.delayMs, resHeaders: merged.headers, resBody: slice.toString("base64"), resStatus: merged.status });
        }).catch((err: Error) => {
          console.error(`[mock downstream] ${mapping.target}${reqPath} —`, err.message);
          sendHtml(socket, 502, `<h1>502 Bad Gateway</h1><p>Could not connect to <code>${mapping.target}</code>.</p>`);
          emitLog({ ...baseEntry, status: 502, via: "error", target: null, durationMs: Date.now() - t0, resHeaders: {}, resBody: "", resStatus: 502 });
        });
        return;
      }

      if (mapping) {
        proxyToUpstream(socket, method, mapping.target, reqPath, headers, bodyBuf, (status, dur, resH, resB) => {
          emitLog({ ...baseEntry, status, via: "rfc6761", target: mapping.target, durationMs: dur, resHeaders: resH, resBody: resB, resStatus: status });
        }, baseEntry.id);
      } else {
        sendHtml(socket, 404, buildNotMappedPage(host, currentPort));
        emitLog({ ...baseEntry, status: 404, via: "error", target: null, durationMs: Date.now() - t0, resHeaders: {}, resBody: "", resStatus: 404 });
      }
      return;
    }

    // 3. Forward proxy path (always active for absolute http:// targets)
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

    sendHtml(socket, 400, "<h1>400 Bad Request</h1><p>Not a localhost domain.</p>");
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
    const cfg = workspaceCfg(
      currentConfig ?? loadConfig(),
      fullRules,
      enabledSets,
      wsSlice ?? loadWorkspaceDispatchSlice(servedWorkspaceId()),
    );
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
  };
}

// ── The module-level facade ───────────────────────────────────────────────────
//
// One lazily-created default instance backing the pre-P14 module functions, the same shape the webhook
// receiver took. Lazy rather than eager so importing this module has no side effect.
//
// **Not "removed in WI 7", which is what this header used to say.** WI 7's desktop half landed 2026-09-22
// and kept the facade *deliberately*: 23 engine modules import `reloadConfig()` from here (17 ×
// `importExport/importers/*`, plus `startup.ts`, `store/{configCommands,coreCommands,entityCrudCommands}.ts`,
// `sync/syncCommands.ts`, `transport/legacyCompanion.ts`), and the facade is what lets that keep meaning
// "the active workspace's server, wherever it lives". What changed is that it can now be **bound** to an
// instance someone else owns — see `bindProxyInstance()`.

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
 * **Why this module does not import `runtime/`:** `runtimeRegistry.ts`'s header records that its graph must
 * stay acyclic, and `runtime/workspaceRuntime.ts` imports *this* module as a value. A value import in the
 * other direction would close that loop. A binding is a parameter, not a lookup, so there is no edge at all.
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

export { replayRequest } from "./serverReplay";
