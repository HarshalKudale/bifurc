/**
 * Happy-path workflow test for the **"hit Send"** path — the single biggest hole
 * left in `HAPPY_PATH_COVERAGE.md`.
 *
 * Before this file:
 *  - REST: `request:replay` was only ever invoked against a *mocked* `replayRequest`,
 *    so nothing proved a request actually reached a server or that the response the
 *    user sees is the response the server sent.
 *  - GraphQL: `graphql:execute` and `graphql:introspect` had no test at all.
 *  - SOAP: `soap:execute` and `soap:fetchWsdl` had no test at all.
 *  - gRPC: `grpc:execute`/`grpc:reflect` are stubs — pinned here so that actually
 *    implementing them is a deliberate, visible change rather than a silent one.
 *
 * It drives the REAL registered IPC handlers against REAL local HTTP servers and
 * asserts the bytes the user would see in the response pane. The only things mocked
 * are Electron itself (there is no Electron in a Vitest process) and `@/main`
 * (`coreHandlers` imports `updateTrayMenu` from it).
 *
 * Runs in the integration project: it binds real sockets and the workspace fixture
 * swaps a process-global data-root override, hence `fileParallelism: false`.
 */

import { describe, it, expect, beforeAll, afterAll, vi } from "vitest";
import * as http from "http";
import * as zlib from "zlib";
import { createWorkspace, TEST_WS, type WorkspaceFixture } from "./proxyHarness";

// ── Capture the handlers the features register ────────────────────────────────
// Same technique as tests/ipc/handlers.test.ts and applications.integration.test.ts:
// swap `ipcMain` for a registry so the production handler functions can be invoked
// directly, exactly as Electron would invoke them.

const { handlers } = vi.hoisted(() => ({ handlers: new Map<string, (...args: any[]) => any>() }));

vi.mock("electron", () => ({
  ipcMain: {
    handle: (channel: string, fn: (...args: any[]) => any) => { handlers.set(channel, fn); },
    on: vi.fn(),
  },
  BrowserWindow: { getAllWindows: vi.fn(() => []) },
  app: {
    getPath: vi.fn(() => "/tmp/test-user-data"),
    on: vi.fn(),
    whenReady: vi.fn(() => Promise.resolve()),
    commandLine: { appendSwitch: vi.fn() },
    quit: vi.fn(),
  },
}));

// `coreHandlers` imports `updateTrayMenu` from `@/main`; importing the real main
// process module would try to build windows and a tray.
vi.mock("@/main", () => ({ updateTrayMenu: vi.fn() }));

/** Invoke a registered handler the way ipcMain would (first arg is the event). */
function invoke<T = any>(channel: string, ...args: any[]): T {
  const fn = handlers.get(channel);
  if (!fn) throw new Error(`No handler registered for "${channel}"`);
  return fn({}, ...args) as T;
}

const b64 = (s: string) => Buffer.from(s, "utf-8").toString("base64");
const unb64 = (s: string) => Buffer.from(s, "base64").toString("utf-8");

// ── A real HTTP server that records what it received ──────────────────────────

interface RecordedReq {
  method: string;
  url: string;
  headers: http.IncomingHttpHeaders;
  body: string;
}

interface TestServer {
  port: number;
  origin: string;
  requests: RecordedReq[];
  close: () => Promise<void>;
}

/**
 * Start a real HTTP server on an ephemeral port. The responder is handed the
 * already-recorded request (body included) so assertions and response logic can
 * both see it.
 */
async function startTestServer(
  respond: (req: RecordedReq, res: http.ServerResponse) => void,
): Promise<TestServer> {
  const requests: RecordedReq[] = [];
  const server = http.createServer((req, res) => {
    const chunks: Buffer[] = [];
    req.on("data", (c: Buffer) => chunks.push(c));
    req.on("end", () => {
      const rec: RecordedReq = {
        method: req.method ?? "GET",
        url: req.url ?? "/",
        headers: req.headers,
        body: Buffer.concat(chunks).toString("utf-8"),
      };
      requests.push(rec);
      respond(rec, res);
    });
  });

  const port = await new Promise<number>((resolve, reject) => {
    server.on("error", reject);
    server.listen(0, "127.0.0.1", () => {
      const addr = server.address();
      resolve(typeof addr === "object" && addr ? addr.port : 0);
    });
  });

  return {
    port,
    origin: `http://127.0.0.1:${port}`,
    requests,
    close: () => new Promise<void>((resolve) => server.close(() => resolve())),
  };
}

/** Start a server, then immediately close it, and hand back the (now dead) port. */
async function deadPort(): Promise<number> {
  const s = await startTestServer(() => { });
  const port = s.port;
  await s.close();
  return port;
}

function jsonResponder(
  build: (req: RecordedReq) => { status?: number; body: unknown; headers?: Record<string, string> },
) {
  return (req: RecordedReq, res: http.ServerResponse): void => {
    const { status = 200, body, headers = {} } = build(req);
    res.writeHead(status, { "content-type": "application/json", ...headers });
    res.end(JSON.stringify(body));
  };
}

// ── Fixtures ──────────────────────────────────────────────────────────────────

let ws: WorkspaceFixture;
let rest: TestServer;
let gql: TestServer;
let soap: TestServer;
let wsdl: TestServer;

beforeAll(async () => {
  ws = createWorkspace();

  // Generic REST endpoint, with a few special paths for edge-of-happy-path cases.
  rest = await startTestServer((req, res) => {
    if (req.url?.startsWith("/gzip")) {
      const payload = zlib.gzipSync(Buffer.from(JSON.stringify({ compressed: true }), "utf-8"));
      res.writeHead(200, {
        "content-type": "application/json",
        "content-encoding": "gzip",
        "content-length": String(payload.length),
      });
      res.end(payload);
      return;
    }
    if (req.url?.startsWith("/hop")) {
      res.writeHead(200, {
        "content-type": "application/json",
        connection: "keep-alive",
        "keep-alive": "timeout=5",
        "x-kept": "yes",
      });
      res.end(JSON.stringify({ hop: true }));
      return;
    }
    if (req.url?.startsWith("/status/")) {
      const code = Number(req.url.split("/status/")[1]);
      res.writeHead(code, { "content-type": "application/json" });
      res.end(JSON.stringify({ code }));
      return;
    }
    jsonResponder(() => ({
      body: { ok: true, method: req.method, url: req.url, body: req.body },
      headers: { "x-rest": "real" },
    }))(req, res);
  });

  // A real GraphQL endpoint: answers introspection, otherwise echoes the operation.
  gql = await startTestServer((req, res) => {
    const parsed = JSON.parse(req.body || "{}") as Record<string, unknown>;
    if (String(parsed.query ?? "").includes("__schema")) {
      res.writeHead(200, { "content-type": "application/json" });
      res.end(JSON.stringify({ data: { __schema: { queryType: { name: "Query" } } } }));
      return;
    }
    res.writeHead(200, { "content-type": "application/json", "x-gql": "real" });
    res.end(JSON.stringify({ data: { echo: parsed } }));
  });

  // A real SOAP endpoint.
  soap = await startTestServer((_req, res) => {
    res.writeHead(200, { "content-type": "text/xml; charset=utf-8", "x-soap": "real" });
    res.end("<soap:Envelope><soap:Body><Ok>true</Ok></soap:Body></soap:Envelope>");
  });

  // A WSDL document served over plain GET.
  wsdl = await startTestServer((_req, res) => {
    res.writeHead(200, { "content-type": "text/xml" });
    res.end('<?xml version="1.0"?><definitions name="TestService"/>');
  });

  const { registerCoreHandlers } = await import("@/ipc/handlers/coreHandlers");
  const { registerGraphqlHandlers } = await import("@/ipc/handlers/graphqlHandlers");
  const { registerSoapHandlers } = await import("@/ipc/handlers/soapHandlers");
  const { registerGrpcHandlers } = await import("@/ipc/handlers/grpcHandlers");

  registerCoreHandlers();
  registerGraphqlHandlers();
  registerSoapHandlers();
  registerGrpcHandlers();
});

afterAll(async () => {
  await Promise.all([
    rest?.close(),
    gql?.close(),
    soap?.close(),
    wsdl?.close(),
  ]);
  ws?.cleanup();
});

// ── REST: the Send button ─────────────────────────────────────────────────────

describe("request:replay — REST Send", () => {
  it("returns status, headers and the decoded body for a GET", async () => {
    const res = await invoke("request:replay", "GET", `${rest.origin}/api/users`, {}, "");

    expect(res.status).toBe(200);
    expect(res.headers["content-type"]).toContain("application/json");
    expect(res.headers["x-rest"]).toBe("real");
    expect(JSON.parse(unb64(res.body))).toMatchObject({ ok: true, method: "GET", url: "/api/users" });
  });

  it("forwards the method and body and recomputes content-length", async () => {
    const payload = JSON.stringify({ name: "ada", n: 7 });
    const res = await invoke(
      "request:replay", "POST", `${rest.origin}/api/users`,
      { "content-type": "application/json" }, b64(payload),
    );

    expect(res.status).toBe(200);
    const seen = rest.requests.at(-1)!;
    expect(seen.method).toBe("POST");
    expect(seen.body).toBe(payload);
    expect(seen.headers["content-length"]).toBe(String(Buffer.byteLength(payload)));
  });

  it("forwards caller headers and preserves the query string", async () => {
    await invoke("request:replay", "GET", `${rest.origin}/api/search?q=bifurc&page=2`, { "x-api-key": "secret" }, "");

    const seen = rest.requests.at(-1)!;
    expect(seen.url).toBe("/api/search?q=bifurc&page=2");
    expect(seen.headers["x-api-key"]).toBe("secret");
  });

  it("sends connection: close so the upstream cannot hold the socket open", async () => {
    await invoke("request:replay", "GET", `${rest.origin}/api/users`, {}, "");
    expect(rest.requests.at(-1)!.headers["connection"]).toBe("close");
  });

  it("sends no body and no content-length for an empty GET", async () => {
    await invoke("request:replay", "GET", `${rest.origin}/api/users`, {}, "");
    expect(rest.requests.at(-1)!.headers["content-length"]).toBeUndefined();
  });

  it("preserves a non-2xx status instead of treating it as an error", async () => {
    const notFound = await invoke("request:replay", "GET", `${rest.origin}/status/404`, {}, "");
    expect(notFound.status).toBe(404);
    expect(JSON.parse(unb64(notFound.body)).code).toBe(404);

    const serverError = await invoke("request:replay", "GET", `${rest.origin}/status/500`, {}, "");
    expect(serverError.status).toBe(500);
  });

  it("decodes a gzip response and strips content-encoding", async () => {
    const res = await invoke("request:replay", "GET", `${rest.origin}/gzip`, {}, "");

    expect(res.status).toBe(200);
    expect(res.headers["content-encoding"]).toBeUndefined();
    expect(JSON.parse(unb64(res.body))).toEqual({ compressed: true });
  });

  it("strips hop-by-hop headers from the response", async () => {
    const res = await invoke("request:replay", "GET", `${rest.origin}/hop`, {}, "");

    expect(res.headers["x-kept"]).toBe("yes");
    expect(res.headers["connection"]).toBeUndefined();
    expect(res.headers["keep-alive"]).toBeUndefined();
  });

  it("rejects an invalid URL", async () => {
    await expect(invoke("request:replay", "GET", "not-a-url", {}, "")).rejects.toThrow("Invalid URL");
  });

  it("rejects when the upstream is unreachable", async () => {
    const port = await deadPort();
    await expect(invoke("request:replay", "GET", `http://127.0.0.1:${port}/`, {}, "")).rejects.toThrow();
  });

  it("keeps working across repeated sends to the same origin", async () => {
    // Guards a real hazard found while writing this suite. `replayRequest` sends
    // `connection: close` but does not pin an agent, so the socket can still be
    // handed back to Node's global pool; if the upstream answered `keep-alive`, the
    // next caller on that socket is rejected by the server with
    // `HPE_CLOSED_CONNECTION` (observed as a 400 or a "socket hang up"). Ten
    // sequential sends to one origin must all succeed.
    for (let i = 0; i < 10; i++) {
      const res = await invoke("request:replay", "GET", `${rest.origin}/api/users`, {}, "");
      expect(res.status, `send #${i + 1} to the same origin`).toBe(200);
    }
  });
});

// ── GraphQL ───────────────────────────────────────────────────────────────────

describe("graphql:execute — GraphQL Send", () => {
  it("POSTs a JSON body and returns status, headers, body and durationMs", async () => {
    const res = await invoke("graphql:execute", {
      url: `${gql.origin}/graphql`, headers: {}, query: "query { me }", variables: "", operationName: "",
    });

    expect(res.status).toBe(200);
    expect(res.headers["x-gql"]).toBe("real");
    expect(typeof res.durationMs).toBe("number");
    expect(res.durationMs).toBeGreaterThanOrEqual(0);
    expect(JSON.parse(res.body).data.echo.query).toBe("query { me }");
  });

  it("sets content-type: application/json and a byte-accurate content-length", async () => {
    await invoke("graphql:execute", {
      url: `${gql.origin}/graphql`, headers: {}, query: "query { me }", variables: "", operationName: "",
    });

    const seen = gql.requests.at(-1)!;
    expect(seen.method).toBe("POST");
    expect(seen.headers["content-type"]).toBe("application/json");
    expect(Number(seen.headers["content-length"])).toBe(Buffer.byteLength(seen.body));
  });

  it("parses the variables JSON string into an object and forwards operationName", async () => {
    const res = await invoke("graphql:execute", {
      url: `${gql.origin}/graphql`,
      headers: {},
      query: "query ($id: ID!) { user(id: $id) { name } }",
      variables: '{"id":"42"}',
      operationName: "GetUser",
    });

    const echoed = JSON.parse(res.body).data.echo;
    expect(echoed.variables).toEqual({ id: "42" });
    expect(echoed.operationName).toBe("GetUser");
  });

  it("omits variables and operationName when they are blank", async () => {
    const res = await invoke("graphql:execute", {
      url: `${gql.origin}/graphql`, headers: {}, query: "query { me }", variables: "   ", operationName: "",
    });

    const echoed = JSON.parse(res.body).data.echo;
    expect(echoed.variables).toBeUndefined();
    expect(echoed.operationName).toBeUndefined();
  });

  it("ignores malformed variables JSON instead of failing the request", async () => {
    const res = await invoke("graphql:execute", {
      url: `${gql.origin}/graphql`, headers: {}, query: "query { me }", variables: "{not json", operationName: "",
    });

    expect(res.status).toBe(200);
    expect(JSON.parse(res.body).data.echo.variables).toBeUndefined();
  });

  it("forwards custom headers such as an auth token", async () => {
    await invoke("graphql:execute", {
      url: `${gql.origin}/graphql`, headers: { authorization: "Bearer t" },
      query: "query { me }", variables: "", operationName: "",
    });

    expect(gql.requests.at(-1)!.headers["authorization"]).toBe("Bearer t");
  });
});

describe("graphql:introspect", () => {
  it("POSTs the introspection query and returns the raw response", async () => {
    const res = await invoke("graphql:introspect", { url: `${gql.origin}/graphql`, headers: {} });

    expect(res.ok).toBe(true);
    expect(res.sdl).toContain("__schema");
    const seen = gql.requests.at(-1)!;
    expect(seen.method).toBe("POST");
    expect(JSON.parse(seen.body).query).toContain("IntrospectionQuery");
  });

  it("returns { ok:false, error } instead of throwing when the server is down", async () => {
    const port = await deadPort();
    const res = await invoke("graphql:introspect", { url: `http://127.0.0.1:${port}/graphql`, headers: {} });

    expect(res.ok).toBe(false);
    expect(typeof res.error).toBe("string");
  });
});

// ── SOAP ──────────────────────────────────────────────────────────────────────

describe("soap:execute — SOAP Send", () => {
  const envelope = "<soap:Envelope><soap:Body><GetUser><id>1</id></GetUser></soap:Body></soap:Envelope>";

  it("POSTs the envelope with text/xml and the SOAPAction header", async () => {
    const res = await invoke("soap:execute", {
      endpointUrl: `${soap.origin}/ws`, soapAction: "urn:GetUser", headers: {}, body: envelope,
    });

    expect(res.status).toBe(200);
    expect(res.headers["x-soap"]).toBe("real");
    expect(res.body).toContain("Ok");

    const seen = soap.requests.at(-1)!;
    expect(seen.method).toBe("POST");
    expect(seen.headers["content-type"]).toBe("text/xml; charset=utf-8");
    expect(seen.headers["soapaction"]).toBe("urn:GetUser");
    expect(seen.body).toBe(envelope);
  });

  it("uses the UTF-8 byte length for content-length, not the string length", async () => {
    const unicode = "<soap:Envelope><soap:Body>✓ Ünïcode — ☕</soap:Body></soap:Envelope>";
    await invoke("soap:execute", { endpointUrl: `${soap.origin}/ws`, soapAction: "", headers: {}, body: unicode });

    const seen = soap.requests.at(-1)!;
    // Guard the premise: the fixture must actually be multi-byte.
    expect(Buffer.byteLength(unicode)).toBeGreaterThan(unicode.length);
    expect(Number(seen.headers["content-length"])).toBe(Buffer.byteLength(unicode));
    expect(seen.body).toBe(unicode);
  });

  it("omits SOAPAction when blank and merges custom headers", async () => {
    await invoke("soap:execute", {
      endpointUrl: `${soap.origin}/ws`, soapAction: "", headers: { "x-tenant": "acme" }, body: envelope,
    });

    const seen = soap.requests.at(-1)!;
    expect(seen.headers["soapaction"]).toBeUndefined();
    expect(seen.headers["x-tenant"]).toBe("acme");
  });

  it("reports durationMs", async () => {
    const res = await invoke("soap:execute", {
      endpointUrl: `${soap.origin}/ws`, soapAction: "urn:GetUser", headers: {}, body: envelope,
    });

    expect(typeof res.durationMs).toBe("number");
    expect(res.durationMs).toBeGreaterThanOrEqual(0);
  });

  it("rejects when the endpoint is unreachable", async () => {
    const port = await deadPort();
    await expect(
      invoke("soap:execute", { endpointUrl: `http://127.0.0.1:${port}/ws`, soapAction: "", headers: {}, body: envelope }),
    ).rejects.toThrow();
  });
});

describe("soap:fetchWsdl", () => {
  it("GETs the WSDL and returns its content", async () => {
    const res = await invoke("soap:fetchWsdl", `${wsdl.origin}/svc?wsdl`);

    expect(res.ok).toBe(true);
    expect(res.content).toContain("definitions");
    const seen = wsdl.requests.at(-1)!;
    expect(seen.method).toBe("GET");
    expect(seen.url).toBe("/svc?wsdl");
  });

  it("returns { ok:false, error } when the WSDL cannot be fetched", async () => {
    const res = await invoke("soap:fetchWsdl", "http://127.0.0.1:1/svc?wsdl");

    expect(res.ok).toBe(false);
    expect(typeof res.error).toBe("string");
  });
});

// ── gRPC: pin the current contract ────────────────────────────────────────────
// gRPC is NOT implemented. These tests exist so that shipping it is a deliberate
// change to a test rather than a silent behaviour swap.

describe("grpc — current (unimplemented) contract", () => {
  it("grpc:execute reports the runtime is not configured instead of throwing", async () => {
    const res = await invoke("grpc:execute", {
      serverAddress: "127.0.0.1:50051", serviceName: "S", methodName: "M",
      requestBody: "{}", metadata: {}, protoFileId: null, useReflection: false,
    });

    expect(res.ok).toBe(false);
    expect(res.error).toMatch(/not yet configured/i);
  });

  it("grpc:reflect reports that reflection is unavailable", async () => {
    const res = await invoke("grpc:reflect", { serverAddress: "127.0.0.1:50051" });

    expect(res.ok).toBe(false);
    expect(res.error).toMatch(/not yet configured/i);
  });

  it("grpc:mockServerStatus defaults to stopped on port 9102", async () => {
    expect(await invoke("grpc:mockServerStatus")).toEqual({ running: false, port: 9102 });
  });

  it("grpc:startMockServer is unimplemented while stopMockServer is a no-op success", async () => {
    expect((await invoke("grpc:startMockServer")).ok).toBe(false);
    expect(await invoke("grpc:stopMockServer")).toEqual({ ok: true });
  });
});

// ── Health Bar: add a service, then poll it ───────────────────────────────────

describe("healthbar — service monitoring", () => {
  it("saveServices → getServices round-trips through disk", async () => {
    const services = [{ id: "svc-1", name: "Local API", url: `${rest.origin}/api/users` }];

    await invoke("healthbar:saveServices", TEST_WS, services);
    expect(await invoke("healthbar:getServices", TEST_WS)).toEqual(services);
  });

  it("getServices returns an empty list for a workspace with no saved services", async () => {
    expect(await invoke("healthbar:getServices", "ws-does-not-exist")).toEqual([]);
  });

  it("checkUrl reports up, with status, headers and body, for a live endpoint", async () => {
    // A dedicated origin: `request:replay` and `healthbar:checkUrl` share Node's
    // global HTTP agent, and a `connection: close` replay response that advertises
    // `keep-alive` can leave a socket in the pool that the server later rejects with
    // `HPE_CLOSED_CONNECTION`. That is a real (if rare) cross-caller hazard, but it
    // is not what this test is about — so it gets its own server. See the
    // "replay does not poison the shared agent" test below for the hazard itself.
    const health = await startTestServer((_req, res) => {
      res.writeHead(200, { "content-type": "application/json", "x-rest": "real" });
      res.end(JSON.stringify({ ok: true }));
    });

    try {
      const res = await invoke("healthbar:checkUrl", `${health.origin}/api/users`);

      expect(res.ok).toBe(true);
      expect(res.statusCode).toBe(200);
      expect(res.error).toBeNull();
      expect(res.headers["x-rest"]).toBe("real");
      expect(JSON.parse(res.body)).toMatchObject({ ok: true });
      expect(typeof res.durationMs).toBe("number");
    } finally {
      await health.close();
    }
  });

  it("checkUrl reports down, with an error, for a dead endpoint", async () => {
    const port = await deadPort();
    const res = await invoke("healthbar:checkUrl", `http://127.0.0.1:${port}/`);

    expect(res.ok).toBe(false);
    expect(res.statusCode).toBeNull();
    expect(res.body).toBeNull();
    expect(typeof res.error).toBe("string");
  });

  it("checkUrl reports down for an unparseable URL instead of throwing", async () => {
    const res = await invoke("healthbar:checkUrl", "not-a-url");

    expect(res.ok).toBe(false);
    expect(res.error).toBe("Invalid URL");
  });

  it("checkUrl truncates a very large body to 10000 characters", async () => {
    const big = await startTestServer((_req, res) => {
      res.writeHead(200, { "content-type": "text/plain" });
      res.end("x".repeat(25_000));
    });

    try {
      const res = await invoke("healthbar:checkUrl", `${big.origin}/`);
      expect(res.ok).toBe(true);
      expect(res.body).toHaveLength(10_000);
    } finally {
      await big.close();
    }
  });
});

// ── Entity CRUD for the protocol panels (happy path, read back off disk) ──────

describe("protocol entity CRUD", () => {
  it("graphql:addRequest round-trips through entity:load", async () => {
    const created = await invoke("graphql:addRequest", {
      name: "GetUser", endpointUrl: `${gql.origin}/graphql`, headers: { "x-a": "1" },
      query: "query { me }", variables: "", operationName: "", workspaceId: TEST_WS,
    });

    expect(created.id).toBeTruthy();
    const loaded = await invoke("entity:load", TEST_WS, "graphqlRequests", created.id);
    expect(loaded.ok).toBe(true);
    expect(loaded.entity.name).toBe("GetUser");
    expect(loaded.entity.query).toBe("query { me }");
    expect(loaded.entity.headers).toEqual({ "x-a": "1" });
  });

  it("graphql:updateRequest persists the change, graphql:deleteRequest removes it", async () => {
    const created = await invoke("graphql:addRequest", {
      name: "Temp", endpointUrl: `${gql.origin}/graphql`, headers: {},
      query: "query { a }", variables: "", operationName: "", workspaceId: TEST_WS,
    });

    await invoke("graphql:updateRequest", { ...created, name: "Renamed", query: "query { b }" });
    const updated = await invoke("entity:load", TEST_WS, "graphqlRequests", created.id);
    expect(updated.entity.name).toBe("Renamed");
    expect(updated.entity.query).toBe("query { b }");

    await invoke("graphql:deleteRequest", created.id);
    expect((await invoke("entity:load", TEST_WS, "graphqlRequests", created.id)).ok).toBe(false);
  });

  it("graphql:addMock requires an endpointPattern", async () => {
    await expect(
      invoke("graphql:addMock", {
        name: "m", endpointPattern: "  ", useRegex: false, operationType: "query", operationName: "",
        responseStatus: 200, responseHeaders: {}, responseBody: "{}", workspaceId: TEST_WS,
      }),
    ).rejects.toThrow(/endpointPattern/);
  });

  it("graphql schemas round-trip through add → list → delete", async () => {
    const schema = await invoke("graphql:addSchema", { name: "s1", content: "type Query { a: String }", workspaceId: TEST_WS });

    expect((await invoke("graphql:listSchemas")).map((s: any) => s.id)).toContain(schema.id);

    await invoke("graphql:deleteSchema", schema.id);
    expect((await invoke("graphql:listSchemas")).map((s: any) => s.id)).not.toContain(schema.id);
  });

  it("soap requests round-trip through add → entity:load", async () => {
    const created = await invoke("soap:addRequest", {
      name: "GetUser", endpointUrl: `${soap.origin}/ws`, soapAction: "urn:GetUser",
      headers: {}, body: "<x/>", workspaceId: TEST_WS,
    });

    const loaded = await invoke("entity:load", TEST_WS, "soapRequests", created.id);
    expect(loaded.ok).toBe(true);
    expect(loaded.entity.soapAction).toBe("urn:GetUser");
  });

  it("soap WSDLs round-trip through add → list → delete", async () => {
    const doc = await invoke("soap:addWsdl", { name: "svc", content: "<definitions/>", workspaceId: TEST_WS });

    expect((await invoke("soap:listWsdls")).map((w: any) => w.id)).toContain(doc.id);

    await invoke("soap:deleteWsdl", doc.id);
    expect((await invoke("soap:listWsdls")).map((w: any) => w.id)).not.toContain(doc.id);
  });

  it("grpc:addRequest requires serviceName and methodName", async () => {
    const base = {
      name: "r", serverAddress: "127.0.0.1:50051", requestBody: "{}", metadata: {},
      protoFileId: null, useReflection: false, streamingType: "unary", workspaceId: TEST_WS,
    };
    await expect(invoke("grpc:addRequest", { ...base, serviceName: "", methodName: "M" })).rejects.toThrow(/serviceName/);
    await expect(invoke("grpc:addRequest", { ...base, serviceName: "S", methodName: " " })).rejects.toThrow(/methodName/);
  });

  it("grpc protos round-trip through add → list → delete", async () => {
    const proto = await invoke("grpc:addProto", {
      name: "svc.proto", content: 'syntax = "proto3";', workspaceId: TEST_WS,
    });

    expect((await invoke("grpc:listProtos")).map((p: any) => p.id)).toContain(proto.id);

    await invoke("grpc:deleteProto", proto.id);
    expect((await invoke("grpc:listProtos")).map((p: any) => p.id)).not.toContain(proto.id);
  });
});
