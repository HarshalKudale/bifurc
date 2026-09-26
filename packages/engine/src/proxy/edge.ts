/**
 * The **edge router** — `plan/15` §3.3, work item 3.
 *
 * ## What this is
 *
 * The **host-level** listener for product traffic: one port, one server, for every workspace on the
 * machine. It serves the reverse half of what used to be the proxy's single socket — a request whose
 * `Host` matches one of the **active workspace's** mappings is forwarded to that mapping's `target`,
 * with mocks applied first — and falls back to the home page, which is the directory of those
 * mappings. That fallback is `proxy/server.ts`'s old `host === "localhost"` branch, moved here.
 *
 * ## Why it is a separate server, and why *this* half moved
 *
 * `plan/15` §1: the two jobs the proxy did — reverse and forward — shared one socket, so a mock could
 * not be applied to forward-proxied traffic without also affecting reverse-proxied traffic. Splitting
 * them is what makes *"any middle data can be mocked while using forward proxy"* a consequence of the
 * design rather than a feature to build (§3.4).
 *
 * The reverse half is the half that **must** be host-level, and it is §3.1's routing rule that says
 * so — *a port belongs to the thing that has to be addressed once*. A mapping is a hostname
 * (`bifurc.localhost`), and a hostname carries no workspace in it, so the port that answers it has to
 * be one port for the whole host. The forward half is addressed by *a browser's proxy setting*, which
 * is a per-machine choice the user makes per workspace, so it stays per workspace.
 *
 * ## It is an `http.Server`, and it deliberately never touches `res`
 *
 * §3.3 specifies an `http`/`https` server and one `upgrade` handler, and that is what this is: the request
 * path carries a **global mapping from the base domain to the web GUI's own port** — `plan/15` **work item
 * 5, built 2026-09-24**, and *"just another reverse-proxy mapping"* rather than a feature this module owns
 * — and the `upgrade` event is where a WebSocket arrival will land (**work item 6, still reserved**). One of
 * the two is real now and the other has no placeholder, because a branch that returns "not built yet" would
 * be a routing decision nobody made.
 *
 * **The `upgrade` half used to say "the companion bridge", and that reader is now gone.** The companion
 * server was retired outright while implementing `plan/16` — it was a v1 frozen bridge the extension
 * connected to on its own port, never a client of this listener — so nothing upgrades here today. The
 * hook stays because §3.3 still reserves the branch; the arrival it will carry is the GUI's own socket.
 *
 * The reverse branch must hand a request to `proxyToUpstream()` / `serveMock()`, which write raw bytes
 * to a **socket**, and it cannot buffer instead: `handleProxyResponse()` streams, so buffering would
 * break SSE and large downloads. So the handler takes `req.socket` and never touches `res`.
 *
 * **That is a measured decision, not an assumption.** Probe, 2026-09-23, node 22.22.2: a server whose
 * handler writes a complete raw response to `req.socket` and never touches `res` — twice, on one
 * keep-alive connection — delivered both responses and reported no `error` and no `clientError`. Every
 * response written below is `connection: close` and ends the socket, so a hijacked response is never
 * followed by one Node still thinks it owes.
 *
 * ## Which workspace's mappings — re-resolved per request, deliberately
 *
 * The edge serves the **active** workspace's mappings (§3.3) and re-resolves which workspace that is
 * on every request rather than caching it. That is not an optimisation that was skipped: it is what
 * makes `workspace.setActive` need **no invalidation step**. The switch already exists and already
 * deliberately does not call `reloadConfig()` — a port is owned by its workspace, not by which one is
 * selected — so an edge that cached the active id would keep serving the previous workspace's domains
 * until something remembered to tell it.
 *
 * `resolveConfig` is a **parameter**, not a lookup, because the answer belongs to the host: only the
 * host holds `runtimes: Map<workspaceId, WorkspaceRuntime>`. And the config it must return is the
 * *dispatch* config — the same `workspaceCfg()`-composed object the workspace's proxy dispatches
 * with — which is why the host asks the runtime's proxy for it (`ProxyServerInstance.dispatchConfig`)
 * rather than composing a second copy from the same files. Two readers of one config is how the two
 * halves drift; `store/serverUtils.ts#workspaceCfg`'s docblock is the record of what that costs.
 *
 * ## The bind policy is its own, and it has no TLS half
 *
 * Loopback by default, like every other listener, and a non-loopback bind is opt-in and explicit. The
 * reasoning is `createProxyServer()`'s applied to a surface that is at least as exposed: a published
 * edge is a reverse proxy into whatever the machine can reach, addressed by a `Host` header the
 * *client* chooses (`plan/15` §9's SSRF row). So it takes **its own** opt-in — accepting the RPC
 * endpoint's or the forward proxy's exposure must not imply accepting this one.
 *
 * There is deliberately **no TLS half**, for the forward proxy's reason: a browser speaks plain HTTP
 * to this listener. §3.1's "`443` with TLS" is a later item about *terminating* TLS here, not a
 * condition on binding wide.
 *
 * ## The port is not scanned upward, and that is D16 rather than an omission
 *
 * The edge binds exactly the port it is given and reports a failure through `getError()` instead of
 * moving. D16's scan-upward rule exists for a port nobody addressed; the edge's port **is** addressed
 * — it is the port every `*.localhost` mapping already points at, and the port the home page's own
 * links are built from. A silently moved edge would be a proxy whose mappings answer somewhere other
 * than where the user typed them, which is a worse failure than refusing to start.
 */
import * as net from "net";
import * as http from "http";
import { EngineError } from "@bifurc/protocol";
import { isLoopbackHost } from "../lib/net";
import { AppConfig } from "../store/config";
import { HOP_BY_HOP } from "./constants";
import { sendHtml, buildHomePage, buildNotMappedPage, buildUnservedPage } from "./pages";
import {
  matchMock,
  serveMock,
  isFullyMocked,
  mergeMockWithUpstream,
  resolveMockOnlyResponse,
  serveResolvedResponse,
} from "./mockHandler";
import { proxyToUpstream, fetchUpstreamResponse } from "./proxyHandler";
import { RequestLogEntry, logEmitter, emitLog } from "./logEmitter";
import { mkId, activeEnv, hostnameOf } from "./serverUtils";
import { DEFAULT_HOST_DOMAIN, normalizeHostname } from "../lib/net";

export interface EdgeRouterOptions {
  /**
   * Resolve the config the edge routes against. **Called per request**, and it must return the
   * *active* workspace's dispatch config — the same object that workspace's proxy dispatches with.
   *
   * A function rather than a value so that a `workspace.setActive` needs no invalidation step; see the
   * header. It is also why there is no `reloadConfig()` on the instance below: there is nothing to
   * reload.
   *
   * ## `null` means "this host is not serving the active workspace"
   *
   * A real state rather than a defensive one: the host skips a workspace whose port override was taken
   * (D16 — loud, but not fatal to the host) and `--workspace` can filter the active one out. The edge
   * has no mappings to serve in that case, and both obvious substitutes are wrong — the ordinary home
   * page would claim the workspace has no mappings, and *another* workspace's config would route a
   * domain to a target the active workspace never configured. So it is a distinct answer, and `route()`
   * gives it a distinct page.
   */
  resolveConfig: () => AppConfig | null;
  /**
   * The interface the edge binds. Defaults to loopback — the same default, for the same reason, as
   * `createProxyServer()`'s `bindHost` (`plan/15` work item 2), and since 2026-09-25 the same
   * restriction: loopback spellings only, anything else is a hard error.
   *
   * Host-local by nature: the edge is one listener for the whole host, so there is no per-workspace
   * question to answer here even in principle.
   */
  bindHost?: string;
  /**
   * The **global mapping** the edge forwards base-domain requests to — the host and port the web GUI is
   * served on (`plan/15` work item 5).
   *
   * ## Why a target rather than a directory
   *
   * This is the owner's model, and it is the smaller one: *"the web UI starts as usual on some different
   * server like 5753 or any other port. Our edge port reverse proxy will forward base domain requests to
   * this web UI port, so that this base domain → webUI port becomes the global mapping in reverse proxy
   * config."*
   *
   * Serving the bundle from this listener instead would mean teaching a **router** about MIME types, cache
   * headers, path traversal and Vite's `assetsDir` convention — none of which belong in a module whose whole
   * job is *"route a `Host` to a target"*. As a target, the GUI is answered by machinery that already exists
   * (`proxyToUpstream`), and **a GUI served by nginx, `vite preview`, `serve`, or anything else works
   * identically** — which is the property that makes it *"just another reverse-proxy mapping"* rather than a
   * feature the engine has to own.
   *
   * ## Absent is the default, and there is deliberately no default port
   *
   * `undefined` means no GUI mapping, so the base domain falls through to the home page exactly as it did
   * before this work item. A plausible-looking default (`5753`) would forward the base domain to a dead port
   * and answer **502** on a fresh install — a worse failure than the home page, and a much worse one than
   * saying nothing, because it looks like a bug rather than an unconfigured feature.
   *
   * Format is `host:port` — the same shape as `LocalMapping.target`, so the two are read by the same
   * machinery — or a bare port, which means loopback.
   */
  guiTarget?: string;
  /**
   * The hostname this engine answers as — `localhost`, and **not configurable** since `--domain` was removed
   * on 2026-09-24 (`runtime/hostArgs.ts`'s header records why). The parameter stays because the edge takes its
   * own address as an input, exactly as it takes `bindHost`: this module is created *for* a deployment and does
   * not import that deployment's constants. The host composes it from `lib/net.ts#DEFAULT_HOST_DOMAIN`, which
   * is its only source.
   *
   * **This is branch G's first gate, and it is the reason the option exists here at all.** Before work item 5
   * the edge had no idea what its own name was, so "is this request addressed to *me*, or to one of my
   * mappings?" was unanswerable. The name was already threaded to the host (`hello`'s `hostDomain`) and
   * canonicalised with `normalizeHostname`; this is the second reader, and `plan/15` §3.3 predicted it —
   * the same value the mapping editor derives its suggestion from is the one the router compares against.
   *
   * **Canonicalised here as well as at the caller**, because a comparison is only as good as its two
   * spellings and `normalizeHostname` is idempotent. Validated nowhere, for the reason `host.ts` records:
   * a second validation would be a second opinion about what is acceptable.
   */
  hostDomain?: string;
}

/** The address the edge binds when its caller names none. Loopback, like every other listener. */
export const DEFAULT_EDGE_BIND_HOST = "127.0.0.1";

/** The instance a factory call returns. Deliberately narrower than `ProxyServerInstance`: see below. */
export interface EdgeRouterInstance {
  /**
   * Bind the given port. Fire-and-forget, like the proxy's and the webhook receiver's `start()`: a
   * bind failure is reported through `getError()`, not thrown, because the shell has always tolerated
   * a busy port and a host that threw here would turn one taken port into a failed startup.
   */
  start(port: number): void;
  stop(): void;
  isRunning(): boolean;
  getPort(): number;
  getError(): string | null;
}

/**
 * Create the edge router. One per **host** — not one per workspace — which is the whole point of the
 * tier it belongs to.
 *
 * There is no `workspaceId` and no `reloadConfig()` here, and their absence is the design rather than
 * an unfinished edge: this server holds no workspace-scoped state at all. It reads which workspace is
 * active from `resolveConfig` on each request and keeps nothing between them.
 */
/**
 * The global mapping's second gate: is this request addressed to **this machine** rather than to a name?
 *
 * (The first gate is `host === hostDomain`, the engine's own configured name.)
 *
 * ## Why this is not `isLoopbackHost`
 *
 * `lib/net.ts` already has a loopback test, and reusing it here would be wrong. That one answers *"may a
 * listener bind here?"* — a property of an interface. This answers *"did the client type this machine's
 * address?"* — a property of the request, and an edge bound to a LAN address is reached at `192.168.1.9`,
 * which is **not** loopback and **is** the machine. So every IP literal qualifies, not just `127.0.0.1`.
 * §3.3's row says *"the GUI, by bare IP or `localhost`"* for exactly that reason.
 *
 * ## `localhost` is not `*.localhost`
 *
 * The bare name is here; the suffix belongs to the workspace mappings. `host === "localhost"` deliberately
 * does not use `endsWith(".localhost")` and cannot: `"localhost".endsWith(".localhost")` is **false**, which
 * is why the bare name has always fallen past the reverse half — to the home page before work item 5, to the
 * GUI now. Anything under the suffix still routes to its mapping, or 404s.
 *
 * ## The IPv6 arm, and why it is cheap
 *
 * `hostnameOf()` has already removed the port, so a remaining `:` can only be an IPv6 literal's own. The
 * character class is belt-and-braces: a hostname cannot contain a colon after the port is stripped, so
 * `^[0-9a-f:]+$` cannot reject a name that should have matched.
 */
const IPV4_LITERAL = /^\d{1,3}(\.\d{1,3}){3}$/;

function isLocalAddress(host: string): boolean {
  if (host === "localhost") return true;
  if (IPV4_LITERAL.test(host)) return true;
  return host.includes(":") && /^[0-9a-f:]+$/.test(host);
}

export function createEdgeRouter(opts: EdgeRouterOptions): EdgeRouterInstance {
  const bindHost = opts.bindHost ?? DEFAULT_EDGE_BIND_HOST;
  /**
   * Branch 3's gate, and branch 4's. Read once rather than per request, because neither can change while
   * this listener is bound: a deployment's own name is a property of the deployment, and the GUI directory
   * is a property of the installation.
   */
  const hostDomain = normalizeHostname(opts.hostDomain ?? DEFAULT_HOST_DOMAIN);
  const guiTarget = opts.guiTarget;

  /**
   * ## The bind guard — now unconditional
   *
   * `plan/15` work item 3 made this the **third** opt-in, on the argument that the three exposures are
   * not equal — the RPC endpoint executes commands, the forward proxy forwards into the machine's
   * network, and this routes a *client-chosen* `Host` to a user-configured target — so one flag
   * covering them would make accepting the smallest mean accepting the largest. That argument was
   * about *which* flag to accept; the 2026-09-25 decision that the engine is never deployed remotely
   * removed the exposures themselves, and all three opt-ins with them.
   *
   * The same reasoning as `createProxyServer()`'s, and it throws from the factory rather than from
   * `start()` for that method's reason too: a *configuration* error has nothing to retry, and throwing
   * early means the host never bootstraps a data root it did not need to touch.
   */
  if (!isLoopbackHost(bindHost)) {
    throw new EngineError(
      "ENGINE_ERROR",
      `Refusing to bind the edge router to ${bindHost}: this listener binds loopback only. The edge ` +
        `serves the reverse proxy and the home page, addressed by a Host header the client chooses, so ` +
        `a wide bind publishes a reverse proxy into whatever this machine can reach; the engine is ` +
        `never deployed remotely, so there is no opt-in for it. Bind ${DEFAULT_EDGE_BIND_HOST}.`,
    );
  }

  let server: http.Server | null = null;
  let currentPort = 0;
  let lastError: string | null = null;

  function stop(): void {
    if (server) {
      server.close();
      server = null;
      console.log(`[edge:${currentPort}] stopped`);
    }
  }

  /**
   * The whole routing table, in the order `plan/15` §3.3 gives it — for the two branches that exist.
   *
   * | # | Condition | Result |
   * |---|---|---|
   * | 0 | `resolveConfig()` returned `null` | 503 — this host serves no workspace |
   * | G | `Host` is the engine's own domain, `localhost`, or a bare IP literal, **and a `guiTarget` is configured** | reverse proxy to the GUI's port — the **global mapping** |
   * | 2 | `Host` matches a mapping in the **active workspace's** config | reverse proxy to that `target` |
   * | 5 | otherwise | the **home page** |
   *
   * **`G` is checked first, and it is not one of §3.3's numbered rows.** That table describes the *router's*
   * jobs (1 = the upgrade arrival, 2 = a mapping, 3/4 = the GUI, 5 = the home page); the GUI is a mapping —
   * a **global** one — so it takes the position a global mapping takes, ahead of the per-workspace table.
   * Reading the code top to bottom gives precedence, which is why the order of the `if`s below is the
   * authority and this table only names them.
   *
   * Branch **1** (the `upgrade` arrival) is **work item 6** and is still reserved: it is not in the request
   * path at all, which is why it has no row here beyond this note. §3.3's rows 3 and 4 are what `G`
   * implements, and their *inert* state — no `guiTarget` configured — falls through to row 5 rather than
   * answering on its own.
   */
  function route(
    socket: net.Socket,
    method: string,
    reqPath: string,
    hostHeader: string,
    host: string,
    headers: Record<string, string>,
    bodyBuf: Buffer,
  ): void {
    const t0 = Date.now();
    const id = mkId();

    const url = reqPath.startsWith("http") ? reqPath : `http://${hostHeader}${reqPath}`;
    const reqBodyB64 = bodyBuf.length > 0 ? bodyBuf.toString("base64") : "";

    const baseEntry: Omit<RequestLogEntry, "status" | "via" | "target" | "durationMs" | "resHeaders" | "resBody" | "resStatus"> = {
      id, ts: t0, method, url, host,
      reqHeaders: headers,
      reqBody: reqBodyB64,
    };

    const cfg = opts.resolveConfig();

    // ── 0. this host serves no workspace → say so, and say only that ─────────
    //
    // Deliberately not a numbered row of §3.3's table: this is the state in which the table has no
    // subject. See `resolveConfig`'s note for why neither of the two substitutes is acceptable — and
    // note that it is checked **before** the home page, because the home page's whole content is the
    // active workspace's mappings and there are none to show.
    if (cfg === null) {
      sendHtml(socket, 503, buildUnservedPage());
      return;
    }

    // ── the GUI's global mapping: the base domain → the web UI's own port ───
    //
    // `plan/15` work item 5, built 2026-09-24 to the owner's model: *"the web UI starts as usual on some
    // different server like 5753 or any other port. Our edge port reverse proxy will forward base domain
    // requests to this web UI port, so that this base domain → webUI port becomes the global mapping in
    // reverse proxy config."*
    //
    // **It is a mapping, not a branch with a payload.** The action is `proxyToUpstream` — the very call
    // branch 2 makes — so the GUI is served by machinery that already exists, and a GUI run by nginx,
    // `vite preview` or anything else is indistinguishable from one this repository built. That is what
    // makes it *"just another reverse-proxy mapping"* rather than a feature the router has to own.
    //
    // **Checked before the workspace mappings, because it is the global one.** The `Host` this engine
    // answers as is a *deployment* fact; the mapping table is a *workspace* one. The deployment wins, so a
    // user who happens to have mapped `localhost` or the host's own domain still gets the GUI. The other
    // ordering would make the GUI vanish the moment anyone mapped its name — an interaction that reads as a
    // bug for a long time before it reads as a precedence rule.
    //
    // **`localhost` is not `*.localhost`.** `"localhost".endsWith(".localhost")` is false, so the bare name
    // has always fallen past the reverse half — to the home page before this change, to the GUI now. A
    // subdomain of it (`api.localhost`) still routes to its mapping or 404s. That asymmetry is RFC 6761's
    // rather than a special case invented here: the bare name is the machine, a subdomain is a name someone
    // claimed.
    //
    // **Logged, unlike the home page, and the label is `mapping` — because that is what it is.** The request
    // genuinely went through a target, so it belongs in the log, and `target` names where it went: that is
    // how an operator sees the base domain pointing at the GUI's port. A distinct `via` value would need the
    // protocol's union widened and the UI's badge taught, which correctness here does not require.
    if (guiTarget !== undefined && (host === hostDomain || isLocalAddress(host))) {
      proxyToUpstream(socket, method, guiTarget, reqPath, headers, bodyBuf, (status, dur, resH, resB) => {
        emitLog({ ...baseEntry, status, via: "mapping", target: guiTarget, durationMs: dur, resHeaders: resH, resBody: resB, resStatus: status });
      }, baseEntry.id);
      return;
    }

    // ── 2. a domain this host serves → the reverse proxy ────────────────────
    //
    // Moved here verbatim from `proxy/server.ts`'s dispatch, minus the workspace scoping it used to
    // get from the instance: `cfg` is now resolved per request instead of cached per instance, and it
    // is the active workspace's.
    //
    // **The gate is the mapping lookup, not the `.localhost` suffix** — work item 4, 2026-09-23.
    // `.localhost` is still special: RFC 6761 makes it resolve to the *browser's* loopback with no DNS
    // entry, which is why it is a mapping's default domain and the only kind that routes with no
    // configuration at all. It is no longer the only kind that routes. **Any domain the user
    // configured routes**, which is what a remote engine is for — §2.4 measured that nothing about
    // `LocalMapping` ever prevented it, and this suffix test was the whole obstacle.
    //
    // The two halves are separate conditions, and what each one's *miss* does is the difference:
    //
    //   | request | result |
    //   |---|---|
    //   | a mapped domain, of any kind | the reverse proxy |
    //   | an unmapped `.localhost` | the 404 below — whoever typed it expected a mapping |
    //   | an unmapped domain of any other kind | §3.3 row 5, the home page |
    //
    // The third row is worth stating rather than leaving implicit: an unmapped domain that is *not*
    // `.localhost` is not a mapping miss the user made, it is a request for a host this engine does not
    // serve. It reached the home page before this change too — the suffix test sent it there — so the
    // relaxation adds a route without moving one.
    const env = activeEnv(cfg);
    const mapping = cfg.mappings.find((m) => m.enabled && m.domain.toLowerCase() === host);
    const isLocalhostDomain = host.endsWith(".localhost");

    if (mapping !== undefined || isLocalhostDomain) {
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
          console.error(`[edge downstream] ${mapping.target}${reqPath} —`, err.message);
          sendHtml(socket, 502, `<h1>502 Bad Gateway</h1><p>Could not connect to <code>${mapping.target}</code>.</p>`);
          emitLog({ ...baseEntry, status: 502, via: "error", target: null, durationMs: Date.now() - t0, resHeaders: {}, resBody: "", resStatus: 502 });
        });
        return;
      }

      if (mapping) {
        /**
         * The label separates the two reasons a mapping answered, and this is the only place the
         * difference is visible to anyone. `.localhost` answered because RFC 6761 says it resolves to
         * loopback with no DNS entry; anything else answered because the user pointed that domain at
         * this engine. The mechanism is identical either way — see `logEmitter`'s note on `via` for why
         * the label is still worth splitting rather than calling both of them "mapping".
         */
        const via: RequestLogEntry["via"] = isLocalhostDomain ? "rfc6761" : "mapping";
        proxyToUpstream(socket, method, mapping.target, reqPath, headers, bodyBuf, (status, dur, resH, resB) => {
          emitLog({ ...baseEntry, status, via, target: mapping.target, durationMs: dur, resHeaders: resH, resBody: resB, resStatus: status });
        }, baseEntry.id);
      } else {
        sendHtml(socket, 404, buildNotMappedPage(host, currentPort));
        emitLog({ ...baseEntry, status: 404, via: "error", target: null, durationMs: Date.now() - t0, resHeaders: {}, resBody: "", resStatus: 404 });
      }
      return;
    }

    // ── 5. otherwise → the home page ────────────────────────────────────────
    //
    // **Not logged**, deliberately: this is the directory page, and the old `host === "localhost"`
    // branch it replaces was the one branch that did not log either. A request for a page the user
    // typed is not traffic through the proxy.
    sendHtml(socket, 200, buildHomePage(cfg, currentPort));
  }

  function start(port: number): void {
    if (server) stop();
    currentPort = port;
    lastError = null;

    server = http.createServer((req, res) => {
      /**
       * `res` is deliberately unused — see the header. It is named so that the intent is legible at
       * the signature: this server answers through `req.socket`, and every response it writes ends
       * that socket.
       *
       * The body is **fully buffered** before routing, and that is a change from the code this
       * replaces rather than a translation of it. The old hand-rolled parser took the body from the
       * same `data` chunk that carried the header terminator, so a body split across two reads was
       * silently truncated — a latent bug, not a behaviour worth preserving. The cost is that a large
       * upload is held in memory; the alternative is a streaming variant of `proxyToUpstream`, which
       * is a bigger change than this work item makes. Recorded in `plan/15`.
       */
      void res;
      const socket = req.socket;
      const method = req.method ?? "GET";
      const reqPath = req.url || "/";
      const hostHeader = (req.headers.host ?? "").toLowerCase();
      const host = hostnameOf(hostHeader);

      const headers: Record<string, string> = {};
      for (const [k, v] of Object.entries(req.headers)) {
        if (v === undefined) continue;
        const key = k.toLowerCase();
        if (HOP_BY_HOP.has(key)) continue;
        headers[key] = Array.isArray(v) ? v.join(", ") : v;
      }

      const chunks: Buffer[] = [];
      req.on("data", (c: Buffer) => chunks.push(c));
      req.on("error", () => socket.destroy());
      req.on("end", () => {
        /**
         * The routing call is wrapped, and the wrapper is not decoration: this runs inside an event
         * handler, so a `resolveConfig()` that throws would otherwise be an uncaught exception that
         * takes the whole process down — and the trigger would be a config file, which is the last
         * thing that should be able to kill a running engine.
         */
        try {
          route(socket, method, reqPath, hostHeader, host, headers, Buffer.concat(chunks));
        } catch (err) {
          const message = err instanceof Error ? err.message : String(err);
          console.error(`[edge:${currentPort}] dispatch failed for ${host}${reqPath} —`, message);
          logEmitter.emit("server-error", message);
          sendHtml(socket, 500, `<h1>500 Internal Server Error</h1><p>The edge could not resolve its routing config.</p><p><code>${message}</code></p>`);
        }
      });
    });

    /**
     * ## `Expect: 100-continue` is deliberately **not** handled here
     *
     * Registering a `checkContinue` listener would take the interim response away from Node, which
     * does the right thing on its own when no listener is registered. Measured, 2026-09-23, node
     * 22.22.2: a request with `Expect: 100-continue` and a 64-byte body, sent the way a real client
     * sends it — headers first, the body only after the interim reply — reached the `request` handler
     * with the **whole body**, and the client saw `HTTP/1.1 100 Continue` ahead of the final response.
     * The hand-rolled parser this replaces never looked at the header at all, so this is parity rather
     * than a gain; writing a listener here would be replacing correct platform behaviour with a
     * hand-maintained copy of it.
     */

    server.on("clientError", (err: NodeJS.ErrnoException, socket: net.Socket) => {
      // A malformed request is the client's problem, not the host's: answer and close, do not report
      // it as a server error. `err.code === "ECONNRESET"` is the common one and is not even worth a
      // log line — a browser closing a speculative connection looks identical to a broken one.
      if (err.code !== "ECONNRESET") {
        console.error(`[edge:${currentPort}] malformed request —`, err.message);
      }
      if (socket.writable) socket.write("HTTP/1.1 400 Bad Request\r\nconnection: close\r\n\r\n");
      socket.destroy();
    });

    server.on("error", (err: NodeJS.ErrnoException) => {
      if (err.code === "EACCES") {
        lastError = `Permission denied — run as Administrator to bind port ${port}`;
        console.error(`[edge:${port}]`, lastError);
      } else if (err.code === "EADDRINUSE") {
        lastError = `Port ${port} is already in use`;
        console.error(`[edge:${port}]`, lastError);
      } else {
        lastError = err.message;
        console.error(`[edge:${port}] error:`, err.message);
      }
      logEmitter.emit("server-error", lastError);
    });

    server.listen(port, bindHost, () => {
      lastError = null;
      console.log(`[edge:${port}] listening on ${bindHost}:${port}`);
    });
  }

  return {
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
  };
}

// ── The module-level binding ──────────────────────────────────────────────────
//
// `plan/17` §5 work item 7. The edge is the one runtime-movable port whose listener a command could not
// reach: `proxy/server.ts` and `proxy/webhookServer.ts` each keep a module-level **facade** over an instance
// someone else owns, which is how `config.save` re-binds those two. The edge had no such handle, so its port
// could be changed in Settings and nothing happened until the next boot. This is the same mechanism, one port
// over — and `plan/17` §3 records that reusing the existing binding was the deliberate choice over a new
// registry.

/**
 * The instance the module functions below act on, or `null` when nobody has adopted one.
 *
 * ## Bound-only, and why there is no lazy default here
 *
 * `proxy/server.ts` and `proxy/webhookServer.ts` both fall back to a lazily created default. This one
 * deliberately does not: an edge needs a `resolveConfig` callback (plus a `hostDomain`, and a `guiTarget` if
 * there is one), and there is no honest default for any of them. A lazily created edge would have to answer
 * **503 for every request** — which is a *worse* outcome than doing nothing, because it replaces a working
 * home page with a plausible-looking "this host serves no workspace", and the user would read that as a bug
 * rather than as an unbound handle.
 *
 * So `restartEdge()` is a **no-op** until a composition binds an instance, and `isEdgeRunning()` reports
 * `false`. That is the truthful answer to "is an edge bound?", which is the question `config.save` is really
 * asking when it decides whether a save should re-bind one.
 */
let boundInstance: EdgeRouterInstance | null = null;

/**
 * Adopt `instance` as the edge every module function here acts on, or `null` to release it.
 *
 * **A composition that binds must unbind in its `stop()`.** This is process-global module state, exactly like
 * `bindProxyInstance()` / `bindWebhookInstance()`, so a host that binds and does not unbind leaves the next
 * consumer in the same process re-binding *this* host's stopped edge — the same leak `runtime/host.ts`
 * documents for the other two. It is invisible to the CLI (one host per process) and paid for by the test
 * worker.
 */
export function bindEdgeInstance(instance: EdgeRouterInstance | null): void {
  boundInstance = instance;
}

/**
 * Re-bind the bound edge on `port` — `plan/17` §1's *"everything else re-binds at runtime"* for the edge.
 *
 * `start()` already does `if (server) stop()` first, so this is a rebind rather than a second listener: the
 * same call serves both "move it" and "bring it back after a failed bind". A **bind failure is not thrown** —
 * it lands in `getError()`, the engine's oldest bind contract, and the host reports it.
 *
 * A no-op when nothing is bound; see `boundInstance`'s note for why that is the right answer rather than a
 * lazily created default.
 */
export function restartEdge(port: number): void {
  boundInstance?.start(port);
}

/** Whether the bound edge is listening. `false` when no edge is bound — see `boundInstance`. */
export function isEdgeRunning(): boolean {
  return boundInstance?.isRunning() ?? false;
}

/** The bound edge's port, or `0` when none is bound. Mirrors `proxy/server.ts`'s `getPort()`. */
export function getEdgePort(): number {
  return boundInstance?.getPort() ?? 0;
}
