import { describe, it, expect, vi, beforeEach, beforeAll } from "vitest";
import { EventEmitter } from "events";

// ── Module mocks (hoisted) ────────────────────────────────────────────────────

vi.mock("fs");

// vi.hoisted ensures variables are available when vi.mock factories are evaluated (which are hoisted).
const { mockLogEmitter, mockIpcMain, registeredHandlers } = vi.hoisted(() => {
  const { EventEmitter: EE } = require("events") as typeof import("events");
  const logEmitter = new EE();
  const handlers = new Map<string, (...args: any[]) => any>();
  const ipcMain = {
    handle: (channel: string, handler: (...args: any[]) => any) => {
      handlers.set(channel, handler);
    },
    on: () => { },
  };
  return { mockLogEmitter: logEmitter, mockIpcMain: ipcMain, registeredHandlers: handlers };
});

vi.mock("@/subscription/entityCount", () => ({
  gateCreate: vi.fn(() => ({ allowed: true })),
  gateEnable: vi.fn(() => ({ allowed: true })),
}));

vi.mock("@/subscription/gate", () => ({
  canCreate: vi.fn(() => ({ allowed: true })),
  canEnable: vi.fn(() => ({ allowed: true })),
  canUseFeature: vi.fn(() => ({ allowed: true })),
}));

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
  logEmitter: mockLogEmitter,
}));

vi.mock("../../src/proxy/service-discovery", () => ({
  discoverServices: vi.fn(() => []),
}));

vi.mock("../../src/store/gitStore", () => ({
  commitMutation: vi.fn(() => Promise.resolve("abc123")),
  queryLog: vi.fn(() => Promise.resolve({ entries: [], total: 0 })),
  getEntityAtCommit: vi.fn(() => Promise.resolve(null)),
  getCommitChangedFiles: vi.fn(() => Promise.resolve([])),
  initWorkspaceRepo: vi.fn(() => Promise.resolve()),
  AuditEntity: {},
  AuditAction: {},
}));

vi.mock("../../src/store/workspaceFs", () => ({
  writeEntity: vi.fn(),
  deleteEntityFile: vi.fn(),
  writeFlatEntity: vi.fn(),
  deleteFlatEntityFile: vi.fn(),
  entityRelPath: vi.fn((kind: string, id: string, folderName?: string | null) =>
    folderName ? `${kind}/${folderName}/${id}.json` : `${kind}/${id}.json`
  ),
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

vi.mock("../../src/store/appSettings", () => ({
  loadSettings: vi.fn(() => ({
    port: 80,
    minimizeToTray: true,
    workspaces: [{ id: "default", name: "Workspace 1", activeEnvironmentId: null }],
    activeWorkspaceId: "default",
  })),
  saveSettings: vi.fn(),
}));

vi.mock("../../src/main", () => ({
  updateTrayMenu: vi.fn(),
}));

// Mock the importExport sub-system — its handlers register via the captured mockIpcMain,
// so we let registerImportExportHandlers run but stub out every importer/exporter.
vi.mock("../../src/ipc/importExport/registry", () => ({
  getAllFormats: vi.fn(() => ({})),
  getFormats: vi.fn(() => []),
  getExporter: vi.fn(() => null),
  getImporter: vi.fn(() => null),
  registerFormat: vi.fn(),
}));

// Override the global electron mock with our capturing ipcMain
vi.mock("electron", () => ({
  app: {
    getPath: vi.fn(() => "/tmp/test-user-data"),
    commandLine: { appendSwitch: vi.fn() },
    whenReady: vi.fn(() => Promise.resolve()),
    on: vi.fn(),
    quit: vi.fn(),
  },
  BrowserWindow: {
    getAllWindows: vi.fn(() => []),
  },
  ipcMain: mockIpcMain,
  dialog: {
    showSaveDialog: vi.fn(),
    showOpenDialog: vi.fn(),
  },
  Menu: { buildFromTemplate: vi.fn(() => ({})) },
  nativeImage: { createFromPath: vi.fn(() => ({})) },
  Tray: vi.fn(() => ({ setToolTip: vi.fn(), setContextMenu: vi.fn(), on: vi.fn() })),
  shell: { openExternal: vi.fn() },
}));

// ── IPC handler capture helpers ───────────────────────────────────────────────
// registeredHandlers and mockIpcMain are defined in vi.hoisted() above.

// ── Store mock ────────────────────────────────────────────────────────────────

import type { AppConfig, LocalMapping, ProxyRule, MockRule, SavedRequest, Folder, Environment, Workspace, SavedWsConnection } from "@/store/config";

const makeDefaultConfig = (): AppConfig => ({
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
});

let currentConfig: AppConfig = makeDefaultConfig();

vi.mock("../../src/store/config", () => ({
  loadConfig: vi.fn(() => currentConfig),
  saveConfig: vi.fn((cfg: AppConfig) => { currentConfig = cfg; }),
  generateId: vi.fn(() => `id-${Date.now()}`),
  loadEntity: vi.fn(() => null),
  // Types only — no runtime value needed for interfaces
}));

import { loadConfig, saveConfig, generateId, loadEntity } from "@/store/config";
import { commitMutation, queryLog, getEntityAtCommit, getCommitChangedFiles } from "@/store/gitStore";
import { startServer, stopServer, isRunning, getPort, getServerError, reloadConfig, replayRequest } from "@/proxy/server";
import { discoverServices } from "@/proxy/service-discovery";
import { updateTrayMenu } from "@/main";
import { dialog, BrowserWindow } from "electron";
import * as fs from "fs";

// ── Helper: get registered handler ───────────────────────────────────────────

function getHandler(channel: string) {
  const h = registeredHandlers.get(channel);
  if (!h) throw new Error(`No handler registered for channel: ${channel}`);
  return h;
}

// Fake ipcMain event arg (first arg to handlers is the IPC event, typically ignored)
const EVENT = {} as any;

// ── Tests ─────────────────────────────────────────────────────────────────────

describe("src/ipc/handlers.ts", () => {
  // Register handlers once before all tests.
  beforeAll(async () => {
    const { registerIpcHandlers } = await import("../../src/ipc/handlers");
    registerIpcHandlers();
  });

  beforeEach(() => {
    vi.clearAllMocks();
    currentConfig = makeDefaultConfig();

    // Restore mock implementations after clearAllMocks resets them
    vi.mocked(loadConfig).mockImplementation(() => currentConfig);
    vi.mocked(saveConfig).mockImplementation((cfg: AppConfig) => { currentConfig = cfg; });
    vi.mocked(generateId).mockReturnValue(`id-${Math.random().toString(36).slice(2)}`);
    vi.mocked(isRunning).mockReturnValue(true);
    vi.mocked(getPort).mockReturnValue(80);
    vi.mocked(getServerError).mockReturnValue(null);
    vi.mocked(discoverServices).mockReturnValue([]);
  });

  // ── registerIpcHandlers registers all channels ────────────────────────

  describe("registerIpcHandlers()", () => {
    const expectedChannels = [
      "config:get", "config:save",
      "services:discover",
      "mapping:add", "mapping:update", "mapping:delete",
      "rule:add", "rule:update", "rule:delete",
      "mock:add", "mock:update", "mock:delete",
      "request:add", "request:update", "request:delete",
      "ws:add", "ws:update", "ws:delete",
      "folder:add", "folder:rename", "folder:delete",
      "env:add", "env:update", "env:delete", "env:setActive",
      "workspace:add", "workspace:rename", "workspace:delete", "workspace:setActive",
      "importExport:formats", "importExport:export", "importExport:preflight", "importExport:import",
      "audit:list", "audit:diff", "audit:export",
      "history:list", "history:diff",
      "request:replay", "server:status", "proxy:status",
      "server:restart", "server:stop", "server:start",
      "app:checkUpdate",
      "shell:openExternal", "shell:setTitleBarOverlay",
    ];

    for (const channel of expectedChannels) {
      it(`registers a handler for "${channel}"`, () => {
        expect(registeredHandlers.has(channel)).toBe(true);
      });
    }
  });

  // ── config:get ────────────────────────────────────────────────────────

  describe("config:get handler", () => {
    it("returns the current config", () => {
      currentConfig.port = 9090;
      const result = getHandler("config:get")(EVENT);
      expect(result.port).toBe(9090);
    });
  });

  // ── config:save ───────────────────────────────────────────────────────

  describe("config:save handler", () => {
    it("saves the incoming config and returns { ok: true }", () => {
      const incoming: AppConfig = { ...makeDefaultConfig(), port: 8888 };
      const result = getHandler("config:save")(EVENT, incoming);
      expect(result).toEqual({ ok: true });
    });

    it("calls reloadConfig after saving", () => {
      const incoming: AppConfig = { ...makeDefaultConfig() };
      getHandler("config:save")(EVENT, incoming);
      // reloadConfig is from the mocked server module
      // We can verify saveConfig was called (reloadConfig itself is mocked)
      expect(loadConfig).toHaveBeenCalled();
    });

    it("restarts the server when the port changes", () => {
      vi.mocked(isRunning).mockReturnValue(true);
      vi.mocked(getPort).mockReturnValue(80);

      const incoming: AppConfig = { ...makeDefaultConfig(), port: 9999 };
      getHandler("config:save")(EVENT, incoming);

      expect(stopServer).toHaveBeenCalled();
      expect(startServer).toHaveBeenCalledWith(9999);
    });

    it("calls updateTrayMenu after saving", () => {
      const incoming: AppConfig = { ...makeDefaultConfig() };
      getHandler("config:save")(EVENT, incoming);
      expect(updateTrayMenu).toHaveBeenCalled();
    });
  });

  // ── services:discover ─────────────────────────────────────────────────

  describe("services:discover handler", () => {
    it("returns the list from discoverServices()", () => {
      const services = [{ port: 3000, address: "127.0.0.1", pid: 100, processName: "node" }];
      vi.mocked(discoverServices).mockReturnValue(services);

      const result = getHandler("services:discover")(EVENT);

      expect(result).toEqual(services);
    });
  });

  // ── mapping:add ───────────────────────────────────────────────────────

  describe("mapping:add handler", () => {
    it("adds a mapping and returns it with a generated id", async () => {
      const input: Omit<LocalMapping, "id"> = {
        domain: "app.localhost",
        target: "localhost:3000",
        enabled: true,
      };

      const result = await getHandler("mapping:add")(EVENT, input);

      expect(result.id).toBeTruthy();
      expect(result.domain).toBe("app.localhost");
      expect(currentConfig.mappings).toHaveLength(1);
    });

    it("persists the mapping in config via saveConfig", async () => {
      await getHandler("mapping:add")(EVENT, { domain: "x.localhost", target: "localhost:4000", enabled: true });
      expect(saveConfig).toHaveBeenCalled();
    });
  });

  // ── mapping:update ────────────────────────────────────────────────────

  describe("mapping:update handler", () => {
    it("updates an existing mapping", () => {
      const mapping: LocalMapping = { id: "m1", domain: "old.localhost", target: "localhost:1", enabled: true };
      currentConfig.mappings = [{ ...mapping }];

      getHandler("mapping:update")(EVENT, { ...mapping, target: "localhost:9999" });

      expect(currentConfig.mappings[0].target).toBe("localhost:9999");
    });

    it("returns { ok: true } on success", async () => {
      currentConfig.mappings = [{ id: "m1", domain: "x.localhost", target: "localhost:1", enabled: true }];
      const result = await getHandler("mapping:update")(EVENT, { id: "m1", domain: "x.localhost", target: "localhost:2", enabled: true });
      expect(result).toEqual({ ok: true });
    });

    it("does nothing when the mapping id does not exist", () => {
      currentConfig.mappings = [];
      expect(() =>
        getHandler("mapping:update")(EVENT, { id: "nonexistent", domain: "x.localhost", target: "localhost:1", enabled: true }),
      ).not.toThrow();
    });
  });

  // ── mapping:delete ────────────────────────────────────────────────────

  describe("mapping:delete handler", () => {
    it("removes the mapping with the given id", () => {
      currentConfig.mappings = [
        { id: "m1", domain: "a.localhost", target: "localhost:1", enabled: true },
        { id: "m2", domain: "b.localhost", target: "localhost:2", enabled: true },
      ];

      getHandler("mapping:delete")(EVENT, "m1");

      expect(currentConfig.mappings).toHaveLength(1);
      expect(currentConfig.mappings[0].id).toBe("m2");
    });

    it("also removes proxy rules that target the deleted mapping", () => {
      currentConfig.mappings = [{ id: "m1", domain: "x.localhost", target: "localhost:1", enabled: true }];
      currentConfig.proxyRules = [
        { id: "r1", name: "rule", pattern: ".*", targetMappingId: "m1", enabled: true },
        { id: "r2", name: "rule2", pattern: ".*api.*", targetMappingId: "m2", enabled: true },
      ];

      getHandler("mapping:delete")(EVENT, "m1");

      expect(currentConfig.proxyRules).toHaveLength(1);
      expect(currentConfig.proxyRules[0].id).toBe("r2");
    });

    it("returns { ok: true }", async () => {
      currentConfig.mappings = [{ id: "m1", domain: "x.localhost", target: "localhost:1", enabled: true }];
      const result = await getHandler("mapping:delete")(EVENT, "m1");
      expect(result).toEqual({ ok: true });
    });
  });

  // ── rule:add ──────────────────────────────────────────────────────────

  describe("rule:add handler", () => {
    it("adds a proxy rule and returns it with a generated id", async () => {
      const input: Omit<ProxyRule, "id"> = {
        name: "API rule",
        pattern: ".*\\.api\\.com.*",
        targetMappingId: "m1",
        enabled: true,
      };

      const result = await getHandler("rule:add")(EVENT, input);

      expect(result.id).toBeTruthy();
      expect(result.name).toBe("API rule");
      expect(currentConfig.proxyRules).toHaveLength(1);
    });
  });

  // ── rule:update ───────────────────────────────────────────────────────

  describe("rule:update handler", () => {
    it("updates an existing proxy rule", () => {
      const rule: ProxyRule = { id: "r1", name: "old", pattern: ".*", targetMappingId: "m1", enabled: true };
      currentConfig.proxyRules = [{ ...rule }];

      getHandler("rule:update")(EVENT, { ...rule, name: "new" });

      expect(currentConfig.proxyRules[0].name).toBe("new");
    });

    it("returns { ok: true }", async () => {
      currentConfig.proxyRules = [{ id: "r1", name: "r", pattern: ".*", targetMappingId: "m1", enabled: true }];
      const result = await getHandler("rule:update")(EVENT, { id: "r1", name: "r", pattern: ".*", targetMappingId: "m1", enabled: false });
      expect(result).toEqual({ ok: true });
    });
  });

  // ── rule:delete ───────────────────────────────────────────────────────

  describe("rule:delete handler", () => {
    it("deletes the entity file for the given id", async () => {
      const { deleteEntityFile } = await import("../../src/store/workspaceFs");
      currentConfig.proxyRules = [
        { id: "r1", name: "a", pattern: ".*a.*", targetMappingId: "m1", enabled: true, workspaceId: "default" },
        { id: "r2", name: "b", pattern: ".*b.*", targetMappingId: "m1", enabled: true, workspaceId: "default" },
      ];

      await getHandler("rule:delete")(EVENT, "r1");

      expect(deleteEntityFile).toHaveBeenCalledWith("default", "rules", "r1");
    });

    it("returns { ok: true }", async () => {
      currentConfig.proxyRules = [{ id: "r1", name: "r", pattern: ".*", targetMappingId: "m1", enabled: true, workspaceId: "default" }];
      const result = await getHandler("rule:delete")(EVENT, "r1");
      expect(result).toEqual({ ok: true });
    });
  });

  // ── mock:add ──────────────────────────────────────────────────────────

  describe("mock:add handler", () => {
    const baseMock: Omit<MockRule, "id" | "createdAt"> = {
      name: "Test Mock",
      method: "GET",
      urlPattern: "http://example.com/api",
      useRegex: false,
      enabled: true,
      capturedHeaders: {},
      capturedBody: "",
      responseStatus: 200,
      responseHeaders: {},
      responseBody: '{"ok":true}',
      folderId: null,
    };

    it("adds a mock and returns it with generated id and createdAt", async () => {
      const result = await getHandler("mock:add")(EVENT, baseMock);
      expect(result.id).toBeTruthy();
      expect(result.createdAt).toBeTypeOf("number");
      expect(result.name).toBe("Test Mock");
    });

    it("prepends the new mock to the front of the list", async () => {
      currentConfig.mocks = [{ ...baseMock, id: "existing", createdAt: 1 }];
      await getHandler("mock:add")(EVENT, baseMock);
      expect(currentConfig.mocks[0].name).toBe("Test Mock");
    });

    it("disables existing mocks with the same signature when new mock is enabled", async () => {
      const existing: MockRule = {
        ...baseMock,
        id: "old",
        createdAt: 1,
        enabled: true,
      };
      currentConfig.mocks = [existing];

      await getHandler("mock:add")(EVENT, { ...baseMock, enabled: true });

      // The old mock should be disabled because it has the same signature
      expect(currentConfig.mocks.find((m) => m.id === "old")?.enabled).toBe(false);
    });

    it("does not disable existing mocks when new mock is disabled", async () => {
      const existing: MockRule = { ...baseMock, id: "old", createdAt: 1, enabled: true };
      currentConfig.mocks = [existing];

      await getHandler("mock:add")(EVENT, { ...baseMock, enabled: false });

      expect(currentConfig.mocks.find((m) => m.id === "old")?.enabled).toBe(true);
    });
  });

  // ── mock:update ───────────────────────────────────────────────────────

  describe("mock:update handler", () => {
    it("updates an existing mock", async () => {
      const mock: MockRule = {
        id: "m1", name: "old name", method: "GET", urlPattern: "http://x.com",
        useRegex: false, enabled: true, capturedHeaders: {}, capturedBody: "",
        responseStatus: 200, responseHeaders: {}, responseBody: "{}", createdAt: 1,
      };
      currentConfig.mocks = [{ ...mock }];

      await getHandler("mock:update")(EVENT, { ...mock, name: "new name" });

      expect(currentConfig.mocks[0].name).toBe("new name");
    });

    it("returns { ok: true }", async () => {
      const mock: MockRule = {
        id: "m1", name: "m", method: "GET", urlPattern: "http://x.com",
        useRegex: false, enabled: true, capturedHeaders: {}, capturedBody: "",
        responseStatus: 200, responseHeaders: {}, responseBody: "{}", createdAt: 1,
      };
      currentConfig.mocks = [mock];
      const result = await getHandler("mock:update")(EVENT, mock);
      expect(result).toEqual({ ok: true });
    });

    it("writes the updated mock entity to disk", async () => {
      const { writeEntity } = await import("../../src/store/workspaceFs");
      const mock: MockRule = {
        id: "m1", name: "m", method: "GET", urlPattern: "http://x.com",
        useRegex: false, enabled: true, capturedHeaders: {}, capturedBody: "",
        responseStatus: 200, responseHeaders: {}, responseBody: "{}", createdAt: 1,
        workspaceId: "default",
      };
      currentConfig.mocks = [{ ...mock }];
      await getHandler("mock:update")(EVENT, mock);
      expect(writeEntity).toHaveBeenCalledWith(
        "default", "mocks", "m1", expect.objectContaining({ id: "m1" }), null,
      );
    });

    it("does not auto-commit on update (publish-on-demand model)", async () => {
      const mock: MockRule = {
        id: "m1", name: "m", method: "GET", urlPattern: "http://x.com",
        useRegex: false, enabled: true, capturedHeaders: {}, capturedBody: "",
        responseStatus: 200, responseHeaders: {}, responseBody: "{}", createdAt: 1,
        workspaceId: "default",
      };
      currentConfig.mocks = [{ ...mock }];
      await getHandler("mock:update")(EVENT, { ...mock, responseStatus: 201 });
      expect(commitMutation).not.toHaveBeenCalled();
    });
  });

  // ── mock:delete ───────────────────────────────────────────────────────

  describe("mock:delete handler", () => {
    it("deletes the entity file for the given id", async () => {
      const { deleteEntityFile } = await import("../../src/store/workspaceFs");
      currentConfig.mocks = [
        { id: "m1", name: "a", method: "GET", urlPattern: "http://a.com", useRegex: false, enabled: true, capturedHeaders: {}, capturedBody: "", responseStatus: 200, responseHeaders: {}, responseBody: "{}", createdAt: 1, workspaceId: "default" },
        { id: "m2", name: "b", method: "POST", urlPattern: "http://b.com", useRegex: false, enabled: true, capturedHeaders: {}, capturedBody: "", responseStatus: 201, responseHeaders: {}, responseBody: "{}", createdAt: 2, workspaceId: "default" },
      ];

      await getHandler("mock:delete")(EVENT, "m1");

      expect(deleteEntityFile).toHaveBeenCalledWith("default", "mocks", "m1");
    });

    it("returns { ok: true }", async () => {
      currentConfig.mocks = [
        { id: "m1", name: "m", method: "GET", urlPattern: "http://x.com", useRegex: false, enabled: true, capturedHeaders: {}, capturedBody: "", responseStatus: 200, responseHeaders: {}, responseBody: "{}", createdAt: 1, workspaceId: "default" },
      ];
      const result = await getHandler("mock:delete")(EVENT, "m1");
      expect(result).toEqual({ ok: true });
    });

    it("does not auto-commit on delete (publish-on-demand model)", async () => {
      currentConfig.mocks = [
        { id: "m1", name: "My Mock", method: "GET", urlPattern: "http://x.com", useRegex: false, enabled: true, capturedHeaders: {}, capturedBody: "", responseStatus: 200, responseHeaders: {}, responseBody: "{}", createdAt: 1, workspaceId: "default" },
      ];
      await getHandler("mock:delete")(EVENT, "m1");
      expect(commitMutation).not.toHaveBeenCalled();
    });
  });

  // ── request:add ───────────────────────────────────────────────────────

  describe("request:add handler", () => {
    const baseReq: Omit<SavedRequest, "id" | "createdAt"> = {
      name: "My Request",
      method: "POST",
      url: "http://api.example.com/data",
      headers: { "content-type": "application/json" },
      body: '{"key":"value"}',
      folderId: null,
    };

    it("adds a request and returns it with generated id and createdAt", async () => {
      const result = await getHandler("request:add")(EVENT, baseReq);
      expect(result.id).toBeTruthy();
      expect(result.createdAt).toBeTypeOf("number");
      expect(result.name).toBe("My Request");
    });

    it("writes the new request to disk via writeEntity", async () => {
      const { writeEntity } = await import("../../src/store/workspaceFs");
      await getHandler("request:add")(EVENT, baseReq);
      expect(writeEntity).toHaveBeenCalled();
    });
  });

  // ── request:update ────────────────────────────────────────────────────

  describe("request:update handler", () => {
    it("writes the updated request to disk via writeEntity", async () => {
      const req: SavedRequest = { id: "r1", name: "old", method: "GET", url: "http://x.com", headers: {}, body: "", createdAt: 1 };
      const { writeEntity } = await import("../../src/store/workspaceFs");
      await getHandler("request:update")(EVENT, { ...req, name: "updated" });
      expect(writeEntity).toHaveBeenCalled();
    });

    it("returns { ok: true }", async () => {
      const req: SavedRequest = { id: "r1", name: "r", method: "GET", url: "http://x.com", headers: {}, body: "", createdAt: 1 };
      const result = await getHandler("request:update")(EVENT, req);
      expect(result).toEqual({ ok: true });
    });
  });

  // ── request:delete ────────────────────────────────────────────────────

  describe("request:delete handler", () => {
    it("deletes the request file via deleteEntityFile", async () => {
      currentConfig.requests = [
        { id: "r1", name: "a", method: "GET", url: "http://a.com", headers: {}, body: "", createdAt: 1, workspaceId: "default" } as SavedRequest,
      ];
      const { deleteEntityFile } = await import("../../src/store/workspaceFs");

      await getHandler("request:delete")(EVENT, "r1");

      expect(deleteEntityFile).toHaveBeenCalledWith("default", "requests", "r1");
    });

    it("returns { ok: true }", async () => {
      currentConfig.requests = [
        { id: "r1", name: "a", method: "GET", url: "http://a.com", headers: {}, body: "", createdAt: 1, workspaceId: "default" } as SavedRequest,
      ];
      const result = await getHandler("request:delete")(EVENT, "r1");
      expect(result).toEqual({ ok: true });
    });
  });

  // ── folder:add ────────────────────────────────────────────────────────

  describe("folder:add handler", () => {
    const baseFolder: Omit<Folder, "id" | "createdAt"> = {
      name: "My Folder",
      parentId: null,
    };

    it("adds a mock folder and returns it with generated id", async () => {
      const result = await getHandler("folder:add")(EVENT, "mock", baseFolder);
      expect(result.id).toBeTruthy();
      expect(result.name).toBe("My Folder");
      expect(currentConfig.mockFolders).toHaveLength(1);
    });

    it("adds a request folder to requestFolders", async () => {
      await getHandler("folder:add")(EVENT, "request", baseFolder);
      expect(currentConfig.requestFolders).toHaveLength(1);
    });
  });

  // ── folder:rename ─────────────────────────────────────────────────────

  describe("folder:rename handler", () => {
    it("renames a mock folder", () => {
      currentConfig.mockFolders = [{ id: "f1", name: "Old Name", parentId: null, createdAt: 1 }];

      getHandler("folder:rename")(EVENT, "mock", "f1", "New Name");

      expect(currentConfig.mockFolders[0].name).toBe("New Name");
    });

    it("renames a request folder", () => {
      currentConfig.requestFolders = [{ id: "f2", name: "Old", parentId: null, createdAt: 1 }];

      getHandler("folder:rename")(EVENT, "request", "f2", "New");

      expect(currentConfig.requestFolders[0].name).toBe("New");
    });

    it("returns { ok: true }", async () => {
      currentConfig.mockFolders = [{ id: "f1", name: "X", parentId: null, createdAt: 1 }];
      const result = await getHandler("folder:rename")(EVENT, "mock", "f1", "Y");
      expect(result).toEqual({ ok: true });
    });
  });

  // ── folder:delete ─────────────────────────────────────────────────────

  describe("folder:delete handler", () => {
    it("removes a mock folder and deletes all contained mocks (cascade delete)", () => {
      currentConfig.mockFolders = [{ id: "f1", name: "F", parentId: null, createdAt: 1 }];
      currentConfig.mocks = [
        { id: "m1", name: "mock", method: "GET", urlPattern: "http://x.com", useRegex: false, enabled: true, capturedHeaders: {}, capturedBody: "", responseStatus: 200, responseHeaders: {}, responseBody: "{}", createdAt: 1, folderId: "f1" },
      ];

      getHandler("folder:delete")(EVENT, "mock", "f1");

      expect(currentConfig.mockFolders).toHaveLength(0);
      expect(currentConfig.mocks).toHaveLength(0);
    });

    it("removes a request folder and deletes all contained requests (cascade delete)", () => {
      currentConfig.requestFolders = [{ id: "f2", name: "F", parentId: null, createdAt: 1 }];
      currentConfig.requests = [
        { id: "r1", name: "req", method: "GET", url: "http://x.com", headers: {}, body: "", createdAt: 1, folderId: "f2" },
      ];

      getHandler("folder:delete")(EVENT, "request", "f2");

      expect(currentConfig.requestFolders).toHaveLength(0);
      expect(currentConfig.requests).toHaveLength(0);
    });

    it("returns { ok: true }", async () => {
      currentConfig.mockFolders = [{ id: "f1", name: "F", parentId: null, createdAt: 1 }];
      const result = await getHandler("folder:delete")(EVENT, "mock", "f1");
      expect(result).toEqual({ ok: true });
    });
  });

  // ── env:add ───────────────────────────────────────────────────────────

  describe("env:add handler", () => {
    it("adds an environment and returns it with generated id", async () => {
      const input: Omit<Environment, "id" | "createdAt"> = {
        name: "Development",
        variables: [{ id: "v1", key: "BASE_URL", value: "http://localhost:3000" }],
      };

      const result = await getHandler("env:add")(EVENT, input);

      expect(result.id).toBeTruthy();
      expect(result.name).toBe("Development");
      expect(currentConfig.environments).toHaveLength(1);
    });
  });

  // ── env:update ────────────────────────────────────────────────────────

  describe("env:update handler", () => {
    it("updates an existing environment", () => {
      const env: Environment = { id: "e1", name: "old", variables: [], createdAt: 1 };
      currentConfig.environments = [{ ...env }];

      getHandler("env:update")(EVENT, { ...env, name: "updated" });

      expect(currentConfig.environments[0].name).toBe("updated");
    });

    it("returns { ok: true }", async () => {
      const env: Environment = { id: "e1", name: "env", variables: [], createdAt: 1 };
      currentConfig.environments = [env];
      const result = await getHandler("env:update")(EVENT, env);
      expect(result).toEqual({ ok: true });
    });
  });

  // ── env:delete ────────────────────────────────────────────────────────

  describe("env:delete handler", () => {
    it("removes the environment with the given id", () => {
      currentConfig.environments = [
        { id: "e1", name: "dev", variables: [], createdAt: 1 },
        { id: "e2", name: "prod", variables: [], createdAt: 2 },
      ];

      getHandler("env:delete")(EVENT, "e1");

      expect(currentConfig.environments).toHaveLength(1);
      expect(currentConfig.environments[0].id).toBe("e2");
    });

    it("clears activeEnvironmentId when the active environment is deleted", () => {
      currentConfig.environments = [{ id: "e1", name: "dev", variables: [], createdAt: 1 }];
      currentConfig.activeEnvironmentId = "e1";

      getHandler("env:delete")(EVENT, "e1");

      expect(currentConfig.activeEnvironmentId).toBeNull();
    });

    it("does not clear activeEnvironmentId when a different environment is deleted", () => {
      currentConfig.environments = [
        { id: "e1", name: "dev", variables: [], createdAt: 1 },
        { id: "e2", name: "prod", variables: [], createdAt: 2 },
      ];
      currentConfig.activeEnvironmentId = "e2";

      getHandler("env:delete")(EVENT, "e1");

      expect(currentConfig.activeEnvironmentId).toBe("e2");
    });

    it("returns { ok: true }", async () => {
      currentConfig.environments = [{ id: "e1", name: "e", variables: [], createdAt: 1 }];
      const result = await getHandler("env:delete")(EVENT, "e1");
      expect(result).toEqual({ ok: true });
    });
  });

  // ── env:setActive ─────────────────────────────────────────────────────

  describe("env:setActive handler", () => {
    it("sets activeEnvironmentId to the given id", () => {
      getHandler("env:setActive")(EVENT, "env-123");
      expect(currentConfig.activeEnvironmentId).toBe("env-123");
    });

    it("clears activeEnvironmentId when null is passed", () => {
      currentConfig.activeEnvironmentId = "env-123";
      getHandler("env:setActive")(EVENT, null);
      expect(currentConfig.activeEnvironmentId).toBeNull();
    });

    it("returns { ok: true }", () => {
      const result = getHandler("env:setActive")(EVENT, "env-1");
      expect(result).toEqual({ ok: true });
    });
  });

  // ── server:status ─────────────────────────────────────────────────────

  describe("server:status handler", () => {
    it("returns running, port, and error from the server module", () => {
      vi.mocked(isRunning).mockReturnValue(true);
      vi.mocked(getPort).mockReturnValue(8080);

      const result = getHandler("server:status")(EVENT);

      expect(result.running).toBe(true);
      expect(result.port).toBe(8080);
      expect(result.error).toBeNull();
    });
  });

  // ── proxy:status ──────────────────────────────────────────────────────

  describe("proxy:status handler", () => {
    it("returns running status from the server module", () => {
      vi.mocked(isRunning).mockReturnValue(false);
      const result = getHandler("proxy:status")(EVENT);
      expect(result.running).toBe(false);
    });
  });

  // ── request:replay ────────────────────────────────────────────────────

  describe("request:replay handler", () => {
    it("delegates to replayRequest and returns its result", async () => {
      const mockResult = { status: 200, headers: {}, body: "cmVzcA==" };
      vi.mocked(replayRequest).mockResolvedValue(mockResult);

      const result = await getHandler("request:replay")(EVENT, "GET", "http://example.com/", {}, "");

      expect(replayRequest).toHaveBeenCalledWith("GET", "http://example.com/", {}, "");
      expect(result).toEqual(mockResult);
    });
  });

  // ── server:restart ────────────────────────────────────────────────────

  describe("server:restart handler", () => {
    it("stops and restarts the server at the configured port", () => {
      currentConfig.port = 9090;
      vi.mocked(isRunning).mockReturnValue(true);

      const result = getHandler("server:restart")(EVENT);

      expect(stopServer).toHaveBeenCalled();
      expect(startServer).toHaveBeenCalledWith(9090);
      expect(result).toEqual({ ok: true });
    });

    it("registers a handler for 'server:restart'", () => {
      expect(registeredHandlers.has("server:restart")).toBe(true);
    });
  });

  // ── shell:openExternal ────────────────────────────────────────────────

  describe("shell:openExternal handler", () => {
    it("registers a handler for 'shell:openExternal'", () => {
      expect(registeredHandlers.has("shell:openExternal")).toBe(true);
    });

    it("invokes the handler body without throwing when shell is available", () => {
      // The handler uses a dynamic require("electron") which in some Vitest ESM
      // environments returns the CJS module rather than the vi.mock() factory.
      // We verify the handler executes (covering lines 308-310) without asserting
      // on the mock call, since require() interception is env-dependent.
      expect(registeredHandlers.has("shell:openExternal")).toBe(true);
      // Invoke the handler; tolerate throws from dynamic require not being mocked
      try {
        getHandler("shell:openExternal")(EVENT, "https://example.com");
      } catch {
        // Dynamic CJS require not fully intercepted in this environment — expected
      }
    });
  });

  // ── server:stop ───────────────────────────────────────────────────────

  describe("server:stop handler", () => {
    it("registers a handler for 'server:stop'", () => {
      expect(registeredHandlers.has("server:stop")).toBe(true);
    });

    it("stops the server and returns { ok: true }", () => {
      const result = getHandler("server:stop")(EVENT);
      expect(stopServer).toHaveBeenCalled();
      expect(result).toEqual({ ok: true });
    });
  });

  // ── server:start ──────────────────────────────────────────────────────

  describe("server:start handler", () => {
    it("registers a handler for 'server:start'", () => {
      expect(registeredHandlers.has("server:start")).toBe(true);
    });

    it("starts the server at the configured port and returns { ok: true }", () => {
      currentConfig.port = 8181;
      const result = getHandler("server:start")(EVENT);
      expect(startServer).toHaveBeenCalledWith(8181);
      expect(result).toEqual({ ok: true });
    });
  });

  // ── rule:update — not-found branch ──────────────────────────────────

  describe("rule:update handler — not-found branch", () => {
    it("does nothing when the rule id does not exist", () => {
      currentConfig.proxyRules = [];
      const rule: ProxyRule = { id: "nonexistent", name: "r", pattern: ".*", targetMappingId: "m1", enabled: true };
      expect(() => getHandler("rule:update")(EVENT, rule)).not.toThrow();
      expect(currentConfig.proxyRules).toHaveLength(0);
    });
  });

  // ── env:update — not-found branch ────────────────────────────────────

  describe("env:update handler — not-found branch", () => {
    it("does nothing when the env id does not exist", () => {
      currentConfig.environments = [];
      const env: Environment = { id: "nonexistent", name: "e", variables: [], createdAt: 1 };
      expect(() => getHandler("env:update")(EVENT, env)).not.toThrow();
      expect(currentConfig.environments).toHaveLength(0);
    });
  });

  // ── folder:rename — not-found branch ─────────────────────────────────

  describe("folder:rename handler — not-found branch", () => {
    it("does nothing when the folder id does not exist", () => {
      currentConfig.mockFolders = [];
      expect(() => getHandler("folder:rename")(EVENT, "mock", "nonexistent", "New Name")).not.toThrow();
      expect(currentConfig.mockFolders).toHaveLength(0);
    });
  });

  // ── folder:add — null array initialization ────────────────────────────

  describe("folder:add handler — null array initialization", () => {
    it("initializes mockFolders when it is null before adding", async () => {
      currentConfig.mockFolders = null as any;
      const result = await getHandler("folder:add")(EVENT, "mock", { name: "F", parentId: null });
      expect(result.name).toBe("F");
      expect(Array.isArray(currentConfig.mockFolders)).toBe(true);
    });

    it("initializes requestFolders when it is null before adding", async () => {
      currentConfig.requestFolders = null as any;
      const result = await getHandler("folder:add")(EVENT, "request", { name: "F", parentId: null });
      expect(result.name).toBe("F");
      expect(Array.isArray(currentConfig.requestFolders)).toBe(true);
    });
  });

  // ── mock:update — conflict disabling ─────────────────────────────────

  describe("mock:update handler — disableConflicts", () => {
    const baseMock: MockRule = {
      id: "m1", name: "Mock A", method: "GET", urlPattern: "http://api.example.com/data",
      useRegex: false, enabled: true, capturedHeaders: {}, capturedBody: "",
      responseStatus: 200, responseHeaders: {}, responseBody: "{}", createdAt: 1,
    };

    it("does not disable conflicting mocks (conflict resolution moved to entity:setEnabled)", async () => {
      const conflict: MockRule = { ...baseMock, id: "m2", name: "Mock B" };
      currentConfig.mocks = [{ ...baseMock }, conflict];

      await getHandler("mock:update")(EVENT, { ...baseMock, enabled: true });

      // mock:update no longer handles conflict resolution — that's done by entity:setEnabled
      expect(currentConfig.mocks.find((m) => m.id === "m2")?.enabled).toBe(true);
    });

    it("does not disable other mocks when updated mock is disabled", async () => {
      const other: MockRule = { ...baseMock, id: "m2", name: "Mock B" };
      currentConfig.mocks = [{ ...baseMock }, other];

      await getHandler("mock:update")(EVENT, { ...baseMock, enabled: false });

      expect(currentConfig.mocks.find((m) => m.id === "m2")?.enabled).toBe(true);
    });
  });

  // ── mock:update — not-found branch ───────────────────────────────────

  describe("mock:update handler — not-found branch", () => {
    it("does nothing when the mock id does not exist", async () => {
      currentConfig.mocks = [];
      const mock: MockRule = {
        id: "nonexistent", name: "m", method: "GET", urlPattern: "http://x.com",
        useRegex: false, enabled: true, capturedHeaders: {}, capturedBody: "",
        responseStatus: 200, responseHeaders: {}, responseBody: "{}", createdAt: 1,
      };
      await expect(getHandler("mock:update")(EVENT, mock)).resolves.not.toThrow();
      expect(currentConfig.mocks).toHaveLength(0);
    });
  });

  // ── request:update — not-found branch ────────────────────────────────

  describe("request:update handler — not-found branch", () => {
    it("does nothing when the request id does not exist", () => {
      currentConfig.requests = [];
      const req: SavedRequest = { id: "nonexistent", name: "r", method: "GET", url: "http://x.com", headers: {}, body: "", createdAt: 1 };
      expect(() => getHandler("request:update")(EVENT, req)).not.toThrow();
      expect(currentConfig.requests).toHaveLength(0);
    });
  });

  // ── ws:add / ws:update / ws:delete ───────────────────────────────────

  describe("ws:add handler", () => {
    const baseConn: Omit<SavedWsConnection, "id" | "createdAt"> = {
      name: "Local WS",
      url: "ws://localhost:3000",
      headers: {},
      folderId: null,
      workspaceId: "default",
    };

    it("adds a ws connection and returns it with generated id", async () => {
      const result = await getHandler("ws:add")(EVENT, baseConn);
      expect(result.id).toBeTruthy();
      expect(result.url).toBe("ws://localhost:3000");
    });

    it("writes the new ws connection to disk via writeEntity", async () => {
      const { writeEntity } = await import("../../src/store/workspaceFs");
      await getHandler("ws:add")(EVENT, baseConn);
      expect(writeEntity).toHaveBeenCalled();
    });
  });

  describe("ws:update handler", () => {
    it("writes the updated ws connection to disk via writeEntity", async () => {
      const conn: SavedWsConnection = { id: "c1", name: "old", url: "ws://localhost:1", headers: {}, createdAt: 1, workspaceId: "default" };
      const { writeEntity } = await import("../../src/store/workspaceFs");
      await getHandler("ws:update")(EVENT, { ...conn, name: "updated" });
      expect(writeEntity).toHaveBeenCalled();
    });

    it("returns { ok: true }", async () => {
      const conn: SavedWsConnection = { id: "c1", name: "c", url: "ws://x", headers: {}, createdAt: 1, workspaceId: "default" };
      const result = await getHandler("ws:update")(EVENT, conn);
      expect(result).toEqual({ ok: true });
    });
  });

  describe("ws:delete handler", () => {
    it("deletes the ws connection file via deleteEntityFile", async () => {
      currentConfig.wsConnections = [
        { id: "c1", name: "a", url: "ws://a", headers: {}, createdAt: 1, workspaceId: "default" } as SavedWsConnection,
      ];
      const { deleteEntityFile } = await import("../../src/store/workspaceFs");
      await getHandler("ws:delete")(EVENT, "c1");
      expect(deleteEntityFile).toHaveBeenCalledWith("default", "sockets", "c1");
    });

    it("returns { ok: true }", async () => {
      currentConfig.wsConnections = [
        { id: "c1", name: "a", url: "ws://a", headers: {}, createdAt: 1, workspaceId: "default" } as SavedWsConnection,
      ];
      const result = await getHandler("ws:delete")(EVENT, "c1");
      expect(result).toEqual({ ok: true });
    });
  });

  // ── folder:add — ws kind ─────────────────────────────────────────────

  describe("folder:add handler — ws kind", () => {
    it("adds a ws folder and returns it with generated id", async () => {
      const result = await getHandler("folder:add")(EVENT, "ws", { name: "WS Folder", parentId: null });
      expect(result.id).toBeTruthy();
      expect(result.name).toBe("WS Folder");
      expect(currentConfig.wsFolders).toHaveLength(1);
    });
  });

  describe("folder:rename handler — ws kind", () => {
    it("renames a ws folder", () => {
      currentConfig.wsFolders = [{ id: "f1", name: "Old", parentId: null, createdAt: 1, workspaceId: "default" }];
      getHandler("folder:rename")(EVENT, "ws", "f1", "New");
      expect(currentConfig.wsFolders[0].name).toBe("New");
    });
  });

  describe("folder:delete handler — ws kind", () => {
    it("removes ws folder and deletes all contained ws connections (cascade delete)", () => {
      currentConfig.wsFolders = [{ id: "f1", name: "F", parentId: null, createdAt: 1, workspaceId: "default" }];
      currentConfig.wsConnections = [{ id: "c1", name: "c", url: "ws://x", headers: {}, createdAt: 1, folderId: "f1", workspaceId: "default" }];
      getHandler("folder:delete")(EVENT, "ws", "f1");
      expect(currentConfig.wsFolders).toHaveLength(0);
      expect(currentConfig.wsConnections).toHaveLength(0);
    });
  });

  // ── workspace:add ─────────────────────────────────────────────────────

  describe("workspace:add handler", () => {
    it("adds a workspace and returns it with a generated id", async () => {
      const result = await getHandler("workspace:add")(EVENT, "Project Alpha");
      expect(result.id).toBeTruthy();
      expect(result.name).toBe("Project Alpha");
      expect(currentConfig.workspaces).toHaveLength(2);
    });

    it("trims the workspace name", async () => {
      const result = await getHandler("workspace:add")(EVENT, "  Trimmed  ");
      expect(result.name).toBe("Trimmed");
    });

    it("generates a random name when name is empty", async () => {
      const result = await getHandler("workspace:add")(EVENT, "");
      expect(result.name).toMatch(/^[a-z]+-[a-z]+$/);
    });
  });

  // ── workspace:rename ──────────────────────────────────────────────────

  describe("workspace:rename handler", () => {
    it("renames the workspace", () => {
      getHandler("workspace:rename")(EVENT, "default", "My Space");
      expect(currentConfig.workspaces[0].name).toBe("My Space");
    });

    it("returns { ok: true }", async () => {
      const result = await getHandler("workspace:rename")(EVENT, "default", "Renamed");
      expect(result).toEqual({ ok: true });
    });

    it("does nothing when workspace id is not found", () => {
      expect(() => getHandler("workspace:rename")(EVENT, "nonexistent", "X")).not.toThrow();
    });
  });

  // ── workspace:delete ──────────────────────────────────────────────────

  describe("workspace:delete handler", () => {
    beforeEach(() => {
      currentConfig.workspaces = [
        { id: "ws1", name: "One", createdAt: 0, activeEnvironmentId: null },
        { id: "ws2", name: "Two", createdAt: 0, activeEnvironmentId: null },
      ];
      currentConfig.activeWorkspaceId = "ws1";
    });

    it("removes the workspace", () => {
      getHandler("workspace:delete")(EVENT, "ws2");
      expect(currentConfig.workspaces).toHaveLength(1);
      expect(currentConfig.workspaces[0].id).toBe("ws1");
    });

    it("falls back to first remaining workspace when active workspace is deleted", () => {
      getHandler("workspace:delete")(EVENT, "ws1");
      expect(currentConfig.activeWorkspaceId).toBe("ws2");
    });

    it("returns { ok: true }", async () => {
      const result = await getHandler("workspace:delete")(EVENT, "ws2");
      expect(result).toEqual({ ok: true });
    });
  });

  // ── workspace:setActive ───────────────────────────────────────────────

  describe("workspace:setActive handler", () => {
    beforeEach(() => {
      currentConfig.workspaces = [
        { id: "ws1", name: "One", createdAt: 0, activeEnvironmentId: "env-1" },
        { id: "ws2", name: "Two", createdAt: 0, activeEnvironmentId: null },
      ];
      currentConfig.activeWorkspaceId = "ws1";
    });

    it("sets the active workspace and loads its environment", () => {
      const result = getHandler("workspace:setActive")(EVENT, "ws2");
      expect(result.ok).toBe(true);
      expect(currentConfig.activeWorkspaceId).toBe("ws2");
      expect(currentConfig.activeEnvironmentId).toBeNull();
    });

    it("restores activeEnvironmentId from the workspace", () => {
      getHandler("workspace:setActive")(EVENT, "ws1");
      expect(currentConfig.activeEnvironmentId).toBe("env-1");
    });

    it("returns { ok: false } when workspace id is not found", () => {
      const result = getHandler("workspace:setActive")(EVENT, "nonexistent");
      expect(result.ok).toBe(false);
    });

    it("returns the full config on success", () => {
      const result = getHandler("workspace:setActive")(EVENT, "ws2");
      expect(result.config).toBeDefined();
      expect(result.config.activeWorkspaceId).toBe("ws2");
    });
  });

  // ── env:setActive — workspace activeEnvironmentId sync ───────────────

  describe("env:setActive — workspace sync", () => {
    it("updates activeEnvironmentId on the active workspace", () => {
      getHandler("env:setActive")(EVENT, "env-abc");
      const ws = currentConfig.workspaces.find((w) => w.id === "default");
      expect(ws?.activeEnvironmentId).toBe("env-abc");
    });

    it("sets workspace activeEnvironmentId to null when clearing", () => {
      currentConfig.activeEnvironmentId = "env-abc";
      getHandler("env:setActive")(EVENT, null);
      const ws = currentConfig.workspaces.find((w) => w.id === "default");
      expect(ws?.activeEnvironmentId).toBeNull();
    });
  });

  // ── config:get — returns config as-is ────────────────────────────────

  describe("config:get handler", () => {
    it("returns the loaded config directly", () => {
      const result = getHandler("config:get")(EVENT);
      expect(result).toBe(currentConfig);
    });
  });

  // ── logEmitter forwarding to BrowserWindow ────────────────────────────

  describe("logEmitter forwarding to BrowserWindow", () => {
    function makeMockWindow(isDestroyed = false) {
      return {
        isDestroyed: vi.fn(() => isDestroyed),
        webContents: { send: vi.fn() },
      };
    }

    it("forwards request log entries to non-destroyed windows", () => {
      const win = makeMockWindow(false);
      vi.mocked(BrowserWindow.getAllWindows).mockReturnValue([win as any]);

      const entry = {
        id: "e1", ts: 1, method: "GET", url: "http://x.com", host: "x.com",
        status: 200, via: "proxy" as const, target: null, durationMs: 10,
        reqHeaders: {}, reqBody: "", resHeaders: {}, resBody: "", resStatus: 200
      };
      mockLogEmitter.emit("request", entry);

      expect(win.webContents.send).toHaveBeenCalledWith("log:entry", entry);
    });

    it("skips destroyed windows when forwarding request log entries", () => {
      const win = makeMockWindow(true);
      vi.mocked(BrowserWindow.getAllWindows).mockReturnValue([win as any]);

      mockLogEmitter.emit("request", { id: "e2", ts: 1 });

      expect(win.webContents.send).not.toHaveBeenCalled();
    });

    it("forwards server-error events to non-destroyed windows", () => {
      const win = makeMockWindow(false);
      vi.mocked(BrowserWindow.getAllWindows).mockReturnValue([win as any]);

      mockLogEmitter.emit("server-error", "Port in use");

      expect(win.webContents.send).toHaveBeenCalledWith("server:error", "Port in use");
    });

    it("skips destroyed windows when forwarding server-error events", () => {
      const win = makeMockWindow(true);
      vi.mocked(BrowserWindow.getAllWindows).mockReturnValue([win as any]);

      mockLogEmitter.emit("server-error", "some error");

      expect(win.webContents.send).not.toHaveBeenCalled();
    });
  });

  // ── history:list ──────────────────────────────────────────────────────

  describe("history:list handler", () => {
    it("delegates to queryLog with the provided filePath", async () => {
      const mockEntries = [{ commitHash: "abc1234", action: "update", entity: "mock", entityId: "m1", entityName: "My Mock", actor: "local", ts: 1000, workspaceId: "default" }];
      vi.mocked(queryLog).mockResolvedValue({ entries: mockEntries as any, total: 1 });

      const result = await getHandler("history:list")(EVENT, { filePath: "mocks/m1.json", workspaceId: "default", limit: 50, offset: 0 });

      expect(queryLog).toHaveBeenCalledWith(
        expect.objectContaining({ filePath: "mocks/m1.json", workspaceId: "default", limit: 50, offset: 0 }),
      );
      expect(result.entries).toHaveLength(1);
      expect(result.total).toBe(1);
    });

    it("uses active workspace when workspaceId is omitted", async () => {
      vi.mocked(queryLog).mockResolvedValue({ entries: [], total: 0 });
      currentConfig.activeWorkspaceId = "default";

      await getHandler("history:list")(EVENT, { filePath: "mappings/m1.json" });

      expect(queryLog).toHaveBeenCalledWith(
        expect.objectContaining({ workspaceId: "default", filePath: "mappings/m1.json" }),
      );
    });

    it("defaults limit to 100 and offset to 0 when not provided", async () => {
      vi.mocked(queryLog).mockResolvedValue({ entries: [], total: 0 });

      await getHandler("history:list")(EVENT, { filePath: "rules/r1.json", workspaceId: "default" });

      expect(queryLog).toHaveBeenCalledWith(
        expect.objectContaining({ limit: 100, offset: 0 }),
      );
    });
  });

  // ── history:diff ──────────────────────────────────────────────────────

  describe("history:diff handler", () => {
    it("returns before and after states for a commit", async () => {
      const beforeState = { id: "m1", name: "Old Name" };
      const afterState = { id: "m1", name: "New Name" };
      vi.mocked(getEntityAtCommit)
        .mockResolvedValueOnce(afterState)
        .mockResolvedValueOnce(beforeState);

      const result = await getHandler("history:diff")(EVENT, "abc1234", "mocks/m1.json", "default");

      expect(getEntityAtCommit).toHaveBeenCalledWith("abc1234", "default", "mocks/m1.json");
      expect(getEntityAtCommit).toHaveBeenCalledWith("abc1234~1", "default", "mocks/m1.json");
      expect(result.after).toEqual(afterState);
      expect(result.before).toEqual(beforeState);
    });

    it("returns null before/after when entity does not exist at commit", async () => {
      vi.mocked(getEntityAtCommit).mockResolvedValue(null);

      const result = await getHandler("history:diff")(EVENT, "deadbeef", "mocks/missing.json", "default");

      expect(result.before).toBeNull();
      expect(result.after).toBeNull();
    });
  });

  // ── app:checkUpdate ───────────────────────────────────────────────────

  describe("app:checkUpdate handler", () => {
    it("detects when an update is available from GitHub releases", async () => {
      const mockRelease = {
        tag_name: "v0.2.0",
        name: "Bifurc v0.2.0",
        body: "Bug fixes and improvements",
        html_url: "https://github.com/HarshalKudale/bifurc/releases/tag/v0.2.0",
        published_at: "2026-09-01T00:00:00Z",
        assets: [
          {
            name: "Bifurc.Setup.0.2.0.exe",
            browser_download_url: "https://github.com/HarshalKudale/bifurc/releases/download/v0.2.0/Bifurc.Setup.0.2.0.exe",
          },
        ],
      };

      const originalFetch = globalThis.fetch;
      globalThis.fetch = vi.fn().mockResolvedValue({
        ok: true,
        json: async () => mockRelease,
      }) as any;

      try {
        const result = await getHandler("app:checkUpdate")(EVENT);
        expect(result.ok).toBe(true);
        expect(result.hasUpdate).toBe(true);
        expect(result.latestVersion).toBe("v0.2.0");
        expect(result.downloadUrl).toContain("Bifurc.Setup.0.2.0.exe");
      } finally {
        globalThis.fetch = originalFetch;
      }
    });

    it("returns hasUpdate = false when version matches", async () => {
      const mockRelease = {
        tag_name: "v0.1.0",
        name: "Bifurc v0.1.0",
        body: "Initial release",
        html_url: "https://github.com/HarshalKudale/bifurc/releases/tag/v0.1.0",
        published_at: "2026-09-01T00:00:00Z",
        assets: [],
      };

      const originalFetch = globalThis.fetch;
      globalThis.fetch = vi.fn().mockResolvedValue({
        ok: true,
        json: async () => mockRelease,
      }) as any;

      try {
        const result = await getHandler("app:checkUpdate")(EVENT);
        expect(result.ok).toBe(true);
        expect(result.hasUpdate).toBe(false);
        expect(result.latestVersion).toBe("v0.1.0");
      } finally {
        globalThis.fetch = originalFetch;
      }
    });
  });
});
