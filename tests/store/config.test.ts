import { describe, it, expect, vi, beforeEach } from "vitest";

// config.ts is now an adapter over appSettings + workspaceFs.
// We mock those two dependencies so tests don't touch the filesystem.

vi.mock("../../src/store/appSettings", () => ({
  loadSettings: vi.fn(),
  saveSettings: vi.fn(),
}));

vi.mock("../../src/store/workspaceFs", () => ({
  readAllEntities: vi.fn(() => []),
  readEntityStubs: vi.fn(() => []),
  readIndex: vi.fn(() => ({ folders: [], order: [] })),
  autoSyncFsDirectories: vi.fn(() => ({ folders: [], order: [] })),
  writeEntity: vi.fn(),
  writeFlatEntity: vi.fn(),
  deleteFlatEntityFile: vi.fn(),
  deleteEntityFile: vi.fn(),
  initWorkspaceDir: vi.fn(),
  dataRoot: vi.fn(() => "/tmp/test-user-data/data"),
  wsDir: vi.fn((wsId: string) => `/tmp/test-user-data/data/${wsId}`),
  readEnabledSet: vi.fn(() => null),
  bootstrapEnabledSet: vi.fn(() => new Set<string>()),
  readNamesIndex: vi.fn(() => ({})),
  bootstrapNamesIndex: vi.fn(() => ({})),
  upsertNameEntry: vi.fn(),
  removeNameEntry: vi.fn(),
  sanitizeDirName: vi.fn((n: string) => n),
  getPendingDeletions: vi.fn(() => []),
  addPendingDeletion: vi.fn(),
  removePendingDeletion: vi.fn(),
  clearPendingDeletions: vi.fn(),
}));

// electron is mocked globally in tests/setup.ts

import { loadSettings, saveSettings } from "@/store/appSettings";
import { readAllEntities, readIndex, autoSyncFsDirectories } from "@/store/workspaceFs";

const makeDefaultSettings = () => ({
  port: 80,
  minimizeToTray: true,
  workspaces: [{ id: "default", name: "Workspace 1", activeEnvironmentId: null }],
  activeWorkspaceId: "default",
});

describe("src/store/config.ts", () => {
  let loadConfig: () => import("../../src/store/config").AppConfig;
  let saveConfig: (cfg: import("../../src/store/config").AppConfig) => void;
  let generateId: () => string;

  beforeEach(async () => {
    vi.clearAllMocks();
    vi.resetModules();

    vi.mocked(loadSettings).mockReturnValue(makeDefaultSettings());
    vi.mocked(readAllEntities).mockReturnValue([]);
    vi.mocked(readIndex).mockReturnValue({ folders: [], order: [] });

    const mod = await import("../../src/store/config");
    loadConfig = mod.loadConfig;
    saveConfig = mod.saveConfig;
    generateId = mod.generateId;
  });

  // ── loadConfig ──────────────────────────────────────────────────────────

  describe("loadConfig()", () => {
    it("returns config with default port from settings", () => {
      const cfg = loadConfig();
      expect(cfg.port).toBe(80);
      expect(cfg.minimizeToTray).toBe(true);
    });

    it("assembles empty arrays when workspace has no entities", () => {
      const cfg = loadConfig();
      expect(cfg.mappings).toEqual([]);
      expect(cfg.proxyRules).toEqual([]);
      expect(cfg.mocks).toEqual([]);
      expect(cfg.requests).toEqual([]);
      expect(cfg.environments).toEqual([
        expect.objectContaining({
          id: "__global__",
          name: "Global",
          variables: [],
          workspaceId: "default",
        }),
      ]);
      expect(cfg.mockFolders).toEqual([]);
      expect(cfg.requestFolders).toEqual([]);
      expect(cfg.wsFolders).toEqual([]);
      expect(cfg.activeEnvironmentId).toBeNull();
    });

    it("includes workspaces from settings", () => {
      const cfg = loadConfig();
      expect(cfg.workspaces).toHaveLength(1);
      expect(cfg.workspaces[0].id).toBe("default");
      expect(cfg.workspaces[0].name).toBe("Workspace 1");
    });

    it("sets activeWorkspaceId from settings", () => {
      const cfg = loadConfig();
      expect(cfg.activeWorkspaceId).toBe("default");
    });

    it("reads mocks from the active workspace", () => {
      const mocks = [{ id: "m1", name: "My Mock", workspaceId: "default" }];
      vi.mocked(readAllEntities).mockImplementation((wsId, kind) => kind === "mocks" ? mocks as any : []);
      const cfg = loadConfig();
      expect(cfg.mocks).toHaveLength(1);
      expect(cfg.mocks[0].id).toBe("m1");
    });

    it("reads folders from the index file", () => {
      const folders = [{ id: "f1", name: "Auth", parentId: null, createdAt: 0, workspaceId: "default" }];
      vi.mocked(autoSyncFsDirectories).mockReturnValue({ folders, order: [] });
      const cfg = loadConfig();
      expect(cfg.mockFolders).toHaveLength(1);
      expect(cfg.mockFolders[0].id).toBe("f1");
    });

    it("propagates activeEnvironmentId from workspace meta", () => {
      vi.mocked(loadSettings).mockReturnValue({
        ...makeDefaultSettings(),
        workspaces: [{ id: "default", name: "W1", activeEnvironmentId: "env-99" }],
      });
      const cfg = loadConfig();
      expect(cfg.activeEnvironmentId).toBe("env-99");
    });
  });

  // ── saveConfig ──────────────────────────────────────────────────────────

  describe("saveConfig()", () => {
    it("calls saveSettings with updated port and minimizeToTray", () => {
      const cfg = loadConfig();
      saveConfig({ ...cfg, port: 9090, minimizeToTray: false });
      expect(saveSettings).toHaveBeenCalledWith(
        expect.objectContaining({ port: 9090, minimizeToTray: false }),
      );
    });

    it("syncs workspace metadata into settings", () => {
      const cfg = loadConfig();
      cfg.workspaces[0].name = "Renamed WS";
      saveConfig(cfg);
      const saved = vi.mocked(saveSettings).mock.calls[0][0];
      expect(saved.workspaces[0].name).toBe("Renamed WS");
    });

    it("removes workspaces that were deleted", () => {
      vi.mocked(loadSettings).mockReturnValue({
        ...makeDefaultSettings(),
        workspaces: [
          { id: "ws1", name: "One", activeEnvironmentId: null },
          { id: "ws2", name: "Two", activeEnvironmentId: null },
        ],
      });
      const cfg = loadConfig();
      cfg.workspaces = cfg.workspaces.filter((w) => w.id !== "ws2");
      saveConfig(cfg);
      const saved = vi.mocked(saveSettings).mock.calls[0][0];
      expect(saved.workspaces).toHaveLength(1);
      expect(saved.workspaces[0].id).toBe("ws1");
    });
  });

  // ── generateId ──────────────────────────────────────────────────────────

  describe("generateId()", () => {
    it("returns a non-empty string", () => {
      const id = generateId();
      expect(typeof id).toBe("string");
      expect(id.length).toBeGreaterThan(0);
    });

    it("returns unique values on successive calls", () => {
      const ids = new Set(Array.from({ length: 20 }, () => generateId()));
      expect(ids.size).toBe(20);
    });

    it("uses base-36 characters only", () => {
      const id = generateId();
      expect(id).toMatch(/^[0-9a-z]+$/);
    });
  });
});
