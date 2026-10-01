/**
 * `entity.duplicate` — a copy of a saved entity, made through `entity.create` so validation, events and
 * file writes are the ones a normal create has. Real workspace on disk, real per-kind options; only
 * Electron and the proxy's `reloadConfig` are mocked, as in `entityConflictScope.integration.test.ts`.
 */

import { describe, it, expect, beforeAll, beforeEach, afterEach, vi } from "vitest";

vi.mock("electron", () => ({
  ipcMain: { handle: vi.fn(), on: vi.fn() },
  BrowserWindow: { getAllWindows: vi.fn(() => []) },
  app: {
    getPath: vi.fn(() => "/tmp/test-user-data"),
    on: vi.fn(),
    whenReady: vi.fn(() => Promise.resolve()),
    commandLine: { appendSwitch: vi.fn() },
    quit: vi.fn(),
  },
}));

vi.mock("@bifurc/engine/proxy/server", () => ({
  reloadConfig: vi.fn(),
  startServer: vi.fn(),
  stopServer: vi.fn(),
  isRunning: vi.fn(() => false),
  getPort: vi.fn(() => 80),
  getServerError: vi.fn(() => null),
}));

import { createWorkspace, mockFixture, TEST_WS, type WorkspaceFixture } from "./proxyHarness";
import { loadConfig } from "@bifurc/engine/workspace/config";
import { bus } from "@bifurc/engine/kernel/eventBus";
import { commandRegistry } from "@bifurc/engine/kernel/registry";
import { registerCrudKinds } from "@bifurc/engine/workspace/entities/kinds/crudKinds";
import { registerCoreCommands } from "@bifurc/engine/workspace/entities/coreCommands";
import { registerEntityCrudCommands } from "@bifurc/engine/workspace/entities/entityCrudCommands";

const ctx = { bus };

const create = async (kind: string, entity: Record<string, unknown>) =>
  (await commandRegistry.invoke("entity.create", { kind, entity, workspaceId: TEST_WS }, ctx) as { entity: any }).entity;

const duplicate = (kind: string, id: string) =>
  commandRegistry.invoke("entity.duplicate", { workspaceId: TEST_WS, kind, id }, ctx) as Promise<{ entity: any }>;

let ws: WorkspaceFixture;

beforeAll(() => {
  registerCoreCommands(commandRegistry);
  registerCrudKinds();
  registerEntityCrudCommands(commandRegistry);
});

beforeEach(() => {
  ws = createWorkspace();
});

afterEach(() => {
  ws.cleanup();
});

describe("entity.duplicate", () => {
  it("copies a REST request under a new id, named '<name> copy'", async () => {
    const original = await create("requests", {
      name: "X", method: "POST", url: "http://example.test/x", folderId: "folder-1", headers: { a: "b" },
    });

    const { entity: copy } = await duplicate("requests", original.id);

    expect(copy.id).toBeTruthy();
    expect(copy.id).not.toBe(original.id);
    expect(copy.name).toBe("X copy");
    expect(copy.folderId).toBe("folder-1");
    expect(copy.url).toBe("http://example.test/x");
    expect(copy.method).toBe("POST");
    expect(copy.headers).toEqual({ a: "b" });

    const stored = loadConfig().requests;
    expect(stored.map((r: any) => r.id).sort()).toEqual([original.id, copy.id].sort());
    expect(stored.find((r: any) => r.id === original.id).name).toBe("X");
  });

  it("fails for an id that does not exist", async () => {
    await expect(duplicate("requests", "missing")).rejects.toThrow(/No requests entity "missing"/);
  });

  it("duplicates a mock", async () => {
    const original = await create("mocks", mockFixture({ name: "M", method: "GET", urlPattern: "/m", capturedBody: "" }));

    const { entity: copy } = await duplicate("mocks", original.id);

    expect(copy.id).not.toBe(original.id);
    expect(copy.name).toBe("M copy");
    expect(copy.urlPattern).toBe("/m");
    expect(loadConfig().mocks.filter((m: any) => m.name === "M copy")).toHaveLength(1);
  });

  it("creates the copy of a mock switched off and leaves the original on", async () => {
    const original = await create("mocks", mockFixture({ name: "M", method: "GET", urlPattern: "/m", capturedBody: "" }));

    const { entity: copy } = await duplicate("mocks", original.id);

    const mocks = loadConfig().mocks;
    expect(mocks.find((m: any) => m.id === copy.id).enabled).toBe(false);
    expect(mocks.find((m: any) => m.id === original.id).enabled).toBe(true);
  });
});
