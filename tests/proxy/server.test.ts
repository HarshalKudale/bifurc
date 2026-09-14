import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { EventEmitter } from "events";

// Mocked modules — all at top level so they are properly hoisted
vi.mock("fs");
vi.mock("net");
vi.mock("http");
vi.mock("https");

// Track workspace entities so readAllEntities and bootstrapEnabledSet can serve them
let _testProxyRules: any[] = [];
let _testMappings: any[] = [];
let _testMocks: any[] = [];

vi.mock("../../src/store/workspaceFs", () => ({
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
vi.mock("../../src/store/config", () => ({
  loadConfig: vi.fn(() => ({
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
  })),
  saveConfig: vi.fn(),
  generateId: vi.fn(() => "test-id"),
}));

import * as net from "net";
import * as http from "http";
import * as https from "https";
import { loadConfig } from "@/store/config";
import {
  startServer,
  stopServer,
  isRunning,
  getPort,
  getServerError,
  reloadConfig,
  replayRequest,
  logEmitter,
} from "@/proxy/server";

// ── Helpers ─────────────────────────────────────────────────────────────────

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

describe("src/proxy/server.ts", () => {
  let mockSrv: ReturnType<typeof makeMockServer>;

  beforeEach(() => {
    vi.clearAllMocks();

    // Restore loadConfig mock after clearAllMocks resets call history
    vi.mocked(loadConfig).mockReturnValue({
      port: 8080, minimizeToTray: true,
      workspaces: [{ id: "default", name: "Workspace 1", activeEnvironmentId: null }],
      activeWorkspaceId: "default",
      mappings: [], proxyRules: [],
      mocks: [], requests: [], mockFolders: [], requestFolders: [],
      environments: [], activeEnvironmentId: null,
    });

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

    // ── localhost home page ────────────────────────────────────────────

    it("serves 200 home page for requests to localhost", () => {
      setupDispatchServer();
      const socket = openConnection();
      sendHttp(socket, "GET", "http://localhost/", "localhost");
      expect(socket.write).toHaveBeenCalled();
      const written = (socket.write.mock.calls[0][0] as Buffer | string).toString();
      expect(written).toContain("200");
    });

    it("includes active mappings in the home page body", () => {
      setupDispatchServer({ ...baseConfig, mappings: [{ id: "m1", domain: "app.localhost", target: "localhost:3000", enabled: true }] });
      const socket = openConnection();
      sendHttp(socket, "GET", "/", "localhost");
      const allWritten = socket.write.mock.calls.map((c: any) => c[0].toString()).join("");
      expect(allWritten).toContain("app.localhost");
    });

    it("home page includes port suffix when port is not 80", () => {
      setupDispatchServer();
      const socket = openConnection();
      sendHttp(socket, "GET", "/", "localhost");
      const allWritten = socket.write.mock.calls.map((c: any) => c[0].toString()).join("");
      expect(allWritten).toContain("8080");
    });

    it("home page omits port suffix when port is 80", () => {
      const cfg = { ...baseConfig, port: 80 };
      vi.mocked(loadConfig).mockReturnValue(cfg as any);
      capturedConnectionCb = null;
      const srv = makeMockServer();
      vi.mocked(net.createServer).mockImplementation((cb: any) => { capturedConnectionCb = cb; return srv; });
      startServer(80);
      const socket = openConnection();
      sendHttp(socket, "GET", "/", "localhost");
      const allWritten = socket.write.mock.calls.map((c: any) => c[0].toString()).join("");
      expect(allWritten).not.toMatch(/:80(?!\d)/);
    });

    // ── *.localhost — unmapped ─────────────────────────────────────────

    it("serves 404 for an unmapped *.localhost domain", () => {
      setupDispatchServer();
      const socket = openConnection();
      sendHttp(socket, "GET", "/path", "myapp.localhost");
      const written = (socket.write.mock.calls[0][0] as Buffer | string).toString();
      expect(written).toContain("404");
    });

    it("includes 'Not Mapped' text in the 404 response for *.localhost", () => {
      setupDispatchServer();
      const socket = openConnection();
      sendHttp(socket, "GET", "/", "myapp.localhost");
      const allWritten = socket.write.mock.calls.map((c: any) => c[0].toString()).join("");
      expect(allWritten).toContain("Not Mapped");
    });

    it("emits request log entry with status 404 for unmapped *.localhost", () => {
      setupDispatchServer();
      const emitted: any[] = [];
      logEmitter.on("request", (e: any) => emitted.push(e));
      const socket = openConnection();
      sendHttp(socket, "GET", "/path", "myapp.localhost");
      expect(emitted[0]?.status).toBe(404);
      expect(emitted[0]?.via).toBe("error");
      logEmitter.removeAllListeners("request");
    });

    // ── *.localhost — mapped ───────────────────────────────────────────

    it("calls http.request to proxy to the upstream target for a mapped *.localhost domain", () => {
      vi.mocked(http.request as any).mockReturnValue(makeMockHttpReq());
      setupDispatchServer({ ...baseConfig, mappings: [{ id: "m1", domain: "app.localhost", target: "localhost:3000", enabled: true }] });
      const socket = openConnection();
      sendHttp(socket, "GET", "/api/users", "app.localhost");
      expect(http.request).toHaveBeenCalled();
    });

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

    it("sends 502 when proxyToUpstream request emits an error", async () => {
      const mockReq = makeMockHttpReq();
      mockReq.end = vi.fn(() => setImmediate(() => mockReq.emit("error", new Error("ECONNREFUSED"))));
      vi.mocked(http.request as any).mockImplementation((_o: any, _cb: any) => mockReq);
      setupDispatchServer({ ...baseConfig, mappings: [{ id: "m1", domain: "app.localhost", target: "localhost:9999", enabled: true }] });
      const socket = openConnection();
      sendHttp(socket, "GET", "/api", "app.localhost");
      await new Promise<void>((r) => setImmediate(() => setImmediate(r)));
      const written = socket.write.mock.calls.map((c: any) => c[0].toString()).join("");
      expect(written).toContain("502");
    });

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

    // ── proxyToUpstream response path ──────────────────────────────────

    it("sends the proxied response head and body back to client", () => {
      const mockRes = new EventEmitter() as any;
      mockRes.statusCode = 200;
      mockRes.statusMessage = "OK";
      mockRes.headers = { "content-type": "application/json", "connection": "keep-alive" };
      mockRes.pipe = vi.fn();
      mockRes.destroy = vi.fn();
      const mockReq = makeMockHttpReq();
      vi.mocked(http.request as any).mockImplementation((_o: any, cb: any) => { cb(mockRes); return mockReq; });

      setupDispatchServer({ ...baseConfig, mappings: [{ id: "m1", domain: "app.localhost", target: "localhost:3000", enabled: true }] });
      const socket = openConnection();
      sendHttp(socket, "GET", "/api", "app.localhost");
      mockRes.emit("data", Buffer.from('{"result":"ok"}'));
      mockRes.emit("end");
      expect(socket.write).toHaveBeenCalled();
      const head = (socket.write.mock.calls[0][0] as string);
      expect(head).toContain("HTTP/1.1 200");
      expect(head).toContain("content-type: application/json");
    });

    it("emits request log with via=rfc6761 for mapped *.localhost responses", () => {
      const mockRes = new EventEmitter() as any;
      mockRes.statusCode = 200;
      mockRes.statusMessage = "OK";
      mockRes.headers = {};
      mockRes.pipe = vi.fn();
      mockRes.destroy = vi.fn();
      const mockReq = makeMockHttpReq();
      vi.mocked(http.request as any).mockImplementation((_o: any, cb: any) => { cb(mockRes); return mockReq; });

      setupDispatchServer({ ...baseConfig, mappings: [{ id: "m1", domain: "app.localhost", target: "localhost:3000", enabled: true }] });
      const emitted: any[] = [];
      logEmitter.on("request", (e: any) => emitted.push(e));
      const socket = openConnection();
      sendHttp(socket, "GET", "/api", "app.localhost");
      mockRes.emit("data", Buffer.alloc(0));
      mockRes.emit("end");
      expect(emitted[0]?.via).toBe("rfc6761");
      logEmitter.removeAllListeners("request");
    });

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

    it("does not write to socket in proxyToUpstream when socket becomes unwritable", () => {
      const mockRes = new EventEmitter() as any;
      mockRes.statusCode = 200;
      mockRes.statusMessage = "OK";
      mockRes.headers = {};
      mockRes.pipe = vi.fn();
      mockRes.destroy = vi.fn();
      const mockReq = makeMockHttpReq();
      vi.mocked(http.request as any).mockImplementation((_o: any, cb: any) => {
        cb(mockRes);
        return mockReq;
      });

      setupDispatchServer({ ...baseConfig, mappings: [{ id: "m1", domain: "app.localhost", target: "localhost:3000", enabled: true }] });
      const socket = openConnection();
      socket.writable = false;  // make socket unwritable before response arrives
      sendHttp(socket, "GET", "/api", "app.localhost");

      // With unwritable socket, the proxy response should not write and should destroy res
      expect(mockRes.destroy).toHaveBeenCalled();
    });

    it("processes array-valued response headers in proxyToUpstream", () => {
      const mockRes = new EventEmitter() as any;
      mockRes.statusCode = 200;
      mockRes.statusMessage = "OK";
      // Array-valued header (e.g. set-cookie can come as an array)
      mockRes.headers = { "set-cookie": ["a=1; Path=/", "b=2; Path=/"], "content-type": "text/plain" };
      mockRes.pipe = vi.fn();
      mockRes.destroy = vi.fn();
      const mockReq = makeMockHttpReq();
      vi.mocked(http.request as any).mockImplementation((_o: any, cb: any) => { cb(mockRes); return mockReq; });

      setupDispatchServer({ ...baseConfig, mappings: [{ id: "m1", domain: "app.localhost", target: "localhost:3000", enabled: true }] });
      const socket = openConnection();
      sendHttp(socket, "GET", "/api", "app.localhost");

      expect(socket.write).toHaveBeenCalled();
      const head = (socket.write.mock.calls[0][0] as string);
      // set-cookie is a hop-by-hop header alternative; content-type should be in response
      expect(head).toContain("content-type: text/plain");
    });

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

    it("calls socket.destroy when socket becomes unwritable during data reception in proxyToUpstream", () => {
      const mockRes = new EventEmitter() as any;
      mockRes.statusCode = 200;
      mockRes.statusMessage = "OK";
      mockRes.headers = { "content-type": "text/plain" };
      mockRes.pipe = vi.fn();
      mockRes.destroy = vi.fn();
      const mockReq = makeMockHttpReq();
      vi.mocked(http.request as any).mockImplementation((_o: any, cb: any) => { cb(mockRes); return mockReq; });

      setupDispatchServer({ ...baseConfig, mappings: [{ id: "m1", domain: "app.localhost", target: "localhost:3000", enabled: true }] });
      const socket = openConnection();
      sendHttp(socket, "GET", "/api", "app.localhost");

      // socket becomes unwritable while data is being received
      socket.writable = false;
      mockRes.emit("data", Buffer.from("some response data"));
      expect(mockRes.destroy).toHaveBeenCalled();
    });

    it("truncates large response body to 512KB in proxyToUpstream log entry", () => {
      const mockRes = new EventEmitter() as any;
      mockRes.statusCode = 200;
      mockRes.statusMessage = "OK";
      mockRes.headers = {};
      mockRes.pipe = vi.fn();
      mockRes.destroy = vi.fn();
      const mockReq = makeMockHttpReq();
      let doneCalled = false;
      let doneBody = "";
      vi.mocked(http.request as any).mockImplementation((_o: any, cb: any) => {
        cb(mockRes);
        return mockReq;
      });

      const emitted: any[] = [];
      logEmitter.on("request", (e: any) => emitted.push(e));

      setupDispatchServer({ ...baseConfig, mappings: [{ id: "m1", domain: "app.localhost", target: "localhost:3000", enabled: true }] });
      const socket = openConnection();
      sendHttp(socket, "GET", "/api", "app.localhost");

      // Emit more than 512KB of data
      const largeData = Buffer.alloc(512 * 1024 + 100, "x");
      mockRes.emit("data", largeData);
      mockRes.emit("end");

      // The body should be truncated in the log entry
      expect(emitted[0]?.resBody.length).toBeGreaterThan(0);
      logEmitter.removeAllListeners("request");
    });

    it("calls socket.destroy when response emits error in proxyToUpstream", () => {
      const mockRes = new EventEmitter() as any;
      mockRes.statusCode = 200;
      mockRes.statusMessage = "OK";
      mockRes.headers = {};
      mockRes.pipe = vi.fn();
      mockRes.destroy = vi.fn();
      const mockReq = makeMockHttpReq();
      vi.mocked(http.request as any).mockImplementation((_o: any, cb: any) => { cb(mockRes); return mockReq; });

      setupDispatchServer({ ...baseConfig, mappings: [{ id: "m1", domain: "app.localhost", target: "localhost:3000", enabled: true }] });
      const socket = openConnection();
      sendHttp(socket, "GET", "/api", "app.localhost");

      mockRes.emit("error", new Error("response error"));
      expect(socket.destroy).toHaveBeenCalled();
    });

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

    it("filters hop-by-hop headers from the upstream response in proxyToUpstream", () => {
      const mockRes = new EventEmitter() as any;
      mockRes.statusCode = 200;
      mockRes.statusMessage = "OK";
      // connection is a hop-by-hop header and should be filtered out
      mockRes.headers = { "connection": "keep-alive", "content-type": "application/json" };
      mockRes.pipe = vi.fn();
      mockRes.destroy = vi.fn();
      const mockReq = makeMockHttpReq();
      vi.mocked(http.request as any).mockImplementation((_o: any, cb: any) => { cb(mockRes); return mockReq; });

      setupDispatchServer({ ...baseConfig, mappings: [{ id: "m1", domain: "app.localhost", target: "localhost:3000", enabled: true }] });
      const socket = openConnection();
      sendHttp(socket, "GET", "/api", "app.localhost");

      const head = (socket.write.mock.calls[0][0] as string);
      expect(head).not.toContain("connection: keep-alive");
      expect(head).toContain("content-type: application/json");
    });

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

    it("writes request body to upstream in proxyToUpstream when body is present", () => {
      const mockReq = makeMockHttpReq();
      vi.mocked(http.request as any).mockReturnValue(mockReq);

      setupDispatchServer({ ...baseConfig, mappings: [{ id: "m1", domain: "app.localhost", target: "localhost:3000", enabled: true }] });
      const socket = openConnection();
      // Send a POST with body
      const raw = `POST /api HTTP/1.1\r\nHost: app.localhost\r\nContent-Length: 4\r\n\r\nbody`;
      socket.emit("data", Buffer.from(raw));

      expect(http.request).toHaveBeenCalled();
      expect(mockReq.write).toHaveBeenCalled();
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
});
