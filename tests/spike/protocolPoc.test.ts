/**
 * Spike 4 — protocol proof-of-concept, RUNTIME smoke test.
 *
 * Complements `spike/protocol/shape.ts` (the compile-time proof) with the thing a
 * type-check cannot show: that the generic envelope, dispatched through the
 * REAL registered handlers (same registration path `registerIpcHandlers()`
 * uses, same mocking pattern as `tests/ipc/handlers.test.ts`), reproduces the
 * legacy `ipcRenderer.invoke` contract for all 10 representative commands —
 * including the error and network-failure paths.
 *
 * SPIKE CODE — throwaway, do not productionise. Findings recorded in
 * `plan/spike-results.md`.
 */
import { describe, it, expect, beforeAll, beforeEach, vi } from "vitest";
import { EventEmitter } from "events";

// ── Same capturing-ipcMain pattern as tests/ipc/handlers.test.ts ─────────────

const { mockIpcMain, registeredHandlers } = vi.hoisted(() => {
  const handlers = new Map<string, (...args: any[]) => any>();
  const ipcMain = {
    handle: (channel: string, handler: (...args: any[]) => any) => {
      handlers.set(channel, handler);
    },
    on: () => { },
  };
  return { mockIpcMain: ipcMain, registeredHandlers: handlers };
});

vi.mock("fs");

vi.mock("@bifurc/engine/subscription/entityCount", () => ({
  gateCreate: vi.fn(() => ({ allowed: true })),
  gateEnable: vi.fn(() => ({ allowed: true })),
}));

// NOTE: a `vi.mock("@bifurc/engine/subscription/gate")` block used to sit here. It mocked a module that
// does not exist anywhere in this repository (the real module is `@/subscription/entityCount`,
// mocked above, and its API is `gateCreate`/`gateEnable` — not `canCreate`/`canEnable`). It was
// left over from a pre-history refactor and removed during P2 work item 8.

vi.mock("../../src/proxy/server", () => ({
  startServer: vi.fn(),
  stopServer: vi.fn(),
  isRunning: vi.fn(() => true),
  getPort: vi.fn(() => 80),
  getServerError: vi.fn(() => null),
  reloadConfig: vi.fn(),
  replayRequest: vi.fn(),
}));

vi.mock("../../src/proxy/logEmitter", () => ({
  logEmitter: new EventEmitter(),
}));

vi.mock("../../src/proxy/service-discovery", () => ({
  discoverServices: vi.fn(() => []),
}));

vi.mock("@bifurc/engine/store/gitStore", () => ({
  commitMutation: vi.fn(() => Promise.resolve("abc123")),
  queryLog: vi.fn(() => Promise.resolve({ entries: [], total: 0 })),
  getEntityAtCommit: vi.fn(() => Promise.resolve(null)),
  getCommitChangedFiles: vi.fn(() => Promise.resolve([])),
  initWorkspaceRepo: vi.fn(() => Promise.resolve()),
  AuditEntity: {},
  AuditAction: {},
}));

vi.mock("@bifurc/engine/store/workspaceFs", () => ({
  writeEntity: vi.fn(),
  deleteEntityFile: vi.fn(),
  writeFlatEntity: vi.fn(),
  deleteFlatEntityFile: vi.fn(),
  entityRelPath: vi.fn((kind: string, id: string, folderName?: string | null) =>
    folderName ? `${kind}/${folderName}/${id}.json` : `${kind}/${id}.json`),
  flatEntityRelPath: vi.fn((kind: string, id: string) => `${kind}/${id}.json`),
  findEntityRelPath: vi.fn(() => null),
  deleteEntityDir: vi.fn(),
  readIndex: vi.fn(() => ({ folders: [], order: [] })),
  writeIndex: vi.fn(),
  readAllEntities: vi.fn(() => []),
  readEnabledSet: vi.fn(() => new Set<string>()),
  writeEnabledSet: vi.fn(),
  bootstrapEnabledSet: vi.fn(() => new Set<string>()),
  upsertNameEntry: vi.fn(),
  removeNameEntry: vi.fn(),
  initWorkspaceDir: vi.fn(),
  sanitizeDirName: vi.fn((name: string) => name),
  wsDir: vi.fn((wsId: string) => `/tmp/test-user-data/data/${wsId}`),
  dataRoot: vi.fn(() => "/tmp/test-user-data/data"),
  getPendingDeletions: vi.fn(() => []),
  addPendingDeletion: vi.fn(),
  removePendingDeletion: vi.fn(),
  clearPendingDeletions: vi.fn(),
}));

vi.mock("@bifurc/engine/store/appSettings", () => ({
  loadSettings: vi.fn(() => ({
    port: 80,
    minimizeToTray: true,
    workspaces: [{ id: "default", name: "Workspace 1", activeEnvironmentId: null }],
    activeWorkspaceId: "default",
  })),
  saveSettings: vi.fn(),
  // tls:generate needs a real-shaped data dir; fs is automocked so writes are no-ops.
  appDataDir: vi.fn(() => "/tmp/test-user-data"),
}));

vi.mock("../../src/main", () => ({
  updateTrayMenu: vi.fn(),
}));

vi.mock("../../src/ipc/importExport/registry", () => ({
  getAllFormats: vi.fn(() => ({})),
  getFormats: vi.fn(() => []),
  getExporter: vi.fn(() => null),
  getImporter: vi.fn(() => null),
  registerFormat: vi.fn(),
}));

vi.mock("electron", () => ({
  app: {
    getPath: vi.fn(() => "/tmp/test-user-data"),
    commandLine: { appendSwitch: vi.fn() },
    whenReady: vi.fn(() => Promise.resolve()),
    on: vi.fn(),
    quit: vi.fn(),
  },
  BrowserWindow: { getAllWindows: vi.fn(() => []) },
  ipcMain: mockIpcMain,
  dialog: { showSaveDialog: vi.fn(), showOpenDialog: vi.fn() },
  Menu: { buildFromTemplate: vi.fn(() => ({})) },
  nativeImage: { createFromPath: vi.fn(() => ({})) },
  Tray: vi.fn(() => ({ setToolTip: vi.fn(), setContextMenu: vi.fn(), on: vi.fn() })),
  shell: { openExternal: vi.fn() },
}));

import type { AppConfig } from "@bifurc/engine/store/config";

const makeDefaultConfig = (): AppConfig =>
  ({
    port: 80,
    minimizeToTray: true,
    workspaces: [{ id: "default", name: "Workspace 1", createdAt: 0, activeEnvironmentId: null }],
    activeWorkspaceId: "default",
    mappings: [],
    proxyRules: [],
    mocks: [],
    requests: [],
    mockFolders: [],
    requestFolders: [],
    wsConnections: [],
    wsFolders: [],
    environments: [],
    activeEnvironmentId: null,
  }) as unknown as AppConfig;

let currentConfig: AppConfig = makeDefaultConfig();

vi.mock("@bifurc/engine/store/config", () => ({
  loadConfig: vi.fn(() => currentConfig),
  saveConfig: vi.fn((cfg: AppConfig) => { currentConfig = cfg; }),
  generateId: vi.fn(() => `id-${Math.random().toString(36).slice(2)}`),
  loadEntity: vi.fn(() => null),
}));

import { createInProcessTransport } from "../../spike/protocol/legacyTransport";
import { createProtocolClient, type ProtocolClient } from "../../spike/protocol/client";

let client: ProtocolClient;

describe("Spike 4 — protocol PoC runtime smoke test", () => {
  beforeAll(async () => {
    const { registerCoreHandlers } = await import("../../src/ipc/handlers/coreHandlers");
    const { registerCrudHandlers } = await import("../../src/ipc/handlers/crudHandlers");
    const { registerFolderHandlers } = await import("../../src/ipc/handlers/folderHandlers");
    const { registerGraphqlHandlers } = await import("../../src/ipc/handlers/graphqlHandlers");
    const { registerSoapHandlers } = await import("../../src/ipc/handlers/soapHandlers");
    const { registerTlsHandlers } = await import("../../src/ipc/handlers/tlsHandlers");

    registerCoreHandlers();
    registerCrudHandlers();
    registerFolderHandlers();
    registerGraphqlHandlers();
    registerSoapHandlers();
    registerTlsHandlers();

    const transport = createInProcessTransport((channel) => registeredHandlers.get(channel));
    client = createProtocolClient(transport);
  });

  beforeEach(() => {
    currentConfig = makeDefaultConfig();
  });

  // ── read ──────────────────────────────────────────────────────────────

  it("config.get — returns the current config", async () => {
    currentConfig.port = 9191;
    const cfg = await client.getConfig();
    expect(cfg.port).toBe(9191);
  });

  it("server.status — returns running/port/error", async () => {
    const status = await client.serverStatus();
    expect(status).toEqual({ running: true, port: 80, error: null });
  });

  // ── mutate ────────────────────────────────────────────────────────────

  it("mock.add — creates a mock and returns it with a generated id", async () => {
    const mock = await client.addMock({
      name: "Spike mock",
      method: "GET",
      urlPattern: "http://api.test/spike",
      useRegex: false,
      enabled: true,
      capturedHeaders: {},
      capturedBody: "",
      responseStatus: 200,
      responseHeaders: {},
      responseBody: "{}",
      createdAt: 0,
    } as any);
    expect(mock.name).toBe("Spike mock");
    expect(mock.id).toBeTruthy();
    expect(currentConfig.mocks).toHaveLength(1);
  });

  it("mock.delete — removes the mock", async () => {
    currentConfig.mocks = [{ id: "m1", name: "x" } as any];
    const result = await client.deleteMock("m1");
    expect(result).toEqual({ ok: true });
    expect(currentConfig.mocks).toHaveLength(0);
  });

  it("folder.add — creates a folder", async () => {
    const folder = await client.addFolder("mock", { name: "Spike folder", parentId: null } as any);
    expect(folder.name).toBe("Spike folder");
  });

  it("folder.move — moves a folder to a new parent", async () => {
    currentConfig.mockFolders = [{ id: "f1", name: "F1", parentId: null, createdAt: 0 } as any];
    const result = await client.moveFolder("mock", "f1", null);
    expect(result).toEqual({ ok: true });
  });

  it("entity.setEnabled — toggles the enabled flag on a mock", async () => {
    currentConfig.mocks = [{ id: "m1", name: "x", enabled: true } as any];
    const result = await client.setEntityEnabled("default", "mocks", "m1", false);
    expect(result.ok).toBe(true);
  });

  // ── network-calling / error-producing (dead endpoint) ───────────────────
  //
  // NOTE: this sandbox's network stack answers 127.0.0.1:1 with a synthetic
  // 404 instead of refusing the connection (documented in `plan/baseline.md`
  // "Environment caveats" — verified against a raw http.request from this
  // shell). On real CI/desktop this is a connection-refused error instead.
  // Either way the point under test holds: the envelope never throws, and
  // whatever the legacy handler resolves rides through in `data` untouched.

  it("graphql.introspect — a dead/unreachable endpoint never throws through the envelope", async () => {
    const result = await client.graphqlIntrospect("http://127.0.0.1:1/graphql", {});
    expect(typeof result.ok).toBe("boolean");
  });

  it("soap.execute — a dead/unreachable endpoint never throws through the envelope", async () => {
    const result = await client.soapExecute("http://127.0.0.1:1/soap", "urn:spike", {}, "<xml/>");
    expect(typeof result.status).toBe("number");
  });

  // ── privileged ────────────────────────────────────────────────────────

  it("tls.generate — real mkcert CA generation succeeds end-to-end", async () => {
    const result = await client.tlsGenerate();
    expect(result.ok).toBe(true);
    expect(result.certPath).toContain("ca-cert.pem");
    expect(result.keyPath).toContain("ca-key.pem");
  });

  // ── envelope semantics ────────────────────────────────────────────────

  it("unknown action -> UNKNOWN_COMMAND, not a throw", async () => {
    const transport = createInProcessTransport((channel) => registeredHandlers.get(channel));
    const raw = await transport.send({ id: "x1", action: "totally.unknown", payload: {} });
    expect(raw.ok).toBe(false);
    if (!raw.ok) expect(raw.error.code).toBe("UNKNOWN_COMMAND");
  });

  it("invalid payload -> INVALID_PARAMS, not a throw", async () => {
    const transport = createInProcessTransport((channel) => registeredHandlers.get(channel));
    const raw = await transport.send({ id: "x2", action: "mock.delete", payload: { wrong: "shape" } });
    expect(raw.ok).toBe(false);
    if (!raw.ok) expect(raw.error.code).toBe("INVALID_PARAMS");
  });
});
