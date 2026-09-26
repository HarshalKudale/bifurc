/**
 * Integration harness for the Bifurc proxy server.
 *
 * WHY THIS EXISTS
 * ---------------
 * `tests/proxy/server.test.ts` mocks `net`, `http`, `https`, `@/store/config` and
 * `@/store/workspaceFs` completely. The consequence is that it can only prove the
 * proxy *attempted* to do something (e.g. "http.request was called") — it can never
 * prove that a mapping actually forwards to the right port, that a proxy rule routes
 * to the right target, or that a mock actually replaces the response.
 *
 * This harness boots the REAL servers against a REAL on-disk workspace and drives them
 * with REAL TCP/HTTP sockets. That is the only way to verify the end-to-end behaviour a
 * user actually depends on:
 *
 *   workspace files on disk  →  loadConfig()  →  enabled sets  →  workspaceCfg()
 *     →  dispatch()  →  mock / rule / mapping / passthrough  →  response bytes
 *
 * ## TWO listeners, not one — `plan/15` work item 3
 *
 * Until 2026-09-23 this harness booted a **single** server and the module answered both jobs on one
 * socket: the reverse half (a mapped `Host` forwarded to its target, plus the home page that listed the
 * mappings and the 404 for an unmapped `*.localhost`) and the forward half (a browser configured to use
 * the port as its proxy). **WI 3 split them, and the split is the port model, not a refactor:**
 *
 *   - the **edge router** (`proxy/edge.ts`) is **host-level** and owns the reverse half. It reads which
 *     workspace is active **per request** through `resolveConfig`.
 *   - the **forward proxy** (`proxy/server.ts`) is **per workspace** and owns absolute-form traffic,
 *     mocks, and rules on the absolute-form path.
 *
 * **They are on different ports, and a test must name the one it means.** `proxyRequest()` and
 * `edgeRequest()` are the two seams; `startProxy()` returns both ports. Asking the forward proxy for a
 * `*.localhost` mapping is no longer a routing question — it is a **400** by design, because that module
 * no longer answers for that host.
 *
 * The connection between the two is composed here the same way `runtime/host.ts` composes it: the edge's
 * `resolveConfig` returns **the same dispatch config the proxy dispatches with** (`dispatchConfig()`),
 * never a second `workspaceCfg()`. That identity is the point — a second composition would be a second
 * answer, and a test asserting through it would prove nothing about the app.
 *
 * The production store exposes `setDataRootOverride` (workspaceFs) and
 * `setSettingsPathOverride` (appSettings) precisely so tests can point it at a
 * throwaway directory, so nothing here is a special test-only code path.
 */

import * as fs from "fs";
import * as os from "os";
import * as path from "path";
import * as net from "net";
import * as http from "http";
import { setDataRootOverride } from "@bifurc/engine/store/workspaceFs";
import { setSettingsPathOverride } from "@bifurc/engine/store/appSettings";
import {
  createProxyServer,
  bindProxyInstance,
  type ProxyServerInstance,
} from "@bifurc/engine/proxy/server";
import { createEdgeRouter, type EdgeRouterInstance } from "@bifurc/engine/proxy/edge";

export const TEST_WS = "ws-test";
export const TEST_WS_NAME = "Integration Workspace";

// ── Ports ─────────────────────────────────────────────────────────────────────

/** Ask the OS for an unused TCP port, then release it. */
export function getFreePort(): Promise<number> {
  return new Promise((resolve, reject) => {
    const srv = net.createServer();
    srv.unref();
    srv.on("error", reject);
    srv.listen(0, "127.0.0.1", () => {
      const addr = srv.address();
      const port = typeof addr === "object" && addr ? addr.port : 0;
      srv.close(() => resolve(port));
    });
  });
}

// ── Real upstream service ─────────────────────────────────────────────────────

export interface UpstreamServer {
  port: number;
  target: string;             // "127.0.0.1:<port>"
  requests: Array<{ method: string; url: string; headers: http.IncomingHttpHeaders; body: string }>;
  close: () => Promise<void>;
}

/**
 * Start a real HTTP server that stands in for the user's local service.
 * Records every request it receives so tests can assert what the proxy forwarded.
 */
export async function startUpstream(
  handler: (req: http.IncomingMessage, res: http.ServerResponse) => void = defaultUpstream,
): Promise<UpstreamServer> {
  const requests: UpstreamServer["requests"] = [];
  const server = http.createServer((req, res) => {
    const chunks: Buffer[] = [];
    req.on("data", (c: Buffer) => chunks.push(c));
    req.on("end", () => {
      requests.push({
        method: req.method ?? "GET",
        url: req.url ?? "/",
        headers: req.headers,
        body: Buffer.concat(chunks).toString("utf-8"),
      });
      handler(req, res);
    });
  });

  const port = await getFreePort();
  await new Promise<void>((resolve) => server.listen(port, "127.0.0.1", resolve));

  return {
    port,
    target: `127.0.0.1:${port}`,
    requests,
    close: () => new Promise<void>((resolve) => server.close(() => resolve())),
  };
}

function defaultUpstream(req: http.IncomingMessage, res: http.ServerResponse): void {
  res.writeHead(200, { "content-type": "application/json", "x-upstream": "real" });
  res.end(JSON.stringify({ upstream: true, path: req.url }));
}

// ── Workspace fixture ─────────────────────────────────────────────────────────

export interface WorkspaceEntities {
  mappings?: any[];
  rules?: any[];
  mocks?: any[];
  environments?: any[];
  activeEnvironmentId?: string | null;
  port?: number;
  /**
   * The **forward proxy's** port — its own setting since `plan/17` §1, when it stopped sharing the edge's.
   *
   * A suite that boots a proxy the way the engine does has to say which port that is, because the engine now
   * binds `settings.forwardProxyPort` **outright** — there is no scan to fall back on, and the default `8080`
   * is a port this machine may well already be using.
   *
   * **Written only when provided**, so every existing suite's `app.json` keeps exactly the shape it had;
   * `loadConfig()` supplies the default for a file without the key.
   */
  forwardProxyPort?: number;
  tlsEnabled?: boolean;
  /** Real CA paths to load when `tlsEnabled` is true (see `generateCA()` in certManager). */
  tlsCaCertPath?: string | null;
  tlsCaKeyPath?: string | null;
  /** IDs to explicitly disable even if `enabled: true` is set on the entity. */
  disabledMappings?: string[];
  disabledRules?: string[];
  disabledMocks?: string[];
}

export interface WorkspaceFixture {
  root: string;
  dataRoot: string;
  cleanup: () => void;
}

/**
 * Write a complete, valid workspace tree to a temp dir and point the store at it.
 * `app.json` lives next to `data/`, exactly as the real app expects.
 */
export function createWorkspace(entities: WorkspaceEntities = {}): WorkspaceFixture {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "bifurc-it-"));
  const dataRoot = path.join(root, "data");
  const wsDir = path.join(dataRoot, TEST_WS);

  const dirs = [
    wsDir,
    path.join(wsDir, "mappings"),
    path.join(wsDir, "rules"),
    path.join(wsDir, "mocks"),
    path.join(wsDir, "environments"),
    path.join(wsDir, "requests"),
    path.join(wsDir, "sockets"),
    path.join(wsDir, "webhooks"),
    path.join(wsDir, "capture"),
  ];
  for (const d of dirs) fs.mkdirSync(d, { recursive: true });

  // app.json (settings)
  fs.writeFileSync(
    path.join(root, "app.json"),
    JSON.stringify(
      {
        port: entities.port ?? 80,
        // Written only when a suite asks for it — see `WorkspaceEntities.forwardProxyPort`. Omitted, the key is
        // absent and `loadConfig()` supplies the default, which is the shape every pre-`plan/17` suite expects.
        ...(entities.forwardProxyPort === undefined ? {} : { forwardProxyPort: entities.forwardProxyPort }),
        webhookPort: 9101,
        minimizeToTray: true,
        tlsEnabled: entities.tlsEnabled ?? false,
        tlsCaCertPath: entities.tlsCaCertPath ?? null,
        tlsCaKeyPath: entities.tlsCaKeyPath ?? null,
        workspaces: [
          { id: TEST_WS, name: TEST_WS_NAME, activeEnvironmentId: entities.activeEnvironmentId ?? null },
        ],
        activeWorkspaceId: TEST_WS,
        hasSeenWelcome: true,
        zoomLevel: 0,
        zoomLevelSetByUser: false,
        themeId: null,
      },
      null,
      2,
    ),
    "utf-8",
  );

  fs.writeFileSync(
    path.join(wsDir, "workspace.json"),
    JSON.stringify(
      { id: TEST_WS, name: TEST_WS_NAME, createdAt: 1704067200000, activeEnvironmentId: entities.activeEnvironmentId ?? null },
      null,
      2,
    ),
    "utf-8",
  );

  const disabledMappings = new Set(entities.disabledMappings ?? []);
  const disabledRules = new Set(entities.disabledRules ?? []);
  const disabledMocks = new Set(entities.disabledMocks ?? []);

  const writeEntity = (kind: string, entity: any): void => {
    const { enabled: _e, ...rest } = entity;
    void _e;
    fs.writeFileSync(
      path.join(wsDir, kind, `${entity.id}.json`),
      JSON.stringify(rest, null, 2),
      "utf-8",
    );
  };

  // mappings (flat, enabled tracked in enabled.json)
  const mappings = (entities.mappings ?? []).map((m) => ({ ...m, workspaceId: TEST_WS }));
  for (const m of mappings) writeEntity("mappings", m);
  fs.writeFileSync(
    path.join(wsDir, "mappings", "enabled.json"),
    JSON.stringify(mappings.filter((m) => m.enabled !== false && !disabledMappings.has(m.id)).map((m) => m.id), null, 2),
    "utf-8",
  );

  // rules (full files; the proxy reads full rules, names.json is UI-only)
  const rules = (entities.rules ?? []).map((r) => ({ ...r, workspaceId: TEST_WS }));
  for (const r of rules) writeEntity("rules", r);
  fs.writeFileSync(
    path.join(wsDir, "rules", "enabled.json"),
    JSON.stringify(rules.filter((r) => r.enabled !== false && !disabledRules.has(r.id)).map((r) => r.id), null, 2),
    "utf-8",
  );
  fs.writeFileSync(
    path.join(wsDir, "rules", "names.json"),
    JSON.stringify(Object.fromEntries(rules.map((r) => [r.id, { name: r.name, url: r.pattern }])), null, 2),
    "utf-8",
  );
  fs.writeFileSync(
    path.join(wsDir, "rules", "index.json"),
    JSON.stringify({ folders: [], order: rules.map((r) => r.id) }, null, 2),
    "utf-8",
  );

  // mocks (flat, enabled tracked in enabled.json)
  const mocks = (entities.mocks ?? []).map((m) => ({ ...m, workspaceId: TEST_WS }));
  for (const m of mocks) writeEntity("mocks", m);
  fs.writeFileSync(
    path.join(wsDir, "mocks", "enabled.json"),
    JSON.stringify(mocks.filter((m) => m.enabled !== false && !disabledMocks.has(m.id)).map((m) => m.id), null, 2),
    "utf-8",
  );
  fs.writeFileSync(
    path.join(wsDir, "mocks", "index.json"),
    JSON.stringify({ folders: [], order: mocks.map((m) => m.id) }, null, 2),
    "utf-8",
  );

  // environments — always include the implicit __global__ env
  const environments = [
    { id: "__global__", name: "Global", variables: [], createdAt: 0, workspaceId: TEST_WS },
    ...(entities.environments ?? []).map((e) => ({ ...e, workspaceId: TEST_WS })),
  ];
  for (const e of environments) writeEntity("environments", e);

  // Point the real store at the throwaway dir. These are production hooks.
  setDataRootOverride(dataRoot);
  setSettingsPathOverride(path.join(root, "app.json"));

  return {
    root,
    dataRoot,
    cleanup: () => {
      setDataRootOverride(null);
      setSettingsPathOverride(null);
      try { fs.rmSync(root, { recursive: true, force: true }); } catch { /* best effort */ }
    },
  };
}

// ── Driving the proxy ─────────────────────────────────────────────────────────

export interface ProxyResponse {
  status: number;
  headers: http.IncomingHttpHeaders;
  body: string;
}

/**
 * Send a raw HTTP request **to the forward proxy** (`proxy/server.ts`).
 *
 * Use this for anything the forward half owns: an absolute-form target
 * ("http://api.test/data"), a mock served on the proxy path, a rule applied to it, or a passthrough.
 *
 * **Do not ask for a `*.localhost` mapping or `/` here.** `plan/15` work item 3 moved the reverse half to
 * the edge, and this module answers such a request with a deliberate **400** — see `edgeRequest()`.
 */
export function proxyRequest(
  port: number,
  opts: {
    method?: string;
    target: string;
    host: string;
    headers?: Record<string, string>;
    body?: string;
  },
): Promise<ProxyResponse> {
  return rawRequest(port, opts);
}

/**
 * Send a raw HTTP request **to the edge router** (`proxy/edge.ts`) — the reverse half.
 *
 * Use this for a mapped `Host` (forwarded to its mapping's target), the home page, and the 404 that an
 * unmapped `*.localhost` gets. `host` is the whole routing input: the edge looks the `Host` up in the
 * active workspace's mapping table.
 *
 * A relative `target` ("/api/users") is what a browser sends through the edge; an absolute-form target is
 * also accepted so a case can prove where the halves *do* differ.
 */
export function edgeRequest(
  port: number,
  opts: {
    method?: string;
    target: string;
    host: string;
    headers?: Record<string, string>;
    body?: string;
  },
): Promise<ProxyResponse> {
  return rawRequest(port, opts);
}

/** The one implementation both seams share, so the two cannot drift in how they frame a request. */
function rawRequest(
  port: number,
  opts: {
    method?: string;
    target: string;
    host: string;
    headers?: Record<string, string>;
    body?: string;
  },
): Promise<ProxyResponse> {
  return new Promise((resolve, reject) => {
    const bodyBuf = opts.body ? Buffer.from(opts.body, "utf-8") : null;
    // The proxy reads the body as "everything after the header terminator", so it
    // cannot decode chunked request bodies. Always send an explicit content-length.
    const headers: Record<string, string> = { host: opts.host, ...(opts.headers ?? {}) };
    if (bodyBuf && !Object.keys(headers).some((k) => k.toLowerCase() === "content-length")) {
      headers["content-length"] = String(bodyBuf.length);
    }
    const req = http.request(
      {
        host: "127.0.0.1",
        port,
        method: opts.method ?? "GET",
        path: opts.target,
        headers,
      },
      (res) => {
        const chunks: Buffer[] = [];
        res.on("data", (c: Buffer) => chunks.push(c));
        res.on("end", () =>
          resolve({
            status: res.statusCode ?? 0,
            headers: res.headers,
            body: Buffer.concat(chunks).toString("utf-8"),
          }),
        );
      },
    );
    req.on("error", reject);
    if (bodyBuf) req.write(bodyBuf);
    req.end();
  });
}

// ── Server lifecycle ──────────────────────────────────────────────────────────

/**
 * The two listeners, and both ports.
 *
 * `port` is kept as the **forward** port's name because that is what the majority of call sites mean, and
 * `edgePort` is new. **Both are live**: a case that reaches for the wrong one gets a `400` (or a
 * connection error) rather than a silently wrong answer, which is the property that makes this worth
 * spelling out.
 */
export interface RunningProxy {
  /** The **forward** proxy's port — absolute-form traffic, the mock path, rules on that path. */
  port: number;
  /** The **edge router's** port — mappings, `*.localhost`, the home page, the 404. */
  edgePort: number;
  /** The proxy instance, so a case can reach `dispatchConfig()` — the object the edge routes with. */
  instance: ProxyServerInstance;
  stop: () => void;
}

/**
 * Boot **both** servers: the per-workspace forward proxy on a free port, and the host-level edge on
 * another. Mirrors `runtime/host.ts`'s composition, including the part that matters most — the edge's
 * `resolveConfig` returns the proxy's **own** `dispatchConfig()`, so the two are provably looking at the
 * same workspace config rather than two independently-composed copies of it.
 *
 * Two free ports are requested up front (and re-requested if they collide, which they can under parallel
 * load) because `getFreePort()` is bind-then-close: the port is a reservation, not a guarantee.
 *
 * `opts` carries the edge options that describe a **deployment** rather than a request — `plan/15` work item
 * 5's `guiTarget` and `plan/15` work item 9's `hostDomain`. Both are absent by default, which is the state
 * every suite written before work item 5 was written against, so adding them here moved no existing case.
 */
export async function startProxy(
  opts: { guiTarget?: string; hostDomain?: string } = {},
): Promise<RunningProxy> {
  const port = await getFreePort();
  let edgePort = await getFreePort();
  while (edgePort === port) edgePort = await getFreePort();

  // The instance is created and bound explicitly rather than through the module-level `startServer()`
  // facade, because the facade's instance is reached through `defaultServer()` — private. Binding it here
  // also keeps the module-level `stopServer()` / `isRunning()` / `getPort()` working for callers that
  // still use them, since they act on whatever is bound.
  const instance = createProxyServer({ workspaceId: null });
  bindProxyInstance(instance);
  instance.start(port);

  // `resolveConfig` closes over the instance and is read per request — the same shape as the host's, and
  // the reason the edge needs no invalidation step. It reads the *active* workspace lazily, so a case that
  // switches the active workspace mid-test is answered correctly on the next request.
  const edge: EdgeRouterInstance = createEdgeRouter({
    resolveConfig: () => instance.dispatchConfig(),
    guiTarget: opts.guiTarget,
    hostDomain: opts.hostDomain,
  });
  edge.start(edgePort);

  // Give both listen() calls a tick to bind before the first request.
  await new Promise((r) => setTimeout(r, 50));

  return {
    port,
    edgePort,
    instance,
    stop: () => {
      edge.stop();
      instance.stop();
      bindProxyInstance(null);
    },
  };
}

/** Convenience: build a mock entity with sensible defaults. */
export function mockFixture(overrides: Record<string, any> = {}): any {
  return {
    id: "mock-1",
    name: "Fixture Mock",
    method: "GET",
    urlPattern: "/api/fixture",
    useRegex: false,
    enabled: true,
    capturedHeaders: {},
    capturedBody: "",
    responseStatus: 200,
    responseHeaders: { "content-type": "application/json" },
    responseBody: JSON.stringify({ mocked: true }),
    responseBodyEncoding: "utf8",
    responseDelay: 0,
    streamingMode: "none",
    createdAt: 1,
    ...overrides,
  };
}

/** Convenience: build a mapping entity with sensible defaults. */
export function mappingFixture(overrides: Record<string, any> = {}): any {
  return {
    id: "map-1",
    domain: "app.localhost",
    target: "127.0.0.1:3000",
    enabled: true,
    label: "",
    createdAt: 1,
    ...overrides,
  };
}

/** Convenience: build a proxy rule entity with sensible defaults. */
export function ruleFixture(overrides: Record<string, any> = {}): any {
  return {
    id: "rule-1",
    name: "Fixture Rule",
    pattern: "http://api.test/data",
    useRegex: false,
    targetType: "external",
    targetMappingId: "",
    targetExternal: "",
    requestScript: "",
    responseScript: "",
    enabled: true,
    createdAt: 1,
    folderId: null,
    ...overrides,
  };
}
