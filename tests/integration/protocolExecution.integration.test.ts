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
import * as fs from "fs";
import * as http from "http";
import * as path from "path";
import * as zlib from "zlib";
import { createWorkspace, TEST_WS, type WorkspaceFixture } from "./proxyHarness";
import { commandRegistry } from "@bifurc/engine/kernel/registry";
import { bus } from "@bifurc/engine/kernel/eventBus";

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

/**
 * Write a saved entity straight to disk, at the workspace root — what the *engine* reads, with no
 * `entity.create` in between.
 *
 * That is deliberate for the `*Saved` block below, and it is a different choice from `createEntity`
 * above rather than an inconsistency. The subject there is the *command*; the subject here is the
 * **load-then-dispatch** path, and driving it through `entity.create` would route the fixture through
 * the very CRUD machinery under test. Writing the file also keeps this block independent of which
 * kinds the harness happens to have registered: `requests` is a `CrudFactoryOpts` kind, whose
 * registration (`registerCrudKinds()`) is deliberately **not** part of this suite's `beforeAll`, so
 * `createEntity("requests", …)` would fail here with `kind "requests" is not routed through the
 * CommandRegistry yet` — a fact about the harness, not about the command.
 *
 * Root-level is enough: `findEntityFile()` checks `<kind>/<id>.json` before it walks sub-folders, so
 * no `index.json` or `names.json` is needed to be *readable*.
 */
function writeSavedEntity(kind: string, entity: Record<string, unknown> & { id: string }): void {
  const dir = path.join(ws.dataRoot, TEST_WS, kind);
  fs.mkdirSync(dir, { recursive: true });
  fs.writeFileSync(path.join(dir, `${entity.id}.json`), JSON.stringify(entity, null, 2), "utf-8");
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

  const { registerCoreKinds } = await import("@bifurc/engine/workspace/entities/kinds/coreKinds");
  const { registerGraphqlKinds } = await import("@bifurc/engine/graphql/kinds");
  const { registerSoapKinds } = await import("@bifurc/engine/soap/kinds");
  const { registerGrpcKinds } = await import("@bifurc/engine/grpc/kinds");
  // P6 §5c: `entity.load` moved out of `coreHandlers`' module scope into the engine — and the
  // reduction's sixth group then deleted that file's channels and renamed it to `coreKinds.ts`
  // (`plan/07` §5b-septendecies). `registerCoreKinds()` is now the shell's `environments` kind and
  // nothing else; the *commands* still need wiring, which is the line below.
  const { commandRegistry } = await import("@bifurc/engine/kernel/registry");
  const { registerCoreCommands } = await import("@bifurc/engine/workspace/entities/coreCommands");
  // P6 §5c slice 4: the nine protocol-client commands left the three handler files' module scope as
  // well. Every channel in those files *delegates* through `commandRegistry.invoke()`, so without
  // these calls the channels below exist with nothing behind them — a `No handler registered` at
  // invocation time, which is why the suite is green-looking either way until it isn't.
  const { registerGraphqlCommands } = await import("@bifurc/engine/graphql/commands");
  const { registerSoapCommands } = await import("@bifurc/engine/soap/commands");
  const { registerGrpcCommands } = await import("@bifurc/engine/grpc/commands");
  // P6 §5c slice 7: the four collapsed `entity.*` commands left `entityCrudFactory.ts`'s module scope.
  // All three handler groups registered below reach that factory, so without this call their channels
  // exist with nothing behind them.
  const { registerEntityCrudCommands } = await import("@bifurc/engine/workspace/entities/entityCrudCommands");
  // `request.replay` and the three `healthbar.*` commands were registered by `miscCommands` (dissolved
  // in engine-layout phase 7 — now `rest/` and `workspace/` respectively), and until the
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
  // **Engine-layout phase 7 dissolved `miscCommands.ts`** (`Engine_Module_Plan.md` §4.5), so the one
  // import became three — the commands did not change, only the module that registers them. The
  // `request.replay` half is `rest/`; the three `healthbar.*` are `workspace/`; `script.execute` is
  // `mocks/` (not driven by this suite).
  const { registerRestCommands } = await import("@bifurc/engine/rest/commands");
  const { registerHealthbarCommands } = await import("@bifurc/engine/services/healthbarCommands");
  // `config.save` is the other command this suite drives by the name of a channel that §5c already
  // deleted — `invoke("config:save", …)` resolves through the `handlers.get()` fallback to a plain
  // command, so it needs its registering module wired rather than a table entry.
  const { registerConfigCommands } = await import("@bifurc/engine/workspace/entities/configCommands");
  // `request.send` — `plan/19` D24's REST pipeline, and the only one of the four `*.send` commands that runs
  // scripts. `graphql.send`, `soap.send` and `grpc.send` register from `registerGraphqlCommands` /
  // `registerSoapCommands` / `registerGrpcCommands` below, because each resolves its own fields before
  // dispatching. This is the line `tests/engine/layout.test.ts`'s "reachable from registerAll.ts" case keeps
  // in step with `registerAll.ts`.
  const { registerPipelineCommands } = await import("@bifurc/engine/rest/sendCommands");

  registerCoreKinds();
  registerCoreCommands(commandRegistry);
  registerRestCommands(commandRegistry);
  registerHealthbarCommands(commandRegistry);
  registerConfigCommands(commandRegistry);
  registerGraphqlCommands(commandRegistry);
  registerSoapCommands(commandRegistry);
  registerGrpcCommands(commandRegistry);
  registerEntityCrudCommands(commandRegistry);
  // After the commands it dispatches to. Not *required* to be — `invoke()` resolves the handler at invoke
  // time, not registration time — but it keeps this list readable in the same order as `registerAll.ts`,
  // which is the list a reader is told to compare against.
  registerPipelineCommands(commandRegistry);
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

  /**
   * `grpc.mockServerStatus`, `grpc.startMockServer` and `grpc.stopMockServer` were pinned here until
   * 2026-09-25. `plan/17` work item 8 **deleted** all three rather than leaving them `wontfix`, so
   * there is nothing left to pin: `grpc.execute` and `grpc.reflect` above are now the whole of the
   * gRPC command surface, and the mock server's port default (`9102`) went with them.
   *
   * Deliberately not replaced by an "is now unregistered" assertion — that would pin
   * `UNKNOWN_COMMAND`, which the registry covers generically, and it would re-introduce the three
   * command *names* this work item exists to remove.
   */
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

  /**
   * "a graphql mock requires an endpointPattern" stood here until 2026-09-25, when `plan/17` work
   * item 8 removed the `graphqlMocks` kind.
   *
   * It was **not** retargeted to the surviving REST mock, and the reason is worth keeping: this
   * suite's `beforeAll` registers only `registerCoreKinds()` plus the three *protocol* kind files
   * (`graphqlKinds` / `soapKinds` / `grpcKinds`) — `registerCrudKinds()` is deliberately not among
   * them, so `mocks` has never been routable here. The old case passed only because
   * `registerGraphqlKinds()` registered `graphqlMocks`; now that those three files register no mock
   * kind at all, an `entity.create({kind:"mocks"})` here fails with
   * `kind "mocks" is not routed through the CommandRegistry yet` — which is a fact about this
   * harness, not about mocks.
   *
   * The coverage therefore moved to `tests/ipc/handlers.test.ts`'s `entity.create — mocks` block,
   * which registers through the shell and does reach `crudKinds.ts`'s `validate`.
   */

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

// ── The saved-entity family ───────────────────────────────────────────────────

/**
 * `request.send` and its three thin siblings — the **headless shape** of the request-executing commands.
 * The caller names an entity by id, the engine loads it, and the entity is run.
 *
 * **`request.send` is no longer a thin counterpart** (2026-09-30, `plan/19` D24/D27): it runs the REST
 * pipeline, so it resolves the request and executes its scripts on the way through, and it also accepts an
 * **inline** request so that a draft tab and a CLI use one command. The three siblings still
 * load-and-dispatch; `packages/protocol/src/commands/saved.ts` owns the asymmetry table and
 * `plan/protocol-changes.md`'s 2026-09-30 entry owns the wire argument.
 *
 * What these cases are for is the **load** half plus the entity→params mapping. The dispatch half is the
 * blocks above, and asserting it again here would prove nothing new — so each case asserts the request that
 * reached the *real* server, which is where a mapping mistake actually shows up.
 *
 * Every entity is written to disk rather than created through `entity.create`; `writeSavedEntity()`'s
 * docblock records why that is a deliberate difference from the block above.
 */
describe("the saved-entity commands — execute a saved entity by id", () => {
  it("request.send replays the stored method, url, headers and body", async () => {
    const id = "req-saved-1";
    const body = JSON.stringify({ from: "disk" });
    writeSavedEntity("requests", {
      id, name: "Saved POST", method: "POST", url: `${rest.origin}/api/saved`,
      headers: { "content-type": "application/json", "x-saved": "1" }, body,
    });

    const res = await invokeCommand<any>("request.send", { workspaceId: TEST_WS, id });

    expect(res.status).toBe(200);
    const seen = rest.requests.at(-1)!;
    expect(seen.method).toBe("POST");
    expect(seen.url).toBe("/api/saved");
    expect(seen.headers["x-saved"]).toBe("1");

    // **The trap this case exists for.** A saved `requests` entity stores `body` as plain TEXT —
    // `stateToSavePayload` writes `state.reqBody` verbatim, and all five importers do the same — while
    // `request.replay` takes base64. A counterpart that passed the stored field straight through would
    // have the upstream receive the base64 *of nothing*, i.e. garbage bytes, and every assertion above
    // would still pass.
    expect(seen.body).toBe(body);
    expect(seen.headers["content-length"]).toBe(String(Buffer.byteLength(body)));
    // And the response is base64, exactly as the unsaved command returns it.
    expect(JSON.parse(unb64(res.body)).body).toBe(body);
  });

  it("encodes the stored body as UTF-8, so a multi-byte body survives the round trip", async () => {
    const id = "req-saved-utf8";
    const body = '{"greeting":"✓ Ünïcode — ☕"}';
    writeSavedEntity("requests", {
      id, name: "Unicode", method: "POST", url: `${rest.origin}/api/utf8`, headers: {}, body,
    });

    await invokeCommand("request.send", { workspaceId: TEST_WS, id });

    const seen = rest.requests.at(-1)!;
    // Guard the premise: unless the fixture is genuinely multi-byte this case passes for a latin-1
    // encoder too, which is the failure it is here to catch.
    expect(Buffer.byteLength(body)).toBeGreaterThan(body.length);
    expect(seen.body).toBe(body);
    expect(Number(seen.headers["content-length"])).toBe(Buffer.byteLength(body));
  });

  it("sends no body for a whitespace-only stored body — the editor's blank-means-empty rule", async () => {
    // `textToB64` returns `""` for a blank string, so the same request sent from the editor carries no
    // body at all. A counterpart that encoded the stored field literally would send three spaces, which
    // is a different request to some upstreams and an invisible one to most.
    const id = "req-saved-blank";
    const url = `${rest.origin}/api/blank`;
    writeSavedEntity("requests", {
      id, name: "Blank", method: "POST", url, headers: {}, body: "   ",
    });

    await invokeCommand("request.send", { workspaceId: TEST_WS, id });
    const viaSaved = rest.requests.at(-1)!;

    // Compared against the unsaved path rather than against a literal, because that is the claim: the
    // two must be the *same* request. (`content-length` is asserted by comparison and not against
    // `undefined` on purpose — Node adds `content-length: 0` to a bodyless POST, so pinning the header
    // itself would be asserting Node's framing rather than the counterpart's encoding.)
    await invoke("request:replay", "POST", url, {}, "");
    const viaUnsaved = rest.requests.at(-1)!;

    expect(viaSaved.body).toBe("");
    expect(viaSaved.body).toBe(viaUnsaved.body);
    expect(viaSaved.headers["content-length"]).toBe(viaUnsaved.headers["content-length"]);
  });

  it("resolves the stored values against the workspace's environment — the pipeline, not a raw replay", async () => {
    /**
     * **The behaviour change this command exists for, and the case this block's predecessor asserted the
     * opposite of.** Until 2026-09-30 there was a case here pinning *"forwards the stored values literally
     * — no `{{variable}}` resolution"*, because resolution was the renderer's half and a counterpart that
     * grew it would have been a different command from the one it delegated to.
     *
     * `request.send` **is** that different command, deliberately: it runs `plan/19` D24's pipeline, so
     * resolution is the engine's and the stored token is *meant* to disappear. This is also §10's
     * acceptance criterion stated end-to-end — a saved request whose fields contain a token resolves,
     * with the renderer holding no resolver at all.
     */
    writeSavedEntity("environments", {
      id: "__global__",
      name: "Global",
      createdAt: 0,
      workspaceId: TEST_WS,
      variables: [{ id: "apiToken", key: "apiToken", value: "resolved-token" }],
    });

    const id = "req-send-resolved";
    writeSavedEntity("requests", {
      id, name: "Resolved", method: "GET", url: `${rest.origin}/api/{{apiToken}}`,
      headers: { "x-token": "{{apiToken}}" }, body: "",
    });

    await invokeCommand("request.send", { workspaceId: TEST_WS, id });

    const seen = rest.requests.at(-1)!;
    expect(seen.url).toBe("/api/resolved-token");
    expect(seen.headers["x-token"]).toBe("resolved-token");
  });

  it("produces the same upstream request as request.replay given the same values", async () => {
    // The property the counterpart is *defined* by — `entity.load` composed with the command — so the
    // two must agree on what actually leaves the machine.
    const id = "req-saved-agree";
    const headers = { "content-type": "application/json" };
    const body = JSON.stringify({ agree: true });
    writeSavedEntity("requests", {
      id, name: "Agree", method: "PUT", url: `${rest.origin}/api/agree`, headers, body,
    });

    await invokeCommand("request.send", { workspaceId: TEST_WS, id });
    const viaSaved = rest.requests.at(-1)!;

    await invoke("request:replay", "PUT", `${rest.origin}/api/agree`, headers, b64(body));
    const viaUnsaved = rest.requests.at(-1)!;

    expect(viaSaved.method).toBe(viaUnsaved.method);
    expect(viaSaved.url).toBe(viaUnsaved.url);
    expect(viaSaved.body).toBe(viaUnsaved.body);
    expect(viaSaved.headers["content-length"]).toBe(viaUnsaved.headers["content-length"]);
  });

  it("graphql.executeSaved executes the stored endpoint, query, variables and operationName", async () => {
    const id = "gql-saved-1";
    const query = "query ($id: ID!) { user(id: $id) { name } }";
    writeSavedEntity("graphqlRequests", {
      // Note the stored field is `endpointUrl` and the command's is `url`. That the request reaches the
      // server at all is the proof the rename happened — there is no other source for the URL.
      id, name: "Saved GQL", endpointUrl: `${gql.origin}/graphql`, headers: { "x-saved": "gql" },
      query, variables: '{"id":"42"}', operationName: "GetUser",
    });

    const res = await invokeCommand<any>("graphql.send", { workspaceId: TEST_WS, id });

    expect(res.status).toBe(200);
    expect(res.headers["x-gql"]).toBe("real");
    const echoed = JSON.parse(res.body).data.echo;
    expect(echoed.query).toBe(query);
    // `variables` is stored as a JSON **string** and parsed by the command, so an object arriving here
    // is proof it travelled as the string rather than being pre-parsed on the way through.
    expect(echoed.variables).toEqual({ id: "42" });
    expect(echoed.operationName).toBe("GetUser");
    expect(gql.requests.at(-1)!.headers["x-saved"]).toBe("gql");
  });

  it("graphql.send resolves EVERY field — including the query and the operation name", async () => {
    /**
     * **The behaviour change, and why this migration is a fix rather than a move.**
     *
     * The tab resolved `endpointUrl`, `headers` and `variables` but **not** `query` or `operationName`, while
     * the Send gate's token scan treated all five as resolvable — so a `{{token}}` in a GraphQL query never
     * resolved in *either* path (the gate only ever chose between two ways of sending the same unresolved
     * field). The engine follows the gate, because that is the only form under which `usesVariables` can
     * leave the gate at all. `plan/protocol-changes.md`'s 2026-09-30 `graphql.send` entry owns the argument.
     */
    writeSavedEntity("environments", {
      id: "__global__",
      name: "Global",
      createdAt: 0,
      workspaceId: TEST_WS,
      variables: [{ id: "apiToken", key: "apiToken", value: "resolved-token" }],
    });

    const id = "gql-send-resolved";
    writeSavedEntity("graphqlRequests", {
      id, name: "Resolved GQL", endpointUrl: `${gql.origin}/graphql/{{apiToken}}`,
      headers: { "x-token": "{{apiToken}}" },
      query: 'query { user(token: "{{apiToken}}") { name } }',
      variables: '{"token":"{{apiToken}}"}',
      operationName: "Get{{apiToken}}",
    });

    await invokeCommand("graphql.send", { workspaceId: TEST_WS, id });

    const seen = gql.requests.at(-1)!;
    expect(seen.url).toContain("resolved-token");
    expect(seen.headers["x-token"]).toBe("resolved-token");
    // The body the *server received* is the payload — `{query, variables, operationName}` — which is more
    // direct than going through the response's echo, and it is what the upstream actually saw.
    const sent = JSON.parse(seen.body);
    // The two the tab never resolved — the whole point of this case.
    expect(sent.query).toBe('query { user(token: "resolved-token") { name } }');
    expect(sent.operationName).toBe("Getresolved-token");
    expect(sent.variables).toEqual({ token: "resolved-token" });
  });

  it("soap.executeSaved POSTs the stored envelope and SOAPAction", async () => {
    const id = "soap-saved-1";
    const envelope = "<soap:Envelope><soap:Body><GetUser><id>7</id></GetUser></soap:Body></soap:Envelope>";
    writeSavedEntity("soapRequests", {
      id, name: "Saved SOAP", endpointUrl: `${soap.origin}/ws`, soapAction: "urn:GetUser",
      headers: {}, body: envelope,
    });

    const res = await invokeCommand<any>("soap.send", { workspaceId: TEST_WS, id });

    expect(res.status).toBe(200);
    expect(res.headers["x-soap"]).toBe("real");
    const seen = soap.requests.at(-1)!;
    expect(seen.method).toBe("POST");
    expect(seen.headers["soapaction"]).toBe("urn:GetUser");
    expect(seen.body).toBe(envelope);
  });

  it("soap.send resolves EVERY field — including the SOAPAction", async () => {
    /**
     * The same class of fix as `graphql.send`'s, one field wide: the tab resolved `endpointUrl`, `headers`
     * and `body` but **not `soapAction`**, while the Send gate's token scan treated all four as resolvable —
     * so a `{{token}}` in a SOAPAction never resolved in *either* path. See
     * `plan/protocol-changes.md`'s 2026-09-30 `soap.send` entry.
     */
    writeSavedEntity("environments", {
      id: "__global__",
      name: "Global",
      createdAt: 0,
      workspaceId: TEST_WS,
      variables: [{ id: "apiToken", key: "apiToken", value: "resolved-token" }],
    });

    const id = "soap-send-resolved";
    writeSavedEntity("soapRequests", {
      id, name: "Resolved SOAP", endpointUrl: `${soap.origin}/ws/{{apiToken}}`,
      soapAction: "urn:{{apiToken}}", headers: { "x-token": "{{apiToken}}" },
      body: "<ping>{{apiToken}}</ping>",
    });

    await invokeCommand("soap.send", { workspaceId: TEST_WS, id });

    const seen = soap.requests.at(-1)!;
    expect(seen.url).toContain("resolved-token");
    // The one the tab never resolved — the whole point of this case.
    expect(seen.headers["soapaction"]).toBe("urn:resolved-token");
    expect(seen.headers["x-token"]).toBe("resolved-token");
    expect(seen.body).toBe("<ping>resolved-token</ping>");
  });

  it("grpc.executeSaved delegates to grpc.execute, which is still a stub", async () => {
    // The result asserted is the *stub's*, and that is the point: the counterpart's job is to load and
    // dispatch, so it starts working the day the stub is implemented and needs no change then. The load
    // half is not proven here — a stub ignores every parameter — and the `NOT_FOUND` cases below are
    // what prove it, because they throw before the stub is reached.
    const id = "grpc-saved-1";
    writeSavedEntity("grpcRequests", {
      id, name: "Saved gRPC", serverAddress: "127.0.0.1:50051", serviceName: "S", methodName: "M",
      requestBody: "{}", metadata: {}, protoFileId: null, useReflection: false, streamingType: "unary",
    });

    const res = await invokeCommand<any>("grpc.send", { workspaceId: TEST_WS, id });

    expect(res.ok).toBe(false);
    expect(res.error).toMatch(/not yet configured/i);
  });

  /**
   * The load half, asserted directly. It matters most for `grpc.executeSaved`, whose target answers the
   * same thing whatever it is handed — without this block, a counterpart that never read the disk at all
   * would pass every case above.
   */
  it.each([
    "request.send",
    "graphql.send",
    "soap.send",
    "grpc.send",
  ])("%s rejects with NOT_FOUND for an id that is not on disk", async (command) => {
    await expect(
      invokeCommand(command, { workspaceId: TEST_WS, id: "no-such-entity" }),
    ).rejects.toMatchObject({ code: "NOT_FOUND" });
  });

  it("does not fall back to the active workspace when a different workspace is named", async () => {
    // The entity exists — in `TEST_WS`, which *is* the active workspace here. A counterpart that
    // ignored `workspaceId` and loaded from whatever is active would therefore succeed, and the silent
    // default would be invisible. Naming another workspace is what makes the field load-bearing.
    const id = "req-saved-other-ws";
    writeSavedEntity("requests", {
      id, name: "Here", method: "GET", url: `${rest.origin}/api/here`, headers: {}, body: "",
    });

    await expect(
      invokeCommand("request.send", { workspaceId: "ws-does-not-exist", id }),
    ).rejects.toMatchObject({ code: "NOT_FOUND" });
  });
});
