/**
 * One **valid** and one **invalid** payload per command in `COMMANDS`.
 *
 * This started life inside `index.test.ts`, where it satisfied P1's acceptance criterion ("every
 * command has a Zod schema; a smoke test validates a valid and an invalid payload per command"). It
 * moved here in P4 work item 3 because the same data is what the **transport conformance suite** needs
 * for its every-command matrix, and two copies of a 93-entry table is a guarantee that one of them
 * will be wrong.
 *
 * The property that makes this worth sharing is not the payloads themselves, it is the invariant
 * `index.test.ts` enforces over them:
 *
 * ```ts
 * expect(Object.keys(COMMAND_FIXTURES).sort()).toEqual(Object.keys(COMMANDS).sort());
 * ```
 *
 * So adding a command without adding a fixture is a **test failure**, and the transport matrix — which
 * is driven off this table — widens automatically. That is what turns "the matrix samples one command"
 * into "the matrix covers every command, and cannot silently stop covering one".
 *
 * ## Why the `invalid` fixtures matter more than they look
 *
 * They are **near-misses**, not `null`. `entity.create` with `kind: "not-a-kind"` and a full entity,
 * `application.start` with `mode: "bogus"`, `blob.read` with a negative `offset` — each exercises a
 * schema's *rejection* path with a payload that is otherwise well-formed. A transport tested only
 * against `null` would pass even if it let everything through, because `null` fails every schema for
 * reasons that have nothing to do with the transport. These fixtures are what make "invalid payload →
 * `BAD_REQUEST`" a real assertion rather than a tautology.
 */
import type { CommandAction } from "./index";

export const COMMAND_FIXTURES: Record<CommandAction, { valid: unknown; invalid: unknown }> = {
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
  // P3: these two take a `blobId`, not inline `content` — the client uploads with `blob.put` first
  // (`File_Ops_Protocol.md` §5). See `plan/protocol-changes.md`.
  "tls.importCert": { valid: { blobId: "blob_test" }, invalid: {} },
  "tls.importKey": { valid: { blobId: "blob_test" }, invalid: {} },

  "export.formats": { valid: {}, invalid: { extra: true } },
  "export.create": { valid: { kind: "mocks", format: "json", workspaceId: "ws1" }, invalid: { kind: "mocks", format: "json" } },
  // `blobId` became required in P3 — the pre-P3 handlers took a `filePath` instead, and the blob
  // layer is now the only way in. The "invalid" fixtures still exercise a missing *required* field
  // rather than the new one, so they keep testing the same thing they always did.
  "import.preflight": { valid: { kind: "mocks", format: "json", workspaceId: "ws1", blobId: "blob_test" }, invalid: { format: "json", workspaceId: "ws1", blobId: "blob_test" } },
  "import.commit": { valid: { kind: "mocks", format: "json", workspaceId: "ws1", blobId: "blob_test" }, invalid: { kind: "mocks", workspaceId: "ws1", blobId: "blob_test" } },

  "blob.put": {
    valid: { filename: "export.json", mimeType: "application/json", size: 2, data: "e30=" },
    // missing filename
    invalid: { mimeType: "application/json", size: 2, data: "e30=" },
  },
  "blob.stat": { valid: { blobId: "b1" }, invalid: {} },
  "blob.read": { valid: { blobId: "b1", offset: 0, length: 1024 }, invalid: { blobId: "b1", offset: -1 } },
  "blob.release": { valid: { blobId: "b1" }, invalid: {} },

  "script.execute": { valid: { script: "1+1", context: "pre", envVars: {} }, invalid: { script: "1+1", context: "bogus", envVars: {} } },
  "request.replay": { valid: { method: "GET", url: "http://x", headers: {}, body: "" }, invalid: { method: "GET", url: "http://x" } },
  "healthbar.getServices": { valid: { workspaceId: "ws1" }, invalid: {} },
  "healthbar.saveServices": { valid: { workspaceId: "ws1", services: [] }, invalid: { workspaceId: "ws1" } },
  "healthbar.checkUrl": { valid: { url: "http://x" }, invalid: {} },
  "app.checkUpdate": { valid: {}, invalid: { extra: true } },
  "capture.shareJson": { valid: { entries: [] }, invalid: {} },
};
