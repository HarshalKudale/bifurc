/**
 * P1 acceptance criterion: "Every command has a Zod schema; a smoke test validates a valid and
 * an invalid payload per command." One fixture pair per entry in `COMMANDS`.
 */
import { describe, it, expect } from "vitest";
import { COMMANDS, type CommandAction } from "./index";

const FIXTURES: Record<CommandAction, { valid: unknown; invalid: unknown }> = {
  "entity.create": { valid: { kind: "mocks", entity: { name: "x" } }, invalid: { kind: "not-a-kind", entity: {} } },
  "entity.update": { valid: { kind: "mocks", entity: { id: "m1" } }, invalid: { kind: "mocks" } },
  "entity.delete": { valid: { kind: "mocks", id: "m1" }, invalid: { kind: "mocks" } },
  "entity.load": { valid: { workspaceId: "ws1", kind: "mocks", id: "m1" }, invalid: { kind: "mocks", id: "m1" } },
  "entity.list": { valid: { workspaceId: "ws1", kind: "mocks" }, invalid: { kind: "mocks" } },
  "entity.setEnabled": {
    valid: { workspaceId: "ws1", kind: "mocks", id: "m1", enabled: true },
    invalid: { workspaceId: "ws1", kind: "not-a-kind", id: "m1", enabled: true },
  },

  "folder.add": { valid: { kind: "mock", name: "F", parentId: null }, invalid: { kind: "not-a-kind", name: "F", parentId: null } },
  "folder.rename": { valid: { kind: "mock", id: "f1", name: "F2" }, invalid: { kind: "mock", id: "f1" } },
  "folder.move": { valid: { kind: "mock", id: "f1", parentId: null }, invalid: { kind: "mock", id: "f1" } },
  "folder.delete": { valid: { kind: "mock", id: "f1" }, invalid: { kind: "mock" } },
  "folder.publish": { valid: { workspaceId: "ws1", kind: "mock", folderName: null }, invalid: { workspaceId: "ws1", kind: "mock" } },

  "config.get": { valid: {}, invalid: { extra: true } },
  "config.save": { valid: { config: { port: 80 } }, invalid: {} },
  "workspace.add": { valid: { name: "WS" }, invalid: {} },
  "workspace.rename": { valid: { id: "w1", name: "New" }, invalid: { id: "w1" } },
  "workspace.delete": { valid: { id: "w1" }, invalid: {} },
  "workspace.setActive": { valid: { id: "w1" }, invalid: {} },
  "env.setActive": { valid: { id: null }, invalid: {} },

  "server.status": { valid: {}, invalid: { extra: true } },
  "server.start": { valid: {}, invalid: { extra: true } },
  "server.stop": { valid: {}, invalid: { extra: true } },
  "server.restart": { valid: {}, invalid: { extra: true } },
  "proxy.status": { valid: {}, invalid: { extra: true } },
  "services.discover": { valid: {}, invalid: { extra: true } },

  "graphql.introspect": { valid: { url: "http://x", headers: {} }, invalid: { url: "http://x" } },
  "graphql.execute": {
    valid: { url: "http://x", headers: {}, query: "{}", variables: "{}", operationName: "" },
    invalid: { url: "http://x" },
  },

  "soap.fetchWsdl": { valid: { url: "http://x" }, invalid: {} },
  "soap.execute": {
    valid: { endpointUrl: "http://x", soapAction: "a", headers: {}, body: "<xml/>" },
    invalid: { endpointUrl: "http://x" },
  },

  "grpc.execute": {
    valid: { serverAddress: "a", serviceName: "s", methodName: "m", requestBody: "{}", metadata: {}, protoFileId: null, useReflection: false },
    invalid: { serverAddress: "a" },
  },
  "grpc.reflect": { valid: { serverAddress: "a" }, invalid: {} },
  "grpc.mockServerStatus": { valid: {}, invalid: { extra: true } },
  "grpc.startMockServer": { valid: {}, invalid: { extra: true } },
  "grpc.stopMockServer": { valid: {}, invalid: { extra: true } },

  "application.list": { valid: { workspaceId: "ws1" }, invalid: {} },
  "application.save": { valid: { application: { name: "a" } }, invalid: {} },
  "application.delete": { valid: { workspaceId: "ws1", id: "a1" }, invalid: { workspaceId: "ws1" } },
  "application.start": { valid: { workspaceId: "ws1", appId: "a1", mode: "run" }, invalid: { workspaceId: "ws1", appId: "a1", mode: "bogus" } },
  "application.stop": { valid: { appId: "a1" }, invalid: {} },
  "application.getState": { valid: { appId: "a1" }, invalid: {} },
  "application.getAllStates": { valid: {}, invalid: { extra: true } },
  "application.getLogs": { valid: { appId: "a1" }, invalid: {} },
  "application.checkPort": { valid: { port: 3000 }, invalid: { port: "3000" } },
  "application.killPort": { valid: { port: 3000 }, invalid: { port: "3000" } },

  "webhook.registerActive": { valid: { webhookId: "w1", urlSuffix: "/x" }, invalid: { webhookId: "w1" } },
  "webhook.unregisterActive": { valid: { webhookId: "w1" }, invalid: {} },
  "webhookServer.start": { valid: {}, invalid: { port: "abc" } },
  "webhookServer.stop": { valid: {}, invalid: { extra: true } },
  "webhookServer.status": { valid: {}, invalid: { extra: true } },

  "runner.saveReport": {
    valid: { workspaceId: "ws1", report: { folderId: "f1", startedAt: 1 } },
    invalid: { workspaceId: "ws1", report: {} },
  },
  "runner.exportReport": {
    valid: { report: { folderId: "f1", startedAt: 1 }, format: "html" },
    invalid: { report: { folderId: "f1", startedAt: 1 }, format: "pdf" },
  },
  "runner.getHistory": { valid: { workspaceId: "ws1", folderId: "f1" }, invalid: { workspaceId: "ws1" } },
  "runner.saveConfig": {
    valid: { workspaceId: "ws1", folderId: "f1", config: { requestOrder: [], delayMs: 0 } },
    invalid: { workspaceId: "ws1", folderId: "f1", config: {} },
  },
  "runner.loadConfig": { valid: { workspaceId: "ws1", folderId: "f1" }, invalid: { workspaceId: "ws1" } },
  "runner.listFolderIds": { valid: { workspaceId: "ws1" }, invalid: {} },

  "sync.setRemote": { valid: { workspaceId: "ws1", remote: "r", branch: "main" }, invalid: { workspaceId: "ws1", remote: "r" } },
  "sync.disconnect": { valid: { workspaceId: "ws1" }, invalid: {} },
  "sync.push": { valid: { workspaceId: "ws1" }, invalid: {} },
  "sync.pull": { valid: { workspaceId: "ws1" }, invalid: {} },
  "sync.getState": { valid: { workspaceId: "ws1" }, invalid: {} },
  "sync.setAutoSync": { valid: { workspaceId: "ws1", enabled: true }, invalid: { workspaceId: "ws1", enabled: "yes" } },
  "sync.getEntityStatus": { valid: { workspaceId: "ws1" }, invalid: {} },
  "git.diff": { valid: { workspaceId: "ws1", relPath: "a/b" }, invalid: { workspaceId: "ws1" } },
  "git.discard": { valid: { workspaceId: "ws1", relPath: "a/b" }, invalid: { workspaceId: "ws1" } },
  "git.sync": { valid: { workspaceId: "ws1", paths: ["a"] }, invalid: { workspaceId: "ws1" } },
  "git.history": { valid: { workspaceId: "ws1", relPath: "a/b" }, invalid: { workspaceId: "ws1" } },
  "entity.publish": { valid: { workspaceId: "ws1", paths: ["a"] }, invalid: { workspaceId: "ws1" } },
  "entity.restore": { valid: { workspaceId: "ws1", relPath: "a/b" }, invalid: { workspaceId: "ws1" } },

  "audit.list": { valid: {}, invalid: { limit: "10" } },
  "audit.diff": { valid: { commitHash: "c1", entity: "mock", entityId: "m1", workspaceId: "ws1" }, invalid: { commitHash: "c1" } },
  "audit.export": { valid: { format: "json" }, invalid: { format: "xml" } },
  "history.list": { valid: { filePath: "a/b" }, invalid: {} },
  "history.diff": { valid: { commitHash: "c1", filePath: "a/b", workspaceId: "ws1" }, invalid: { commitHash: "c1" } },

  "tls.generate": { valid: {}, invalid: { extra: true } },
  "tls.certStatus": { valid: {}, invalid: { extra: true } },
  "tls.removeCert": { valid: {}, invalid: { extra: true } },
  "tls.exportCert": { valid: {}, invalid: { extra: true } },
  "tls.importCert": { valid: { content: "PEM" }, invalid: {} },
  "tls.importKey": { valid: { content: "PEM" }, invalid: {} },

  "export.formats": { valid: {}, invalid: { extra: true } },
  "export.create": { valid: { kind: "mocks", format: "json", workspaceId: "ws1" }, invalid: { kind: "mocks", format: "json" } },
  "import.preflight": { valid: { kind: "mocks", format: "json", workspaceId: "ws1" }, invalid: { format: "json", workspaceId: "ws1" } },
  "import.commit": { valid: { kind: "mocks", format: "json", workspaceId: "ws1" }, invalid: { kind: "mocks", workspaceId: "ws1" } },

  "script.execute": { valid: { script: "1+1", context: "pre", envVars: {} }, invalid: { script: "1+1", context: "bogus", envVars: {} } },
  "request.replay": { valid: { method: "GET", url: "http://x", headers: {}, body: "" }, invalid: { method: "GET", url: "http://x" } },
  "healthbar.getServices": { valid: { workspaceId: "ws1" }, invalid: {} },
  "healthbar.saveServices": { valid: { workspaceId: "ws1", services: [] }, invalid: { workspaceId: "ws1" } },
  "healthbar.checkUrl": { valid: { url: "http://x" }, invalid: {} },
  "app.checkUpdate": { valid: {}, invalid: { extra: true } },
  "capture.shareJson": { valid: { entries: [] }, invalid: {} },
};

describe("@bifurc/protocol command schemas", () => {
  const actions = Object.keys(COMMANDS) as CommandAction[];

  it("every command in COMMANDS has a fixture pair (no command silently unverified)", () => {
    const fixtureKeys = Object.keys(FIXTURES).sort();
    expect(fixtureKeys).toEqual([...actions].sort());
  });

  for (const action of actions) {
    const { params } = COMMANDS[action];
    const fixture = FIXTURES[action];

    it(`${action} — accepts a valid payload`, () => {
      const result = params.safeParse(fixture.valid);
      expect(result.success, result.success ? "" : JSON.stringify(result.error?.issues)).toBe(true);
    });

    it(`${action} — rejects an invalid payload`, () => {
      const result = params.safeParse(fixture.invalid);
      expect(result.success).toBe(false);
    });
  }
});
