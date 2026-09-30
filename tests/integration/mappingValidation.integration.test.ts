import { describe, it, expect, beforeAll, beforeEach, afterEach, vi } from "vitest";

// The engine is where a mapping's rules live: a CLI or any other client must hit the same wall the Mappings
// panel does. These drive `entity.create` / `entity.update` against a real temp workspace.

vi.mock("electron", () => ({
  ipcMain: { handle: vi.fn(), on: vi.fn() },
  BrowserWindow: { getAllWindows: vi.fn(() => []) },
  app: { getPath: vi.fn(() => "/tmp/test-user-data"), on: vi.fn(), whenReady: vi.fn(() => Promise.resolve()) },
}));

vi.mock("@bifurc/engine/proxy/server", () => ({
  reloadConfig: vi.fn(),
  startServer: vi.fn(),
  stopServer: vi.fn(),
  isRunning: vi.fn(() => false),
  getPort: vi.fn(() => 80),
  getServerError: vi.fn(() => null),
}));

import { createWorkspace, mappingFixture, TEST_WS, type WorkspaceFixture } from "./proxyHarness";
import { loadConfig } from "@bifurc/engine/workspace/config";
import { bus } from "@bifurc/engine/kernel/eventBus";
import { commandRegistry } from "@bifurc/engine/kernel/registry";
import { registerCrudKinds, validateMapping } from "@bifurc/engine/workspace/entities/kinds/crudKinds";
import { registerEntityCrudCommands } from "@bifurc/engine/workspace/entities/entityCrudCommands";

const ctx = { bus };

const create = (entity: Record<string, unknown>) =>
  commandRegistry.invoke("entity.create", { kind: "mappings", entity, workspaceId: TEST_WS }, ctx);
const update = (entity: Record<string, unknown>) =>
  commandRegistry.invoke("entity.update", { kind: "mappings", entity, workspaceId: TEST_WS }, ctx);
const domains = () => loadConfig().mappings.map((m) => m.domain).sort();

let ws: WorkspaceFixture;

beforeAll(() => {
  registerCrudKinds();
  if (!commandRegistry.isRegistered("entity.create")) registerEntityCrudCommands(commandRegistry);
});

beforeEach(() => {
  ws = createWorkspace({ mappings: [mappingFixture({ id: "map-1", domain: "app.localhost" })] });
});

afterEach(() => ws.cleanup());

describe("entity.create — mappings", () => {
  it("stores a valid mapping", async () => {
    await create({ domain: "api.localhost", target: "127.0.0.1:4000", label: "", enabled: true });
    expect(domains()).toEqual(["api.localhost", "app.localhost"]);
  });

  it.each([
    ["a duplicate domain, whatever its case", { domain: "APP.localhost", target: "127.0.0.1:4000" }, "domain"],
    ["a domain outside .localhost", { domain: "api.example.com", target: "127.0.0.1:4000" }, "domain"],
    ["a malformed label", { domain: "-api.localhost", target: "127.0.0.1:4000" }, "domain"],
    ["a target without a port", { domain: "api.localhost", target: "127.0.0.1" }, "target"],
    ["a port out of range", { domain: "api.localhost", target: "127.0.0.1:70000" }, "target"],
  ])("refuses %s, naming the field, and stores nothing", async (_label, entity, field) => {
    await expect(create({ label: "", enabled: true, ...entity })).rejects.toMatchObject({
      code: "BAD_REQUEST",
      details: { field },
    });
    expect(domains()).toEqual(["app.localhost"]);
  });
});

describe("entity.update — mappings", () => {
  it("lets a mapping keep its own domain", async () => {
    await update(mappingFixture({ id: "map-1", domain: "app.localhost", target: "127.0.0.1:5000", workspaceId: TEST_WS }));
    expect(loadConfig().mappings.find((m) => m.id === "map-1")?.target).toBe("127.0.0.1:5000");
  });

  it("refuses to rename a mapping onto another mapping's domain", async () => {
    await create({ domain: "api.localhost", target: "127.0.0.1:4000", label: "", enabled: true });
    await expect(
      update(mappingFixture({ id: "map-1", domain: "api.localhost", workspaceId: TEST_WS })),
    ).rejects.toMatchObject({ code: "BAD_REQUEST", details: { field: "domain" } });
    expect(domains()).toEqual(["api.localhost", "app.localhost"]);
  });
});

describe("validateMapping()", () => {
  it("accepts the two target forms the upstream dials: host:port and an http(s) URL", () => {
    for (const target of ["localhost:3000", "10.0.0.2:8443", "http://127.0.0.1:3000", "https://dev.example.com/api"]) {
      expect(() => validateMapping(mappingFixture({ domain: "x.localhost", target }), [])).not.toThrow();
    }
  });

  it("accepts nested subdomains", () => {
    expect(() => validateMapping(mappingFixture({ domain: "v2.api.localhost" }), [])).not.toThrow();
  });
});
