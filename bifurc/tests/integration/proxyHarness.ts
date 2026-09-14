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
 * This harness boots the REAL proxy server (`startServer`) against a REAL on-disk
 * workspace and drives it with REAL TCP/HTTP sockets. That is the only way to verify
 * the end-to-end behaviour a user actually depends on:
 *
 *   workspace files on disk  →  loadConfig()  →  enabled sets  →  workspaceCfg()
 *     →  dispatch()  →  mock / rule / mapping / passthrough  →  response bytes
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
import { setDataRootOverride } from "@/store/workspaceFs";
import { setSettingsPathOverride } from "@/store/appSettings";
import { startServer, stopServer } from "@/proxy/server";

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
        webhookPort: 9101,
        companionPort: 9271,
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
 * Send a raw HTTP request to the proxy.
 *
 * - `target` is the request line target. Pass an absolute URL
 *   ("http://api.test/data") to exercise the forward-proxy path, or a path
 *   ("/api/users") to exercise the `*.localhost` mapping path.
 * - `host` sets the Host header, which is what the proxy routes on.
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

export interface RunningProxy {
  port: number;
  stop: () => void;
}

/** Boot the real proxy server on a free port. */
export async function startProxy(): Promise<RunningProxy> {
  const port = await getFreePort();
  startServer(port);
  // Give listen() a tick to bind before the first request.
  await new Promise((r) => setTimeout(r, 50));
  return { port, stop: () => stopServer() };
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
