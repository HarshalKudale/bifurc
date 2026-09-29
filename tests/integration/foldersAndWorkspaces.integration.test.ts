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
 * Everything here drives the REAL machinery behind those features — the surviving registered IPC
 * handlers, and the `folder.*` / `entity.*` commands that replaced the deleted folder and per-kind
 * channels (`DELETED_FOLDER_CHANNELS` / `createEntity` below) — against a REAL workspace on disk, then
 * asserts the on-disk result: directories, `index.json`, and the entity files inside the folder. Only
 * Electron itself and `@/main` are mocked.
 *
 * Runs in the integration project (shared process-global data-root override ⇒
 * `fileParallelism: false`).
 */

import { describe, it, expect, beforeAll, afterAll, vi } from "vitest";
import * as fs from "fs";
import * as path from "path";
import { createWorkspace, TEST_WS, type WorkspaceFixture } from "./proxyHarness";
import { commandRegistry } from "@bifurc/engine/kernel/registry";
import { bus } from "@bifurc/engine/kernel/eventBus";

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

/**
 * The four deleted folder channels, reproduced as commands — the P6 reduction's third group
 * (`plan/07` §5b-quaterdecies).
 *
 * `folder:add` / `folder:rename` / `folder:move` / `folder:delete` were deleted with
 * `src/ipc/handlers/folderHandlers.ts` on 2026-09-21. All four were one-line `commandRegistry.invoke()`
 * adapters, so this table is what the channel did minus the channel — and the mapping is written out
 * rather than derived because **`folder:add` is the one place the two shapes differ**.
 *
 * The channel spread the whole argument (`{ kind, ...folder }`); the client picks `name` and `parentId`
 * only. That difference is inert *for the renderer*, whose contract excludes `workspaceId`
 * (`packages/gui/src/types/window.ts:63`: `Omit<Folder, "id" | "createdAt" | "workspaceId">`) — but this suite's
 * `addFolder()` helper deliberately passes `workspaceId: TEST_WS`, which the spread carried and the
 * client's field-picking would drop. So the translation keeps the spread. Getting this wrong would not
 * fail loudly: it would move the folder into the *active* workspace, which happens to be `TEST_WS` here,
 * so the suite would still pass while testing something else.
 */
const DELETED_FOLDER_CHANNELS: Record<string, (args: any[]) => { command: string; payload: any }> = {
  "folder:add": ([kind, folder]) => ({ command: "folder.add", payload: { kind, ...folder } }),
  "folder:rename": ([kind, id, name]) => ({ command: "folder.rename", payload: { kind, id, name } }),
  "folder:move": ([kind, id, parentId]) => ({ command: "folder.move", payload: { kind, id, parentId } }),
  "folder:delete": ([kind, id]) => ({ command: "folder.delete", payload: { kind, id } }),
};

/**
 * The five `coreHandlers.ts` channels this suite drives — the P6 reduction's **sixth** group
 * (`plan/07` §5b-septendecies, 2026-09-21).
 *
 * Every entry is a *positional* adapter, unlike the folder table's single-object shape, and that is
 * forced by the call sites rather than chosen: this suite invokes the shell's own signature
 * (`invoke("config:get")`, `invoke("workspace:setActive", id)`), so the translation has to turn
 * positionals into each command's **named** payload. Written out one at a time because the argument
 * lists are not what the parameter names suggest, and two of them were got wrong on the first pass:
 *
 *   - **`workspace:add` takes one argument — the name.** Not `(id, name)`: the command generates the id
 *     (`WorkspaceAddParams.id` is optional). Mapping `([id, name])` shifts the name into the `id` slot
 *     and fails as `Invalid payload for command "workspace.add": expected string, received undefined`
 *     on `name`.
 *   - `workspace:rename` / `workspace:delete` / `workspace:setActive` take `(id[, name])`, as the
 *     parameter names do suggest.
 *   - `entity:load` takes `(workspaceId, kind, id)` and is the one case where the names line up — and
 *     also the one where **`kind` is a protocol name** (`"proxyRules"`, `"wsConnections"`) that the
 *     command translates via `toEngineKind`, so it is passed straight through rather than pre-translated.
 *
 * `config:get` is the only entry with no payload at all; `ConfigGetParams` is an empty strict object, so
 * `{}` is the contract rather than a convention.
 *
 * **This table is not a second contract.** `src/preload.ts` is the authority for what the renderer
 * sends; this is a test-local translation standing in for channels that no longer exist. Re-derive it
 * from the call sites — never from this comment — if another group reaches this suite.
 */
const DELETED_CORE_CHANNELS: Record<string, (args: any[]) => { command: string; payload: any }> = {
  "config:get": () => ({ command: "config.get", payload: {} }),
  "workspace:add": ([name]) => ({ command: "workspace.add", payload: { name } }),
  "workspace:rename": ([id, name]) => ({ command: "workspace.rename", payload: { id, name } }),
  "workspace:delete": ([id]) => ({ command: "workspace.delete", payload: { id } }),
  "workspace:setActive": ([id]) => ({ command: "workspace.setActive", payload: { id } }),
  "entity:load": ([workspaceId, kind, id]) => ({ command: "entity.load", payload: { workspaceId, kind, id } }),
};

function invoke<T = any>(channel: string, ...args: any[]): T {
  const folder = DELETED_FOLDER_CHANNELS[channel];
  if (folder) {
    const { command, payload } = folder(args);
    return commandRegistry.invoke(command as never, payload, { bus }) as T;
  }
  const core = DELETED_CORE_CHANNELS[channel];
  if (core) {
    const { command, payload } = core(args);
    return commandRegistry.invoke(command as never, payload, { bus }) as T;
  }
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

/**
 * The collapsed `entity.create`, driven directly — the per-kind `request:add` / `mock:add` channels are
 * the step-3 reduction's target (`plan/07` §5b-ter) and were only one-line `commandRegistry.invoke()`
 * adapters, so this is what the channel did minus the channel.
 *
 * Two details that are load-bearing, not incidental:
 *  - `{id, entity}` is **unwrapped**, because the channel unwrapped it (`entityCrudFactory.ts:72`) and
 *    the cases below read the created entity's `id` to find its file on disk.
 *  - `workspaceId` is passed through **as-is** rather than defaulted to `TEST_WS`. The channel sent
 *    `entity.workspaceId` (often `undefined`), and `entity.create` then falls back to the *active*
 *    workspace. The scoped-workspace case below asserts exactly that — the entity must land in the
 *    switched-to workspace, so defaulting here would make the assertion vacuous.
 */
const createEntity = async <T = any>(kind: string, entity: any): Promise<T> => {
  const r: any = await commandRegistry.invoke(
    "entity.create",
    { kind, entity, workspaceId: entity.workspaceId },
    { bus },
  );
  return ("entity" in r ? r.entity : r) as T;
};

beforeAll(async () => {
  ws = createWorkspace();

  const { registerCoreKinds } = await import("@bifurc/engine/workspace/entities/kinds/coreKinds");
  const { registerCrudKinds } = await import("@bifurc/engine/workspace/entities/kinds/crudKinds");
  // P6 §5c: `config.get`, `entity.load` and `workspace.setActive` moved out of `coreHandlers`' module
  // scope into the engine. The reduction's sixth group then deleted that file's channels and renamed it
  // to `coreKinds.ts` (`plan/07` §5b-septendecies), so `registerCoreKinds()` is the shell's
  // `environments` kind alone — a suite that registers handler groups by hand still has to wire the
  // commands, which is the `registerCoreCommands()` call below.
  const { commandRegistry } = await import("@bifurc/engine/kernel/registry");
  const { registerCoreCommands } = await import("@bifurc/engine/workspace/entities/coreCommands");
  // P6 §5c slice 3: the four `folder.*` commands left `folderHandlers.ts`'s module scope. Every channel
  // in that file *delegated* through `commandRegistry.invoke()`, so the channel was only ever as alive as
  // this registry. **The channels are now deleted** (P6 reduction, third group, 2026-09-21), so
  // `DELETED_FOLDER_CHANNELS` invokes this registry directly and `registerFolderCommands()` is the only
  // registration the four need.
  const { registerFolderCommands } = await import("@bifurc/engine/workspace/entities/folderCommands");
  // P6 §5c slice 7: the four collapsed `entity.*` commands left `entityCrudFactory.ts`'s module scope.
  // `registerCrudKinds()` wires the per-kind options those four commands dispatch through. It registers
  // no channels — the reduction's fourth group took the file's last five on 2026-09-21.
  const { registerEntityCrudCommands } = await import("@bifurc/engine/workspace/entities/entityCrudCommands");
  // The `workspace.*` commands this suite drives nine times come from `configCommands`, and until the
  // sixth group this suite never had to wire it: `coreHandlers.ts`'s `workspace:add` channel registered
  // them as a side effect of existing. Deleting the channel took the registration with it, and the
  // failure is `No handler registered for command "workspace.add"` — a *routed-surface* error, i.e. the
  // envelope reached the registry and the registry had nothing to dispatch to.
  //
  // The general rule, now seen four times in one group (spike → `registerServerCommands`; protocolExecution
  // → `registerMiscCommands` + `registerConfigCommands`; settingsMutations → `registerConfigCommands`):
  // **a command whose only registration came from a handler file the reduction deleted must be wired
  // explicitly by every suite that drives it.** Compare against `src/ipc/handlers.ts`'s `register*Commands()`
  // list rather than against the suite's own history.
  const { registerConfigCommands } = await import("@bifurc/engine/workspace/entities/configCommands");

  registerCoreKinds();
  registerCoreCommands(commandRegistry);
  registerFolderCommands(commandRegistry);
  registerConfigCommands(commandRegistry);
  registerEntityCrudCommands(commandRegistry);
  registerCrudKinds();
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
    // `soapMock` stood here until 2026-09-25, when `plan/17` work item 8 removed the three mock
    // folder kinds. `soapRequest` is the replacement: the same protocol family, and its `fsKind`
    // (`soapRequests`) is distinct from `requests`, so this case still discriminates the table
    // rather than duplicating the `request` case above.
    await addFolder("soapRequest", "SOAP Folder");

    expect(exists(wsPath("requests", "Req Folder"))).toBe(true);
    expect(exists(wsPath("rules", "Rule Folder"))).toBe(true);
    expect(exists(wsPath("graphqlRequests", "GQL Folder"))).toBe(true);
    expect(exists(wsPath("soapRequests", "SOAP Folder"))).toBe(true);
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

    await createEntity("requests", {
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

    const created = await createEntity("requests", {
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

    await createEntity("mocks", {
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

    const entity = await createEntity("requests", {
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
