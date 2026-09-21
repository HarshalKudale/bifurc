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
 * (the engine half of the shell's `coreHandlers` used to import `updateTrayMenu` from it).
 *
 * **The nine protocol-client cases now drive the registry, not a channel.** P6 stage 3b
 * deleted `graphql:introspect` / `graphql:execute` / `soap:execute` / `soap:fetchWsdl` and
 * the five `grpc:*` channels — each was a one-line `commandRegistry.invoke()` adapter with
 * no behaviour of its own — so those cases call `invokeCommand("<command>", params)`
 * instead. See that helper's docblock for the one thing the deletion could have got wrong
 * silently (the argument shape). Every other case here still drives a live channel.
 *
 * Runs in the integration project: it binds real sockets and the workspace fixture
 * swaps a process-global data-root override, hence `fileParallelism: false`.
 */

import { describe, it, expect, beforeAll, afterAll, vi } from "vitest";
import * as http from "http";
import * as zlib from "zlib";
import { createWorkspace, TEST_WS, type WorkspaceFixture } from "./proxyHarness";
import { commandRegistry } from "@bifurc/engine/commands/registry";
import { bus } from "@bifurc/engine/eventBus";

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

// The engine half of the shell's `coreHandlers` used to import `updateTrayMenu` from `@/main` —
// that call became a `settings.changed` bus emission in P2 and the file is now `coreKinds.ts`, so
// nothing under test imports it any more. The mock stays because a transitively imported real `main`
// would try to build windows and a tray.
vi.mock("@/main", () => ({ updateTrayMenu: vi.fn() }));

/**
 * Invoke a registered handler the way ipcMain would (first arg is the event), with the P6 reduction's
 * deleted channels translated back to the commands that replaced them.
 *
 * Two groups reached this suite, and both are handled here rather than in the call sites because the
 * call sites are the *evidence* — they still read as `invoke("request:replay", …)`, which is precisely
 * the shell signature the renderer used, and rewriting twelve of them into `commandRegistry.invoke`
 * would leave nothing in the file that records what was deleted.
 *
 *   - **`src/ipc/handlers/coreHandlers.ts`** — the reduction's sixth group (`plan/07`
 *     §5b-septendecies, 2026-09-21) deleted `config:get` and `request:replay` and the rest of its
 *     seventeen. This suite drives `request:replay` (eleven call sites) and `config:get` transitively
 *     through the REST-send setup.
 *   - **the step-3b `*Handlers.ts` → `*Kinds.ts` renames** deleted `healthbar:checkUrl` /
 *     `healthbar:getServices` / `healthbar:saveServices` and `entity:load` from this suite's path too.
 *
 * The translation keeps the **positional** shape the channels used (`healthbar:saveServices` took
 * `(workspaceId, services)`, `entity:load` took `(workspaceId, kind, id)`) and maps into each command's
 * named payload. The one entry that is not a plain rename is `entity:load`: the channel's `kind` was a
 * **protocol** name and the command translates it (`toEngineKind`), so the field is passed straight
 * through and the translation happens inside the engine — see `coreCommands.ts:98`.
 */
const DELETED_CHANNELS: Record<string, (args: any[]) => { command: string; payload: any }> = {
  "config:get": () => ({ command: "config.get", payload: {} }),
  // `request:replay` is the one entry here whose channel took a **positional tuple of four** rather than
  // a single object — `(method, url, headers, body)` — and the command takes one named payload. A first
  // draft wrote `([params]) => ({ command: "request.replay", payload: params })`, which passed the method
  // string through as the whole payload and failed as `Invalid payload for command "request.replay":
  // expected object, received string`. The error arrives as a thrown `EngineError` from
  // `registry.invoke`'s `safeParse`, i.e. it surfaces in the middle of the *test*, not at the call.
  "request:replay": ([method, url, headers, body]) =>
    ({ command: "request.replay", payload: { method, url, headers, body } }),
  // `config:save` **does** need an entry, and the reason is worth keeping: §5c moved the channel's body
  // into `configCommands.ts`, so the channel name has been dead since P2 — but the call sites' *shape*
  // is still the channel's, i.e. one plain object (`invoke("config:save", config)`), and the command
  // takes `{ config }` rather than the config itself (`ConfigSaveParams`). So this is the case where a
  // name that predates the reduction still needs a translation line added by it.
  "config:save": ([config]) => ({ command: "config.save", payload: { config } }),
  "entity:load": ([workspaceId, kind, id]) => ({ command: "entity.load", payload: { workspaceId, kind, id } }),
  "healthbar:checkUrl": ([url]) => ({ command: "healthbar.checkUrl", payload: { url } }),
  "healthbar:getServices": ([workspaceId]) => ({ command: "healthbar.getServices", payload: { workspaceId } }),
  "healthbar:saveServices": ([workspaceId, services]) =>
    ({ command: "healthbar.saveServices", payload: { workspaceId, services } }),
};

function invoke<T = any>(channel: string, ...args: any[]): T {
  const deleted = DELETED_CHANNELS[channel];
  if (deleted) {
    const { command, payload } = deleted(args);
    return commandRegistry.invoke(command as never, payload, { bus }) as T;
  }
  const fn = handlers.get(channel);
  if (!fn) throw new Error(`No handler registered for "${channel}"`);
  return fn({}, ...args) as T;
}

/**
 * Invoke a registered **command** directly — the replacement for `invoke("<channel>", …)` on the nine
 * protocol-client channels, which were deleted in P6 step 3 (`plan/07` §5b-ter).
 *
 * They were **not all the same shape**, and that is why `params` is explicit here rather than forwarded
 * from a rest-args list: five were literal passthroughs (`graphql.introspect`, `graphql.execute`,
 * `soap.execute`, `grpc.execute`, `grpc.reflect`), `soap:fetchWsdl` **wrapped** its bare `url` argument
 * as `{ url }`, and the three argument-less gRPC channels passed `{}` while ignoring whatever they were
 * called with. So each call site below passes the command's real params object — which is precisely what
 * the channel produced — and the default covers the three that dropped their arguments.
 *
 * This is the one thing the deletion could have got wrong silently: a channel whose body is a single
 * `commandRegistry.invoke()` has no behaviour of its own to lose, *except* the argument shape, and that
 * shape is now visible at the call site instead of hidden inside a deleted adapter.
 */
const invokeCommand = <T = any>(command: string, params: unknown = {}) =>
  commandRegistry.invoke(command, params, { bus }) as Promise<T>;

/**
 * The collapsed `entity.*` commands, driven directly — the per-kind channels below are the step-3
 * reduction's target (`plan/07` §5b-ter) and each was only ever a one-line `commandRegistry.invoke()`
 * adapter, so calling the command is what the channel did, minus the channel.
 *
 * **`create` unwraps `{id, entity}`, and that is not optional.** The channel unwrapped it
 * (`entityCrudFactory.ts:72`) and every assertion below reads the created entity's own fields; without
 * the unwrap they would compare against the envelope and pass on `id` alone. `list` unwraps `{entities}`
 * for the same reason. `plan/07` §5b-quater is the same defect one layer up, on the client.
 *
 * Kinds are the **protocol** names, which is what the channel sent after `toProtocolKind()`. Only
 * `rules`/`sockets` are renamed by that map, so the kinds used here pass through unchanged.
 */
const createEntity = async <T = any>(kind: string, entity: any): Promise<T> => {
  const r: any = await commandRegistry.invoke(
    "entity.create",
    // `workspaceId` is passed through as-is, not defaulted: the channel sent `entity.workspaceId`
    // (usually `undefined`), and `entity.create` then falls back to the **active** workspace. Defaulting
    // it here would pin these cases to `TEST_WS` and hide a workspace-resolution regression.
    { kind, entity, workspaceId: entity.workspaceId },
    { bus },
  );
  return ("entity" in r ? r.entity : r) as T;
};
const updateEntity = (kind: string, entity: any) =>
  commandRegistry.invoke("entity.update", { kind, entity }, { bus });
const deleteEntity = (kind: string, id: string) =>
  commandRegistry.invoke("entity.delete", { kind, id }, { bus });
const listEntities = async <T = any>(kind: string): Promise<T[]> => {
  const r: any = await commandRegistry.invoke("entity.list", { kind, workspaceId: TEST_WS }, { bus });
  return r.entities as T[];
};

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

  const { registerCoreKinds } = await import("@/ipc/handlers/coreKinds");
  const { registerGraphqlKinds } = await import("@/ipc/handlers/graphqlKinds");
  const { registerSoapKinds } = await import("@/ipc/handlers/soapKinds");
  const { registerGrpcKinds } = await import("@/ipc/handlers/grpcKinds");
  // P6 §5c: `entity.load` moved out of `coreHandlers`' module scope into the engine — and the
  // reduction's sixth group then deleted that file's channels and renamed it to `coreKinds.ts`
  // (`plan/07` §5b-septendecies). `registerCoreKinds()` is now the shell's `environments` kind and
  // nothing else; the *commands* still need wiring, which is the line below.
  const { commandRegistry } = await import("@bifurc/engine/commands/registry");
  const { registerCoreCommands } = await import("@bifurc/engine/store/coreCommands");
  // P6 §5c slice 4: the nine protocol-client commands left the three handler files' module scope as
  // well. Every channel in those files *delegates* through `commandRegistry.invoke()`, so without
  // these calls the channels below exist with nothing behind them — a `No handler registered` at
  // invocation time, which is why the suite is green-looking either way until it isn't.
  const { registerGraphqlCommands } = await import("@bifurc/engine/protocols/graphqlCommands");
  const { registerSoapCommands } = await import("@bifurc/engine/protocols/soapCommands");
  const { registerGrpcCommands } = await import("@bifurc/engine/protocols/grpcCommands");
  // P6 §5c slice 7: the four collapsed `entity.*` commands left `entityCrudFactory.ts`'s module scope.
  // All three handler groups registered below reach that factory, so without this call their channels
  // exist with nothing behind them.
  const { registerEntityCrudCommands } = await import("@bifurc/engine/store/entityCrudCommands");
  // `request.replay` and the three `healthbar.*` commands are registered by `miscCommands`, and until the
  // reduction's sixth group this suite never had to wire it: `coreHandlers.ts`'s `request:replay` channel
  // *was* the registration, so deleting that channel removed this suite's route to a command it drives
  // eleven times. Without this line the failure is `[INTERNAL] No handler registered for command
  // "request.replay"` — a routed-surface failure, not a transport one, and it surfaces as an envelope
  // error exactly like the `soap.execute` case below. The `healthbar:*` channels belong to the step-3b
  // `*Kinds.ts` renames and were already gone; they are in the fixture table for the same reason.
  //
  // The general rule this is the second instance of today (the spike needed `registerServerCommands()`
  // for the same reason): **a command whose only registration came from a handler file the reduction
  // deleted must be wired explicitly by every suite that drives it.** `src/ipc/handlers.ts` is the
  // authority for the full set — compare against its `register*Commands()` calls, not against the
  // previous version of this list.
  const { registerMiscCommands } = await import("@bifurc/engine/miscCommands");
  // `config.save` is the other command this suite drives by the name of a channel that §5c already
  // deleted — `invoke("config:save", …)` resolves through the `handlers.get()` fallback to a plain
  // command, so it needs its registering module wired rather than a table entry.
  const { registerConfigCommands } = await import("@bifurc/engine/store/configCommands");

  registerCoreKinds();
  registerCoreCommands(commandRegistry);
  registerMiscCommands(commandRegistry);
  registerConfigCommands(commandRegistry);
  registerGraphqlCommands(commandRegistry);
  registerSoapCommands(commandRegistry);
  registerGrpcCommands(commandRegistry);
  registerEntityCrudCommands(commandRegistry);
  registerGraphqlKinds();
  registerSoapKinds();
  registerGrpcKinds();
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

describe("graphql.execute — GraphQL Send", () => {
  it("POSTs a JSON body and returns status, headers, body and durationMs", async () => {
    const res = await invokeCommand("graphql.execute", {
      url: `${gql.origin}/graphql`, headers: {}, query: "query { me }", variables: "", operationName: "",
    });

    expect(res.status).toBe(200);
    expect(res.headers["x-gql"]).toBe("real");
    expect(typeof res.durationMs).toBe("number");
    expect(res.durationMs).toBeGreaterThanOrEqual(0);
    expect(JSON.parse(res.body).data.echo.query).toBe("query { me }");
  });

  it("sets content-type: application/json and a byte-accurate content-length", async () => {
    await invokeCommand("graphql.execute", {
      url: `${gql.origin}/graphql`, headers: {}, query: "query { me }", variables: "", operationName: "",
    });

    const seen = gql.requests.at(-1)!;
    expect(seen.method).toBe("POST");
    expect(seen.headers["content-type"]).toBe("application/json");
    expect(Number(seen.headers["content-length"])).toBe(Buffer.byteLength(seen.body));
  });

  it("parses the variables JSON string into an object and forwards operationName", async () => {
    const res = await invokeCommand("graphql.execute", {
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
    const res = await invokeCommand("graphql.execute", {
      url: `${gql.origin}/graphql`, headers: {}, query: "query { me }", variables: "   ", operationName: "",
    });

    const echoed = JSON.parse(res.body).data.echo;
    expect(echoed.variables).toBeUndefined();
    expect(echoed.operationName).toBeUndefined();
  });

  it("ignores malformed variables JSON instead of failing the request", async () => {
    const res = await invokeCommand("graphql.execute", {
      url: `${gql.origin}/graphql`, headers: {}, query: "query { me }", variables: "{not json", operationName: "",
    });

    expect(res.status).toBe(200);
    expect(JSON.parse(res.body).data.echo.variables).toBeUndefined();
  });

  it("forwards custom headers such as an auth token", async () => {
    await invokeCommand("graphql.execute", {
      url: `${gql.origin}/graphql`, headers: { authorization: "Bearer t" },
      query: "query { me }", variables: "", operationName: "",
    });

    expect(gql.requests.at(-1)!.headers["authorization"]).toBe("Bearer t");
  });
});

describe("graphql.introspect", () => {
  it("POSTs the introspection query and returns the raw response", async () => {
    const res = await invokeCommand("graphql.introspect", { url: `${gql.origin}/graphql`, headers: {} });

    expect(res.ok).toBe(true);
    expect(res.sdl).toContain("__schema");
    const seen = gql.requests.at(-1)!;
    expect(seen.method).toBe("POST");
    expect(JSON.parse(seen.body).query).toContain("IntrospectionQuery");
  });

  it("returns { ok:false, error } instead of throwing when the server is down", async () => {
    const port = await deadPort();
    const res = await invokeCommand("graphql.introspect", { url: `http://127.0.0.1:${port}/graphql`, headers: {} });

    expect(res.ok).toBe(false);
    expect(typeof res.error).toBe("string");
  });
});

// ── SOAP ──────────────────────────────────────────────────────────────────────

describe("soap.execute — SOAP Send", () => {
  const envelope = "<soap:Envelope><soap:Body><GetUser><id>1</id></GetUser></soap:Body></soap:Envelope>";

  it("POSTs the envelope with text/xml and the SOAPAction header", async () => {
    const res = await invokeCommand("soap.execute", {
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
    await invokeCommand("soap.execute", { endpointUrl: `${soap.origin}/ws`, soapAction: "", headers: {}, body: unicode });

    const seen = soap.requests.at(-1)!;
    // Guard the premise: the fixture must actually be multi-byte.
    expect(Buffer.byteLength(unicode)).toBeGreaterThan(unicode.length);
    expect(Number(seen.headers["content-length"])).toBe(Buffer.byteLength(unicode));
    expect(seen.body).toBe(unicode);
  });

  it("omits SOAPAction when blank and merges custom headers", async () => {
    await invokeCommand("soap.execute", {
      endpointUrl: `${soap.origin}/ws`, soapAction: "", headers: { "x-tenant": "acme" }, body: envelope,
    });

    const seen = soap.requests.at(-1)!;
    expect(seen.headers["soapaction"]).toBeUndefined();
    expect(seen.headers["x-tenant"]).toBe("acme");
  });

  it("reports durationMs", async () => {
    const res = await invokeCommand("soap.execute", {
      endpointUrl: `${soap.origin}/ws`, soapAction: "urn:GetUser", headers: {}, body: envelope,
    });

    expect(typeof res.durationMs).toBe("number");
    expect(res.durationMs).toBeGreaterThanOrEqual(0);
  });

  it("rejects when the endpoint is unreachable", async () => {
    const port = await deadPort();
    await expect(
      invokeCommand("soap.execute", { endpointUrl: `http://127.0.0.1:${port}/ws`, soapAction: "", headers: {}, body: envelope }),
    ).rejects.toThrow();
  });
});

describe("soap.fetchWsdl", () => {
  it("GETs the WSDL and returns its content", async () => {
    const res = await invokeCommand("soap.fetchWsdl", { url: `${wsdl.origin}/svc?wsdl` });

    expect(res.ok).toBe(true);
    expect(res.content).toContain("definitions");
    const seen = wsdl.requests.at(-1)!;
    expect(seen.method).toBe("GET");
    expect(seen.url).toBe("/svc?wsdl");
  });

  it("returns { ok:false, error } when the WSDL cannot be fetched", async () => {
    const res = await invokeCommand("soap.fetchWsdl", { url: "http://127.0.0.1:1/svc?wsdl" });

    expect(res.ok).toBe(false);
    expect(typeof res.error).toBe("string");
  });
});

// ── gRPC: pin the current contract ────────────────────────────────────────────
// gRPC is NOT implemented. These tests exist so that shipping it is a deliberate
// change to a test rather than a silent behaviour swap.

describe("grpc — current (unimplemented) contract", () => {
  it("grpc.execute reports the runtime is not configured instead of throwing", async () => {
    const res = await invokeCommand("grpc.execute", {
      serverAddress: "127.0.0.1:50051", serviceName: "S", methodName: "M",
      requestBody: "{}", metadata: {}, protoFileId: null, useReflection: false,
    });

    expect(res.ok).toBe(false);
    expect(res.error).toMatch(/not yet configured/i);
  });

  it("grpc.reflect reports that reflection is unavailable", async () => {
    const res = await invokeCommand("grpc.reflect", { serverAddress: "127.0.0.1:50051" });

    expect(res.ok).toBe(false);
    expect(res.error).toMatch(/not yet configured/i);
  });

  it("grpc.mockServerStatus defaults to stopped on port 9102", async () => {
    expect(await invokeCommand("grpc.mockServerStatus")).toEqual({ running: false, port: 9102 });
  });

  it("grpc.startMockServer is unimplemented while grpc.stopMockServer is a no-op success", async () => {
    expect((await invokeCommand("grpc.startMockServer")).ok).toBe(false);
    expect(await invokeCommand("grpc.stopMockServer")).toEqual({ ok: true });
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
  it("a graphql request round-trips through entity:load", async () => {
    const created = await createEntity("graphqlRequests", {
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

  it("an edit persists the change and a delete removes it", async () => {
    const created = await createEntity("graphqlRequests", {
      name: "Temp", endpointUrl: `${gql.origin}/graphql`, headers: {},
      query: "query { a }", variables: "", operationName: "", workspaceId: TEST_WS,
    });

    await updateEntity("graphqlRequests", { ...created, name: "Renamed", query: "query { b }" });
    const updated = await invoke("entity:load", TEST_WS, "graphqlRequests", created.id);
    expect(updated.entity.name).toBe("Renamed");
    expect(updated.entity.query).toBe("query { b }");

    await deleteEntity("graphqlRequests", created.id);
    expect((await invoke("entity:load", TEST_WS, "graphqlRequests", created.id)).ok).toBe(false);
  });

  it("a graphql mock requires an endpointPattern", async () => {
    await expect(
      createEntity("graphqlMocks", {
        name: "m", endpointPattern: "  ", useRegex: false, operationType: "query", operationName: "",
        responseStatus: 200, responseHeaders: {}, responseBody: "{}", workspaceId: TEST_WS,
      }),
    ).rejects.toThrow(/endpointPattern/);
  });

  it("graphql schemas round-trip through add → list → delete", async () => {
    const schema = await createEntity("graphqlSchemas", { name: "s1", content: "type Query { a: String }", workspaceId: TEST_WS });

    expect((await listEntities("graphqlSchemas")).map((s: any) => s.id)).toContain(schema.id);

    await deleteEntity("graphqlSchemas", schema.id);
    expect((await listEntities("graphqlSchemas")).map((s: any) => s.id)).not.toContain(schema.id);
  });

  it("soap requests round-trip through add → entity:load", async () => {
    const created = await createEntity("soapRequests", {
      name: "GetUser", endpointUrl: `${soap.origin}/ws`, soapAction: "urn:GetUser",
      headers: {}, body: "<x/>", workspaceId: TEST_WS,
    });

    const loaded = await invoke("entity:load", TEST_WS, "soapRequests", created.id);
    expect(loaded.ok).toBe(true);
    expect(loaded.entity.soapAction).toBe("urn:GetUser");
  });

  it("soap WSDLs round-trip through add → list → delete", async () => {
    const doc = await createEntity("wsdls", { name: "svc", content: "<definitions/>", workspaceId: TEST_WS });

    expect((await listEntities("wsdls")).map((w: any) => w.id)).toContain(doc.id);

    await deleteEntity("wsdls", doc.id);
    expect((await listEntities("wsdls")).map((w: any) => w.id)).not.toContain(doc.id);
  });

  it("a grpc request requires serviceName and methodName", async () => {
    const base = {
      name: "r", serverAddress: "127.0.0.1:50051", requestBody: "{}", metadata: {},
      protoFileId: null, useReflection: false, streamingType: "unary", workspaceId: TEST_WS,
    };
    await expect(createEntity("grpcRequests", { ...base, serviceName: "", methodName: "M" })).rejects.toThrow(/serviceName/);
    await expect(createEntity("grpcRequests", { ...base, serviceName: "S", methodName: " " })).rejects.toThrow(/methodName/);
  });

  it("grpc protos round-trip through add → list → delete", async () => {
    const proto = await createEntity("protoFiles", {
      name: "svc.proto", content: 'syntax = "proto3";', workspaceId: TEST_WS,
    });

    expect((await listEntities("protoFiles")).map((p: any) => p.id)).toContain(proto.id);

    await deleteEntity("protoFiles", proto.id);
    expect((await listEntities("protoFiles")).map((p: any) => p.id)).not.toContain(proto.id);
  });
});
