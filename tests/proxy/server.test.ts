import { describe, it, expect, vi, beforeAll, beforeEach, afterEach } from "vitest";
import { EventEmitter } from "events";
import * as fs from "fs";
import * as os from "os";
import * as path from "path";

// Mocked modules — all at top level so they are properly hoisted
vi.mock("fs");
vi.mock("net");
vi.mock("http");
vi.mock("https");

// Track workspace entities so readAllEntities and bootstrapEnabledSet can serve them
let _testProxyRules: any[] = [];
let _testMappings: any[] = [];
let _testMocks: any[] = [];

vi.mock("@bifurc/engine/workspace/workspaceFs", () => ({
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

// Mock store/config at top level so it is effective for the imported server module
vi.mock("@bifurc/engine/workspace/config", () => {
  const loadConfig = vi.fn(() => ({
    port: 8080,
    minimizeToTray: true,
    workspaces: [{ id: "default", name: "Workspace 1", activeEnvironmentId: null }],
    activeWorkspaceId: "default",
    mappings: [],
    proxyRules: [],
    mocks: [],
    requests: [],
    mockFolders: [],
    requestFolders: [],
    environments: [],
    activeEnvironmentId: null,
  }));

  return {
    loadConfig,
    /**
     * Derived from `loadConfig`'s **most recent** return value, rather than by calling it again.
     *
     * Both halves of that are load-bearing, and both are the kind of thing this file has already been bitten
     * by once (see the `loadEnabledSets` wrapper's comment below):
     *
     * - **The slice must agree with `loadConfig()`.** `workspaceCfg()` filters the slice's entities by the
     *   enabled sets, so a slice that disagreed would silently change dispatch for every case here — which is
     *   exactly what the first version of that instrument did, at a cost of 22 failures.
     * - **It must not *call* `loadConfig()`.** Several cases assert `toHaveBeenCalledOnce()` on it, and the
     *   proxy now calls both in sequence, so an extra call from the instrument would fail assertions about a
     *   different mechanism. `mock.results` is how a test double reports what it was last asked to return.
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

// Mocked so the WI 4 cases can observe *which* workspace the proxy asks for. It is the only place the
// severed `activeWorkspaceId` read is observable, and it is not part of the instance's public surface.
//
// **The wrapper must delegate, not stub.** The first version returned empty enabled sets for every call,
// which silently changed dispatch for all 74 pre-existing cases in this file (22 failed) — the enabled set
// decides which mocks and mappings are live. A pass-through spy records the workspace id and then calls the
// real implementation, so the instrument is inert.
vi.mock("@bifurc/engine/proxy/serverUtils", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@bifurc/engine/proxy/serverUtils")>();
  return { ...actual, loadEnabledSets: vi.fn(actual.loadEnabledSets) };
});

import * as net from "net";
import * as http from "http";
import * as https from "https";
import { loadConfig, type AppConfig } from "@bifurc/engine/workspace/config";
import { isCALoaded, unloadCA } from "@bifurc/engine/proxy/tlsCert";
// Imported so the WI 4 cases can override them: which workspace each read asks for is the whole assertion,
// and it is not observable from the instance's public surface.
import { readAllEntities } from "@bifurc/engine/workspace/workspaceFs";
import { loadEnabledSets } from "@bifurc/engine/proxy/serverUtils";
import {
  createProxyServer,
  startServer,
  stopServer,
  isRunning,
  getPort,
  getServerError,
  reloadConfig,
  replayRequest,
  logEmitter,
} from "@bifurc/engine/proxy/server";

// ── Helpers ─────────────────────────────────────────────────────────────────

/** Poll a predicate until it holds, or fail after ~1s. `start()` binds asynchronously. */
async function waitFor(predicate: () => boolean, timeoutMs = 1000): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (!predicate()) {
    if (Date.now() > deadline) throw new Error("waitFor: predicate never became true");
    await new Promise((r) => setTimeout(r, 5));
  }
}

/**
 * A config shaped for the CA-ownership cases. Deliberately independent of the `baseConfig` declared
 * inside the dispatch `describe` — that one is scoped there, and the TLS cases need a config whose
 * `tlsEnabled` / `tlsCaCertPath` / `tlsCaKeyPath` they control per case.
 */
function tlsConfig() {
  return {
    port: 8080,
    minimizeToTray: true,
    workspaces: [{ id: "default", name: "Workspace 1", activeEnvironmentId: null }],
    activeWorkspaceId: "default",
    mappings: [],
    proxyRules: [],
    mocks: [],
    requests: [],
    mockFolders: [],
    requestFolders: [],
    environments: [],
    activeEnvironmentId: null,
    tlsEnabled: false,
    tlsCaCertPath: null as string | null,
    tlsCaKeyPath: null as string | null,
  };
}

function makeMockServer() {
  const srv = new EventEmitter() as any;
  srv.listening = false;
  srv.close = vi.fn((_cb?: () => void) => {
    srv.listening = false;
  });
  srv.listen = vi.fn((_port: number, _host: string, cb?: () => void) => {
    srv.listening = true;
    cb?.();
    return srv;
  });
  return srv;
}

function makeMockHttpResponse(
  statusCode: number,
  headers: Record<string, string>,
  bodyChunks: Buffer[],
) {
  const res = new EventEmitter() as any;
  res.statusCode = statusCode;
  res.headers = headers;
  return {
    res,
    emitData: () => bodyChunks.forEach((c) => res.emit("data", c)),
    emitEnd: () => res.emit("end"),
  };
}

// ── Tests ────────────────────────────────────────────────────────────────────

describe("@bifurc/engine/proxy/server", () => {
  let mockSrv: ReturnType<typeof makeMockServer>;

  beforeEach(() => {
    vi.clearAllMocks();

    // Restore loadConfig mock after clearAllMocks resets call history
    //
    // **Deliberately partial, and the cast says so** — the forward half reads `port` and the entity lists,
    // and a complete 37-field `AppConfig` literal would be noise. Same trade as `edge.test.ts`'s fixture, with
    // the same cost (a *newly read* field would arrive as `undefined` rather than as a compile error). The
    // tests tree is in no `typecheck` script, so this only surfaces when `tsc` is run over `tests/` by hand.
    vi.mocked(loadConfig).mockReturnValue({
      port: 8080, minimizeToTray: true,
      workspaces: [{ id: "default", name: "Workspace 1", activeEnvironmentId: null }],
      activeWorkspaceId: "default",
      mappings: [], proxyRules: [],
      mocks: [], requests: [], mockFolders: [], requestFolders: [],
      environments: [], activeEnvironmentId: null,
    } as unknown as AppConfig);

    mockSrv = makeMockServer();
    vi.mocked(net.createServer).mockReturnValue(mockSrv);
  });

  afterEach(() => {
    stopServer();
  });

  // ── isRunning ─────────────────────────────────────────────────────────

  describe("isRunning()", () => {
    it("returns true after startServer() creates a listening server", () => {
      startServer(8080);
      expect(isRunning()).toBe(true);
    });

    it("returns false after stopServer() closes the server", () => {
      startServer(8080);
      stopServer();
      expect(isRunning()).toBe(false);
    });

    it("reflects server.listening state directly", () => {
      startServer(8080);
      expect(isRunning()).toBe(true);
      mockSrv.listening = false;
      expect(isRunning()).toBe(false);
    });
  });

  // ── getPort ───────────────────────────────────────────────────────────

  describe("getPort()", () => {
    it("returns the port passed to startServer()", () => {
      startServer(9999);
      expect(getPort()).toBe(9999);
    });

    it("retains the last port after stopServer()", () => {
      startServer(7777);
      stopServer();
      expect(getPort()).toBe(7777);
    });

    it("returns the updated port when restarted on a new port", () => {
      const firstSrv = makeMockServer();
      const secondSrv = makeMockServer();
      vi.mocked(net.createServer)
        .mockReturnValueOnce(firstSrv)
        .mockReturnValueOnce(secondSrv);

      startServer(8080);
      startServer(9090);

      expect(getPort()).toBe(9090);
    });
  });

  // ── getServerError ────────────────────────────────────────────────────

  describe("getServerError()", () => {
    it("returns null when no error has occurred", () => {
      startServer(8080);
      expect(getServerError()).toBeNull();
    });

    it("returns the EADDRINUSE error message when the port is in use", () => {
      startServer(80);
      const err: NodeJS.ErrnoException = new Error("address already in use");
      err.code = "EADDRINUSE";
      mockSrv.emit("error", err);
      expect(getServerError()).toContain("already in use");
    });

    it("returns the EACCES error message when permission is denied", () => {
      startServer(80);
      const err: NodeJS.ErrnoException = new Error("permission denied");
      err.code = "EACCES";
      mockSrv.emit("error", err);
      expect(getServerError()).toContain("Permission denied");
    });

    it("returns the raw error message for unknown errors", () => {
      startServer(8080);
      const err: NodeJS.ErrnoException = new Error("something weird");
      mockSrv.emit("error", err);
      expect(getServerError()).toBe("something weird");
    });

    it("resets to null when a new server starts successfully", () => {
      startServer(8080);
      const err: NodeJS.ErrnoException = new Error("fail");
      err.code = "EADDRINUSE";
      mockSrv.emit("error", err);
      expect(getServerError()).not.toBeNull();

      const secondSrv = makeMockServer();
      vi.mocked(net.createServer).mockReturnValueOnce(secondSrv);
      startServer(9090); // resets lastError before listen

      expect(getServerError()).toBeNull();
    });
  });

  // ── startServer ───────────────────────────────────────────────────────

  describe("startServer()", () => {
    it("calls net.createServer to create a TCP server", () => {
      startServer(8080);
      expect(net.createServer).toHaveBeenCalledOnce();
    });

    it("calls server.listen with the given port bound to 127.0.0.1", () => {
      startServer(8080);
      expect(mockSrv.listen).toHaveBeenCalledWith(8080, "127.0.0.1", expect.any(Function));
    });

    it("stops and restarts when called while a server is already running", () => {
      const firstSrv = makeMockServer();
      const secondSrv = makeMockServer();
      vi.mocked(net.createServer)
        .mockReturnValueOnce(firstSrv)
        .mockReturnValueOnce(secondSrv);

      startServer(8080);
      startServer(9090);

      expect(firstSrv.close).toHaveBeenCalledOnce();
      expect(secondSrv.listen).toHaveBeenCalledWith(9090, "127.0.0.1", expect.any(Function));
    });

    it("calls loadConfig() to populate currentConfig", () => {
      startServer(8080);
      expect(loadConfig).toHaveBeenCalled();
    });

    it("emits 'server-error' on logEmitter when the server errors", () => {
      startServer(8080);
      const errorListener = vi.fn();
      logEmitter.on("server-error", errorListener);

      const err: NodeJS.ErrnoException = new Error("addr in use");
      err.code = "EADDRINUSE";
      mockSrv.emit("error", err);

      expect(errorListener).toHaveBeenCalledOnce();
      logEmitter.removeListener("server-error", errorListener);
    });
  });

  // ── stopServer ────────────────────────────────────────────────────────

  describe("stopServer()", () => {
    it("does nothing when no server is running", () => {
      // afterEach already called stopServer, so no server is running now
      expect(() => stopServer()).not.toThrow();
    });

    it("calls server.close() when a server is running", () => {
      startServer(8080);
      stopServer();
      expect(mockSrv.close).toHaveBeenCalledOnce();
    });
  });

  // ── reloadConfig ──────────────────────────────────────────────────────

  describe("reloadConfig()", () => {
    it("calls loadConfig() to refresh the in-memory config", () => {
      vi.mocked(loadConfig).mockClear();
      reloadConfig();
      expect(loadConfig).toHaveBeenCalledOnce();
    });
  });

  // ── replayRequest ─────────────────────────────────────────────────────

  describe("replayRequest()", () => {
    it("rejects when the URL is invalid", async () => {
      await expect(replayRequest("GET", "not-a-valid-url", {}, "")).rejects.toThrow("Invalid URL");
    });

    it("uses http.request for http:// URLs", async () => {
      const { res, emitData, emitEnd } = makeMockHttpResponse(
        200, { "content-type": "application/json" }, [Buffer.from('{"ok":true}')],
      );
      const req = new EventEmitter() as any;
      req.write = vi.fn();
      req.end = vi.fn(() => { emitData(); emitEnd(); });
      vi.mocked(http.request as any).mockImplementation((_opts: any, cb: any) => {
        cb(res);
        return req;
      });

      const result = await replayRequest("GET", "http://example.com/test", {}, "");

      expect(http.request).toHaveBeenCalled();
      expect(result.status).toBe(200);
    });

    it("uses https.request for https:// URLs", async () => {
      const { res, emitData, emitEnd } = makeMockHttpResponse(201, {}, [Buffer.from("created")]);
      const req = new EventEmitter() as any;
      req.write = vi.fn();
      req.end = vi.fn(() => { emitData(); emitEnd(); });
      vi.mocked(https.request as any).mockImplementation((_opts: any, cb: any) => {
        cb(res);
        return req;
      });

      const result = await replayRequest("POST", "https://example.com/api", {}, "");

      expect(https.request).toHaveBeenCalled();
      expect(result.status).toBe(201);
    });

    it("returns the response body as base64", async () => {
      const body = Buffer.from("hello world");
      const { res, emitData, emitEnd } = makeMockHttpResponse(200, {}, [body]);
      const req = new EventEmitter() as any;
      req.write = vi.fn();
      req.end = vi.fn(() => { emitData(); emitEnd(); });
      vi.mocked(http.request as any).mockImplementation((_opts: any, cb: any) => {
        cb(res);
        return req;
      });

      const result = await replayRequest("GET", "http://example.com/", {}, "");

      expect(result.body).toBe(body.toString("base64"));
    });

    it("sends the request body when bodyBase64 is non-empty", async () => {
      const { res, emitData, emitEnd } = makeMockHttpResponse(200, {}, []);
      const req = new EventEmitter() as any;
      req.write = vi.fn();
      req.end = vi.fn(() => { emitData(); emitEnd(); });
      vi.mocked(http.request as any).mockImplementation((_opts: any, cb: any) => {
        cb(res);
        return req;
      });

      await replayRequest(
        "POST", "http://example.com/", {},
        Buffer.from("payload").toString("base64"),
      );

      expect(req.write).toHaveBeenCalledOnce();
    });

    it("filters hop-by-hop headers from the response", async () => {
      const { res, emitData, emitEnd } = makeMockHttpResponse(
        200,
        { "connection": "close", "content-type": "application/json", "transfer-encoding": "chunked" },
        [],
      );
      const req = new EventEmitter() as any;
      req.write = vi.fn();
      req.end = vi.fn(() => { emitData(); emitEnd(); });
      vi.mocked(http.request as any).mockImplementation((_opts: any, cb: any) => {
        cb(res);
        return req;
      });

      const result = await replayRequest("GET", "http://example.com/", {}, "");

      expect(result.headers["connection"]).toBeUndefined();
      expect(result.headers["transfer-encoding"]).toBeUndefined();
      expect(result.headers["content-type"]).toBe("application/json");
    });

    it("joins array-valued response headers in replayRequest", async () => {
      const { res, emitData, emitEnd } = makeMockHttpResponse(
        200,
        { "set-cookie": ["a=1; Path=/", "b=2; Path=/"], "content-type": "text/plain" } as any,
        [],
      );
      const req = new EventEmitter() as any;
      req.write = vi.fn();
      req.end = vi.fn(() => { emitData(); emitEnd(); });
      vi.mocked(http.request as any).mockImplementation((_opts: any, cb: any) => {
        cb(res);
        return req;
      });

      const result = await replayRequest("GET", "http://example.com/", {}, "");

      expect(result.headers["set-cookie"]).toBe("a=1; Path=/, b=2; Path=/");
    });

    it("rejects on request error", async () => {
      const req = new EventEmitter() as any;
      req.write = vi.fn();
      req.end = vi.fn(() =>
        setImmediate(() => req.emit("error", new Error("ECONNREFUSED"))),
      );
      vi.mocked(http.request as any).mockImplementation((_opts: any, _cb: any) => req);

      await expect(
        replayRequest("GET", "http://localhost:9999/", {}, ""),
      ).rejects.toThrow("ECONNREFUSED");
    });

    it("uses the correct port and path from the URL", async () => {
      const { res, emitData, emitEnd } = makeMockHttpResponse(200, {}, []);
      const req = new EventEmitter() as any;
      req.write = vi.fn();
      req.end = vi.fn(() => { emitData(); emitEnd(); });
      let capturedOptions: any;
      vi.mocked(http.request as any).mockImplementation((opts: any, cb: any) => {
        capturedOptions = opts;
        cb(res);
        return req;
      });

      await replayRequest("GET", "http://example.com:3000/path?q=1", {}, "");

      expect(capturedOptions.port).toBe(3000);
      expect(capturedOptions.path).toBe("/path?q=1");
    });
  });

  // ── logEmitter ────────────────────────────────────────────────────────

  describe("logEmitter", () => {
    it("is an EventEmitter with on and emit methods", () => {
      expect(typeof logEmitter.on).toBe("function");
      expect(typeof logEmitter.emit).toBe("function");
    });

    it("can broadcast custom events to listeners", () => {
      const listener = vi.fn();
      logEmitter.on("test-event", listener);
      logEmitter.emit("test-event", { data: "test" });
      expect(listener).toHaveBeenCalledWith({ data: "test" });
      logEmitter.removeListener("test-event", listener);
    });
  });

  // ── HTTP dispatch (via socket connection handler) ──────────────────────

  describe("HTTP dispatch (via socket connection handler)", () => {
    const baseConfig = {
      port: 8080, minimizeToTray: true,
      workspaces: [{ id: "default", name: "Workspace 1", activeEnvironmentId: null }],
      activeWorkspaceId: "default",
      mappings: [] as any[], proxyRules: [] as any[],
      mocks: [] as any[], requests: [] as any[],
      mockFolders: [] as any[], requestFolders: [] as any[],
      environments: [] as any[], activeEnvironmentId: null as string | null,
    };

    let capturedConnectionCb: ((socket: net.Socket) => void) | null = null;

    function makeDispatchSocket() {
      const s = new EventEmitter() as any;
      s.writable = true;
      s.write = vi.fn();
      s.end = vi.fn();
      s.destroy = vi.fn();
      s.pipe = vi.fn();
      return s;
    }

    function setupDispatchServer(cfg: any = baseConfig) {
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
      capturedConnectionCb = null;
      const srv = makeMockServer();
      vi.mocked(net.createServer).mockImplementation((cb: any) => {
        capturedConnectionCb = cb;
        return srv;
      });
      startServer(8080);
    }

    function openConnection() {
      const socket = makeDispatchSocket();
      capturedConnectionCb!(socket as any);
      return socket;
    }

    function sendHttp(socket: any, method: string, target: string, host: string, extraHeaders: Record<string, string> = {}) {
      let raw = `${method} ${target} HTTP/1.1\r\nHost: ${host}\r\n`;
      for (const [k, v] of Object.entries(extraHeaders)) raw += `${k}: ${v}\r\n`;
      raw += "\r\n";
      socket.emit("data", Buffer.from(raw));
    }

    function makeMockHttpReq() {
      const mockReq = new EventEmitter() as any;
      mockReq.write = vi.fn();
      mockReq.end = vi.fn();
      return mockReq;
    }

    beforeEach(() => {
      vi.clearAllMocks();
      vi.mocked(loadConfig).mockReturnValue(baseConfig as any);
      vi.mocked(http.request as any).mockReset();
    });

    // ── the reverse half is NOT here ────────────────────────────────────
    //
    // The home page, the `*.localhost` 404, and the mapping `/` port-suffix cases moved to
    // `tests/proxy/edge.test.ts` with `plan/15` work item 3, which moved the reverse half onto the edge.
    // What remains below is the forward half — absolute-form requests and CONNECT — plus the two negatives
    // that pin this module's *refusal* to answer the reverse half.

    // ── CONNECT tunnel ─────────────────────────────────────────────────

    it("calls net.connect for CONNECT method requests", async () => {
      const upstream = new EventEmitter() as any;
      upstream.destroy = vi.fn();
      upstream.pipe = vi.fn();
      upstream.write = vi.fn();
      // Call cb asynchronously so that `upstream` is assigned before the callback runs
      // (avoids temporal dead zone: the callback references `upstream` from outer scope)
      vi.mocked(net.connect as any).mockImplementation((_p: any, _h: any, cb?: any) => {
        setImmediate(() => cb?.());
        return upstream;
      });
      setupDispatchServer();
      const socket = openConnection();
      sendHttp(socket, "CONNECT", "example.com:443", "example.com:443");
      expect(net.connect).toHaveBeenCalled();
      await new Promise<void>((r) => setImmediate(r));
      expect(socket.write).toHaveBeenCalledWith("HTTP/1.1 200 Connection Established\r\n\r\n");
    });

    it("sends 502 when tcpTunnel upstream emits an error (with preamble path)", async () => {
      const upstream = new EventEmitter() as any;
      upstream.destroy = vi.fn();
      upstream.pipe = vi.fn();
      upstream.write = vi.fn();
      // Do NOT call the connect callback so preamble is not sent before error
      vi.mocked(net.connect as any).mockImplementation((_p: any, _h: any, _cb?: any) => upstream);
      setupDispatchServer();
      const socket = openConnection();
      sendHttp(socket, "CONNECT", "example.com:443", "example.com:443");
      upstream.emit("error", new Error("ECONNREFUSED"));
      await new Promise<void>((r) => setImmediate(r));
      const written = socket.write.mock.calls.map((c: any) => c[0].toString()).join("");
      expect(written).toContain("502");
    });

    // ── Mock matching ──────────────────────────────────────────────────

    it("serves mock response when URL matches exactly", () => {
      setupDispatchServer({
        ...baseConfig,
        mocks: [{ id: "m1", name: "Mock", method: "GET", urlPattern: "http://api.example.com/data",
          useRegex: false, enabled: true, capturedHeaders: {}, capturedBody: "",
          responseStatus: 200, responseHeaders: {}, responseBody: '{"ok":true}', createdAt: 1 }],
      });
      const socket = openConnection();
      sendHttp(socket, "GET", "http://api.example.com/data", "api.example.com");
      const written = (socket.write.mock.calls[0][0] as Buffer | string).toString();
      expect(written).toContain("200");
    });

    it("serves mock response when URL matches a regex pattern", () => {
      setupDispatchServer({
        ...baseConfig,
        mocks: [{ id: "m1", name: "Regex Mock", method: "GET", urlPattern: "http://.*\\.example\\.com/data",
          useRegex: true, enabled: true, capturedHeaders: {}, capturedBody: "",
          responseStatus: 201, responseHeaders: {}, responseBody: '{}', createdAt: 1 }],
      });
      const socket = openConnection();
      sendHttp(socket, "GET", "http://api.example.com/data", "api.example.com");
      const written = (socket.write.mock.calls[0][0] as Buffer | string).toString();
      expect(written).toContain("201");
    });

    it("matches mock with method '*' against any HTTP method", () => {
      setupDispatchServer({
        ...baseConfig,
        mocks: [{ id: "m1", name: "Any Method", method: "*", urlPattern: "http://api.example.com/data",
          useRegex: false, enabled: true, capturedHeaders: {}, capturedBody: "",
          responseStatus: 200, responseHeaders: {}, responseBody: '{}', createdAt: 1 }],
      });
      const socket = openConnection();
      sendHttp(socket, "DELETE", "http://api.example.com/data", "api.example.com");
      const written = (socket.write.mock.calls[0][0] as Buffer | string).toString();
      expect(written).toContain("200");
    });

    it("skips disabled mocks and falls through to passthrough", () => {
      vi.mocked(http.request as any).mockReturnValue(makeMockHttpReq());
      setupDispatchServer({
        ...baseConfig,
        mocks: [{ id: "m1", name: "Disabled", method: "GET", urlPattern: "http://api.example.com/data",
          useRegex: false, enabled: false, capturedHeaders: {}, capturedBody: "",
          responseStatus: 200, responseHeaders: {}, responseBody: '{}', createdAt: 1 }],
      });
      const socket = openConnection();
      sendHttp(socket, "GET", "http://api.example.com/data", "api.example.com");
      expect(http.request).toHaveBeenCalled();
    });

    it("skips mock when request method does not match mock's method", () => {
      vi.mocked(http.request as any).mockReturnValue(makeMockHttpReq());
      setupDispatchServer({
        ...baseConfig,
        mocks: [{ id: "m1", name: "POST only", method: "POST", urlPattern: "http://api.example.com/data",
          useRegex: false, enabled: true, capturedHeaders: {}, capturedBody: "",
          responseStatus: 200, responseHeaders: {}, responseBody: '{}', createdAt: 1 }],
      });
      const socket = openConnection();
      sendHttp(socket, "GET", "http://api.example.com/data", "api.example.com");
      expect(http.request).toHaveBeenCalled();
    });

    it("skips mock with invalid regex and falls through to passthrough", () => {
      vi.mocked(http.request as any).mockReturnValue(makeMockHttpReq());
      setupDispatchServer({
        ...baseConfig,
        mocks: [{ id: "m1", name: "Bad Regex", method: "GET", urlPattern: "[invalid",
          useRegex: true, enabled: true, capturedHeaders: {}, capturedBody: "",
          responseStatus: 200, responseHeaders: {}, responseBody: '{}', createdAt: 1 }],
      });
      const socket = openConnection();
      expect(() => sendHttp(socket, "GET", "http://api.example.com/data", "api.example.com")).not.toThrow();
      expect(http.request).toHaveBeenCalled();
    });

    it("resolves environment variables in mock response body", () => {
      setupDispatchServer({
        ...baseConfig,
        environments: [{ id: "env1", name: "Dev",
          variables: [{ id: "v1", key: "API_URL", value: "http://real-api.example.com" }], createdAt: 1 }],
        activeEnvironmentId: "env1",
        mocks: [{ id: "m1", name: "Env Mock", method: "GET", urlPattern: "http://api.example.com/",
          useRegex: false, enabled: true, capturedHeaders: {}, capturedBody: "",
          responseStatus: 200, responseHeaders: {}, responseBody: '{"url":"{{API_URL}}"}', createdAt: 1 }],
      });
      const socket = openConnection();
      sendHttp(socket, "GET", "http://api.example.com/", "api.example.com");
      const allWritten = socket.write.mock.calls.map((c: any) => {
        const a = c[0];
        return Buffer.isBuffer(a) ? a.toString("utf-8") : String(a);
      }).join("");
      expect(allWritten).toContain("http://real-api.example.com");
    });

    it("resolves env variables in mock response headers", () => {
      setupDispatchServer({
        ...baseConfig,
        environments: [{ id: "env1", name: "Dev",
          variables: [{ id: "v1", key: "ORIGIN", value: "https://myapp.com" }], createdAt: 1 }],
        activeEnvironmentId: "env1",
        mocks: [{ id: "m1", name: "Header Mock", method: "GET", urlPattern: "http://api.example.com/h",
          useRegex: false, enabled: true, capturedHeaders: {},  capturedBody: "",
          responseStatus: 200, responseHeaders: { "access-control-allow-origin": "{{ORIGIN}}" },
          mockedResponseHeaders: ["access-control-allow-origin"],
          responseBody: '{}', createdAt: 1 }],
      });
      const socket = openConnection();
      sendHttp(socket, "GET", "http://api.example.com/h", "api.example.com");
      const allWritten = socket.write.mock.calls.map((c: any) => {
        const a = c[0];
        return Buffer.isBuffer(a) ? a.toString("utf-8") : String(a);
      }).join("");
      expect(allWritten).toContain("https://myapp.com");
    });

    it("emits request log entry with via=mock for mock-served responses", () => {
      setupDispatchServer({
        ...baseConfig,
        mocks: [{ id: "m1", name: "Mock", method: "GET", urlPattern: "http://api.example.com/",
          useRegex: false, enabled: true, capturedHeaders: {}, capturedBody: "",
          responseStatus: 200, responseHeaders: {}, responseBody: '{}', createdAt: 1 }],
      });
      const emitted: any[] = [];
      logEmitter.on("request", (e: any) => emitted.push(e));
      const socket = openConnection();
      sendHttp(socket, "GET", "http://api.example.com/", "api.example.com");
      expect(emitted[0]?.via).toBe("mock");
      logEmitter.removeAllListeners("request");
    });

    // ── Proxy rules ────────────────────────────────────────────────────

    it("proxies via rule when URL matches proxy rule pattern", () => {
      vi.mocked(http.request as any).mockReturnValue(makeMockHttpReq());
      setupDispatchServer({
        ...baseConfig,
        mappings: [{ id: "m1", domain: "api.localhost", target: "localhost:3000", enabled: true }],
        proxyRules: [{ id: "r1", name: "API", pattern: ".*api\\.example\\.com.*", useRegex: true, targetMappingId: "m1", enabled: true }],
      });
      const socket = openConnection();
      sendHttp(socket, "GET", "http://api.example.com/data", "api.example.com");
      expect(http.request).toHaveBeenCalled();
    });

    it("serves 502 when proxy rule matches but target mapping is missing", () => {
      setupDispatchServer({
        ...baseConfig,
        proxyRules: [{ id: "r1", name: "Dead", pattern: ".*api\\.example\\.com.*", useRegex: true, targetMappingId: "nonexistent", enabled: true }],
      });
      const socket = openConnection();
      sendHttp(socket, "GET", "http://api.example.com/data", "api.example.com");
      const written = (socket.write.mock.calls[0][0] as Buffer | string).toString();
      expect(written).toContain("502");
    });

    it("skips disabled proxy rules and falls through to passthrough", () => {
      vi.mocked(http.request as any).mockReturnValue(makeMockHttpReq());
      setupDispatchServer({
        ...baseConfig,
        mappings: [{ id: "m1", domain: "api.localhost", target: "localhost:3000", enabled: true }],
        proxyRules: [{ id: "r1", name: "Disabled", pattern: ".*api\\.example\\.com.*", useRegex: true, targetMappingId: "m1", enabled: false }],
      });
      const socket = openConnection();
      sendHttp(socket, "GET", "http://api.example.com/data", "api.example.com");
      expect(http.request).toHaveBeenCalled();
      const opts = (http.request as any).mock.calls.at(-1)[0];
      expect(opts.hostname).toBe("api.example.com");
    });

    it("skips proxy rule with invalid regex and falls through to passthrough", () => {
      vi.mocked(http.request as any).mockReturnValue(makeMockHttpReq());
      setupDispatchServer({
        ...baseConfig,
        proxyRules: [{ id: "r1", name: "Bad regex", pattern: "[invalid", useRegex: true, targetMappingId: "m1", enabled: true }],
      });
      const socket = openConnection();
      expect(() => sendHttp(socket, "GET", "http://api.example.com/data", "api.example.com")).not.toThrow();
      expect(http.request).toHaveBeenCalled();
    });

    // ── Passthrough ────────────────────────────────────────────────────

    it("passthroughs http:// requests when no mock or rule matches", () => {
      vi.mocked(http.request as any).mockReturnValue(makeMockHttpReq());
      setupDispatchServer();
      const socket = openConnection();
      sendHttp(socket, "GET", "http://api.example.com/data", "api.example.com");
      expect(http.request).toHaveBeenCalled();
      const opts = (http.request as any).mock.calls[0][0];
      expect(opts.hostname).toBe("api.example.com");
    });

    it("emits request log entry with via=proxy for passthrough responses", () => {
      const mockRes = new EventEmitter() as any;
      mockRes.statusCode = 200;
      mockRes.statusMessage = "OK";
      mockRes.headers = { "content-type": "text/plain" };
      mockRes.pipe = vi.fn();
      mockRes.destroy = vi.fn();
      const mockReq = makeMockHttpReq();
      vi.mocked(http.request as any).mockImplementation((_o: any, cb: any) => { cb(mockRes); return mockReq; });

      setupDispatchServer();
      const emitted: any[] = [];
      logEmitter.on("request", (e: any) => emitted.push(e));
      const socket = openConnection();
      sendHttp(socket, "GET", "http://api.example.com/data", "api.example.com");
      mockRes.emit("end");
      expect(emitted[0]?.via).toBe("proxy");
      logEmitter.removeAllListeners("request");
    });

    // ── 400 Bad Request ────────────────────────────────────────────────

    it("serves 400 for requests with a non-localhost external host header", () => {
      setupDispatchServer();
      const socket = openConnection();
      sendHttp(socket, "GET", "/just-a-path", "external.example.com");
      const written = (socket.write.mock.calls[0][0] as Buffer | string).toString();
      expect(written).toContain("400");
    });

    it("emits request log entry with status 400 for bad requests", () => {
      setupDispatchServer();
      const emitted: any[] = [];
      logEmitter.on("request", (e: any) => emitted.push(e));
      const socket = openConnection();
      sendHttp(socket, "GET", "/path", "external.example.com");
      expect(emitted[0]?.status).toBe(400);
      logEmitter.removeAllListeners("request");
    });

    // ── Socket lifecycle ───────────────────────────────────────────────

    it("calls socket.destroy() when socket emits an error event", () => {
      setupDispatchServer();
      const socket = openConnection();
      socket.emit("error", new Error("socket error"));
      expect(socket.destroy).toHaveBeenCalled();
    });

    it("does not dispatch when data arrives without the header terminator", () => {
      setupDispatchServer();
      const socket = openConnection();
      socket.emit("data", Buffer.from("GET / HTTP/1.1\r\nHost: localhost"));
      expect(socket.write).not.toHaveBeenCalled();
    });

    it("dispatches after buffering multiple data chunks into a complete header", () => {
      setupDispatchServer();
      const socket = openConnection();
      socket.emit("data", Buffer.from("GET / HTTP/1.1\r\n"));
      socket.emit("data", Buffer.from("Host: localhost\r\n\r\n"));
      expect(socket.write).toHaveBeenCalled();
    });

    it("ignores subsequent data events once dispatched", () => {
      setupDispatchServer();
      const socket = openConnection();
      sendHttp(socket, "GET", "/", "localhost");
      const countAfterFirst = socket.write.mock.calls.length;
      sendHttp(socket, "GET", "/other", "external.example.com");
      expect(socket.write.mock.calls.length).toBe(countAfterFirst);
    });

    // ── Error propagation from upstream ───────────────────────────────

    // `sends 502 when proxyToUpstream request emits an error` moved to `edge.test.ts` — it drove a
    // `*.localhost` mapping, which is the reverse half.

    it("sends 502 when passthroughToUpstream request emits an error", async () => {
      const mockReq = makeMockHttpReq();
      mockReq.end = vi.fn(() => setImmediate(() => mockReq.emit("error", new Error("ECONNREFUSED"))));
      vi.mocked(http.request as any).mockImplementation((_o: any, _cb: any) => mockReq);
      setupDispatchServer();
      const socket = openConnection();
      sendHttp(socket, "GET", "http://api.example.com/data", "api.example.com");
      await new Promise<void>((r) => setImmediate(() => setImmediate(r)));
      const written = socket.write.mock.calls.map((c: any) => c[0].toString()).join("");
      expect(written).toContain("502");
    });

    // ── the forward half's own response path ───────────────────────────
    //
    // `sends the proxied response head and body back to client` and
    // `emits request log with via=rfc6761 for mapped *.localhost responses` moved to `edge.test.ts` —
    // both drove a `*.localhost` mapping. What is left is the half that routes on a **rule**, which is the
    // forward proxy's own mechanism.

    it("emits request log with via=rule for proxy-rule-routed responses", () => {
      const mockRes = new EventEmitter() as any;
      mockRes.statusCode = 200;
      mockRes.statusMessage = "OK";
      mockRes.headers = {};
      mockRes.pipe = vi.fn();
      mockRes.destroy = vi.fn();
      const mockReq = makeMockHttpReq();
      vi.mocked(http.request as any).mockImplementation((_o: any, cb: any) => { cb(mockRes); return mockReq; });

      setupDispatchServer({
        ...baseConfig,
        mappings: [{ id: "m1", domain: "api.localhost", target: "localhost:3000", enabled: true }],
        proxyRules: [{ id: "r1", name: "API", pattern: ".*api\\.example\\.com.*", useRegex: true, targetMappingId: "m1", enabled: true }],
      });
      const emitted: any[] = [];
      logEmitter.on("request", (e: any) => emitted.push(e));
      const socket = openConnection();
      sendHttp(socket, "GET", "http://api.example.com/data", "api.example.com");
      mockRes.emit("data", Buffer.alloc(0));
      mockRes.emit("end");
      expect(emitted[0]?.via).toBe("rule");
      logEmitter.removeAllListeners("request");
    });

    // ── Mock serving details ───────────────────────────────────────────

    it("uses content-type application/json when mock has no content-type header", () => {
      setupDispatchServer({
        ...baseConfig,
        mocks: [{ id: "m1", name: "Mock", method: "GET", urlPattern: "http://api.example.com/",
          useRegex: false, enabled: true, capturedHeaders: {}, capturedBody: "",
          responseStatus: 200, responseHeaders: {}, responseBody: '{}', createdAt: 1 }],
      });
      const socket = openConnection();
      sendHttp(socket, "GET", "http://api.example.com/", "api.example.com");
      const allWritten = socket.write.mock.calls.map((c: any) => c[0].toString()).join("");
      expect(allWritten).toContain("content-type: application/json");
    });

    it("does not serve mock when socket is not writable", () => {
      setupDispatchServer({
        ...baseConfig,
        mocks: [{ id: "m1", name: "Mock", method: "GET", urlPattern: "http://api.example.com/",
          useRegex: false, enabled: true, capturedHeaders: {}, capturedBody: "",
          responseStatus: 200, responseHeaders: {}, responseBody: '{}', createdAt: 1 }],
      });
      const socket = openConnection();
      socket.writable = false;
      sendHttp(socket, "GET", "http://api.example.com/", "api.example.com");
      // socket.write should not be called when not writable
      expect(socket.write).not.toHaveBeenCalled();
    });

    it("serves 502 when passthroughToUpstream cannot parse the upstream URL", () => {
      // An HTTP URL that starts with "http://" but is invalid for URL parsing
      setupDispatchServer();
      const socket = openConnection();
      // Sending an HTTP-absolute request with an unparseable target
      sendHttp(socket, "GET", "http://[invalid", "[invalid");
      const written = socket.write.mock.calls.map((c: any) => c[0].toString()).join("");
      // Either 502 from passthrough URL parse error, or dispatched differently
      // The important thing is no crash and the code path is exercised
      expect(socket.write).toHaveBeenCalled();
    });

    // `does not write to socket in proxyToUpstream when socket becomes unwritable` and
    // `processes array-valued response headers in proxyToUpstream` moved to `edge.test.ts` — both drove a
    // `*.localhost` mapping. The `passthroughToUpstream` twins below stay: they are the forward half.

    it("processes array-valued response headers in passthroughToUpstream", () => {
      const mockRes = new EventEmitter() as any;
      mockRes.statusCode = 200;
      mockRes.statusMessage = "OK";
      mockRes.headers = { "x-custom": ["val1", "val2"], "content-type": "application/json" };
      mockRes.pipe = vi.fn();
      mockRes.destroy = vi.fn();
      const mockReq = makeMockHttpReq();
      vi.mocked(http.request as any).mockImplementation((_o: any, cb: any) => { cb(mockRes); return mockReq; });

      setupDispatchServer();
      const socket = openConnection();
      sendHttp(socket, "GET", "http://api.example.com/data", "api.example.com");

      expect(socket.write).toHaveBeenCalled();
      const head = (socket.write.mock.calls[0][0] as string);
      expect(head).toContain("x-custom: val1, val2");
    });

    // `calls socket.destroy when socket becomes unwritable during data reception in proxyToUpstream`,
    // `truncates large response body to 512KB in proxyToUpstream log entry`, and
    // `calls socket.destroy when response emits error in proxyToUpstream` moved to `edge.test.ts` — all
    // three drove a `*.localhost` mapping.

    it("uses default port 443 for CONNECT when port part is empty", async () => {
      const upstream = new EventEmitter() as any;
      upstream.destroy = vi.fn();
      upstream.pipe = vi.fn();
      upstream.write = vi.fn();
      let capturedPort: number | undefined;
      vi.mocked(net.connect as any).mockImplementation((port: any, _h: any, cb?: any) => {
        capturedPort = port;
        setImmediate(() => cb?.());
        return upstream;
      });

      setupDispatchServer();
      const socket = openConnection();
      // CONNECT with empty port part after colon
      sendHttp(socket, "CONNECT", "example.com:", "example.com:");
      expect(net.connect).toHaveBeenCalled();
      expect(capturedPort).toBe(443);
    });

    // `filters hop-by-hop headers from the upstream response in proxyToUpstream` and
    // `writes request body to upstream in proxyToUpstream when body is present` moved to `edge.test.ts` —
    // both drove a `*.localhost` mapping. Their `passthroughToUpstream` twins stay: the forward half.

    it("filters hop-by-hop headers from the upstream response in passthroughToUpstream", () => {
      const mockRes = new EventEmitter() as any;
      mockRes.statusCode = 200;
      mockRes.statusMessage = "OK";
      mockRes.headers = { "transfer-encoding": "chunked", "x-custom": "value" };
      mockRes.pipe = vi.fn();
      mockRes.destroy = vi.fn();
      const mockReq = makeMockHttpReq();
      vi.mocked(http.request as any).mockImplementation((_o: any, cb: any) => { cb(mockRes); return mockReq; });

      setupDispatchServer();
      const socket = openConnection();
      sendHttp(socket, "GET", "http://api.example.com/data", "api.example.com");

      const head = (socket.write.mock.calls[0][0] as string);
      expect(head).not.toContain("transfer-encoding: chunked");
      expect(head).toContain("x-custom: value");
    });

    it("writes request body to upstream in passthroughToUpstream when body is present", () => {
      const mockReq = makeMockHttpReq();
      vi.mocked(http.request as any).mockReturnValue(mockReq);

      setupDispatchServer();
      const socket = openConnection();
      const raw = `POST http://api.example.com/data HTTP/1.1\r\nHost: api.example.com\r\nContent-Length: 4\r\n\r\nbody`;
      socket.emit("data", Buffer.from(raw));

      expect(http.request).toHaveBeenCalled();
      expect(mockReq.write).toHaveBeenCalled();
    });
  });

  // ── createProxyServer() instance isolation (P14 WI 2) ────────────────
  //
  // WI 2's whole point is that the six module-level mutables (`server`, `currentPort`,
  // `currentConfig`, `fullRules`, `lastError`, `enabledSets`) become instance state. The cases
  // below prove the two halves that matter: two instances do NOT share that state, and the
  // module-level facade still resolves to exactly ONE instance (no accidental N-by-import).

  describe("createProxyServer() — instance isolation (P14 WI 2)", () => {
    // The file-level beforeEach installs ONE shared mock server via mockReturnValue. For
    // isolation cases that is the wrong fixture: `net.createServer` returning the same object
    // would make two instances share the `net.Server` even after the refactor. Hand each call a
    // fresh mock so the assertions are about the instance state, not about the mock.
    beforeEach(() => {
      vi.mocked(net.createServer).mockImplementation(() => makeMockServer());
    });

    it("two instances report independent running state", () => {
      const a = createProxyServer({ workspaceId: "ws-a" });
      const b = createProxyServer({ workspaceId: "ws-b" });

      a.start(8080);

      expect(a.isRunning()).toBe(true);
      // b was never started — if `server` were still module-global, b would read a's server.
      expect(b.isRunning()).toBe(false);

      a.stop();
    });

    it("two instances report independent ports", () => {
      const a = createProxyServer({ workspaceId: "ws-a" });
      const b = createProxyServer({ workspaceId: "ws-b" });

      a.start(8080);
      b.start(8090);

      expect(a.getPort()).toBe(8080);
      expect(b.getPort()).toBe(8090);

      a.stop();
      b.stop();
    });

    it("each instance carries its own workspaceId", () => {
      const a = createProxyServer({ workspaceId: "ws-a" });
      const b = createProxyServer({ workspaceId: "ws-b" });
      const c = createProxyServer();

      expect(a.workspaceId).toBe("ws-a");
      expect(b.workspaceId).toBe("ws-b");
      // Omitted → null, the same default the module-level facade uses.
      expect(c.workspaceId).toBeNull();
    });

    it("stopping one instance does not stop the other", () => {
      const a = createProxyServer({ workspaceId: "ws-a" });
      const b = createProxyServer({ workspaceId: "ws-b" });

      a.start(8080);
      b.start(8090);
      a.stop();

      expect(a.isRunning()).toBe(false);
      expect(b.isRunning()).toBe(true);

      b.stop();
    });

    it("a bind failure lands on the losing instance only", () => {
      const srv = makeMockServer();
      srv.listen = vi.fn((_port: number, _host: string) => {
        // Simulate EADDRINUSE arriving asynchronously, as net.Server does.
        setTimeout(() => srv.emit("error", Object.assign(new Error("in use"), { code: "EADDRINUSE" })), 0);
        return srv;
      });
      vi.mocked(net.createServer).mockReturnValue(srv);

      const a = createProxyServer({ workspaceId: "ws-a" });
      const b = createProxyServer({ workspaceId: "ws-b" });

      a.start(8080);

      // `start()` is fire-and-forget for the error path; wait for the error to land.
      return waitFor(() => a.getError() !== null).then(() => {
        expect(a.getError()).toContain("8080");
        // b never bound anything, so it must not have inherited a's lastError.
        expect(b.getError()).toBeNull();
      });
    });

    it("reloadConfig on one instance does not disturb the other's port", () => {
      const a = createProxyServer({ workspaceId: "ws-a" });
      const b = createProxyServer({ workspaceId: "ws-b" });

      a.start(8080);
      b.start(8090);
      a.reloadConfig();

      expect(a.getPort()).toBe(8080);
      expect(b.getPort()).toBe(8090);

      a.stop();
      b.stop();
    });
  });

  // ── WI 4: each instance serves its OWN workspace's rules ─────────────
  //
  // The acceptance criterion is "two workspaces, each serving its own rules", and it must be asserted on
  // **identity** — each instance reading a rule set the other cannot see — not on a value both could share.
  // A proxy that read `cfg.activeWorkspaceId` would pass every other case in this file, because with one
  // workspace there is nothing to confuse it with.

  describe("WI 4 — workspace severance (each instance serves its own rules)", () => {
    /** Records which workspace each read asks for, and returns a distinguishable rule set per workspace. */
    function recordEntityReads() {
      const reads: string[] = [];
      vi.mocked(readAllEntities).mockImplementation((wsId: string) => {
        reads.push(wsId);
        // The identity: only ws-a gets a rule, and it names its own workspace, so a mixed-up read is
        // detectable rather than plausible.
        return wsId === "ws-a"
          ? ([{ id: "rule-a", workspaceId: "ws-a", targetType: "mapping" }] as any)
          : ([] as any);
      });
      // Left delegating to the real implementation — stubbing it here would change dispatch for every
      // other case in this file (see the note on the `serverUtils` mock).
      vi.mocked(loadEnabledSets).mockClear();
      return reads;
    }

    it("reads its own workspaceId, not cfg.activeWorkspaceId", () => {
      const reads = recordEntityReads();
      // The config's active workspace is deliberately a *third* value: if the instance consulted it, the
      // recorded reads would say "ws-active" and this case would fail on the assertion below.
      vi.mocked(loadConfig).mockReturnValue({ ...tlsConfig(), activeWorkspaceId: "ws-active" } as any);

      const a = createProxyServer({ workspaceId: "ws-a" });
      a.start(8080);

      expect(reads).toContain("ws-a");
      expect(reads).not.toContain("ws-active");
      a.stop();
    });

    it("two instances read two different workspaces", () => {
      const reads = recordEntityReads();
      vi.mocked(loadConfig).mockReturnValue({ ...tlsConfig(), activeWorkspaceId: "ws-active" } as any);

      const a = createProxyServer({ workspaceId: "ws-a" });
      const b = createProxyServer({ workspaceId: "ws-b" });

      a.start(8080);
      b.start(8090);

      expect(reads).toContain("ws-a");
      expect(reads).toContain("ws-b");
      a.stop();
      b.stop();
    });

    it("an instance with no workspaceId still follows the active workspace (the desktop's path)", () => {
      // This is the facade's behaviour, and P14 WI 7 keeps it until the desktop adopts the runtime. It is
      // asserted here so the fallback is *load-bearing* rather than incidental.
      const reads = recordEntityReads();
      vi.mocked(loadConfig).mockReturnValue({ ...tlsConfig(), activeWorkspaceId: "ws-active" } as any);

      const facade = createProxyServer();
      facade.start(8080);

      expect(reads).toContain("ws-active");
      facade.stop();
    });

    it("the fallback is re-read on reloadConfig, so the desktop still follows a workspace switch", () => {
      const reads = recordEntityReads();
      vi.mocked(loadConfig).mockReturnValue({ ...tlsConfig(), activeWorkspaceId: "ws-first" } as any);

      const facade = createProxyServer();
      facade.start(8080);
      expect(reads).toContain("ws-first");

      // The user switches workspace in the UI; the single proxy must follow.
      vi.mocked(loadConfig).mockReturnValue({ ...tlsConfig(), activeWorkspaceId: "ws-second" } as any);
      facade.reloadConfig();

      expect(reads).toContain("ws-second");
      facade.stop();
    });
  });

  // ── CA ownership: the shared-TLS-anchor rule (P14 WI 3) ──────────────
  //
  // `tlsCert.ts` keeps the CA in module state, so `unloadCA()` is process-global. A host running N
  // proxies must not let runtime A's stop tear down the CA runtime B is signing with — hence
  // `ownsCA: false` for host members. These cases exercise the real `tlsCert` module (it is not
  // mocked in this file), writing a real temp CA pair so `loadCA()` genuinely succeeds.

  describe("CA ownership (ownsCA) — the process-global TLS anchor", () => {
    let tmpDir: string;
    let caCertPath: string;
    let caKeyPath: string;
    // This file mocks `fs` globally for the dispatch cases, so `fs.mkdtempSync` is a no-op here.
    // The CA cases need real files on disk — `tlsCert.loadCA()` does a real `readFileSync`.
    let realFs: typeof import("fs");

    beforeAll(async () => {
      realFs = await vi.importActual<typeof import("fs")>("fs");
    });

    beforeEach(() => {
      // The file-level beforeEach hands every `net.createServer` call the SAME mock object, so a
      // `stop()` on one instance flips `listening` for both. These cases assert on per-instance
      // liveness, so each call gets its own mock — same reason as the isolation describe above.
      vi.mocked(net.createServer).mockImplementation(() => makeMockServer());

      tmpDir = realFs.mkdtempSync(path.join(os.tmpdir(), "proxy-ca-"));
      caCertPath = path.join(tmpDir, "ca.pem");
      caKeyPath = path.join(tmpDir, "ca-key.pem");
      realFs.writeFileSync(caCertPath, "CA_CERT_PEM");
      realFs.writeFileSync(caKeyPath, "CA_KEY_PEM");
      unloadCA();
    });

    afterEach(() => {
      unloadCA();
      realFs.rmSync(tmpDir, { recursive: true, force: true });
    });

    it("an owning instance unloads the CA on stop (the desktop's behaviour, unchanged)", () => {
      const cfg = { ...tlsConfig(), tlsEnabled: true, tlsCaCertPath: caCertPath, tlsCaKeyPath: caKeyPath };
      vi.mocked(loadConfig).mockReturnValue(cfg as any);

      const a = createProxyServer({ workspaceId: "ws-a" }); // ownsCA defaults to true
      a.start(8080);
      expect(isCALoaded()).toBe(true);
      a.stop();
      expect(isCALoaded()).toBe(false);
    });

    it("a non-owning instance leaves the shared CA loaded when it stops", () => {
      const cfg = { ...tlsConfig(), tlsEnabled: true, tlsCaCertPath: caCertPath, tlsCaKeyPath: caKeyPath };
      vi.mocked(loadConfig).mockReturnValue(cfg as any);

      const a = createProxyServer({ workspaceId: "ws-a", ownsCA: false });
      const b = createProxyServer({ workspaceId: "ws-b", ownsCA: false });

      a.start(8080);
      b.start(8090);
      expect(isCALoaded()).toBe(true);

      a.stop();

      // The point of the option: B is still interpolating, so the CA must survive A's teardown.
      expect(isCALoaded()).toBe(true);
      expect(b.isRunning()).toBe(true);

      b.stop();
      // B does not unload it either — unloading belongs to whoever owns the anchor, not the last
      // proxy to fall. A host that wants it gone calls unloadCA() itself.
      expect(isCALoaded()).toBe(true);
    });

    it("a non-owning instance with TLS off does not clear a sibling's CA", () => {
      // A is a host member configured with TLS enabled and loads the CA.
      vi.mocked(loadConfig).mockReturnValue(
        { ...tlsConfig(), tlsEnabled: true, tlsCaCertPath: caCertPath, tlsCaKeyPath: caKeyPath } as any,
      );
      const a = createProxyServer({ workspaceId: "ws-a", ownsCA: false });
      a.start(8080);
      expect(isCALoaded()).toBe(true);
      a.stop();
      expect(isCALoaded()).toBe(true);

      // B is a host member whose own workspace has TLS off. Pre-P14 it would have called unloadCA()
      // here and silently broken A's interception.
      vi.mocked(loadConfig).mockReturnValue({ ...tlsConfig(), tlsEnabled: false } as any);
      const b = createProxyServer({ workspaceId: "ws-b", ownsCA: false });
      b.start(8090);

      expect(isCALoaded()).toBe(true);
      b.stop();
    });
  });

  describe("the module-level facade still backs a single default instance", () => {
    it("startServer() and getPort() read the same instance", () => {
      startServer(8080);
      expect(isRunning()).toBe(true);
      expect(getPort()).toBe(8080);
    });

    it("isRunning() is false before anything is started", () => {
      // The facade's default instance is lazy — it is not created until first use. This case
      // deliberately does not assert the *port*, because vitest shares module state across cases
      // in a file and a prior case's `currentPort` survives `stop()` (it is only reset by the
      // next `start()`). The meaningful invariant here is that nothing is listening.
      expect(isRunning()).toBe(false);
    });

    it("switching ports through the facade does not resurrect a stopped server", () => {
      startServer(8080);
      stopServer();
      expect(isRunning()).toBe(false);
      startServer(8090);
      expect(getPort()).toBe(8090);
      stopServer();
    });
  });
});
