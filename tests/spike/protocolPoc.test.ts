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
 * Four of the ten channels have been deleted by P6 stage 3b so far — `graphql:introspect` and
 * `soap:execute` (slice 1: the nine one-line passthroughs), then `mock:add` and `mock:delete`
 * (slice 2: the 48 factory-generated channels). `beforeAll` re-attaches each to the command it
 * delegated to, with the exact body it had. See the fixture there.
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

vi.mock("@bifurc/engine/proxy/server", () => ({
  startServer: vi.fn(),
  stopServer: vi.fn(),
  isRunning: vi.fn(() => true),
  getPort: vi.fn(() => 80),
  getServerError: vi.fn(() => null),
  reloadConfig: vi.fn(),
  replayRequest: vi.fn(),
}));

vi.mock("@bifurc/engine/proxy/logEmitter", () => ({
  logEmitter: new EventEmitter(),
}));

vi.mock("@bifurc/engine/proxy/service-discovery", () => ({
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

vi.mock("@bifurc/engine/importExport/registry", () => ({
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
    const { registerCoreKinds } = await import("../../src/ipc/handlers/coreKinds");
    const { registerCrudKinds } = await import("../../src/ipc/handlers/crudKinds");
    const { registerGraphqlKinds } = await import("../../src/ipc/handlers/graphqlKinds");
    const { registerSoapKinds } = await import("../../src/ipc/handlers/soapKinds");
    // P3 work item 5: `tls:generate` became a thin client of the `tls.generate` **command**, which
    // moved out of the shell into the engine (`@bifurc/engine/proxy/certCommands`). `src/ipc/handlers.ts`
    // wires both; a suite that registers handlers by hand has to wire both too, or the channel exists
    // with nothing behind it.
    const { commandRegistry } = await import("@bifurc/engine/commands/registry");
    const { registerCertCommands } = await import("@bifurc/engine/proxy/certCommands");
    // P6 §5c: `config.get` and `entity.setEnabled` moved out of `coreHandlers`' module scope into the
    // engine, for the same reason `tls.generate` moved in P3 — a suite that registers handlers by hand
    // has to wire the engine half too, or the channel exists with nothing behind it.
    //
    // **The channels are now gone too.** The reduction's sixth group deleted all seventeen of
    // `coreHandlers.ts`'s on 2026-09-21 and the file became `coreKinds.ts` (`plan/07` §5b-septendecies).
    // Three of the ten this spike drives were among them — `config:get`, `server:status` and
    // `entity:setEnabled` — so all three are re-attached in the fixture below, and the fixture reaches
    // **10 of 10**. `server:status` additionally needed `registerServerCommands()` wired above, which
    // the channel had been supplying by side effect; see the note at that call.
    const { registerCoreCommands } = await import("@bifurc/engine/store/coreCommands");
    // P6 §5c slice 3: `folder.*` moved out of `folderHandlers.ts`'s module scope as well, and that
    // file's channels delegated rather than duplicating, so the engine half has to be wired here too.
    // **The channels are now deleted** (P6 reduction, third group, 2026-09-21) — and `folder:add` /
    // `folder:move` are two of the ten this spike drives, so both are re-attached in the fixture below.
    const { registerFolderCommands } = await import("@bifurc/engine/store/folderCommands");
    // P6 §5c slice 4: same again for `graphql.*` and `soap.*` — and this suite is the one that
    // actually drives `soap.execute`, so without these two calls it fails as `No handler registered`.
    const { registerGraphqlCommands } = await import("@bifurc/engine/protocols/graphqlCommands");
    const { registerSoapCommands } = await import("@bifurc/engine/protocols/soapCommands");
    // P6 §5c slice 7: the four collapsed `entity.*` commands left `entityCrudFactory.ts`'s module scope.
    // `registerCrudKinds()` above wires the per-kind options the four commands dispatch through; it
    // registers no channels (the reduction's fourth group deleted the file's last five, 2026-09-21).
    const { registerEntityCrudCommands } = await import("@bifurc/engine/store/entityCrudCommands");
    // `server.status` is registered here for the first time as of the reduction's sixth group, and the
    // reason is worth recording because **nothing about deleting `coreHandlers.ts` suggested this file
    // needed a new line.** This spike drives `server:status` as one of its ten channels, and until
    // 2026-09-21 that channel *was* the registration: `systemHandlers.ts` called
    // `registerServerCommands()` at module scope, and this file happened to get it transitively. The
    // fifth group deleted `server:restart` / `server:stop` / `server:start` and left `server:status`
    // live, so the transitive path survived it; the sixth group deleted `server:status` — which this
    // spike never calls through the channel anymore, only through the fixture — and the path went with
    // it. The failure was `[INTERNAL] No handler registered for command "server.status"`, i.e. the
    // fixture faithfully re-attached the channel to a command no one had registered. That is the shape
    // to expect generally: **a suite that registers handler groups by hand gets its commands as a side
    // effect of whichever group called `register*Commands()`, and the reduction is exactly the process
    // of deleting those side effects.**
    const { registerServerCommands } = await import("@bifurc/engine/proxy/serverCommands");

    registerCoreKinds();
    registerCrudKinds();
    registerGraphqlKinds();
    registerSoapKinds();
    registerCertCommands(commandRegistry);
    registerCoreCommands(commandRegistry);
    registerServerCommands(commandRegistry);
    registerFolderCommands(commandRegistry);
    registerGraphqlCommands(commandRegistry);
    registerSoapCommands(commandRegistry);
    registerEntityCrudCommands(commandRegistry);

    // ── The retired-channel fixture ──────────────────────────────────────────────────────────────
    //
    // P6 stage 3b deletes the shell's legacy channels, and this spike drives **ten** of them, so this
    // table grows with each **driven** channel — not with each slice, which is the correction the
    // reduction's second group produced: `graphql:introspect` / `soap:execute` went in slice 1 (the nine
    // one-line passthroughs), `mock:add` / `mock:delete` in slice 2 (the 48 factory-generated channels),
    // `tls:generate` in the reduction's first group (the six `tls:*` channels), `folder:add` /
    // `folder:move` in its third (the four `folder:*` channels), and `config:get` / `server:status` /
    // `entity:setEnabled` in its sixth (the seventeen `coreHandlers.ts` channels). Groups 2, 4 and 5
    // added none, because this spike drives no `sync.*` / `git.*` / `audit.*` / `history.*` channel, no
    // `webhook*` channel and no `server:*` / `capture:*` channel respectively.
    //
    // This spike's subject is the **envelope**, not the routing, so each retired channel is re-attached
    // with *the exact body it had*. That is not a weakening: every one of those bodies was itself a
    // `commandRegistry.invoke()` adapter, so the real command still runs and every assertion below still
    // reads real output. It also doubles as the check that the deletion lost nothing — the ten commands
    // answer identically, only some of them now arrive by a shorter path.
    //
    // **The table now covers 10 of the 10 channels this spike drives — it is complete as of the
    // reduction's sixth group (2026-09-21).** The previous version of this paragraph said that would
    // happen "exactly when the legacy surface is gone", and that inference is **wrong in a way worth
    // keeping**: the two events are independent. The spike drives ten *specific* channels; when this
    // was written the legacy surface still held twenty dead ones (`applicationHandlers.ts` 10,
    // `runnerHandlers.ts` 6, `importExportHandlers.ts` 4) that this spike never touches. So "this file
    // is complete" and "the legacy surface is gone" are different claims, and the first is now true
    // while the second is not.
    //
    // **Updated 2026-09-21, after the reduction's eighth group:** the surface is down to eighteen dead
    // channels in two files (`runnerHandlers.ts`'s six went in that group — see `plan/07`
    // §5b-novendecies — and `importExportHandlers.ts`'s four remain). Ten of the twenty have now been
    // deleted, and this spike drove **none of those ten**, which is the same non-growth the seventh
    // group recorded and the confirmation that this file's completeness was never coupled to the
    // surface's. What the completeness *does* establish is the thing the open question in `plan/07`
    // §5b-ter asked: the ten commands answer identically through a channel-free path, so this file's
    // remaining value is the envelope, and it no longer depends on the shell at all.
    const { bus } = await import("@bifurc/engine/eventBus");
    // The deleted `tls:generate` body called both of these; the fixture that reproduces it must too.
    const { caCertPath, caKeyPath } = await import("@bifurc/engine/proxy/certManager");
    const deletedChannelBodies: Record<string, (...args: any[]) => unknown> = {
      // The three `coreHandlers.ts` channels this spike drives (reduction's sixth group). All three were
      // one-line `commandRegistry.invoke()` adapters, so the re-attachment is the adapter with the
      // channel removed — `toArgs` supplies the positional arguments the channels took, and each is
      // named back here. Note `entity:setEnabled`'s channel also carried an `invalid_kind` guard ahead of
      // the invoke; it is **not** reproduced, because the guard is client-facing behaviour that moved to
      // `packages/client/src/index.ts` (`174a01d`) and this spike's client predates it. The only kind
      // this spike sends is `"mocks"`, which the guard passes either way.
      "config:get": (_e) => commandRegistry.invoke("config.get", {}, { bus }),
      "server:status": (_e) => commandRegistry.invoke("server.status", {}, { bus }),
      "entity:setEnabled": (_e, wsId: string, kind: string, id: string, enabled: boolean) =>
        commandRegistry.invoke("entity.setEnabled", { workspaceId: wsId, kind, id, enabled }, { bus }),
      "graphql:introspect": (_e, params) => commandRegistry.invoke("graphql.introspect", params, { bus }),
      "soap:execute": (_e, params) => commandRegistry.invoke("soap.execute", params, { bus }),
      // `mocks` is one of the ten kinds whose protocol name equals its engine kind, so the adapter's
      // `toProtocolKind("mocks")` was the identity and the command it called was `kind: "mocks"`. The
      // `"entity" in result ? … : …` unwrap is the frozen protocol shape, not a convenience: a
      // gate-blocked create answers `{error, …}` and has no `.entity` to unwrap.
      "mock:add": async (_e, entity: any) => {
        const result = await commandRegistry.invoke(
          "entity.create",
          { kind: "mocks", entity, workspaceId: entity?.workspaceId },
          { bus },
        ) as { id: string; entity: Record<string, unknown> } | { error: string };
        return "entity" in result ? result.entity : result;
      },
      "mock:delete": (_e, id: string) => commandRegistry.invoke("entity.delete", { kind: "mocks", id }, { bus }),
      // The two `folder:*` channels this spike drives (reduction's third group). `folder:add` is the one
      // whose body **spread its argument** (`{ kind, ...folder }`) — reproduced rather than replaced with
      // the client's field-picking, because the spread is what the deletion removed and this fixture's
      // job is to show the command answers identically either way. The spike's own `toArgs` supplies
      // `[kind, folder]` / `[kind, id, parentId]`, matching the channels' positional signatures.
      "folder:add": (_e, kind: string, folder: any) =>
        commandRegistry.invoke("folder.add", { kind, ...folder }, { bus }),
      "folder:move": (_e, kind: string, id: string, parentId: string | null) =>
        commandRegistry.invoke("folder.move", { kind, id, parentId }, { bus }),
      // **The one entry whose body is not a bare `invoke()` adapter**, and that is exactly why the
      // reduction could not delete this channel with the rest of its group: the shell appended the two
      // paths the engine deliberately refuses to report (`File_Ops_Protocol.md` §8) and the client twin
      // had silently dropped them (`plan/07` §5b-undecies). Reproduced verbatim — paths included — so
      // the assertion below still checks the shape the renderer actually consumes.
      "tls:generate": async () => {
        const result = await commandRegistry.invoke("tls.generate", {}, { bus }) as {
          ok: boolean;
          fingerprint?: string;
          error?: string;
        };
        if (!result.ok) return { ok: false, error: result.error };
        return {
          ok: true,
          certPath: caCertPath(),
          keyPath: caKeyPath(),
          fingerprint: result.fingerprint,
        };
      },
    };

    const transport = createInProcessTransport(
      (channel) => registeredHandlers.get(channel) ?? deletedChannelBodies[channel],
    );
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
    // The pre-P3 renderer shape, preserved verbatim: `TlsSettingsSection.tsx` needs both paths to
    // record the CA in the settings, and `window.api` is byte-identical through P6.
    expect(result.certPath).toContain("ca-cert.pem");
    expect(result.keyPath).toContain("ca-key.pem");
    // Additive in P3 work item 5: the engine reports an identity instead of paths, and the shell
    // passes the fingerprint through on the channel. The renderer ignores it until P7.
    expect(result.fingerprint).toMatch(/^sha256:[0-9a-f]{64}$/);
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
