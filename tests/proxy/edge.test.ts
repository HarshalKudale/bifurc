/**
 * The **edge router** — `plan/15` §3.3, work item 3. Unit, no real sockets.
 *
 * ## Why this file exists, and why it was missing
 *
 * `plan/15`'s WI 3 block recorded this file as written (*"new, 40 cases"*) and it did not exist. The
 * reverse-half cases below are **moved here from `tests/proxy/server.test.ts`**, which is where they lived
 * while `proxy/server.ts` still owned the reverse half. WI 3 moved that half — and the home page with it —
 * onto `edge.ts`, so those cases were asking the wrong module for the answer and failing with a deliberate
 * **400**. This is that move, done.
 *
 * ## What is asserted here, and what is not
 *
 * These cases drive the edge the way the old ones drove the proxy: a **mock-based** unit harness, with
 * `vi.mock("net")` / `vi.mock("http")` and a synthetic socket, so each case can assert on the exact bytes
 * written and on which helper was called. That is deliberate — `tests/integration/edgeSplit.integration.test.ts`
 * is where the same behaviours are proven over **real** sockets, and the two files answer different
 * questions. A case belongs here when its subject is *which branch of `route()` ran*; it belongs there when
 * its subject is *what came back over a socket*.
 *
 * ## The harness shape differs from `server.test.ts`'s, and that is not cosmetic
 *
 * `proxy/server.ts` hand-rolled its own HTTP parsing: it took a raw `net.Socket` and the test emitted a
 * `data` event carrying the request line. `edge.ts` is a real `http.Server` — a measured decision recorded
 * in its header — so the request arrives as an `http.IncomingMessage` on the server's `request` event.
 * `dispatch()` below fabricates that shape: it captures the `request` handler the edge registers, then
 * calls it with an `IncomingMessage`-like object whose `data`/`end` it drives itself.
 *
 * **The consequence worth naming:** the edge buffers the whole body before routing (`req.on("data")` /
 * `req.on("end")`), so a case asserting on dispatch must emit `end`. A case that emitted only `data` would
 * assert nothing and pass vacuously.
 */
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { EventEmitter } from "events";

// Mocked modules — all at top level so they are properly hoisted.
vi.mock("net");
vi.mock("http");

// Track workspace entities so readAllEntities and bootstrapEnabledSet can serve them.
let _testProxyRules: any[] = [];
let _testMappings: any[] = [];
let _testMocks: any[] = [];

vi.mock("@bifurc/engine/store/workspaceFs", () => ({
  readEnabledSet: vi.fn(() => null),
  bootstrapEnabledSet: vi.fn((_wsId: string, kind: string) => {
    if (kind === "rules") return new Set<string>(_testProxyRules.filter((r) => r.enabled).map((r) => r.id));
    if (kind === "mappings") return new Set<string>(_testMappings.filter((m) => m.enabled).map((m) => m.id));
    if (kind === "mocks") return new Set<string>(_testMocks.filter((m) => m.enabled).map((m) => m.id));
    return new Set<string>();
  }),
  readAllEntities: vi.fn((_wsId: string, kind: string) => {
    if (kind === "rules") return _testProxyRules;
    if (kind === "mocks") return _testMocks;
    return [];
  }),
  dataRoot: vi.fn(() => "/tmp/test-data"),
  wsDir: vi.fn((id: string) => `/tmp/test-data/${id}`),
}));

vi.mock("@bifurc/engine/store/config", () => {
  const loadConfig = vi.fn(() => ({ port: 8080 }));
  return {
    loadConfig,
    /**
     * Derived from `loadConfig`'s most recent return value rather than by calling it again — the same
     * instrument `server.test.ts` uses, for the same two reasons: the slice must agree with `loadConfig()`
     * (the enabled sets filter it), and it must not *call* it (cases assert call counts on it).
     */
    loadWorkspaceDispatchSlice: vi.fn((wsId: string) => {
      const results = loadConfig.mock.results;
      const cfg = (results.length ? results[results.length - 1].value : {}) as any;
      return {
        workspaceId: wsId,
        mocks: cfg.mocks ?? [],
        mappings: cfg.mappings ?? [],
        environments: cfg.environments ?? [],
        activeEnvironmentId: cfg.activeEnvironmentId ?? null,
      };
    }),
    saveConfig: vi.fn(),
    generateId: vi.fn(() => "test-id"),
  };
});

vi.mock("@bifurc/engine/proxy/serverUtils", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@bifurc/engine/proxy/serverUtils")>();
  return { ...actual, loadEnabledSets: vi.fn(actual.loadEnabledSets) };
});

import * as net from "net";
import * as http from "http";
import { loadConfig, type AppConfig } from "@bifurc/engine/store/config";
import { createEdgeRouter, DEFAULT_EDGE_BIND_HOST } from "@bifurc/engine/proxy/edge";
import { logEmitter } from "@bifurc/engine/kernel/telemetry";

// ── Helpers ─────────────────────────────────────────────────────────────────

/**
 * A synthetic `net.Socket` good enough for `sendHtml` / `proxyToUpstream` to write to.
 *
 * `writable` is set because `sendHtml` guards on it, and `end` records the socket being closed — which is
 * how a case can tell "a response was written" from "the socket was finished".
 */
function makeSocket() {
  const s = new EventEmitter() as any;
  s.writable = true;
  s.write = vi.fn();
  s.end = vi.fn();
  s.destroy = vi.fn();
  s.pipe = vi.fn();
  return s;
}

/** The union of every byte the handler wrote, in order. `write()` is called twice per `sendHtml`. */
function written(socket: any): string {
  return socket.write.mock.calls.map((c: any[]) => c[0].toString()).join("");
}

function makeMockHttpReq() {
  const mockReq = new EventEmitter() as any;
  mockReq.write = vi.fn();
  mockReq.end = vi.fn();
  return mockReq;
}

/**
 * A `http.request` mock that also **fires the response callback**, which is the only way a case can reach
 * `handleProxyResponse`.
 *
 * `mockReturnValue(makeMockHttpReq())` is enough when the case only asserts that `http.request` was
 * *called* — but a case asserting on what the client received needs the callback to run, because
 * `proxyToUpstream` passes it as `http.request`'s second argument. Returning a bare EventEmitter silently
 * skips that callback and the socket stays empty.
 *
 * Returns both halves so a case can emit `response`/`data`/`end` on the upstream response.
 */
function wireUpstream(body: Buffer = Buffer.from("upstream-body"), headers: Record<string, string> = { "content-type": "text/plain" }) {
  const mockReq = makeMockHttpReq();
  const mockRes = makeMockHttpResponse(200, headers, [body]);
  vi.mocked(http.request as any).mockImplementation((_opts: any, cb: any) => {
    // Node calls the callback asynchronously, once the response headers arrive.
    if (typeof cb === "function") queueMicrotask(() => cb(mockRes.res));
    return mockReq;
  });
  return { mockReq, mockRes };
}

function makeMockHttpResponse(
  statusCode: number,
  headers: Record<string, string>,
  bodyChunks: Buffer[],
) {
  const res = new EventEmitter() as any;
  res.statusCode = statusCode;
  res.headers = headers;
  /**
   * `handleProxyResponse` ends with `res.pipe(socket)`, so a mock response without `pipe` throws before any
   * assertion can run.
   *
   * **It must actually forward, not be a no-op.** A `vi.fn()` here makes the head land and the body vanish
   * — `handleProxyResponse` writes the head itself but delegates the body to `pipe` — so a case asserting
   * on the body would fail for a reason that has nothing to do with the edge. Node's `Readable.pipe` is what
   * production uses, so a mock that re-emits each `data` chunk to the destination is the faithful stand-in.
   */
  res.pipe = (dest: any) => {
    res.on("data", (c: Buffer) => dest.write(c));
    return dest;
  };
  res.destroy = vi.fn();
  return {
    res,
    emitData: () => bodyChunks.forEach((c) => res.emit("data", c)),
    emitEnd: () => res.emit("end"),
  };
}

function makeMockServer() {
  const srv = new EventEmitter() as any;
  srv.listening = false;
  srv.close = vi.fn(() => { srv.listening = false; });
  srv.listen = vi.fn((_port: number, _host: string, cb?: () => void) => {
    srv.listening = true;
    cb?.();
    return srv;
  });
  return srv;
}

// ── Tests ────────────────────────────────────────────────────────────────────

describe("@bifurc/engine/proxy/edge", () => {
  /**
   * A deliberately **partial** `AppConfig`, and the cast is how that is said rather than hidden.
   *
   * The edge reads four fields — `port`, `mappings`, `mocks`, `proxyRules` — and this suite is mock-based, so
   * a complete 37-field literal would be noise no case depends on. **But the cast has a real cost, and it is
   * worth naming:** a field the edge *started* reading would arrive as `undefined` here rather than as a
   * compile error, so this file cannot catch a widened config dependency.
   *
   * That is exactly what `tests/integration/edgeSplit.integration.test.ts` is for — it drives the real router
   * over real sockets against a config `loadConfig()` actually produced from files. **The two files are not
   * redundant: this one owns the routing decisions, that one owns the config plumbing.**
   *
   * (Note: the tests tree is not in any `typecheck` script — the root `tsconfig.json` includes `src` only —
   * so this cast is only load-bearing for a reader running `tsc` over `tests/` by hand. It was found that way.)
   */
  const baseConfig = {
    port: 8080, minimizeToTray: true,
    workspaces: [{ id: "default", name: "Workspace 1", activeEnvironmentId: null }],
    activeWorkspaceId: "default",
    mappings: [] as any[], proxyRules: [] as any[],
    mocks: [] as any[], requests: [] as any[],
    mockFolders: [] as any[], requestFolders: [] as any[],
    environments: [] as any[], activeEnvironmentId: null as string | null,
  } as unknown as AppConfig;

  let capturedRequestCb: ((req: any, res: any) => void) | null = null;
  let edge: ReturnType<typeof createEdgeRouter>;
  /** The config the edge resolves per request. A `let` so a case can swap it mid-life. */
  let resolved: any = baseConfig;

  /**
   * Boot the edge with the given config. `http.createServer` is mocked, so nothing actually binds — the
   * `request` handler is captured from the mocked factory instead.
   *
   * **The second parameter is gone.** It carried the edge's `hostDomain` — the one option that was a property
   * of the *deployment* rather than of a request — until 2026-09-26, when the edge stopped taking its own name
   * (see `EdgeRouterOptions`). `resolveConfig` is now the only thing `createEdgeRouter` is given.
   */
  function setupEdge(cfg: any = baseConfig) {
    const wsId = cfg.activeWorkspaceId ?? "default";
    const normalizedCfg = {
      ...cfg,
      mappings: (cfg.mappings ?? []).map((m: any) => ({ workspaceId: wsId, ...m })),
      proxyRules: (cfg.proxyRules ?? []).map((r: any) => ({ workspaceId: wsId, targetType: "mapping", ...r })),
      mocks: (cfg.mocks ?? []).map((m: any) => ({ workspaceId: wsId, ...m })),
    };
    _testProxyRules = normalizedCfg.proxyRules;
    _testMappings = normalizedCfg.mappings;
    _testMocks = normalizedCfg.mocks;
    vi.mocked(loadConfig).mockReturnValue(normalizedCfg);
    resolved = normalizedCfg;

    capturedRequestCb = null;
    const srv = makeMockServer();
    vi.mocked(http.createServer).mockImplementation((cb: any) => {
      capturedRequestCb = cb;
      return srv;
    });

    edge = createEdgeRouter({ resolveConfig: () => resolved });
    edge.start(cfg.port ?? 8080);
  }

  /**
   * Drive one request through the captured handler.
   *
   * **The `end` event is what triggers routing** — the edge buffers the body first — so this must emit it
   * or nothing happens and the case asserts on an empty socket.
   */
  function dispatch(
    method: string,
    target: string,
    host: string,
    opts: { body?: Buffer; headers?: Record<string, string> } = {},
  ) {
    const socket = makeSocket();
    const req = new EventEmitter() as any;
    req.socket = socket;
    req.method = method;
    req.url = target;
    req.headers = { host, ...(opts.headers ?? {}) };

    capturedRequestCb!(req, {});

    if (opts.body) req.emit("data", opts.body);
    req.emit("end");
    return socket;
  }

  beforeEach(() => {
    vi.clearAllMocks();
    resolved = baseConfig;
    vi.mocked(loadConfig).mockReturnValue(baseConfig as any);
    vi.mocked(http.request as any).mockReset();
  });

  afterEach(() => {
    edge?.stop();
  });

  // ── createEdgeRouter() — the bind guard ────────────────────────────────────

  describe("createEdgeRouter() — the bind guard", () => {
    it("binds loopback by default", () => {
      const srv = makeMockServer();
      vi.mocked(http.createServer).mockReturnValue(srv as any);
      const e = createEdgeRouter({ resolveConfig: () => baseConfig });
      e.start(8080);
      expect(srv.listen).toHaveBeenCalledWith(8080, DEFAULT_EDGE_BIND_HOST, expect.any(Function));
    });

    it("REFUSES a non-loopback bind, and there is no opt-in that changes that", () => {
      // **This case replaces the pair that asserted the opposite.** `allowNonLoopback: true` used to make
      // `0.0.0.0` bind, and a second case pinned that only a literal `true` was accepted. That field was
      // deleted on 2026-09-25 with the other bind opt-ins — the engine is not deployable remotely, so the
      // edge has no address to publish and no flag to acknowledge one. `bindHost` stays a parameter because
      // a test may want to *prove* the refusal; it is not a way to configure a deployment.
      //
      // Four spellings rather than one, because the guard is `isLoopbackHost()` and the failure worth
      // guarding is a *particular* non-loopback address slipping through a list written here.
      for (const host of ["0.0.0.0", "::", "192.168.1.10", "10.1.2.3"]) {
        expect(
          () => createEdgeRouter({ resolveConfig: () => baseConfig, bindHost: host }),
          `${host} must not bind — the edge is loopback-only`,
        ).toThrow(/Refusing to bind the edge router to/);
      }
    });
  });

  // ── instance surface ──────────────────────────────────────────────────────

  describe("the instance surface", () => {
    it("reports isRunning() from the underlying server", () => {
      setupEdge();
      expect(edge.isRunning()).toBe(true);
    });

    it("reports getPort() as the port it was started on", () => {
      setupEdge();
      expect(edge.getPort()).toBe(8080);
    });

    it("retains the port after stop(), so a status read does not go to zero", () => {
      setupEdge();
      edge.stop();
      expect(edge.getPort()).toBe(8080);
    });

    it("reports no error after a clean start", () => {
      setupEdge();
      expect(edge.getError()).toBeNull();
    });

    it("reports a busy port through getError() rather than throwing", () => {
      setupEdge();
      const srv = vi.mocked(http.createServer).mock.results[0].value as any;
      srv.emit("error", Object.assign(new Error("busy"), { code: "EADDRINUSE" }));
      expect(edge.getError()).toMatch(/already in use/);
    });
  });

  // ── the home page ─────────────────────────────────────────────────────────

  describe("the home page (row 5)", () => {
    it("serves 200 for a request to localhost", () => {
      setupEdge();
      const socket = dispatch("GET", "http://localhost/", "localhost");
      expect(written(socket)).toContain("200");
    });

    it("includes the active mappings in the body", () => {
      setupEdge({ ...baseConfig, mappings: [{ id: "m1", domain: "app.localhost", target: "localhost:3000", enabled: true }] });
      const socket = dispatch("GET", "/", "localhost");
      expect(written(socket)).toContain("app.localhost");
    });

    it("includes the port suffix when the port is not 80", () => {
      setupEdge();
      const socket = dispatch("GET", "/", "localhost");
      expect(written(socket)).toContain("8080");
    });

    it("omits the port suffix when the port is 80", () => {
      setupEdge({ ...baseConfig, port: 80 });
      const socket = dispatch("GET", "/", "localhost");
      expect(written(socket)).not.toMatch(/:80(?!\d)/);
    });

    it("sends the unserved 503 when no workspace is resolved", () => {
      setupEdge();
      resolved = null;
      const socket = dispatch("GET", "/", "localhost");
      expect(written(socket)).toContain("503");
    });

    it("does not log a request for the home page", () => {
      setupEdge();
      const emitted: any[] = [];
      logEmitter.on("request", (e: any) => emitted.push(e));
      dispatch("GET", "/", "localhost");
      expect(emitted).toHaveLength(0);
      logEmitter.removeAllListeners("request");
    });
  });

  // ── *.localhost — unmapped ────────────────────────────────────────────────

  describe("an unmapped *.localhost domain", () => {
    it("serves 404", () => {
      setupEdge();
      const socket = dispatch("GET", "/path", "myapp.localhost");
      expect(written(socket)).toContain("404");
    });

    it("includes 'Not Mapped' text in the body", () => {
      setupEdge();
      const socket = dispatch("GET", "/", "myapp.localhost");
      expect(written(socket)).toContain("Not Mapped");
    });

    it("emits a request log entry with status 404 and via=error", () => {
      setupEdge();
      const emitted: any[] = [];
      logEmitter.on("request", (e: any) => emitted.push(e));
      dispatch("GET", "/path", "myapp.localhost");
      expect(emitted[0]?.status).toBe(404);
      expect(emitted[0]?.via).toBe("error");
      logEmitter.removeAllListeners("request");
    });

    it("names the requested host in the 404 page", () => {
      setupEdge();
      const socket = dispatch("GET", "/", "myapp.localhost");
      expect(written(socket)).toContain("myapp.localhost");
    });
  });

  // ── *.localhost — mapped → the reverse proxy ──────────────────────────────

  describe("a mapped domain", () => {
    it("proxies to the mapping's target", () => {
      vi.mocked(http.request as any).mockReturnValue(makeMockHttpReq());
      setupEdge({ ...baseConfig, mappings: [{ id: "m1", domain: "app.localhost", target: "localhost:3000", enabled: true }] });
      dispatch("GET", "/api/users", "app.localhost");
      expect(http.request).toHaveBeenCalled();
    });

    it("labels a *.localhost mapping's log entry via=rfc6761", async () => {
      const { mockRes } = wireUpstream();
      setupEdge({ ...baseConfig, mappings: [{ id: "m1", domain: "app.localhost", target: "localhost:3000", enabled: true }] });

      const emitted: any[] = [];
      logEmitter.on("request", (e: any) => emitted.push(e));
      dispatch("GET", "/api/users", "app.localhost");
      await new Promise((r) => setTimeout(r, 0));

      mockRes.emitData();
      mockRes.emitEnd();
      // `handleProxyResponse` decompresses asynchronously, so the log lands a tick after `end`.
      await new Promise((r) => setTimeout(r, 10));

      expect(emitted[0]?.via).toBe("rfc6761");
      logEmitter.removeAllListeners("request");
    });

    it("labels a non-.localhost mapping's log entry via=mapping (WI 4)", async () => {
      const { mockRes } = wireUpstream();
      setupEdge({ ...baseConfig, mappings: [{ id: "m1", domain: "api.example.com", target: "localhost:3000", enabled: true }] });

      const emitted: any[] = [];
      logEmitter.on("request", (e: any) => emitted.push(e));
      dispatch("GET", "/api/users", "api.example.com");
      await new Promise((r) => setTimeout(r, 0));

      mockRes.emitData();
      mockRes.emitEnd();
      await new Promise((r) => setTimeout(r, 10));

      expect(emitted[0]?.via).toBe("mapping");
      logEmitter.removeAllListeners("request");
    });

    it("serves 502 when the mapped target cannot be reached", () => {
      const mockReq = makeMockHttpReq();
      vi.mocked(http.request as any).mockReturnValue(mockReq);
      setupEdge({ ...baseConfig, mappings: [{ id: "m1", domain: "app.localhost", target: "localhost:3000", enabled: true }] });
      const socket = dispatch("GET", "/api/users", "app.localhost");

      mockReq.emit("error", new Error("ECONNREFUSED"));
      expect(written(socket)).toContain("502");
    });

    it("writes the proxied response head and body back to the client", async () => {
      const { mockRes } = wireUpstream();
      setupEdge({ ...baseConfig, mappings: [{ id: "m1", domain: "app.localhost", target: "localhost:3000", enabled: true }] });
      const socket = dispatch("GET", "/api/users", "app.localhost");
      await new Promise((r) => setTimeout(r, 0));

      mockRes.emitData();
      mockRes.emitEnd();
      await new Promise((r) => setTimeout(r, 10));

      const all = written(socket);
      expect(all).toContain("200");
      expect(all).toContain("upstream-body");
    });

    it("sends an unmapped non-.localhost domain to the home page, not the 404 (WI 4 row 5)", () => {
      setupEdge({ ...baseConfig, mappings: [{ id: "m1", domain: "other.localhost", target: "localhost:3000", enabled: true }] });
      const socket = dispatch("GET", "/", "unmapped.example.com");
      const all = written(socket);
      expect(all).toContain("200");
      expect(all).not.toContain("Not Mapped");
    });
  });

  // ── what the edge deliberately does NOT do ────────────────────────────────

  describe("the edge's own boundary", () => {
    /**
     * `CONNECT` stayed on the forward proxy (`server.ts:289`), because tunnelling is what a *browser* asks
     * a proxy it is configured to use — it is not a reverse-half concern, and the edge has no CONNECT branch.
     *
     * **The assertion is on the branch, not on `net.connect`.** `net.connect` is not called in *any* edge
     * path, so asserting that would pass for every reason including the wrong one — including when routing
     * throws before it gets there. What is pinned instead is that the mapped branch was taken: the request
     * reached `http.request`, which is the only thing a CONNECT to a mapped host could have reached if the
     * tunnel logic had been moved here.
     */
    it("has no CONNECT branch — a CONNECT to a mapped host takes the ordinary reverse path", () => {
      vi.mocked(http.request as any).mockReturnValue(makeMockHttpReq());
      setupEdge({ ...baseConfig, mappings: [{ id: "m1", domain: "app.localhost", target: "localhost:3000", enabled: true }] });
      dispatch("CONNECT", "app.localhost:443", "app.localhost:443");
      expect(http.request).toHaveBeenCalled();
      expect(net.connect).not.toHaveBeenCalled();
    });

    it("routes a non-.localhost Host that matches nothing to the home page, never a 400", () => {
      // The inverse of the forward half's deliberate 400: the edge is *supposed* to answer any host.
      setupEdge();
      const socket = dispatch("GET", "/", "whoever.example.com");
      const all = written(socket);
      expect(all).toContain("200");
      expect(all).not.toContain("400");
    });

    /**
     * The `try/catch` around `route()` is not decoration — it exists so a config file cannot kill a running
     * engine from inside an event handler. This case is what makes that claim falsifiable: a `resolveConfig`
     * that throws must produce a 500 **and** a `server-error` emission, not an unhandled rejection.
     */
    it("answers 500 and reports server-error when resolveConfig throws, rather than crashing", async () => {
      setupEdge();
      // The instance created by `setupEdge()` holds `() => resolved`, which cannot throw; the property under
      // test is a *throwing resolver*, so a second instance is built whose option throws directly.
      edge = createEdgeRouter({ resolveConfig: () => { throw new Error("config unreadable"); } });
      edge.start(8080);

      const errors: string[] = [];
      logEmitter.on("server-error", (m: string) => errors.push(m));

      const socket = dispatch("GET", "/", "localhost");

      expect(written(socket)).toContain("500");
      expect(errors.join(" ")).toContain("config unreadable");
      logEmitter.removeAllListeners("server-error");
    });
  });

});
