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
  "settings.update": { valid: { forwardProxyPort: 8081, minimizeToTray: true }, invalid: { port: 70000 } },
  "workspace.add": { valid: { name: "WS" }, invalid: {} },
  "workspace.rename": { valid: { id: "w1", name: "New" }, invalid: { id: "w1" } },
  "workspace.delete": { valid: { id: "w1" }, invalid: {} },
  "workspace.setActive": { valid: { id: "w1" }, invalid: {} },
  "env.setActive": { valid: { id: null }, invalid: {} },
  // `plan/19` WI 5, item 4. The `invalid` row is a **near-miss** — the right shape under the wrong key —
  // because `{}` would fail for "missing `workspaceId`" before and after any schema edit.
  "env.resolveText": { valid: { workspaceId: "ws1", text: "{{host}}/health" }, invalid: { workspaceId: "ws1", value: "{{host}}/health" } },
  // `plan/19` WI 6. `{}` is the invalid row because `workspaceId` is **required** here — the merge is per
  // workspace and there is no "active workspace" to fall back to (`WorkspaceScope` is deliberately not used).
  "env.getMerged": { valid: { workspaceId: "ws1" }, invalid: {} },

  // P14 WI 1 (D13): these six carry an optional `workspaceId` so a host can address one of N workspaces
  // behind a single RPC endpoint. The `valid` fixture supplies it, because "omitted" is already covered by
  // every pre-P14 caller and the new capability is the thing at risk of being untested. The `invalid` fixture
  // stays `{ extra: true }` — re-derived, not assumed: `.strict()` still rejects an unknown key, so the
  // rejection path is still exercised. The near-miss for the *new* field is a wrong-typed `workspaceId`,
  // which is a better test than a missing one (a missing optional field is valid by construction).
  "server.status": { valid: { workspaceId: "ws1" }, invalid: { workspaceId: 42 } },
  "server.start": { valid: { workspaceId: "ws1" }, invalid: { workspaceId: 42 } },
  "server.stop": { valid: { workspaceId: "ws1" }, invalid: { workspaceId: 42 } },
  "server.restart": { valid: { workspaceId: "ws1" }, invalid: { workspaceId: 42 } },
  "proxy.status": { valid: { workspaceId: "ws1" }, invalid: { workspaceId: 42 } },  "services.discover": { valid: {}, invalid: { extra: true } },

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

  "runner.saveReport": {
    valid: { workspaceId: "ws1", report: { folderId: "f1", startedAt: 1 } },
    invalid: { workspaceId: "ws1", report: {} },
  },
  // P14 WI 8 re-derived these three: each `valid` row now supplies `workspaceId`, so the new field is
  // exercised on the happy path rather than merely tolerated. Each `invalid` row was checked
  // individually to fail on a **required-or-typed** field and never on scope — the hazard is a fixture
  // whose invalid payload *is* the newly allowed key, which would silently stop testing rejection.
  //   `runner.exportReport` → the `format` enum ("pdf"); `audit.export` → the enum ("xml");
  //   `capture.shareJson` → `{}`, invalid because `entries` is required (for a reason unrelated to scope).
  "runner.exportReport": {
    valid: { report: { folderId: "f1", startedAt: 1 }, format: "html", workspaceId: "ws1" },
    invalid: { report: { folderId: "f1", startedAt: 1 }, format: "pdf" },
  },
  "runner.getHistory": { valid: { workspaceId: "ws1", folderId: "f1" }, invalid: { workspaceId: "ws1" } },
  // `RunnerConfigSchema` was widened to `z.record` on 2026-09-19 (`plan/protocol-changes.md`), which
  // silently invalidated this entry's old `invalid`: `config: {}` was a near-miss only because the
  // frozen schema *required* `requestOrder`, and an empty record is now a perfectly valid record. The
  // near-miss is a **non-record** instead, which is the one thing the widening still rejects — it made
  // `config` free-form, not "anything goes". `[]`, `null` and `"str"` would also be rejected (checked,
  // not assumed), but `42` reads as unmistakably not-a-record to someone skimming the table.
  "runner.saveConfig": {
    valid: { workspaceId: "ws1", folderId: "f1", config: { requestOrder: [], delayMs: 0 } },
    invalid: { workspaceId: "ws1", folderId: "f1", config: 42 },
  },
  "runner.loadConfig": { valid: { workspaceId: "ws1", folderId: "f1" }, invalid: { workspaceId: "ws1" } },
  "runner.listFolderIds": { valid: { workspaceId: "ws1" }, invalid: {} },
  // The run's own commands (`plan/19` WI 4). Each takes the same `{workspaceId, folderId}` pair the storage
  // commands above take, and the `invalid` fixture is the same near-miss — a missing `folderId` — because the
  // rejection being exercised is the *pair's*, not a command-specific rule.
  "runner.start": { valid: { workspaceId: "ws1", folderId: "f1" }, invalid: { workspaceId: "ws1" } },
  "runner.cancel": { valid: { workspaceId: "ws1", folderId: "f1" }, invalid: { workspaceId: "ws1" } },
  "runner.status": { valid: { workspaceId: "ws1", folderId: "f1" }, invalid: { workspaceId: "ws1" } },

  // `ws.*` (`plan/19` WI 6). The near-misses are chosen per command rather than copied: `ws.connect`'s is a
  // missing `url` (the pair that identifies the socket is present, so the rejection is the *url's*),
  // `ws.send`'s is a `data` of the wrong type, and the two read commands' is a missing `socketId`.
  "ws.connect": {
    valid: { workspaceId: "ws1", socketId: "tab-1", url: "wss://example.test/socket" },
    invalid: { workspaceId: "ws1", socketId: "tab-1" },
  },
  "ws.disconnect": { valid: { workspaceId: "ws1", socketId: "tab-1" }, invalid: { workspaceId: "ws1" } },
  "ws.send": { valid: { workspaceId: "ws1", socketId: "tab-1", data: "ping" }, invalid: { workspaceId: "ws1", socketId: "tab-1", data: 1 } },
  "ws.status": { valid: { workspaceId: "ws1", socketId: "tab-1" }, invalid: { workspaceId: "ws1" } },
  "ws.list": { valid: { workspaceId: "ws1" }, invalid: {} },
  "ws.clearMessages": { valid: { workspaceId: "ws1", socketId: "tab-1" }, invalid: { workspaceId: "ws1" } },

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
  "audit.export": { valid: { format: "json", workspaceId: "ws1" }, invalid: { format: "xml" } },
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

  // Re-derived for P14 D17: all four gained an optional `workspaceId`. The `valid` rows supply it
  // so the field is exercised on the happy path; the `invalid` rows are **near-misses that do not
  // depend on it**, which is the point of re-deriving rather than appending. A row whose invalid
  // payload was `{}` would now be invalid for the *same* reason as before (a missing `blobId`), so
  // it would keep passing while testing nothing about the new field — harmless here, but exactly
  // the pattern that silently stops testing rejection when the newly-allowed key is the one under
  // test. Each `invalid` below still fails on a *required or typed* field, never on scope.
  // P14 WI 1 (D17) — the four `blob.*` rows were **re-derived**, not trusted, when the optional
  // `workspaceId` was merged in. The hazard the plan anticipated is real but narrow: with `.strict()`,
  // adding a field turns a payload that was invalid *because it carried that field* into a **valid** one,
  // so an `invalid` fixture can silently stop being invalid. None of the four did, and the reason each is
  // still invalid is pinned in `index.test.ts`'s "the reason, not just the rejection" block — asserting
  // `success === false` alone cannot tell "rejected for the intended reason" from "rejected for any reason".
  "blob.put": {
    valid: { filename: "export.json", mimeType: "application/json", size: 2, data: "e30=", workspaceId: "ws1" },
    // missing `filename` — a required field, not the optional one
    invalid: { mimeType: "application/json", size: 2, data: "e30=" },
  },
  // `{}` was the original invalid payload for these two, and it is a **constant**: an empty object fails
  // every schema before and after the change, so it could not have detected the change at all. It is
  // replaced by the plan's own named hazard — a payload carrying *only* the newly-added optional field.
  // This one is discriminating: it is otherwise well-formed, it carries `workspaceId`, and it is rejected
  // solely because `blobId` is required. Under `.strict()` a non-optional `workspaceId` would have made
  // it valid, so this row is what turns the D17 compatibility claim into an assertion.
  "blob.stat": { valid: { blobId: "b1", workspaceId: "ws1" }, invalid: { workspaceId: "ws1" } },
  "blob.read": {
    valid: { blobId: "b1", offset: 0, length: 1024, workspaceId: "ws1" },
    // a negative `offset` — the strongest of the four, because the payload is complete apart from the
    // one field under test and the rejection is a range check rather than a missing-field check
    invalid: { blobId: "b1", offset: -1, workspaceId: "ws1" },
  },
  "blob.release": { valid: { blobId: "b1", workspaceId: "ws1" }, invalid: { workspaceId: "ws1" } },

  "script.execute": { valid: { script: "1+1", context: "pre", envVars: {} }, invalid: { script: "1+1", context: "bogus", envVars: {} } },
  "request.replay": { valid: { method: "GET", url: "http://x", headers: {}, body: "" }, invalid: { method: "GET", url: "http://x" } },
  // `plan/19` WI 5. One required field, so the discriminating invalid is a **near-miss**: the right shape
  // under the wrong key name. `{}` would fail for "missing `curl`" before and after any schema edit, which
  // is the non-discriminating fixture this table's header warns about.
  "request.parseCurl": { valid: { curl: "curl http://x" }, invalid: { curlCommand: "curl http://x" } },
  "healthbar.getServices": { valid: { workspaceId: "ws1" }, invalid: {} },
  "healthbar.saveServices": { valid: { workspaceId: "ws1", services: [] }, invalid: { workspaceId: "ws1" } },
  "healthbar.checkUrl": { valid: { url: "http://x" }, invalid: {} },
  "capture.shareJson": { valid: { entries: [], workspaceId: "ws1" }, invalid: {} },
  // `plan/19` WI 5, item 2. One required field each, so the discriminating `invalid` is a **near-miss**:
  // the right shape under the wrong key. `{}` would fail for "missing `entry`" before and after any schema
  // edit, which is the non-discriminating fixture this table's header warns about.
  "capture.buildRequest": { valid: { entry: { id: "log-1", method: "GET", url: "http://x" } }, invalid: { entryId: "log-1" } },
  "capture.buildMock": { valid: { entry: { id: "log-1", method: "GET", url: "http://x" } }, invalid: { entryId: "log-1" } },
  "engine.shutdown": { valid: {}, invalid: { now: true } },
  "capture.list": { valid: { workspaceId: "ws1" }, invalid: {} },
  "capture.clear": { valid: { workspaceId: "ws1" }, invalid: {} },
  "capture.remove": { valid: { workspaceId: "ws1", ids: ["log-1"] }, invalid: { workspaceId: "ws1", ids: [] } },

  // The saved-entity counterparts (`saved.ts`). All four share one schema, so all four get the same
  // pair — and the `invalid` row is `entity.load`'s: a **near-miss** that drops the required
  // `workspaceId` while keeping `id`, rather than `{}`. `{}` would fail for the same reason before and
  // after any change to the schema, which is the non-discriminating fixture this table's header warns
  // about; a missing required field is the shape that actually exercises the rejection path here.
  // `request.send` is the one command here with a **union** payload, so its two fixtures are chosen for
  // the union rather than for the id form its three siblings share. `valid` is the **inline** form — the
  // second member, with the nested `request` / `scripts` shapes, which is the part a schema is most
  // likely to get wrong; the id form is structurally `SavedEntityRef`, already exercised by three
  // fixtures and driven end-to-end by `tests/integration/protocolExecution.integration.test.ts`.
  // `invalid` is a payload that is **partly both** forms — it matches neither member, and `.strict()` on
  // each member is what turns that into a rejection rather than a half-read. That is the failure mode a
  // two-member union introduces, so it is the near-miss worth spending the fixture on.
  "request.send": {
    valid: {
      workspaceId: "ws1",
      request: { method: "GET", url: "http://example.test/", headers: {}, body: "" },
      scripts: {},
    },
    invalid: { workspaceId: "ws1", id: "req-1", scripts: {} },
  },
  // `graphql.send` is the second union payload (after `request.send`), and its two fixtures follow the same
  // rule: `valid` is the **inline** member, because the nested request shape is the part a schema can get
  // wrong, and `invalid` is a payload that is **partly both** forms — `{workspaceId, id, request}` matches
  // neither member, which is what `.strict()` on each member turns into a rejection.
  "graphql.send": {
    valid: {
      workspaceId: "ws1",
      request: {
        endpointUrl: "http://example.test/graphql",
        headers: {},
        query: "query { __typename }",
        variables: "",
        operationName: "",
      },
    },
    invalid: {
      workspaceId: "ws1",
      id: "gql-1",
      request: {
        endpointUrl: "http://example.test/graphql",
        headers: {},
        query: "query { __typename }",
        variables: "",
        operationName: "",
      },
    },
  },
  // `soap.send` — the third union payload, and the same fixture rule: `valid` is the **inline** member, and
  // `invalid` is a payload that is **partly both** forms.
  "soap.send": {
    valid: {
      workspaceId: "ws1",
      request: {
        endpointUrl: "http://example.test/ws",
        soapAction: "urn:Ping",
        headers: {},
        body: "<ping/>",
      },
    },
    invalid: {
      workspaceId: "ws1",
      id: "soap-1",
      request: {
        endpointUrl: "http://example.test/ws",
        soapAction: "urn:Ping",
        headers: {},
        body: "<ping/>",
      },
    },
  },
  // `grpc.send` — the fourth and last union payload, and the same fixture rule as its three siblings.
  "grpc.send": {
    valid: {
      workspaceId: "ws1",
      request: {
        serverAddress: "127.0.0.1:50051",
        serviceName: "Greeter",
        methodName: "SayHello",
        requestBody: "{}",
        metadata: {},
        protoFileId: null,
        useReflection: false,
      },
    },
    invalid: {
      workspaceId: "ws1",
      id: "grpc-1",
      request: {
        serverAddress: "127.0.0.1:50051",
        serviceName: "Greeter",
        methodName: "SayHello",
        requestBody: "{}",
        metadata: {},
        protoFileId: null,
        useReflection: false,
      },
    },
  },
};
