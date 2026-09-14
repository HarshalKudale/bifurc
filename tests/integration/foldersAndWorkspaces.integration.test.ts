/**
 * Happy-path workflow test for **folder management** and **workspace CRUD** — two
 * features that previously existed only as mocked unit tests, so nothing proved they
 * touched the disk correctly.
 *
 * Folder management matters because every panel that groups entities (Mocks, Requests,
 * Proxy Rules, WebSocket, Webhooks, GraphQL, SOAP, gRPC) is built on it, and the folder
 * tree is derived from the **filesystem index**, not from `app.json`. A folder that is
 * created in config but not on disk (or vice versa) silently breaks the panel.
 *
 * Everything here drives the REAL registered IPC handlers against a REAL workspace on
 * disk and then asserts the on-disk result: directories, `index.json`, and the entity
 * files inside the folder. Only Electron itself and `@/main` are mocked.
 *
 * Runs in the integration project (shared process-global data-root override ⇒
 * `fileParallelism: false`).
 */

import { describe, it, expect, beforeAll, afterAll, vi } from "vitest";
import * as fs from "fs";
import * as path from "path";
import { createWorkspace, TEST_WS, type WorkspaceFixture } from "./proxyHarness";

// ── Capture the handlers the features register ────────────────────────────────

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

vi.mock("@/main", () => ({ updateTrayMenu: vi.fn() }));

function invoke<T = any>(channel: string, ...args: any[]): T {
  const fn = handlers.get(channel);
  if (!fn) throw new Error(`No handler registered for "${channel}"`);
  return fn({}, ...args) as T;
}

// ── Fixture helpers ───────────────────────────────────────────────────────────

let ws: WorkspaceFixture;

/** Absolute path inside the active workspace. */
function wsPath(...parts: string[]): string {
  return path.join(ws.dataRoot, TEST_WS, ...parts);
}

function exists(p: string): boolean {
  return fs.existsSync(p);
}

function readIndex(kind: string): { folders: any[]; order: string[] } {
  const file = wsPath(kind, "index.json");
  if (!exists(file)) return { folders: [], order: [] };
  return JSON.parse(fs.readFileSync(file, "utf-8"));
}

function addFolder(kind: string, name: string, parentId: string | null = null, workspaceId = TEST_WS) {
  return invoke("folder:add", kind, { name, parentId, workspaceId });
}

beforeAll(async () => {
  ws = createWorkspace();

  const { registerCoreHandlers } = await import("@/ipc/handlers/coreHandlers");
  const { registerFolderHandlers } = await import("@/ipc/handlers/folderHandlers");
  const { registerCrudHandlers } = await import("@/ipc/handlers/crudHandlers");

  registerCoreHandlers();
  registerFolderHandlers();
  registerCrudHandlers();
});

afterAll(() => {
  ws?.cleanup();
});

// ── Folder management ─────────────────────────────────────────────────────────

describe("folder:add", () => {
  it("returns a folder with an id, timestamp and workspace", async () => {
    const folder = await addFolder("mock", "Auth Mocks");

    expect(folder.id).toBeTruthy();
    expect(folder.name).toBe("Auth Mocks");
    expect(folder.workspaceId).toBe(TEST_WS);
    expect(typeof folder.createdAt).toBe("number");
  });

  it("creates the directory on disk", async () => {
    await addFolder("mock", "Payment Mocks");
    expect(exists(wsPath("mocks", "Payment Mocks"))).toBe(true);
    expect(fs.statSync(wsPath("mocks", "Payment Mocks")).isDirectory()).toBe(true);
  });

  it("registers the folder in the filesystem index and in loadConfig()", async () => {
    const folder = await addFolder("mock", "Search Mocks");

    expect(readIndex("mocks").folders.map((f) => f.id)).toContain(folder.id);
    const cfg = await invoke("config:get");
    expect(cfg.mockFolders.map((f: any) => f.id)).toContain(folder.id);
  });

  it("supports nesting via parentId", async () => {
    const parent = await addFolder("request", "Parent");
    const child = await addFolder("request", "Child", parent.id);

    expect(child.parentId).toBe(parent.id);
    const inIndex = readIndex("requests").folders.find((f) => f.id === child.id);
    expect(inIndex?.parentId).toBe(parent.id);
  });

  it("uses the right directory for each folder kind", async () => {
    await addFolder("request", "Req Folder");
    await addFolder("rule", "Rule Folder");
    await addFolder("graphqlRequest", "GQL Folder");
    await addFolder("soapMock", "SOAP Folder");

    expect(exists(wsPath("requests", "Req Folder"))).toBe(true);
    expect(exists(wsPath("rules", "Rule Folder"))).toBe(true);
    expect(exists(wsPath("graphqlRequests", "GQL Folder"))).toBe(true);
    expect(exists(wsPath("soapMocks", "SOAP Folder"))).toBe(true);
  });

  it("sanitises characters that are illegal in a path", async () => {
    await addFolder("mock", "a/b:c*d");
    expect(exists(wsPath("mocks", "abcd"))).toBe(true);
  });

  it("falls back to 'unnamed' when the name sanitises to nothing", async () => {
    await addFolder("mock", "///");
    expect(exists(wsPath("mocks", "unnamed"))).toBe(true);
  });
});

describe("folder:rename", () => {
  it("renames both the config entry and the directory on disk", async () => {
    const folder = await addFolder("mock", "Before Rename");

    await invoke("folder:rename", "mock", folder.id, "After Rename");

    expect(exists(wsPath("mocks", "Before Rename"))).toBe(false);
    expect(exists(wsPath("mocks", "After Rename"))).toBe(true);

    const cfg = await invoke("config:get");
    expect(cfg.mockFolders.find((f: any) => f.id === folder.id)?.name).toBe("After Rename");
    expect(readIndex("mocks").folders.find((f) => f.id === folder.id)?.name).toBe("After Rename");
  });

  it("keeps the entities that lived inside the renamed folder", async () => {
    const folder = await addFolder("request", "Keep Me");

    await invoke("request:add", {
      name: "R1", method: "GET", url: "http://example.test/a", folderId: folder.id, workspaceId: TEST_WS,
    });
    expect(exists(wsPath("requests", "Keep Me"))).toBe(true);

    await invoke("folder:rename", "request", folder.id, "Renamed Me");

    const files = fs.readdirSync(wsPath("requests", "Renamed Me")).filter((f) => f.endsWith(".json"));
    expect(files).toHaveLength(1);
  });
});

describe("folder:move", () => {
  it("records the new parent in config and in the index", async () => {
    const parent = await addFolder("mock", "Move Parent");
    const child = await addFolder("mock", "Move Child");

    await invoke("folder:move", "mock", child.id, parent.id);

    const cfg = await invoke("config:get");
    expect(cfg.mockFolders.find((f: any) => f.id === child.id)?.parentId).toBe(parent.id);
    expect(readIndex("mocks").folders.find((f) => f.id === child.id)?.parentId).toBe(parent.id);
  });

  it("can move a folder back to the root with a null parent", async () => {
    const parent = await addFolder("mock", "Root Parent");
    const child = await addFolder("mock", "Back To Root", parent.id);

    await invoke("folder:move", "mock", child.id, null);

    expect(readIndex("mocks").folders.find((f) => f.id === child.id)?.parentId).toBeNull();
  });
});

describe("folder:delete", () => {
  it("removes the folder from config, the index and the disk", async () => {
    const folder = await addFolder("mock", "Doomed");

    await invoke("folder:delete", "mock", folder.id);

    expect(exists(wsPath("mocks", "Doomed"))).toBe(false);
    expect(readIndex("mocks").folders.map((f) => f.id)).not.toContain(folder.id);

    const cfg = await invoke("config:get");
    expect(cfg.mockFolders.map((f: any) => f.id)).not.toContain(folder.id);
  });

  it("also removes the entities that lived in the folder", async () => {
    const folder = await addFolder("request", "Temp Requests");

    const created = await invoke("request:add", {
      name: "Doomed Request", method: "GET", url: "http://example.test/doomed",
      folderId: folder.id, workspaceId: TEST_WS,
    });
    expect(exists(wsPath("requests", "Temp Requests", `${created.id}.json`))).toBe(true);

    await invoke("folder:delete", "request", folder.id);

    expect(exists(wsPath("requests", "Temp Requests"))).toBe(false);
    expect((await invoke("entity:load", TEST_WS, "requests", created.id)).ok).toBe(false);

    const cfg = await invoke("config:get");
    expect(cfg.requests.map((r: any) => r.id)).not.toContain(created.id);
  });

  it("drops the folder's entities from the index order too", async () => {
    const folder = await addFolder("mock", "Order Test");

    await invoke("mock:add", {
      name: "Ordered Mock", method: "GET", urlPattern: "/ordered", folderId: folder.id, workspaceId: TEST_WS,
    });

    await invoke("folder:delete", "mock", folder.id);

    const idx = readIndex("mocks");
    expect(idx.folders.map((f) => f.id)).not.toContain(folder.id);
    expect(idx.order).toHaveLength(0);
  });

  it("is a no-op for an unknown folder id", async () => {
    await expect(invoke("folder:delete", "mock", "no-such-folder")).resolves.toEqual({ ok: true });
  });
});

// ── Workspace CRUD ────────────────────────────────────────────────────────────

describe("workspace:add", () => {
  it("registers the workspace and creates its directory tree on disk", async () => {
    const created = await invoke("workspace:add", "Second Workspace");

    expect(created.id).toBeTruthy();
    expect(created.name).toBe("Second Workspace");

    const cfg = await invoke("config:get");
    expect(cfg.workspaces.map((w: any) => w.id)).toContain(created.id);

    for (const dir of ["mappings", "rules", "mocks", "requests", "environments"]) {
      expect(exists(path.join(ws.dataRoot, created.id, dir)), `${dir} should exist`).toBe(true);
    }
  });

  it("generates a name when given a blank one", async () => {
    const created = await invoke("workspace:add", "   ");
    expect(created.name.trim().length).toBeGreaterThan(0);
  });
});

describe("workspace:rename", () => {
  it("updates the name in the config", async () => {
    const created = await invoke("workspace:add", "Rename Me");
    await invoke("workspace:rename", created.id, "Renamed Workspace");

    const cfg = await invoke("config:get");
    expect(cfg.workspaces.find((w: any) => w.id === created.id)?.name).toBe("Renamed Workspace");
  });

  it("keeps the old name when the new one is blank", async () => {
    const created = await invoke("workspace:add", "Keep This Name");
    await invoke("workspace:rename", created.id, "   ");

    const cfg = await invoke("config:get");
    expect(cfg.workspaces.find((w: any) => w.id === created.id)?.name).toBe("Keep This Name");
  });
});

describe("workspace:setActive", () => {
  it("switches the active workspace and returns the new config", async () => {
    const created = await invoke("workspace:add", "Switch Target");

    const res = await invoke("workspace:setActive", created.id);

    expect(res.ok).toBe(true);
    expect(res.config.activeWorkspaceId).toBe(created.id);
    expect((await invoke("config:get")).activeWorkspaceId).toBe(created.id);

    // Put the fixture workspace back so later assertions are unambiguous.
    await invoke("workspace:setActive", TEST_WS);
  });

  it("refuses an unknown workspace id", async () => {
    expect(await invoke("workspace:setActive", "no-such-workspace")).toEqual({ ok: false });
  });

  it("scopes new entities to the newly active workspace", async () => {
    const created = await invoke("workspace:add", "Scoped Workspace");
    await invoke("workspace:setActive", created.id);

    const entity = await invoke("request:add", {
      name: "Scoped Request", method: "GET", url: "http://example.test/scoped", folderId: null,
    });

    expect(exists(path.join(ws.dataRoot, created.id, "requests", `${entity.id}.json`))).toBe(true);
    expect(exists(wsPath("requests", `${entity.id}.json`))).toBe(false);

    await invoke("workspace:setActive", TEST_WS);
  });
});

describe("workspace:delete", () => {
  it("removes the workspace from the config", async () => {
    const created = await invoke("workspace:add", "Delete Me");
    await invoke("workspace:delete", created.id);

    const cfg = await invoke("config:get");
    expect(cfg.workspaces.map((w: any) => w.id)).not.toContain(created.id);
  });

  it("promotes another workspace when the active one is deleted", async () => {
    const created = await invoke("workspace:add", "Active To Delete");
    await invoke("workspace:setActive", created.id);

    await invoke("workspace:delete", created.id);

    const cfg = await invoke("config:get");
    expect(cfg.activeWorkspaceId).not.toBe(created.id);
    expect(cfg.workspaces.map((w: any) => w.id)).toContain(cfg.activeWorkspaceId);
  });

  it("leaves the workspace directory on disk (deleting data is not implied)", async () => {
    // Documented behaviour rather than an endorsement: `workspace:delete` only edits
    // config. Pinning it means turning it into a destructive delete has to be deliberate.
    const created = await invoke("workspace:add", "Files Survive");
    await invoke("workspace:delete", created.id);

    expect(exists(path.join(ws.dataRoot, created.id))).toBe(true);
  });
});
