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
  logEmitter: mockLogEmitter,
}));

vi.mock("@bifurc/engine/proxy/service-discovery", () => ({
  discoverServices: vi.fn(() => []),
}));

// `serverReplay.ts` makes a real outbound HTTP call, so it must be mocked — and it is mocked at *its
// own* path rather than through `proxy/server`'s re-export, because `miscCommands.ts` imports it
// directly (importing the lighter module beats pulling in the whole proxy server for one function).
vi.mock("@bifurc/engine/proxy/serverReplay", () => ({
  replayRequest: vi.fn(() => Promise.resolve({ status: 200, headers: {}, body: "" })),
}));

// `webhookServer.ts` owns a real listening `http.Server` as module-level state, so the real module
// must not be reached from a unit test — `webhookServer.start` would bind a port. Mocked rather than
// left real for the same reason `proxy/server` is. `webhookEmitter` is deliberately absent: nothing
// outside `webhookServer.ts` itself reads it, so omitting it proves no consumer depends on it.
vi.mock("@bifurc/engine/proxy/webhookServer", () => ({
  registerActiveWebhook: vi.fn(),
  unregisterActiveWebhook: vi.fn(),
  startWebhookServer: vi.fn(),
  stopWebhookServer: vi.fn(),
  isWebhookServerRunning: vi.fn(() => false),
  getWebhookPort: vi.fn(() => 9101),
  getWebhookServerError: vi.fn(() => null),
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

vi.mock("@bifurc/engine/store/appSettings", () => ({
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
vi.mock("@bifurc/engine/importExport/registry", () => ({
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

import type { AppConfig, LocalMapping, ProxyRule, MockRule, SavedRequest, Folder, Environment, Workspace, SavedWsConnection } from "@bifurc/engine/store/config";

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

vi.mock("@bifurc/engine/store/config", () => ({
  loadConfig: vi.fn(() => currentConfig),
  saveConfig: vi.fn((cfg: AppConfig) => { currentConfig = cfg; }),
  generateId: vi.fn(() => `id-${Date.now()}`),
  loadEntity: vi.fn(() => null),
  // Types only — no runtime value needed for interfaces
}));

import { loadConfig, saveConfig, generateId, loadEntity } from "@bifurc/engine/store/config";
import { commitMutation, queryLog, getEntityAtCommit, getCommitChangedFiles } from "@bifurc/engine/store/gitStore";
import { startServer, stopServer, isRunning, getPort, getServerError, reloadConfig, replayRequest } from "@bifurc/engine/proxy/server";
import { discoverServices } from "@bifurc/engine/proxy/service-discovery";
import {
  registerActiveWebhook,
  unregisterActiveWebhook,
  startWebhookServer,
  stopWebhookServer,
  isWebhookServerRunning,
  getWebhookPort,
  getWebhookServerError,
} from "@bifurc/engine/proxy/webhookServer";
import { replayRequest as replayRequestImpl } from "@bifurc/engine/proxy/serverReplay";
import { dialog, BrowserWindow } from "electron";
import * as fs from "fs";
import { commandRegistry } from "@bifurc/engine/commands/registry";

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
    ];

    for (const channel of expectedChannels) {
      it(`registers a handler for "${channel}"`, () => {
        expect(registeredHandlers.has(channel)).toBe(true);
      });
    }

    /**
     * The **step 3b inventory** — which protocol commands the registry actually implements.
     *
     * ## Why this exists
     *
     * `plan/07`'s step 3 is "route all methods through the bridge, delete `registerIpcHandlers()`",
     * and it warns to "expect failures in the shell-only handlers first". This is that list, measured
     * rather than discovered one failure at a time — and it is bigger than the plan implies:
     * **25 of the 93 commands had no registry implementation at all.** They were served only by a
     * shell `ipcMain.handle` body on a legacy channel, so `registry.invoke("<command>")` answered
     * `UNKNOWN_COMMAND`.
     *
     * ## Why it is a *ratchet* rather than a report
     *
     * `@bifurc/client` classifies every one of these as `{kind: "transport"}`, i.e. a 1:1 mapping onto
     * a protocol command — so `client.serverStatus()` called `registry.invoke("server.status")` and
     * failed. The client's surface therefore **advertised methods it could not deliver against the
     * real shell**, and nothing noticed because the preload only routes `config:get` today. Pinning
     * the list here means it can only shrink **deliberately**: implementing a command makes this fail
     * until the name is removed, and a new command that lands unregistered fails immediately.
     *
     * It has now shrunk five times, and the `MOVABLE` half of it is empty. Step 3b-2's slices moved
     * the six proxy/server-lifecycle commands into `packages/engine/src/proxy/serverCommands.ts`
     * (25 → 19); `config.save` + the three `workspace.*` commands into
     * `packages/engine/src/store/configCommands.ts` (19 → 15); the five `webhook.*` /
     * `webhookServer.*` commands into `packages/engine/src/proxy/webhookCommands.ts` (15 → 10); the
     * five `misc.ts` commands into `packages/engine/src/miscCommands.ts` (10 → 5); and
     * `runner.saveReport` into `packages/engine/src/runner/runnerCommands.ts` (5 → 4). Every edit is
     * the ratchet doing its job rather than a change to the assertion.
     *
     * **What is left is not a backlog.** The remaining four are one `NARROWED`, two `BLOCKED` and one
     * `SPLIT` — each pinned below with the reason a mechanical move is the wrong response. Step 3
     * therefore ends at **89 registered + 2 blocked + 1 narrowed + 1 split**, not 93, and the
     * remaining four are resolved by P12 (`app.checkUpdate`) and by a protocol change
     * (`runner.*Config`, `audit.list`) rather than by more moving.
     *
     * ## Why the list is now three lists
     *
     * The first version of this block said every remaining command was "engine work that has not been
     * moved yet". **Auditing the remaining 19 against their frozen schemas showed that is false**, and
     * `src/ipc/handlers/runnerHandlers.ts` had already recorded the reason for three of them: a
     * `.strict()` protocol schema that contradicts what real callers actually send. So the entries are
     * split by *why* they are unimplemented, because the three reasons need three different responses:
     *
     * - `MOVABLE` — the schema accepts every payload the handler accepts. Move it.
     * - `NARROWED` — the schema accepts a strict subset, and `.strict()` turns the difference into a
     *   `BAD_REQUEST`. Moving it would silently reduce the command's accepted params, so it needs a
     *   decision, not a move.
     * - `BLOCKED` — the schema rejects payloads that a **currently passing** test uses. These cannot
     *   move until the frozen protocol changes.
     * - `SPLIT` — the protocol declares an engine-half contract that deliberately differs from the
     *   shell's implementation. Not a move at all.
     *
     * This asserts *registration*, not behaviour. A command can be registered and still do the wrong
     * thing; that is what the conformance suite and each handler's own tests are for.
     */
    describe("registry coverage (the step 3b inventory)", () => {
      /**
       * `MOVABLE` used to live here, and is now **gone rather than empty** — which is the point of the
       * list, so it is worth saying why the deletion is the signal.
       *
       * It held every command that could be moved into the engine as-is: the six `server.*` /
       * `proxy.status` / `services.discover`, the four `config.save` / `workspace.*`, the five
       * `webhook.*` / `webhookServer.*`, the five `misc.ts` ones (`healthbar.*` ×3, `request.replay`,
       * `script.execute`) and `runner.saveReport`. All 21 now have registry implementations, so every
       * command that was ever classified movable has moved.
       *
       * What remains is deliberately *not* a to-do list of the same kind: it is one `NARROWED`, two
       * `BLOCKED` and one `SPLIT`, each pinned with the reason it cannot simply be moved. Keeping an
       * empty `MOVABLE` array would invite the next reader to file the next unimplemented command
       * there by default; deleting it forces the classification to be argued again.
       */

      /**
       * The schema is **narrower than the handler**, and `.strict()` makes that fatal rather than
       * lossy. `AuditListParams` has no `filePath` / `fromTs` / `toTs`, but `QueryLogOptions` — the
       * type the handler actually takes — does. So a caller passing `filePath` gets `BAD_REQUEST`
       * through the registry where the legacy channel answered it.
       *
       * **The honest caveat:** nothing currently passes those. `auditList` exists on `window.api` and
       * no renderer code calls it, so the narrowing is latent, not live. That is exactly why this is a
       * separate category rather than a `BLOCKED` one — and why it should not be quietly moved either.
       * The whole point of P9 is *new* clients, and a remote client is precisely the caller that would
       * find the missing fields.
       */
      const NARROWED = ["audit.list"];

      /**
       * Blocked by the frozen protocol, each with a **currently passing test** as the evidence. These
       * are not "not yet moved" — they cannot be moved without either widening the protocol or
       * breaking a real caller, and the protocol is frozen for P1–P6.
       */
      const BLOCKED = [
        {
          command: "runner.saveConfig",
          reason:
            "RunnerConfigSchema requires {requestOrder, delayMs}, but tests/integration/runnerStorage" +
            ".integration.test.ts:207 saves {delayMs, stopOnFailure, iterations} — no requestOrder.",
        },
        {
          command: "runner.loadConfig",
          reason:
            "The same schema, and RunnerLoadConfigResult *declares* it as the loaded shape — so a " +
            "round-trip of the config that test actually saves cannot satisfy it.",
        },
      ];

      /**
       * Not a move: `misc.ts` marks `app.checkUpdate` SPLIT, with the schema as "the interim
       * engine-half contract" and a note that **P12 moves it fully client-side**. The disagreement
       * `plan/07` recorded is therefore resolved by the protocol's own comment, and neither side was
       * simply wrong: the GitHub fetch is engine-safe outbound network, while `app.getVersion()` and
       * the `process.platform` asset match are client-local.
       *
       * The concrete consequence is that `AppCheckUpdateResult` has no `currentVersion` and no
       * `hasUpdate` — the two fields the shell handler's success branch returns — so the engine half
       * cannot be produced by moving that body. It needs the split written, which is P12's job.
       */
      const SPLIT = ["app.checkUpdate"];

      const NOT_IN_REGISTRY = [...NARROWED, ...BLOCKED.map((b) => b.command), ...SPLIT];

      it("implements every protocol command except the pinned list", async () => {
        const { COMMANDS } = await import("@bifurc/protocol");
        const { commandRegistry } = await import("@bifurc/engine/commands/registry");

        const missing = Object.keys(COMMANDS)
          .filter((c) => !commandRegistry.isRegistered(c as never))
          .sort();

        expect(missing).toEqual([...NOT_IN_REGISTRY].sort());
      });

      it("demonstrates why each BLOCKED command cannot move, using the frozen schema", async () => {
        // The other direction of ratchet, and the reason these are separated from `MOVABLE`: this
        // asserts the *block itself*, not just the membership. If someone later widens
        // `RunnerConfigSchema`, this test fails and says so — at which point `runner.saveConfig` and
        // `runner.loadConfig` become movable and should be moved, rather than sitting here forever
        // looking unimplemented.
        const { RunnerSaveConfigParams, RunnerLoadConfigParams } = await import("@bifurc/protocol");

        // Exactly the payload the passing integration test saves, and exactly the shape it expects
        // back. Both must be rejected by the frozen schema for this categorisation to be honest.
        const realConfig = { delayMs: 250, stopOnFailure: true, iterations: 3 };

        expect(
          RunnerSaveConfigParams.safeParse({ workspaceId: "ws", folderId: "f", config: realConfig })
            .success,
        ).toBe(false);

        // `runner.loadConfig` takes no config param at all, so its block is not about the request —
        // it is that the *result* it declares is the same fixed shape. Parsing the real config
        // through that shape is what would fail, which is the assertion below.
        expect(RunnerLoadConfigParams.safeParse({ workspaceId: "ws", folderId: "f" }).success).toBe(true);
        expect(
          RunnerSaveConfigParams.shape.config.safeParse(realConfig).success,
        ).toBe(false);

        expect(BLOCKED.map((b) => b.command).sort()).toEqual(
          ["runner.loadConfig", "runner.saveConfig"],
        );
      });

      it("records that the SPLIT command is a split rather than an unimplemented move", async () => {
        // `app.checkUpdate` takes no params in its engine half, so nothing about it is schema-blocked
        // — which is precisely why it must not be filed under `BLOCKED` or `MOVABLE`. The assertion
        // that matters is that its engine-half schema is empty: the engine half is the GitHub fetch
        // and nothing else, so any implementation that also computes `hasUpdate` is doing the client's
        // job and will disagree with P12.
        const { AppCheckUpdateParams } = await import("@bifurc/protocol");

        expect(AppCheckUpdateParams.safeParse({}).success).toBe(true);
        expect(Object.keys(AppCheckUpdateParams.shape)).toEqual([]);
        expect(SPLIT).toEqual(["app.checkUpdate"]);
      });

      it("still serves every unregistered command on a legacy channel", async () => {
        // The other half of the claim, and the reason step 3 cannot simply delete
        // `registerIpcHandlers()`: each of these commands is *reachable* today, just not through the
        // registry. If one lost its channel without gaining a registry entry, that method would stop
        // working on **both** paths at once — a silent regression the surface test could not see,
        // because the key would still be exposed.
        const { COMMANDS } = await import("@bifurc/protocol");

        for (const command of NOT_IN_REGISTRY) {
          const channel = (COMMANDS as Record<string, { legacyChannel?: string }>)[command]
            ?.legacyChannel;
          expect(channel, `${command} has no legacyChannel in the protocol table`).toBeTruthy();
          expect(registeredHandlers.has(channel!), `${command}'s channel "${channel}" is not registered`)
            .toBe(true);
        }
      });

      it("confirms the client claims every pinned command is routable, which is why the list matters", async () => {
        // The finding in one assertion: the client classifies **every one** of these as `transport`
        // or `shim` — a 1:1 mapping onto a protocol command — so it would call `registry.invoke()`
        // for each and get `UNKNOWN_COMMAND`. So the client does not merely lack these methods; it
        // *advertises* them.
        //
        // Note this holds for all four categories, including `SPLIT`: the client calling
        // `checkUpdate()` and reaching `registry.invoke("app.checkUpdate")` is exactly the mismatch
        // the protocol's "engine half" note describes.
        //
        // Imported relatively because `SURFACE` is not re-exported from the package root (only
        // `surface.ts` has it) and the alias in `vitest.config.ts` covers `@bifurc/engine/*` only.
        // Read-only data, so the relative hop costs nothing but honesty about where the contract lives.
        const { SURFACE } = await import("../../packages/client/src/surface");

        const claimed = NOT_IN_REGISTRY.filter((command) =>
          Object.values(SURFACE).some(
            (e) => (e.kind === "transport" || e.kind === "shim") && e.command === command,
          ),
        ).sort();

        expect(claimed).toEqual([...NOT_IN_REGISTRY].sort());

        // The disagreement `plan/07` recorded about `app.checkUpdate` — "shell half" per the plan,
        // `transport` per the client — is **resolved by the protocol's own comment**, and neither side
        // was simply wrong: `misc.ts` marks it SPLIT, the GitHub fetch being engine-safe outbound
        // network and the `app.getVersion()` / `process.platform` comparison being client-local. So
        // the client's classification is right *for now* (the engine half is reachable over a
        // transport) and P12 moves it fully client-side. Asserted rather than left in prose because
        // "one of the two is wrong" was the previous state and is no longer true.
        expect(SURFACE.checkUpdate).toMatchObject({ kind: "transport", command: "app.checkUpdate" });
      });

      /**
       * The ratchet above asserts *registration*; this asserts *delivery*, for the 21 commands that
       * step 3b-2 has moved.
       *
       * The distinction is the entire point of finding 5, and it is easy to lose: a command can be
       * registered and still return nothing useful. Registration is what makes `registry.invoke()`
       * stop throwing `UNKNOWN_COMMAND` — which is what `@bifurc/client` needs — but only an
       * invocation proves the client's `serverStatus()` would actually get a status back.
       *
       * These call the registry **directly**, with no `ipcMain` in the path, because that is exactly
       * how a transport calls it. `ctx` is built the way the engine's own callers build it: a bus and
       * **no session**, since a session-less caller is the engine itself and holds every scope.
       */
      it("delivers the 21 moved commands through the registry, not just the channel", async () => {
        const { commandRegistry } = await import("@bifurc/engine/commands/registry");
        const { bus } = await import("@bifurc/engine/eventBus");
        const ctx = { bus };

        vi.mocked(isRunning).mockReturnValue(true);
        vi.mocked(getPort).mockReturnValue(8080);
        vi.mocked(getServerError).mockReturnValue(null);

        // Two views of the same `isRunning()`, with deliberately different payloads — collapsing them
        // would silently change one of `serverStatus` / `proxyStatus`.
        expect(commandRegistry.invoke("server.status", {}, ctx)).toEqual({
          running: true,
          port: 8080,
          error: null,
        });
        expect(commandRegistry.invoke("proxy.status", {}, ctx)).toEqual({ running: true });

        // `start` / `stop` / `restart` read the port from `loadConfig()` rather than taking it as a
        // parameter, so the assertion is on the engine calls, not on the returned `{ok:true}`.
        expect(commandRegistry.invoke("server.start", {}, ctx)).toEqual({ ok: true });
        expect(startServer).toHaveBeenCalledWith(currentConfig.port);
        expect(commandRegistry.invoke("server.stop", {}, ctx)).toEqual({ ok: true });
        expect(stopServer).toHaveBeenCalled();

        // `restart` is stop-then-start, not a distinct code path — and the order is the contract:
        // starting before stopping would rebind a port the old listener still holds.
        vi.mocked(startServer).mockClear();
        vi.mocked(stopServer).mockClear();
        expect(commandRegistry.invoke("server.restart", {}, ctx)).toEqual({ ok: true });
        expect(vi.mocked(stopServer).mock.invocationCallOrder[0])
          .toBeLessThan(vi.mocked(startServer).mock.invocationCallOrder[0]);

        expect(commandRegistry.invoke("services.discover", {}, ctx)).toEqual([]);
        expect(discoverServices).toHaveBeenCalled();

        // ── slice 2: config.save and the three workspace.* commands ──────────

        // `config.save` — the settings path, and the restart is the part worth asserting. An
        // implementation that only called `saveConfig` would persist the new port while the live
        // server kept answering on the old one, which is the bug this branch exists to prevent.
        vi.mocked(startServer).mockClear();
        vi.mocked(stopServer).mockClear();
        expect(
          commandRegistry.invoke("config.save", { config: { ...makeDefaultConfig(), port: 9876 } }, ctx),
        ).toEqual({ ok: true });
        expect(saveConfig).toHaveBeenCalled();
        expect(stopServer).toHaveBeenCalled();
        expect(startServer).toHaveBeenCalledWith(9876);

        // `workspace.add` — the gate is mocked to allow, so this is the success path. The trim is the
        // assertion that matters: `name` comes off the wire as an arbitrary string, and a workspace
        // named "  Fresh  " would be a directory named that too.
        const wsCountBefore = (currentConfig.workspaces ?? []).length;
        const added = (await commandRegistry.invoke("workspace.add", { name: "  Fresh  " }, ctx)) as {
          id: string;
          name: string;
          activeEnvironmentId: string | null;
        };
        expect(added.name).toBe("Fresh");
        expect(added.activeEnvironmentId).toBeNull();
        expect((currentConfig.workspaces ?? []).length).toBe(wsCountBefore + 1);

        // `workspace.rename` — and note the fallback: a whitespace-only name must leave the old name
        // rather than blanking it, because `name.trim() || ws.name` is what the shell did.
        expect(
          await commandRegistry.invoke("workspace.rename", { id: added.id, name: "Renamed" }, ctx),
        ).toEqual({ ok: true });
        expect((currentConfig.workspaces ?? []).find((w) => w.id === added.id)?.name).toBe("Renamed");
        await commandRegistry.invoke("workspace.rename", { id: added.id, name: "   " }, ctx);
        expect((currentConfig.workspaces ?? []).find((w) => w.id === added.id)?.name).toBe("Renamed");

        // `workspace.delete` — removing the *inactive* one, so the active-id branch is not what is
        // under test here; the round-trip is.
        expect(await commandRegistry.invoke("workspace.delete", { id: added.id }, ctx)).toEqual({ ok: true });
        expect((currentConfig.workspaces ?? []).find((w) => w.id === added.id)).toBeUndefined();
        expect((currentConfig.workspaces ?? []).length).toBe(wsCountBefore);

        // ── slice 3: webhook.* and webhookServer.* ────────────────────────

        // `webhook.registerActive` / `unregisterActive` take the pair the renderer already sends; the
        // assertion is that both halves reach the engine's active-suffix registry rather than only one.
        expect(
          commandRegistry.invoke("webhook.registerActive", { webhookId: "w1", urlSuffix: "/x" }, ctx),
        ).toEqual({ ok: true });
        expect(registerActiveWebhook).toHaveBeenCalledWith("w1", "/x");

        expect(commandRegistry.invoke("webhook.unregisterActive", { webhookId: "w1" }, ctx)).toEqual({
          ok: true,
        });
        expect(unregisterActiveWebhook).toHaveBeenCalledWith("w1");

        // `webhookServer.start` — the port resolution is the whole point of this assertion. With no
        // `port` on the wire it must fall back to the config chain exactly as the shell body did,
        // because that is what every existing caller sends.
        currentConfig = { ...makeDefaultConfig(), webhookPort: 9200 };
        expect(commandRegistry.invoke("webhookServer.start", {}, ctx)).toEqual({ ok: true });
        expect(startWebhookServer).toHaveBeenCalledWith(9200);

        // …and a declared `port` must win, since `WebhookServerStartParams` advertises one. This is
        // the one handler in this batch that is not a literal copy of its shell body.
        expect(commandRegistry.invoke("webhookServer.start", { port: 9300 }, ctx)).toEqual({ ok: true });
        expect(startWebhookServer).toHaveBeenCalledWith(9300);

        // The `?? 9101` last resort: `webhookPort` is non-optional on `AppConfig`, so this is
        // defensive — but a config written by an older release is exactly how it would be missing.
        currentConfig = { ...makeDefaultConfig() } as AppConfig;
        delete (currentConfig as { webhookPort?: number }).webhookPort;
        expect(commandRegistry.invoke("webhookServer.start", {}, ctx)).toEqual({ ok: true });
        expect(startWebhookServer).toHaveBeenCalledWith(9101);

        expect(commandRegistry.invoke("webhookServer.stop", {}, ctx)).toEqual({ ok: true });
        expect(stopWebhookServer).toHaveBeenCalled();

        // `webhookServer.status` — three fields, and `error` is `null` rather than absent because the
        // renderer's status widget reads it unconditionally.
        vi.mocked(isWebhookServerRunning).mockReturnValue(true);
        vi.mocked(getWebhookPort).mockReturnValue(9200);
        vi.mocked(getWebhookServerError).mockReturnValue(null);
        expect(commandRegistry.invoke("webhookServer.status", {}, ctx)).toEqual({
          running: true,
          port: 9200,
          error: null,
        });

        // ── slice 4: the five misc.ts commands ─────────────────────────────

        // `healthbar.getServices` — the missing-file case is what this reaches (`fs` is automocked, so
        // `existsSync` is falsy), and the contract is `[]` rather than a throw, because the panel
        // renders "no services yet" from it.
        expect(commandRegistry.invoke("healthbar.getServices", { workspaceId: "ws1" }, ctx)).toEqual([]);

        // `healthbar.saveServices` — **the path is the assertion**. It is derived from the workspace
        // id, so a wrong id writes one workspace's healthbar config into another's directory, and the
        // failure would be invisible until a user opened the wrong workspace.
        expect(
          commandRegistry.invoke(
            "healthbar.saveServices",
            { workspaceId: "ws1", services: [{ name: "a" }] },
            ctx,
          ),
        ).toEqual({ ok: true });
        const healthbarPath = String(vi.mocked(fs.writeFileSync).mock.calls.at(-1)?.[0]).replace(
          /\\/g,
          "/",
        );
        expect(healthbarPath).toContain("/data/ws1/healthbar/services.json");

        // `healthbar.checkUrl` is registered but deliberately **not invoked here**: it makes a real
        // outbound HTTP request by design, and a unit test that reaches the network is a flake. The
        // ratchet above asserts its registration; the one non-verbatim change in its body (the
        // `require("https")` → static import, needed because `require` does not exist in the engine's
        // ESM output) is checked by `packages/engine`'s build rather than by a call.
        expect(commandRegistry.isRegistered("healthbar.checkUrl")).toBe(true);

        // `request.replay` — four positional args in the shell body, four named fields on the wire.
        // The assertion is that they land in the right **slots**; a spread would silently swap them.
        await commandRegistry.invoke(
          "request.replay",
          { method: "GET", url: "http://x", headers: { a: "b" }, body: "Ym9keQ==" },
          ctx,
        );
        expect(replayRequestImpl).toHaveBeenCalledWith("GET", "http://x", { a: "b" }, "Ym9keQ==");

        // `script.execute` — a real `vm` run, not a mock, so this proves the sandbox is reachable
        // through the registry. `envVars` coming back mutated is the observable effect.
        const scriptResult = commandRegistry.invoke(
          "script.execute",
          {
            script: "lp.environment.set('k', 'v')",
            context: "pre",
            request: { method: "GET", url: "http://x", headers: {}, body: "" },
            envVars: {},
          },
          ctx,
        ) as { envVars: Record<string, string>; error?: string };
        expect(scriptResult.error).toBeUndefined();
        expect(scriptResult.envVars.k).toBe("v");

        // ── slice 5: runner.saveReport ─────────────────────────────────────

        // Two artifacts, not one — the HTML is what the user opens, and it is rendered by the engine
        // because `runner.exportReport` needs the same renderer. The run directory is keyed by
        // `folderId` **and** `startedAt`, which is what keeps two runs of one folder apart.
        vi.mocked(fs.writeFileSync).mockClear();
        const report = {
          folderId: "f1",
          folderName: "Folder",
          startedAt: 1700000000000,
          completedAt: 1700000001000,
          totalRequests: 1,
          totalTests: 1,
          passedTests: 1,
          failedTests: 0,
          results: [],
        };
        expect(commandRegistry.invoke("runner.saveReport", { workspaceId: "ws1", report }, ctx)).toEqual(
          { ok: true },
        );
        expect(
          vi.mocked(fs.writeFileSync).mock.calls.map((c) => String(c[0]).replace(/\\/g, "/")),
        ).toEqual([
          expect.stringContaining(".runs/f1/1700000000000/report.json"),
          expect.stringContaining(".runs/f1/1700000000000/report.html"),
        ]);
      });

      it("refuses a payload the frozen protocol schema rejects, rather than passing it through", async () => {
        // The registry validates **before** the handler runs, so a transport gets `BAD_REQUEST` from
        // the same place for every command. Asserted here because 3b-2's new handlers take no params,
        // which makes it tempting to assume `{}` is merely conventional — it is enforced.
        const { commandRegistry } = await import("@bifurc/engine/commands/registry");
        const { bus } = await import("@bifurc/engine/eventBus");

        expect(() => commandRegistry.invoke("server.status", { unexpected: 1 }, { bus }))
          .toThrow(/Invalid payload for command "server\.status"/);
      });
    });
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

    it("emits settings.changed on the bus after saving (P2: tray update is the shell's job now)", async () => {
      const { bus } = await import("@bifurc/engine/eventBus");
      const listener = vi.fn();
      bus.onTyped("settings.changed", listener);
      const incoming: AppConfig = { ...makeDefaultConfig() };
      getHandler("config:save")(EVENT, incoming);
      expect(listener).toHaveBeenCalled();
      bus.offTyped("settings.changed", listener);
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
      const { deleteEntityFile } = await import("@bifurc/engine/store/workspaceFs");
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
      const { writeEntity } = await import("@bifurc/engine/store/workspaceFs");
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
      const { deleteEntityFile } = await import("@bifurc/engine/store/workspaceFs");
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
      const { writeEntity } = await import("@bifurc/engine/store/workspaceFs");
      await getHandler("request:add")(EVENT, baseReq);
      expect(writeEntity).toHaveBeenCalled();
    });
  });

  // ── request:update ────────────────────────────────────────────────────

  describe("request:update handler", () => {
    it("writes the updated request to disk via writeEntity", async () => {
      const req: SavedRequest = { id: "r1", name: "old", method: "GET", url: "http://x.com", headers: {}, body: "", createdAt: 1 };
      const { writeEntity } = await import("@bifurc/engine/store/workspaceFs");
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
      const { deleteEntityFile } = await import("@bifurc/engine/store/workspaceFs");

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
      const { writeEntity } = await import("@bifurc/engine/store/workspaceFs");
      await getHandler("ws:add")(EVENT, baseConn);
      expect(writeEntity).toHaveBeenCalled();
    });
  });

  describe("ws:update handler", () => {
    it("writes the updated ws connection to disk via writeEntity", async () => {
      const conn: SavedWsConnection = { id: "c1", name: "old", url: "ws://localhost:1", headers: {}, createdAt: 1, workspaceId: "default" };
      const { writeEntity } = await import("@bifurc/engine/store/workspaceFs");
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
      const { deleteEntityFile } = await import("@bifurc/engine/store/workspaceFs");
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

  // ── logEmitter → bus → EVENT_CHANNEL ───────────────────────────────────
  //
  // Four tests used to live here asserting that `logEmitter` events reached
  // `webContents.send("log:entry" / "server:error", …)`. That was `eventBridge.ts`'s behaviour, and
  // step 3c deleted that file: after the preload flip nothing subscribes to those channels, so the
  // bridge was broadcasting into the void. Two of the four were also vacuous — `not.toHaveBeenCalled()`
  // on a destroyed window passes whether or not anything is wired.
  //
  // Both halves it covered are still tested, now on the correct side of the seam:
  //   - `logEmitter` → **bus**        → `packages/engine/tests/eventBus.logWiring.test.ts` (traffic)
  //   - **bus** → `EVENT_CHANNEL`     → `tests/ipc/eventChannel.test.ts` (incl. `log.entry` end to end)
  //
  // The old block asserted the middle hop that no longer exists, which is why it is deleted rather
  // than migrated — a migrated version would be asserting that the shell forwards events to a channel
  // no client listens on, i.e. pinning dead behaviour.

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

  // ── entity.* CommandRegistry collapse (P2 work item 7) ──────────────────
  //
  // The legacy per-kind channels (`rule:add`, `ws:add`, …) now route through the collapsed
  // `entity.create`/`entity.update`/`entity.delete` commands internally (see
  // `entityCrudFactory.ts`). `EntityKind`'s wire vocabulary renames two of the twelve kinds —
  // engine-internal "rules"/"sockets" become protocol "proxyRules"/"wsConnections" — so these
  // tests exist specifically to pin that translation: without it, `commandRegistry.invoke()`
  // would throw a schema-validation error the moment a rule or websocket connection was
  // created, updated, deleted, or loaded.

  describe("entity.* CommandRegistry collapse", () => {
    it("registers entity.create, entity.update, entity.delete, entity.load, entity.list and entity.setEnabled", () => {
      for (const action of ["entity.create", "entity.update", "entity.delete", "entity.load", "entity.list", "entity.setEnabled"]) {
        expect(commandRegistry.isRegistered(action)).toBe(true);
      }
    });

    it("entity:load translates the engine-internal \"rules\" kind to the protocol's \"proxyRules\" without throwing", async () => {
      const result = await getHandler("entity:load")(EVENT, "default", "rules", "r1");
      // `loadEntity` is mocked to always return null in this file — the meaningful assertion
      // is that this resolves at all instead of rejecting on Zod schema validation.
      expect(result).toEqual({ ok: false });
    });

    it("entity:load translates the engine-internal \"sockets\" kind to the protocol's \"wsConnections\" without throwing", async () => {
      const result = await getHandler("entity:load")(EVENT, "default", "sockets", "ws1");
      expect(result).toEqual({ ok: false });
    });

    it("rule:add routes through entity.create with kind \"proxyRules\" and still stores under \"rules\"", async () => {
      const { writeEntity } = await import("@bifurc/engine/store/workspaceFs");
      const input: Omit<ProxyRule, "id"> = {
        name: "API rule", pattern: ".*\\.api\\.com.*", targetMappingId: "m1", enabled: true,
      };
      await getHandler("rule:add")(EVENT, input);
      expect(vi.mocked(writeEntity).mock.calls[0][1]).toBe("rules");
    });

    it("ws:add routes through entity.create with kind \"wsConnections\" and still stores under \"sockets\"", async () => {
      const { writeEntity } = await import("@bifurc/engine/store/workspaceFs");
      const input: Omit<SavedWsConnection, "id"> = { name: "conn", url: "ws://localhost" } as any;
      await getHandler("ws:add")(EVENT, input);
      expect(vi.mocked(writeEntity).mock.calls[0][1]).toBe("sockets");
    });

    it("graphql:addSchema/deleteSchema/listSchemas route through entity.create/entity.delete/entity.list", async () => {
      const { writeEntity, deleteEntityFile, readAllEntities } = await import("@bifurc/engine/store/workspaceFs");
      await getHandler("graphql:addSchema")(EVENT, { name: "s", content: "type Query {}" } as any);
      expect(vi.mocked(writeEntity).mock.calls[0][1]).toBe("graphqlSchemas");

      await getHandler("graphql:deleteSchema")(EVENT, "s1");
      expect(vi.mocked(deleteEntityFile)).toHaveBeenCalledWith("default", "graphqlSchemas", "s1");

      const list = await getHandler("graphql:listSchemas")(EVENT);
      expect(vi.mocked(readAllEntities)).toHaveBeenCalledWith("default", "graphqlSchemas");
      expect(list).toEqual([]);
    });

    // `environments` is the thirteenth (and last) `EntityKind` to join the collapse — unlike
    // the twelve original CRUD-factory kinds, it carries a create-gate check (`gateKind`) and
    // two delete-time quirks (the "__global__" guard, the active-environment reset). These
    // pin that it now goes through `entity.create`/`entity.update`/`entity.delete` exactly
    // like every other kind, with those quirks preserved.

    it("env:add routes through entity.create with kind \"environments\" and stores flat", async () => {
      const { writeFlatEntity } = await import("@bifurc/engine/store/workspaceFs");
      const input = { name: "Dev", variables: [] };
      const result = await getHandler("env:add")(EVENT, input);
      expect(result.id).toBeTruthy();
      expect(result.name).toBe("Dev");
      expect(vi.mocked(writeFlatEntity).mock.calls[0][1]).toBe("environments");
    });

    it("env:add returns { error: \"limit_reached\", ... } and creates nothing when gateCreate disallows it", async () => {
      const { gateCreate } = await import("@bifurc/engine/subscription/entityCount");
      vi.mocked(gateCreate).mockReturnValueOnce({ allowed: false, current: 1, limit: 1 });

      currentConfig.environments = [];
      const result = await getHandler("env:add")(EVENT, { name: "Dev", variables: [] });

      expect(result).toEqual({ error: "limit_reached", allowed: false, current: 1, limit: 1 });
      expect(currentConfig.environments).toHaveLength(0);
      expect(gateCreate).toHaveBeenCalledWith(expect.any(String), "environment");
    });

    it("env:delete refuses to delete \"__global__\"", async () => {
      currentConfig.environments = [{ id: "__global__", name: "Global", variables: [], createdAt: 1 }];
      const result = await getHandler("env:delete")(EVENT, "__global__");
      expect(result).toEqual({ ok: false, error: "cannot_delete_global" });
      expect(currentConfig.environments).toHaveLength(1);
    });
  });
});
