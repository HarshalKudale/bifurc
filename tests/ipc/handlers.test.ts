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

vi.mock("@bifurc/engine/workspace/entityCount", () => ({
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

/**
 * `Engine_Module_Plan.md` §4.1 — the **reload** fan-out now enters through `runtime/instances.ts`.
 *
 * **Only `notifyConfigChanged` is stubbed; `rebindRuntimePorts` is left real**, and that split is the whole
 * point of this mock. The command under test calls *two* things: a reload (whose target is the proxy, mocked
 * above) and a port rebind (whose targets are the edge / proxy / webhook, each already mocked at *their own*
 * module). Stubbing both here would make the port assertions below vacuous — they would observe a stub
 * calling nothing. So this file mocks the seam's *reload* half and lets the *rebind* half run for real against
 * the listener mocks, which keeps every existing assertion meaningful.
 *
 * `importOriginal` is spread in for the same reason `reverseproxy/edge`'s mock below is: a whole-module replacement
 * makes `rebindRuntimePorts` `undefined` and turns the call into a `TypeError` inside the command.
 */
vi.mock("@bifurc/engine/runtime/instances", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@bifurc/engine/runtime/instances")>()),
  notifyConfigChanged: vi.fn(),
}));

vi.mock("@bifurc/engine/kernel/telemetry", () => ({
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
vi.mock("@bifurc/engine/webhooks/webhookServer", () => ({
  registerActiveWebhook: vi.fn(),
  unregisterActiveWebhook: vi.fn(),
  startWebhookServer: vi.fn(),
  stopWebhookServer: vi.fn(),
  isWebhookServerRunning: vi.fn(() => false),
  getWebhookPort: vi.fn(() => 9101),
  getWebhookServerError: vi.fn(() => null),
}));

/**
 * `plan/17` §5 work item 7 — `config.save` re-binds the **edge** when `settings.port` changes, and the edge is
 * the one listener reached through a module-level binding rather than through a facade with a lazy default.
 *
 * **`importOriginal` is spread in rather than the module being replaced**, and that is deliberate: this file
 * already learned the lesson once (`plan/17` §7's measured −10 — a whole-module `vi.mock` left every
 * `DEFAULT_*` that `store/config.ts` reads `undefined`). `configCommands.ts` imports this module as a value, so
 * a replacement mock would make `restartEdge` the only defined export and turn any future import from it into
 * a `TypeError` inside an unrelated command. Only the two names under assertion are stubbed.
 */
vi.mock("@bifurc/engine/reverseproxy/edge", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@bifurc/engine/reverseproxy/edge")>()),
  restartEdge: vi.fn(),
  // `true`, so the **port-change** arm is the one that fires. With `false` the `!isEdgeRunning()` recovery arm
  // would restart on *every* save, and the `config.save` case below would pass for the wrong reason.
  isEdgeRunning: vi.fn(() => true),
}));

vi.mock("@bifurc/engine/gitops/gitStore", () => ({
  commitMutation: vi.fn(() => Promise.resolve("abc123")),
  queryLog: vi.fn(() => Promise.resolve({ entries: [], total: 0 })),
  getEntityAtCommit: vi.fn(() => Promise.resolve(null)),
  getCommitChangedFiles: vi.fn(() => Promise.resolve([])),
  initWorkspaceRepo: vi.fn(() => Promise.resolve()),
  AuditEntity: {},
  AuditAction: {},
}));

vi.mock("@bifurc/engine/workspace/workspaceFs", () => ({
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

vi.mock("@bifurc/engine/workspace/appSettings", () => ({
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

import type { AppConfig, LocalMapping, ProxyRule, MockRule, SavedRequest, Folder, Environment, Workspace, SavedWsConnection } from "@bifurc/engine/workspace/config";

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

vi.mock("@bifurc/engine/workspace/config", () => ({
  loadConfig: vi.fn(() => currentConfig),
  saveConfig: vi.fn((cfg: AppConfig) => { currentConfig = cfg; }),
  generateId: vi.fn(() => `id-${Date.now()}`),
  loadEntity: vi.fn(() => null),
  /**
   * Derived from the same `currentConfig` the mocked `loadConfig` serves, so the two cannot disagree.
   *
   * They must not: `workspaceCfg()` filters the slice's entities by the enabled sets, so a slice that
   * disagreed with `loadConfig()` would silently change dispatch — the failure mode
   * `tests/proxy/server.test.ts`'s instrument comment records in full.
   */
  loadWorkspaceDispatchSlice: vi.fn((wsId: string) => ({
    workspaceId: wsId,
    mocks: currentConfig.mocks ?? [],
    mappings: currentConfig.mappings ?? [],
    environments: currentConfig.environments ?? [],
    activeEnvironmentId: currentConfig.activeEnvironmentId ?? null,
  })),
  // Types only — no runtime value needed for interfaces
}));

import { loadConfig, saveConfig, generateId, loadEntity } from "@bifurc/engine/workspace/config";
import { commitMutation, queryLog, getEntityAtCommit, getCommitChangedFiles } from "@bifurc/engine/gitops/gitStore";
import { startServer, stopServer, isRunning, getPort, getServerError, replayRequest } from "@bifurc/engine/proxy/server";
// `Engine_Module_Plan.md` §4.1 — the reload fan-out is asserted at the seam now, not at `proxy/server`. The
// proxy's own `reloadConfig` stays mocked above because `notifyConfigChanged` (real) calls it.
import { notifyConfigChanged } from "@bifurc/engine/runtime/instances";
// `plan/17` §5 work item 7 — `config.save` re-binds the edge, and the edge is the one listener reached through
// a module-level binding rather than through a facade with a lazy default.
import { restartEdge } from "@bifurc/engine/reverseproxy/edge";
import { discoverServices } from "@bifurc/engine/proxy/service-discovery";
import {
  registerActiveWebhook,
  unregisterActiveWebhook,
  startWebhookServer,
  stopWebhookServer,
  isWebhookServerRunning,
  getWebhookPort,
  getWebhookServerError,
} from "@bifurc/engine/webhooks/webhookServer";
import { replayRequest as replayRequestImpl } from "@bifurc/engine/proxy/serverReplay";
import { dialog, BrowserWindow } from "electron";
import * as fs from "fs";
import { commandRegistry } from "@bifurc/engine/kernel/registry";
import { bus } from "@bifurc/engine/kernel/eventBus";

// ── Helper: get registered handler ───────────────────────────────────────────

function getHandler(channel: string) {
  const h = registeredHandlers.get(channel);
  if (!h) throw new Error(`No handler registered for channel: ${channel}`);
  return h;
}

// Fake ipcMain event arg (first arg to handlers is the IPC event, typically ignored)
const EVENT = {} as any;

/**
 * The collapsed `entity.*` commands, driven directly — the replacement for the per-kind
 * `mapping:*` / `rule:*` / `mock:*` / `request:*` / `ws:*` / `env:*` channels, which are the step-3
 * reduction's target (`plan/07` §5b-ter).
 *
 * Each of those channels was a one-line `commandRegistry.invoke()` adapter, so calling the command is
 * what the channel did — minus the channel, and minus the factory's `toProtocolKind()`. That map's only
 * non-identity entries are `rules → proxyRules` and `sockets → wsConnections`, which is why the `rule:*`
 * cases below pass `"proxyRules"` and the `ws:*` cases pass `"wsConnections"`: the engine's
 * `toEngineKind()` still translates them back to the storage kind, and that is what the assertions on
 * `writeEntity(…, "rules")` / `(…, "sockets")` pin.
 *
 * **`create` unwraps `{id, entity}`**, because the channel did (`entityCrudFactory.ts:72`) and the cases
 * below read the created entity's own fields; without it they would assert against the envelope and pass
 * on `id` alone — the defect `plan/07` §5b-quater records one layer up, on the client. The gate-blocked
 * `environments` create returns `{error:"limit_reached", …}` with no `.entity`, so it passes through
 * unchanged, exactly as before.
 */
const ENTITY_CTX = { bus };
const createEntity = async <T = any>(kind: string, entity: any): Promise<T> => {
  const r: any = await commandRegistry.invoke(
    "entity.create",
    { kind, entity, workspaceId: entity.workspaceId },
    ENTITY_CTX,
  );
  return ("entity" in r ? r.entity : r) as T;
};
const updateEntity = (kind: string, entity: any) =>
  commandRegistry.invoke("entity.update", { kind, entity }, ENTITY_CTX);
const deleteEntity = (kind: string, id: string) =>
  commandRegistry.invoke("entity.delete", { kind, id }, ENTITY_CTX);
const listEntities = async <T = any>(kind: string): Promise<T[]> => {
  const r: any = await commandRegistry.invoke(
    "entity.list",
    { kind, workspaceId: loadConfig().activeWorkspaceId },
    ENTITY_CTX,
  );
  return r.entities as T[];
};

/**
 * The `history.*` commands, driven directly — the same substitution as the `entity.*` helpers above,
 * for the P6 reduction's **second** group (`plan/07` §5b-terdecies).
 *
 * `history:list` and `history:diff` were deleted with `src/ipc/handlers/syncHandlers.ts` on 2026-09-21.
 * Both were one-line `commandRegistry.invoke()` adapters — `history:list` forwarded its whole argument
 * object, `history:diff` mapped four positional args — so calling the command is what the channel did,
 * minus the channel. The commands are not merely equivalent here, they are where the behaviour moved:
 * `history.list` is what resolves `workspaceId` from the active workspace and defaults `limit`/`offset`,
 * and the assertions below (active-workspace fallback, 100/0 defaults) pin exactly those steps.
 */
const historyList = <T = any>(opts: any): Promise<T> =>
  commandRegistry.invoke("history.list", opts, ENTITY_CTX);
const historyDiff = <T = any>(commitHash: string, filePath: string, workspaceId: string): Promise<T> =>
  commandRegistry.invoke("history.diff", { commitHash, filePath, workspaceId }, ENTITY_CTX);

/**
 * The `folder.*` commands, driven directly — the same substitution again, for the reduction's
 * **third** group (`plan/07` §5b-quaterdecies).
 *
 * `folder:add` / `folder:rename` / `folder:move` / `folder:delete` were deleted with
 * `src/ipc/handlers/folderHandlers.ts` on 2026-09-21; all four were one-line
 * `commandRegistry.invoke()` adapters, so calling the command is what the channel did minus the
 * channel. **`folderAdd` keeps the channel's spread** (`{ kind, ...folder }`) rather than the
 * client's field-picking, because the cases below pass a folder object and the spread is the
 * behaviour being reproduced — see the note in `foldersAndWorkspaces.integration.test.ts` for why
 * the two shapes are equivalent for the renderer but not for that suite.
 */
const folderAdd = (kind: string, folder: any): any =>
  commandRegistry.invoke("folder.add", { kind, ...folder }, ENTITY_CTX);
const folderRename = (kind: string, id: string, name: string): any =>
  commandRegistry.invoke("folder.rename", { kind, id, name }, ENTITY_CTX);
const folderDelete = (kind: string, id: string): any =>
  commandRegistry.invoke("folder.delete", { kind, id }, ENTITY_CTX);
// No `folderMove` helper: this file has no `folder:move` case, because `folder:move` was the one of the
// four that `expectedChannels` never listed either. Its behaviour is driven for real — against a real
// workspace on disk — by `tests/integration/foldersAndWorkspaces.integration.test.ts`.

/**
 * The `coreHandlers.ts` commands, driven directly — the same substitution once more, for the
 * reduction's **sixth** group (`plan/07` §5b-septendecies).
 *
 * All seventeen of that file's channels were deleted on 2026-09-21. Unlike groups 1–4 they were not
 * all one-line `invoke()` adapters, so the criterion (*owns a step the engine does not perform*) had to
 * be re-applied per channel by reading the command — and it found exactly one, `entity:load`, whose
 * `toProtocolKind()` half the client twin had dropped. That is why this group opened with a fix-first
 * commit (`174a01d`) instead of a deletion.
 *
 * **Helpers only where a command has more than one case**, and the single-use ones are written inline
 * below. A helper for a lone call site is indirection that hides the payload mapping it exists to
 * expose; these four earn theirs by being the mapping written once instead of repeatedly.
 *
 * `env.setActive` and `workspace.setActive` are two of §5c's five, moved by a different slice — they
 * are here for the same reason as the rest: the channel that carried them is gone and the cases that
 * pinned their behaviour had nowhere else to live. The other three (`config.get`, `entity.load`,
 * `entity.setEnabled`) are reachable at the command level through `getConfig` / `entityLoad` and the
 * registration block respectively.
 */
const getConfig = (): AppConfig => commandRegistry.invoke("config.get", {}, ENTITY_CTX) as AppConfig;
const envSetActive = (id: string | null): any =>
  commandRegistry.invoke("env.setActive", { id }, ENTITY_CTX);
const workspaceSetActive = (id: string): any =>
  commandRegistry.invoke("workspace.setActive", { id }, ENTITY_CTX);
const entityLoad = (wsId: string, kind: string, id: string): any =>
  commandRegistry.invoke("entity.load", { workspaceId: wsId, kind, id }, ENTITY_CTX);

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
      // ── The seventeen `coreHandlers.ts` channels are gone (P6 reduction, sixth group, 2026-09-21) ──
      // `config:get`, `config:save`, `services:discover`, `entity:load`, `entity:setEnabled`,
      // `env:setActive`, `script:execute`, the four `workspace:*`, `request:replay`, `server:status`,
      // `proxy:status` and the three `healthbar:*` were deleted with the file's reduction to
      // `coreKinds.ts` (`plan/07` §5b-septendecies). Each was reachable only through the client, which
      // routes to the commands directly, so asserting a handler for one would now assert the absence
      // of the thing this list exists to check.
      //
      // **`entity:load` is the one that was not a passthrough**, and it is why this group opened with a
      // fix-first commit rather than a deletion: its body applied `toProtocolKind()` and the client twin
      // had dropped it, so the translation moved into `packages/client/src/index.ts` *before* the
      // channel went (`174a01d`). `entity:setEnabled`'s `invalid_kind` guard travelled with it.
      // ── The 18 per-kind CRUD channels are gone (P6 stage 3b) ──────────────────────────────────
      // `mapping:*`, `rule:*`, `mock:*`, `request:*`, `ws:*` and `env:add/update/delete` were generated
      // by `entityCrudFactory.ts` and deleted with it, so asserting a handler for them would now assert
      // the absence of the thing this list exists to check. Their behaviour is covered at engine level
      // by `entityCrudCommands`' suite and by `entityConflictScope`.
      // ── The three folder channels are gone (P6 reduction, third group, 2026-09-21) ──────────
      // `folder:add` / `folder:rename` / `folder:delete` were deleted with `folderHandlers.ts`
      // (`plan/07` §5b-quaterdecies). Each was a one-line `commandRegistry.invoke()` delegate
      // reachable only through the client, which routes to the `folder.*` commands directly —
      // the cases below now drive those commands. `folder:move` was never in this list.
      // ── The ten `application.*` channels are gone (P6 reduction, seventh group, 2026-09-21) ──
      // `applications:list` / `:save` / `:delete` / `:start` / `:stop` / `:getState` /
      // `:getAllStates` / `:getLogs` / `:checkPort` / `:killPort` were deleted with
      // `src/ipc/applicationHandlers.ts` (`plan/07` §5b-octodecies). Each was a one-line
      // `commandRegistry.invoke()` delegate reachable only through the client, which routes to the
      // `application.*` commands directly — so asserting a handler for one would now assert the
      // absence of the thing this list exists to check. No case here drove them by name either:
      // `tests/integration/applications.integration.test.ts` is their only suite and it now drives the
      // commands, which is where the behaviour always lived.
      // ── The six `runner.*` channels are gone (P6 reduction, eighth group, 2026-09-21) ────────
      // `runner:saveReport` / `runner:exportReport` / `runner:getHistory` / `runner:saveConfig` /
      // `runner:loadConfig` / `runner:listFolderIds` were deleted with
      // `src/ipc/handlers/runnerHandlers.ts` (`plan/07` §5b-novendecies), so asserting a handler for
      // one would now assert the absence of the thing this list exists to check. No case here ever
      // drove one by name: `tests/integration/runnerStorage.integration.test.ts` is their only suite
      // and it now drives the commands, which is where the behaviour always lived.
      //
      // **Two of these were the group's whole reason for care.** `runner:saveReport` and the
      // `runner:saveConfig` / `runner:loadConfig` pair were **self-contained duplicates** rather than
      // `commandRegistry.invoke()` delegates — `runnerCommands.ts`'s header records that the obvious
      // DRY fix was tried in `serverCommands.ts` and reverted, because a channel that delegates to a
      // module registered only inside `registerIpcHandlers()` acquires a hidden ordering dependency.
      // Here the duplication was a *coverage* hazard instead: this suite's `expectedChannels` never
      // listed them (so delta 0 was not evidence), while the integration suite drove the shell copies
      // and would have stayed green with the engine halves rotted. `runner:exportReport` was the
      // third shape — not a duplicate at all, but the P3 client composition (dialog → format from the
      // chosen extension → engine render → artifact write), which lives in `packages/client/src`.
      // ── The four `importExport:*` channels are gone (P6 reduction, ninth and last group, ─────
      // 2026-09-21) ─────────────────────────────────────────────────────────────────────────────
      // `importExport:formats` / `:export` / `:preflight` / `:import` were deleted with
      // `src/ipc/importExportHandlers.ts` (`plan/07` §5b-vicendecies). **These four are the only
      // group in the whole reduction that DID have generated cases here**, so this is the only place
      // the ratchet's list actually shrank (4 → 0) — every earlier group's delta came from a suite
      // re-point or from nothing at all.
      //
      // They are also the only group whose bodies were **not delegates**: they were the P3 *client
      // half* — `dialog.*` + bytes + the `pendingUploads` map — so "dead" here means reachable only
      // through the client, which composes the same steps in `packages/client/src/index.ts`:
      // `getImportExportFormats` (`:584`), `exportData` (`:585`), `preflightImport` (`:626`) and
      // `importData` (`:653`). That file's `KIND_LABEL` docblock (`:354`) named this deletion as the
      // trigger for its copy becoming the only one, so the equivalence was recorded before the
      // deletion rather than argued after it. The three multi-step twins carry two `describe` blocks
      // and 22 references in `packages/client/tests/client.test.ts`, including the ordering
      // assertions ("asks where BEFORE it asks the engine to render") that this file could not make.
      // ── The five audit/history channels are gone (P6 reduction, second group, 2026-09-21) ──────
      // `audit:list`, `audit:diff`, `audit:export`, `history:list` and `history:diff` were deleted
      // with `syncHandlers.ts` (`plan/07` §5b-terdecies). Each was reachable only through the client,
      // which routes to the `audit.*` / `history.*` **commands** directly, so asserting a handler here
      // would now assert the absence of the thing this list exists to check. Their behaviour is
      // covered at engine level — `store/auditCommands.ts` and `sync/syncCommands.ts`.
      // ── The three server-lifecycle channels are gone (P6 reduction, fifth group, 2026-09-21) ──
      // `server:restart` / `server:stop` / `server:start` were deleted with the four dead channels in
      // `src/ipc/handlers/systemHandlers.ts` (`plan/07` §5b-sedecies). They were the first group whose
      // bodies were **not** `commandRegistry.invoke()` delegates — they called `startServer` /
      // `stopServer` directly — so the criterion was re-applied per channel against
      // `proxy/serverCommands.ts` rather than inherited, and it found exact equivalents.
      // Their behaviour is asserted at the command level in the "delivers the 21 moved commands"
      // block below, which is why no case here needed re-pointing.
      // `app:checkUpdate` **stays**: it is client-local, not legacy.
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
     * `packages/engine/src/workspace/entities/configCommands.ts` (19 → 15); the five `webhook.*` /
     * `webhookServer.*` commands into `packages/engine/src/webhooks/webhookCommands.ts` (15 → 10); the
     * five `misc.ts` commands into `packages/engine/src/miscCommands.ts` (10 → 5); and
     * `runner.saveReport` into `packages/engine/src/runner/runnerCommands.ts` (5 → 4). Every edit is
     * the ratchet doing its job rather than a change to the assertion.
     *
     * **What is left is nothing.** The last entry was the `SPLIT`, and it is resolved as of
     * 2026-09-20 — but **not by moving it and not by widening a schema**: the command was *deleted
     * from the protocol*, because there turned out to be no engine half to contract with. See the
     * rewritten `SPLIT` test below. The other three were all **retired by widening the schemas they
     * were blocked on**, not by moving them: `audit.list` (`NARROWED`, 2026-09-19) and
     * `runner.saveConfig` / `runner.loadConfig` (`BLOCKED`, 2026-09-19). So step 3 now ends at
     * **every protocol command registered**, with no pinned list at all.
     *
     * ## Why the list kept shrinking, and why every category is now gone
     *
     * The first version of this block said every remaining command was "engine work that has not been
     * moved yet". **Auditing the remaining 19 against their frozen schemas showed that is false**, and
     * `src/ipc/handlers/runnerHandlers.ts` had already recorded the reason for three of them: a
     * protocol schema that contradicts what real callers actually send. So the entries were split by
     * *why* they were unimplemented:
     *
     * - `MOVABLE` — the schema accepts every payload the handler accepts. Move it. → **all 21 moved.**
     * - `NARROWED` — the schema accepts a strict subset, and `.strict()` turns the difference into a
     *   `BAD_REQUEST`. Needs a decision, not a move. → **widened.**
     * - `BLOCKED` — the schema rejects payloads that a **currently passing** test uses. → **widened.**
     * - `SPLIT` — the protocol declares an engine-half contract that deliberately differs from the
     *   shell's implementation. Not a move at all. → **resolved 2026-09-20 by deleting the command:
     *   the whole operation is client-local, so there was no engine half.**
     *
     * Every retired category was deleted rather than left empty, and the reason is the same each time:
     * an empty array invites the next reader to file the next command that *looks* like it belongs
     * there, instead of asking which of the four questions actually applies.
     *
     * **The pattern worth carrying forward:** three of the four categories turned out to be *schema*
     * problems rather than *routing* problems. "The command is not in the registry" was never the same
     * claim as "the engine cannot serve it" — and in every case the fix was to correct the frozen
     * schema to describe what the real caller sends, then move the command.
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
       * What remains is deliberately *not* a to-do list of the same kind: it is two `BLOCKED` and one
       * `SPLIT`, each pinned with the reason it cannot simply be moved. Keeping an empty `MOVABLE`
       * array would invite the next reader to file the next unimplemented command there by default;
       * deleting it forces the classification to be argued again.
       */

      /**
       * `NARROWED` used to live here too, holding exactly one command — `audit.list` — and it is
       * **retired rather than empty**, for the same reason `MOVABLE` was deleted.
       *
       * Its entry said the schema was narrower than the handler: `AuditListParams` was `.strict()` and
       * had no `filePath` / `fromTs` / `toTs`, while `QueryLogOptions` — the type the handler takes —
       * has all three. `.strict()` makes that difference a `BAD_REQUEST` rather than a dropped filter.
       *
       * **The caveat that entry gave for calling it latent rather than live was wrong, and the way it
       * was wrong is the part worth keeping.** It said "nothing currently passes those — `auditList`
       * exists on `window.api` and no renderer code calls it". That check was a grep for the *method
       * name*, and `packages/gui/src/panels/AuditLogPanel.tsx:61–62` reaches the same command through
       * `window.api` and sets `fromTs`/`toTs` whenever its date filter is used. So the narrowing was
       * **live**, and routing this command as it stood would have broken the audit panel's date
       * filter — invisibly to every test here, because these tests drive the *handler*, not the panel.
       *
       * The schema was widened on 2026-09-19 and `packages/engine/src/workspace/entities/auditCommands.ts` now
       * implements it. The lesson generalises past this command: **"no caller found by grepping the
       * method name" is not evidence that there is no caller.**
       */

      /**
       * `BLOCKED` used to live here, holding `runner.saveConfig` and `runner.loadConfig` — and it is
       * **retired rather than empty**, for the third time and the same reason: an empty array invites
       * the next reader to file the next blocked command there instead of resolving it.
       *
       * Its entries were blocked by a **currently passing test** as the evidence — the strongest kind
       * available — and the block was asserted *directly* rather than by membership, so that widening
       * the schema would fail the test and say these had become movable. That is exactly what happened
       * on 2026-09-19: the assertion below failed, `RunnerConfigSchema` was widened from
       * `{requestOrder, delayMs}` to the `z.record` shape the store actually persists, and both
       * commands moved into `packages/engine/src/runner/runnerCommands.ts`.
       *
       * The guard that replaced it is the test below, and it asserts the *other* half of the old bug —
       * the half `BLOCKED` never caught. The frozen schema would have **stripped** unknown keys even
       * from a payload it accepted, because `z.object()` drops them, which is a data-loss bug rather
       * than a validation error.
       */

      /**
       * `SPLIT` is **empty as of 2026-09-20**, and it is empty because the entry was *deleted* rather
       * than moved — which is why the array is kept and asserted instead of being removed with it.
       *
       * `app.checkUpdate` was the surface's only `SPLIT`: `misc.ts` declared an "interim engine-half
       * contract" and `plan/12` said to move it fully client-side. Resolving it showed there was no
       * engine half to contract with. `currentVersion` is the version of the thing the **user** runs,
       * and `downloadUrl` must be the asset for the **user's** platform — an engine matching its own
       * `process.platform` hands a Windows user the AppImage. So the command left the protocol
       * entirely and the key became `local` in `@bifurc/client`.
       *
       * The array survives for one reason: it is what fails if someone re-adds `app.checkUpdate` to
       * the protocol without implementing it. A deleted entry that still has a live assertion is a
       * guard; a deleted entry with nothing left behind is a hole.
       */
      const SPLIT: string[] = [];

      const NOT_IN_REGISTRY = [...SPLIT];

      it("implements every protocol command — the pinned list is empty", async () => {
        const { COMMANDS } = await import("@bifurc/protocol");
        const { commandRegistry } = await import("@bifurc/engine/kernel/registry");

        // Non-vacuity. An empty or unreadable `COMMANDS` would make the assertion below pass while
        // checking nothing, and this is the one test that is supposed to notice a missing
        // implementation.
        expect(Object.keys(COMMANDS).length).toBeGreaterThan(80);

        const missing = Object.keys(COMMANDS)
          .filter((c) => !commandRegistry.isRegistered(c as never))
          .sort();

        expect(missing).toEqual([...NOT_IN_REGISTRY].sort());
      });

      it("keeps the runner config widening — accepted AND preserved", async () => {
        // The old `BLOCKED` assertion checked the block, which meant asserting that the real config was
        // *rejected*. Now that it is accepted, the assertion worth having is the one the block could
        // never express: that parsing does not **lose** anything.
        //
        // Both halves were live bugs. The frozen schema required `requestOrder`, so it rejected the
        // payload `tests/integration/runnerStorage.integration.test.ts:207` round-trips; and because
        // `z.object()` drops unknown keys, a payload that *had* passed would have lost `stopOnFailure`
        // and `iterations` on the way to disk. A dropped field in saved run configuration is silent —
        // nothing reports it and the file looks fine — which is why `toEqual` on the whole config is
        // the assertion, rather than a check that the two fields merely parse.
        const { RunnerSaveConfigParams } = await import("@bifurc/protocol");

        const realConfig = { delayMs: 250, stopOnFailure: true, iterations: 3 };
        const parsed = RunnerSaveConfigParams.safeParse({
          workspaceId: "ws",
          folderId: "f",
          config: realConfig,
        });

        expect(parsed.success).toBe(true);
        expect(parsed.success && parsed.data.config).toEqual(realConfig);

        // And the renderer's own shape, which *does* send `requestOrder`
        // (`useCollectionRunner.ts:67`) — the widening made it optional, not unsupported.
        expect(
          RunnerSaveConfigParams.shape.config.safeParse({ requestOrder: ["r1", "r2"], delayMs: 0 })
            .success,
        ).toBe(true);
      });

      it("keeps audit.list's widening — the date filter the panel actually sends", async () => {
        // Nothing else pins this. The retired `NARROWED` and `BLOCKED` entries used to assert their own
        // *block* in the opposite direction, which is what made a widening fail loudly rather than pass
        // silently; `audit.list` has left the list entirely now, so without this test the three fields
        // could be dropped again and the only symptom would be the audit panel's date filter quietly
        // returning everything.
        //
        // The payload is exactly what `packages/gui/src/panels/AuditLogPanel.tsx:61–62` builds when a
        // date range is set — which is the whole reason the widening was necessary rather than
        // optional: under the pre-2026-09-19 `.strict()` schema this was a `BAD_REQUEST`.
        const { AuditListParams } = await import("@bifurc/protocol");

        expect(
          AuditListParams.safeParse({ entity: "mock", fromTs: 1, toTs: 2, limit: 50 }).success,
        ).toBe(true);
        expect(AuditListParams.safeParse({ filePath: "mocks/root/mock_a.json" }).success).toBe(true);

        // Still `.strict()`: the widening added three named fields, it did not open the schema. A
        // typo'd filter must keep failing loudly rather than being silently ignored.
        expect(AuditListParams.safeParse({ fromTS: 1 }).success).toBe(false);
      });

      it("records that the SPLIT was resolved by deletion, and cannot return silently", async () => {
        // The old assertion here checked that `AppCheckUpdateParams` existed and was empty — it
        // asserted the *interim* state, so it had to be rewritten the moment the split was resolved.
        // These are the assertions that should have been written: the protocol has **no**
        // `app.checkUpdate`, and the surface classifies `checkUpdate` as `local`.
        //
        // Both halves matter and they fail for different reasons. Re-adding the command to `COMMANDS`
        // breaks the first — the loop above would then demand a registry implementation for a command
        // that has none and should not have one. Re-classifying the surface key back to `transport`
        // breaks the second. Either way the ratchet goes red naming the mistake, rather than a
        // downstream `UNKNOWN_COMMAND` or a silent "up to date" in a shipped build.
        const { COMMANDS } = await import("@bifurc/protocol");
        const { SURFACE } = await import("../../packages/client/src/surface");

        expect(Object.keys(COMMANDS)).not.toContain("app.checkUpdate");
        expect(SPLIT).toEqual([]);
        expect(SURFACE.checkUpdate).toMatchObject({ kind: "local" });
      });

      it("leaves no command both unregistered and unreachable", async () => {
        // This test used to iterate the pinned list. With that list empty it would now iterate
        // **nothing and pass while checking nothing** — the "a guard that can silently match nothing
        // is worse than no guard" failure `TRAPS.md` records. So it is rewritten around the
        // *invariant* rather than around the list, and stays sharp however the list changes.
        //
        // The invariant: every protocol command is either served by the registry, or still has a
        // legacy channel. A command that is neither is unreachable on both paths at once — a silent
        // regression the surface test cannot see, because the key stays exposed either way.
        const { COMMANDS } = await import("@bifurc/protocol");
        const { commandRegistry } = await import("@bifurc/engine/kernel/registry");

        const all = Object.keys(COMMANDS);
        expect(all.length).toBeGreaterThan(80);

        const unreachable = all.filter((command) => {
          if (commandRegistry.isRegistered(command as never)) return false;
          const channel = (COMMANDS as Record<string, { legacyChannel?: string }>)[command]
            ?.legacyChannel;
          return !channel || !registeredHandlers.has(channel);
        });

        expect(unreachable).toEqual([]);

        // Non-vacuity for the branch that carries the weight: this assertion is only interesting
        // because the *registry* covers everything. If the channels were doing the covering instead,
        // the registry would be short and the filter above would be exercising its fallback.
        const registered = all.filter((c) => commandRegistry.isRegistered(c as never));
        expect(registered.length).toBe(all.length);
      });

      it("every routed surface entry names a command the registry implements", async () => {
        // This replaces "confirms the client claims every pinned command is routable", which
        // iterated the pinned list and would now iterate nothing. The finding that test encoded —
        // *the client advertises methods the registry cannot serve* — is closed, so the assertion
        // worth keeping is its contrapositive, over a set that is never empty:
        //
        //   for every surface entry classified `transport` or `shim`, the command it names is
        //   registered.
        //
        // That is the invariant the ratchet existed to protect, stated directly rather than via a
        // list of known offenders. It fails the moment a routed key is added whose command nobody
        // implemented — the actual bug class — and the old list-based form could only catch that for
        // commands already on the list.
        //
        // Imported relatively because `SURFACE` is not re-exported from the package root (only
        // `surface.ts` has it) and the alias in `vitest.config.ts` covers `@bifurc/engine/*` only.
        // Read-only data, so the relative hop costs nothing but honesty about where the contract lives.
        const { SURFACE } = await import("../../packages/client/src/surface");
        const { commandRegistry } = await import("@bifurc/engine/kernel/registry");

        const routed = Object.entries(SURFACE)
          .filter(([, e]) => e.kind === "transport" || e.kind === "shim")
          .map(([key, e]) => [key, (e as { command: string }).command] as const);

        // Non-vacuity: this set is the entire subject of the test, and an empty one would pass.
        // Measured 2026-09-20: **73** entries (67 `transport` + 6 `shim`) — 75 earlier the same day,
        // before `tlsRemoveCert` and `tlsCertStatus` moved to `local` (`plan/07` §5b-bis). The three
        // TLS shims were reclassified from `transport` later the same day and the total did not move,
        // which is the point: a reclassification cannot change what this test covers. A floor rather
        // than the exact figure, because `packages/client/tests/surface.test.ts` already pins the
        // per-kind counts exactly and this guard only needs to prove the set is populated. It was
        // written as `> 80` first and failed at 75 — which is the guard doing its job on its author.
        expect(routed.length).toBeGreaterThan(60);

        const unserved = routed
          .filter(([, command]) => !commandRegistry.isRegistered(command as never))
          .map(([key, command]) => `${key} → ${command}`);

        expect(unserved).toEqual([]);

        // The entry this test used to single out, now in its resolved form. `checkUpdate` is `local`
        // and therefore deliberately absent from `routed` above — which is the whole point of the
        // reclassification, so it is asserted rather than left implied.
        expect(SURFACE.checkUpdate).toMatchObject({ kind: "local" });
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
        const { commandRegistry } = await import("@bifurc/engine/kernel/registry");
        const { bus } = await import("@bifurc/engine/kernel/eventBus");
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

        // `start` / `stop` / `restart` read the port from `getPort()` — the **resolved** port — rather than
        // taking it as a parameter, so the assertion is on the engine calls, not on the returned `{ok:true}`.
        //
        // **`8080`, the `getPort()` mock, and not `currentConfig.port` (80) — `plan/15` work item 3.** The
        // facade branch is `startServer(getPort() || loadConfig().port)`: `settings.port` names the **edge**
        // after the split, so a proxy bound there would collide with the engine's own listener. The resolved
        // port wins whenever there is one, and `getPort()` is mocked to 8080 above. The config-port fallback
        // survives for a process that starts a proxy before any is running — a path this case does not take,
        // and asserting `currentConfig.port` here would pin the fallback instead of the live behaviour.
        expect(commandRegistry.invoke("server.start", {}, ctx)).toEqual({ ok: true });
        expect(startServer).toHaveBeenCalledWith(8080);
        expect(commandRegistry.invoke("server.stop", {}, ctx)).toEqual({ ok: true });
        expect(stopServer).toHaveBeenCalled();

        // `restart` is stop-then-start, not a distinct code path — and the order is the contract:
        // starting before stopping would rebind a port the old listener still holds.
        //
        // `startServer(<resolved port>)` is asserted here rather than in a case of its own: the deleted
        // `server:restart` handler had one, the reduction's fifth group removed it as a duplicate of this
        // block, and this is the one assertion it made that this block did not. Same `8080` as `server.start`
        // above, and for the same reason — `restart` reads `getPort()` before it stops, so the port it
        // rebinds is the one the listener had rather than the one `app.json` now names.
        vi.mocked(startServer).mockClear();
        vi.mocked(stopServer).mockClear();
        expect(commandRegistry.invoke("server.restart", {}, ctx)).toEqual({ ok: true });
        expect(vi.mocked(stopServer).mock.invocationCallOrder[0])
          .toBeLessThan(vi.mocked(startServer).mock.invocationCallOrder[0]);
        expect(startServer).toHaveBeenCalledWith(8080);

        // Folded from the deleted `services:discover` case. The mock in `beforeEach` returns `[]`, and
        // `toEqual([])` cannot tell "passed through" from "replaced with an empty array" — the deleted
        // channel case used a non-empty list, so this one does too.
        const discovered = [{ port: 3000, address: "127.0.0.1", pid: 100, processName: "node" }];
        vi.mocked(discoverServices).mockReturnValue(discovered);
        expect(commandRegistry.invoke("services.discover", {}, ctx)).toEqual(discovered);
        expect(discoverServices).toHaveBeenCalled();

        // ── slice 2: config.save and the three workspace.* commands ──────────

        /**
         * `config.save` — **the port branches are inverted by `plan/17` §5 work item 7**, and this comment used
         * to say the opposite of what follows.
         *
         * It read: *"the port it rebinds is `8080`, the resolved one, not the `9876` just saved — `plan/15` work
         * item 3 … changing the port in Settings persists the preference for the edge's next boot and no longer
         * moves this listener."* That was the correct rule for `plan/15` and is the bug for `plan/17`:
         * `settings.port` is the **edge's** port, and a user who changes it means it. So a save now re-binds the
         * **edge** and leaves the forward proxy exactly where it is.
         *
         * Each assertion below fails against the old implementation, which is what makes the inversion a
         * measurement rather than a rewrite:
         *   - `restartEdge(9876)` — the edge moved, and to the port just saved;
         *   - `startServer` **not** called — the forward proxy's port is its own setting
         *     (`settings.forwardProxyPort`), which this save did not touch. The old code restarted it *because*
         *     the edge's port changed, which is exactly the coupling work item 7 removes;
         *   - `stopServer` **not** called — every `start()` in this engine already does `if (server) stop()`, so
         *     the explicit stop this branch used to make was a second way to say one thing.
         *
         * `settingsMutations.integration.test.ts` owns the behaviour end to end — the edge really moves and the
         * old port really goes dark. This case owns the *call shape*.
         *
         * The bus emission is **folded from the deleted `config:save` case** — it was the one assertion that
         * case made which this block did not. `settings.changed` is the shell's tray-update hook (P2), so losing
         * it would stop the tray following a settings change with every test still green.
         */
        const settingsChanged = vi.fn();
        bus.onTyped("settings.changed", settingsChanged);
        vi.mocked(startServer).mockClear();
        vi.mocked(stopServer).mockClear();
        vi.mocked(restartEdge).mockClear();
        expect(
          commandRegistry.invoke("config.save", { config: { ...makeDefaultConfig(), port: 9876 } }, ctx),
        ).toEqual({ ok: true });
        expect(saveConfig).toHaveBeenCalled();
        expect(restartEdge).toHaveBeenCalledWith(9876);
        expect(startServer).not.toHaveBeenCalled();
        expect(stopServer).not.toHaveBeenCalled();
        expect(settingsChanged).toHaveBeenCalled();
        bus.offTyped("settings.changed", settingsChanged);

        /**
         * **The other two ports, in the same save's shape.** `config.save` now names three listeners, and a
         * branch that read the wrong field would be indistinguishable from the outside — so all three are
         * asserted rather than only the one that changed.
         *
         * `port` stays at `9876` (what the previous save left in `currentConfig`, i.e. `prev`), so the edge's
         * branch must **not** fire. `forwardProxyPort` and `webhookPort` are absent from `makeDefaultConfig()`
         * and are given values here, so both branches see a change — and they must behave differently, because
         * `plan/17` §1 gives them different rules.
         */
        vi.mocked(restartEdge).mockClear();
        vi.mocked(startServer).mockClear();
        vi.mocked(startWebhookServer).mockClear();
        expect(
          commandRegistry.invoke(
            "config.save",
            { config: { ...makeDefaultConfig(), port: 9876, forwardProxyPort: 8081, webhookPort: 9123 } },
            ctx,
          ),
        ).toEqual({ ok: true });
        expect(startServer).toHaveBeenCalledWith(8081);
        /**
         * **The webhook is off by default**, so a port change moves a receiver that is *running* and never
         * starts one — starting it is the user's action (`webhookServer.start`, from the Webhooks panel), and a
         * settings save that bound a port nobody asked for would be this command inventing a feature toggle.
         * `isWebhookServerRunning` is mocked `false` in this file's `beforeEach`, so `not.toHaveBeenCalled()` is
         * the assertion rather than an omission.
         */
        expect(startWebhookServer).not.toHaveBeenCalled();
        // The edge's port did not change in this save, so its branch must not fire either.
        expect(restartEdge).not.toHaveBeenCalled();

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

        /**
         * **The `?? 9101` last resort is gone, and the case that asserted it is retired rather than fixed.**
         *
         * It read: delete `webhookPort` from the config, then expect `startWebhookServer(9101)` — "a config
         * written by an older release is exactly how it would be missing". `plan/17` §1 removed the constant,
         * and the reason the case cannot be restated is the interesting part: the chain is now
         * `params.port ?? loadConfig().webhookPort`, with **one link and no constant**, and `loadConfig()`
         * cannot return a missing `webhookPort` — `loadSettings()` merges `makeDefaultSettings()` under
         * whatever the file holds, so a legacy `app.json` without the key gets `randomWebhookPort()` rather
         * than `undefined`. The branch is therefore unreachable, and a test for an unreachable branch is a
         * test that can only ever be satisfied by re-introducing the thing that was deleted.
         *
         * The two cases above are what remains, and they cover the whole chain: no `port` on the wire falls
         * back to the config (9200), and a declared `port` wins (9300).
         */

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
        // The returned value is asserted too, folded from the deleted `request:replay` case: asserting
        // only the call would pass for a handler that called the engine and discarded the answer.
        const replayResult = { status: 200, headers: {}, body: "cmVzcA==" };
        vi.mocked(replayRequestImpl).mockResolvedValue(replayResult);
        expect(
          await commandRegistry.invoke(
            "request.replay",
            { method: "GET", url: "http://x", headers: { a: "b" }, body: "Ym9keQ==" },
            ctx,
          ),
        ).toEqual(replayResult);
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
        const { commandRegistry } = await import("@bifurc/engine/kernel/registry");
        const { bus } = await import("@bifurc/engine/kernel/eventBus");

        expect(() => commandRegistry.invoke("server.status", { unexpected: 1 }, { bus }))
          .toThrow(/Invalid payload for command "server\.status"/);
      });

      it("registers nothing at module scope — the §5c guard", async () => {
        /**
         * **The mechanism §5c needed and did not have.**
         *
         * The test above asks "is this command registered *in this process*" — and in this process the
         * shell had registered all 51 of them at module scope, so a containerised engine (P9) answered
         * `UNKNOWN_COMMAND` for commands that test called present. The ratchet was not wrong; it was
         * answering a different question, and the difference is invisible from inside one process.
         *
         * The question that would have caught it is **static**, and it is this: registration must go
         * through `register*Commands(registry)`, so the `commandRegistry` singleton is never registered
         * directly. That is not a style preference — it is what makes the engine containerisable,
         * because a consumer that supplies its own registry can only ever see commands registered
         * *explicitly* against it. `createEngine()` registers no commands by design for the same
         * reason.
         *
         * Checked as a **statement**, not a substring: `/^\s*commandRegistry\.register\(/` matches a
         * line whose first non-whitespace content is the call. That catches module scope at column 0
         * **and** a module-scope call wrapped in a top-level `if`/`try` block, while ignoring the two
         * engine comments that quote the §5c grep pattern mid-line.
         *
         * Two shapes it does **not** catch, stated rather than implied: a call preceded by other code
         * on the same line (`if (x) commandRegistry.register(...)`), and an indirect alias
         * (`const r = commandRegistry; r.register(...)`). Both are pathological, and the natural shape
         * is the one above — it is the shape all 51 took.
         */
        const read = async (glob: Record<string, () => Promise<unknown>>): Promise<Record<string, string>> =>
          Object.fromEntries(
            await Promise.all(Object.entries(glob).map(async ([file, load]) => [file, String(await load())])),
          );

        const shell = await read(import.meta.glob("../../src/**/*.ts", { query: "?raw", import: "default" }));
        const engine = await read(
          import.meta.glob("../../packages/engine/src/**/*.ts", { query: "?raw", import: "default" }),
        );

        /**
         * **Anti-vacuity, and it is load-bearing.** A wrong glob returns `{}`, and every assertion
         * below would then pass against an empty string — a guard that reads as coverage while
         * covering nothing, which is worse than no guard. Assert the reader found the tree first.
         *
         * **Expressed as presence, not a count** — and this is a correction the reduction forced.
         * It used to be `Object.keys(shell).length > 20`, which held while `src/` had 22 files.
         * `syncHandlers.ts` and `folderHandlers.ts` are now deleted (20 files), and six more handler
         * files are still to go (14 would remain), so a magic number coupled to the tree's size fails
         * as the tree gets *smaller for the right reason* — the one direction this guard must never
         * misreport, given that its entire subject is deletions. Naming files tests the actual claim
         * ("the reader resolved `src/`") and cannot drift as the reduction proceeds. Keys are
         * normalised because Vite's glob keys are platform-dependent.
         */
        const shellKeys = Object.keys(shell).map((k) => k.replace(/\\/g, "/"));
        expect(shellKeys).toContain("../../src/preload.ts");
        expect(shellKeys).toContain("../../src/ipc/handlers.ts");
        expect(shellKeys).toContain("../../src/ipc/rpcBridge.ts");
        // The engine tree is not eroding (the reduction moves *into* it), so a floor is still the right
        // shape here — and the positive control below independently proves the reader loaded content.
        expect(Object.keys(engine).length).toBeGreaterThan(20);

        /**
         * The positive control: the same reader **does** see real registration code, so a zero for the
         * singleton form means "not written that way" rather than "content never loaded". Without this
         * the test is indistinguishable from one whose reader silently returned nothing.
         */
        const parameterised = Object.values(engine).join("\n").match(/^\s*registry\.register\(/gm) ?? [];
        expect(parameterised.length).toBeGreaterThan(30);

        const offenders = [...Object.entries(shell), ...Object.entries(engine)]
          .filter(([, source]) => /^\s*commandRegistry\.register\(/m.test(source))
          .map(([file]) => file.replace(/\\/g, "/"));

        expect(offenders).toEqual([]);
      });
    });
  });

  // ── config:get ────────────────────────────────────────────────────────

  describe("config.get command (the deleted `config:get` channel)", () => {
    it("returns the current config", () => {
      currentConfig.port = 9090;
      const result = getConfig();
      expect(result.port).toBe(9090);
    });
  });

  // ── config:save — cases deleted ──────────────────────────────────────
  //
  // Four `describe`-level cases stood here. The reduction's sixth group deleted the channel
  // (`plan/07` §5b-septendecies) and **three of the four were already asserted** by the "delivers the
  // 21 moved commands through the registry" block, against the same mocks:
  //
  //  - "saves the incoming config and returns { ok: true }" — the block asserts both the `{ok:true}`
  //    return and `saveConfig` having been called;
  //  - "calls reloadConfig after saving" — it asserted `loadConfig` was called, which is true of every
  //    path in this file and is not a property of this command;
  //  - "restarts the server when the port changes" — the block asserts `stopServer` then
  //    `startServer(9876)`, which is the same contract with a sharper payload.
  //
  // The fourth — "emits `settings.changed` on the bus" — was **not** covered anywhere, so it was
  // folded into that block rather than dropped. That fold is `plan/07` §5b-septies' rule cutting in the
  // delete direction: re-point the cases whose assertions live only in the channel, delete the ones a
  // suite already makes, and *move* the residue rather than discarding it with the case.

  // ── services:discover — case deleted ─────────────────────────────────
  //
  // One case stood here asserting `discoverServices()`'s list is returned rather than dropped. The
  // "21 moved commands" block asserted `services.discover` against a mock returning `[]`, which cannot
  // distinguish "passed through" from "replaced with an empty array" — so the block now uses a
  // **non-empty** list and the case's assertion lives there. Same fold, same reason as `config:save`.

  // ── mapping:add ───────────────────────────────────────────────────────

  describe("entity.create — mappings", () => {
    it("adds a mapping and returns it with a generated id", async () => {
      const input: Omit<LocalMapping, "id"> = {
        domain: "app.localhost",
        target: "localhost:3000",
        enabled: true,
      };

      const result = await createEntity("mappings", input);

      expect(result.id).toBeTruthy();
      expect(result.domain).toBe("app.localhost");
      expect(currentConfig.mappings).toHaveLength(1);
    });

    it("persists the mapping in config via saveConfig", async () => {
      await createEntity("mappings", { domain: "x.localhost", target: "localhost:4000", enabled: true });
      expect(saveConfig).toHaveBeenCalled();
    });
  });

  // ── mapping:update ────────────────────────────────────────────────────

  describe("entity.update — mappings", () => {
    it("updates an existing mapping", () => {
      const mapping: LocalMapping = { id: "m1", domain: "old.localhost", target: "localhost:1", enabled: true };
      currentConfig.mappings = [{ ...mapping }];

      updateEntity("mappings", { ...mapping, target: "localhost:9999" });

      expect(currentConfig.mappings[0].target).toBe("localhost:9999");
    });

    it("returns { ok: true } on success", async () => {
      currentConfig.mappings = [{ id: "m1", domain: "x.localhost", target: "localhost:1", enabled: true }];
      const result = await updateEntity("mappings", { id: "m1", domain: "x.localhost", target: "localhost:2", enabled: true });
      expect(result).toEqual({ ok: true });
    });

    it("does nothing when the mapping id does not exist", () => {
      currentConfig.mappings = [];
      expect(() =>
        updateEntity("mappings", { id: "nonexistent", domain: "x.localhost", target: "localhost:1", enabled: true }),
      ).not.toThrow();
    });
  });

  // ── mapping:delete ────────────────────────────────────────────────────

  describe("entity.delete — mappings", () => {
    it("removes the mapping with the given id", () => {
      currentConfig.mappings = [
        { id: "m1", domain: "a.localhost", target: "localhost:1", enabled: true },
        { id: "m2", domain: "b.localhost", target: "localhost:2", enabled: true },
      ];

      deleteEntity("mappings", "m1");

      expect(currentConfig.mappings).toHaveLength(1);
      expect(currentConfig.mappings[0].id).toBe("m2");
    });

    it("also removes proxy rules that target the deleted mapping", () => {
      currentConfig.mappings = [{ id: "m1", domain: "x.localhost", target: "localhost:1", enabled: true }];
      currentConfig.proxyRules = [
        { id: "r1", name: "rule", pattern: ".*", targetMappingId: "m1", enabled: true },
        { id: "r2", name: "rule2", pattern: ".*api.*", targetMappingId: "m2", enabled: true },
      ];

      deleteEntity("mappings", "m1");

      expect(currentConfig.proxyRules).toHaveLength(1);
      expect(currentConfig.proxyRules[0].id).toBe("r2");
    });

    it("returns { ok: true }", async () => {
      currentConfig.mappings = [{ id: "m1", domain: "x.localhost", target: "localhost:1", enabled: true }];
      const result = await deleteEntity("mappings", "m1");
      expect(result).toEqual({ ok: true });
    });
  });

  // ── rule:add ──────────────────────────────────────────────────────────

  describe("entity.create — proxyRules", () => {
    it("adds a proxy rule and returns it with a generated id", async () => {
      const input: Omit<ProxyRule, "id"> = {
        name: "API rule",
        pattern: ".*\\.api\\.com.*",
        targetMappingId: "m1",
        enabled: true,
      };

      const result = await createEntity("proxyRules", input);

      expect(result.id).toBeTruthy();
      expect(result.name).toBe("API rule");
      expect(currentConfig.proxyRules).toHaveLength(1);
    });

    it("prepends the new rule, the way `mocks` does — the two share one branch", async () => {
      // `rules` and `mocks` are the only two kinds that prepend, and they are the two the engine now
      // selects by `opts.kind`. Pinning `rules` as well as `mocks` is what makes the branch's *extent*
      // observable rather than its mere existence.
      currentConfig.proxyRules = [
        { id: "existing", name: "Existing", pattern: ".*", targetMappingId: "m1", enabled: true },
      ];
      const created = await createEntity("proxyRules", {
        name: "API rule", pattern: ".*\\.api\\.com.*", targetMappingId: "m1", enabled: true,
      });
      expect(currentConfig.proxyRules[0].id).toBe(created.id);
      expect(currentConfig.proxyRules[1].id).toBe("existing");
    });
  });

  // ── rule:update ───────────────────────────────────────────────────────

  describe("entity.update — proxyRules", () => {
    it("updates an existing proxy rule", () => {
      const rule: ProxyRule = { id: "r1", name: "old", pattern: ".*", targetMappingId: "m1", enabled: true };
      currentConfig.proxyRules = [{ ...rule }];

      updateEntity("proxyRules", { ...rule, name: "new" });

      expect(currentConfig.proxyRules[0].name).toBe("new");
    });

    it("returns { ok: true }", async () => {
      currentConfig.proxyRules = [{ id: "r1", name: "r", pattern: ".*", targetMappingId: "m1", enabled: true }];
      const result = await updateEntity("proxyRules", { id: "r1", name: "r", pattern: ".*", targetMappingId: "m1", enabled: false });
      expect(result).toEqual({ ok: true });
    });
  });

  // ── rule:delete ───────────────────────────────────────────────────────

  describe("entity.delete — proxyRules", () => {
    it("deletes the entity file for the given id", async () => {
      const { deleteEntityFile } = await import("@bifurc/engine/workspace/workspaceFs");
      currentConfig.proxyRules = [
        { id: "r1", name: "a", pattern: ".*a.*", targetMappingId: "m1", enabled: true, workspaceId: "default" },
        { id: "r2", name: "b", pattern: ".*b.*", targetMappingId: "m1", enabled: true, workspaceId: "default" },
      ];

      await deleteEntity("proxyRules", "r1");

      expect(deleteEntityFile).toHaveBeenCalledWith("default", "rules", "r1");
    });

    it("returns { ok: true }", async () => {
      currentConfig.proxyRules = [{ id: "r1", name: "r", pattern: ".*", targetMappingId: "m1", enabled: true, workspaceId: "default" }];
      const result = await deleteEntity("proxyRules", "r1");
      expect(result).toEqual({ ok: true });
    });
  });

  // ── mock:add ──────────────────────────────────────────────────────────

  describe("entity.create — mocks", () => {
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
      const result = await createEntity("mocks", baseMock);
      expect(result.id).toBeTruthy();
      expect(result.createdAt).toBeTypeOf("number");
      expect(result.name).toBe("Test Mock");
    });

    it("rejects a blank urlPattern", async () => {
      /**
       * Moved here on 2026-09-25 from `protocolExecution.integration.test.ts`, where it read
       * `"a graphql mock requires an endpointPattern"`. `plan/17` work item 8 removed the
       * `graphqlMocks` kind, and this is the suite where `mocks` is actually registered: it goes
       * through the shell's `registerIpcHandlers()`, which calls all five `register*Kinds()`, whereas
       * the other suite registers only `registerCoreKinds()` plus the three protocol kind files.
       *
       * `crudKinds.ts` checks `urlPattern` **first**, so a whitespace-only pattern with a valid
       * method reaches exactly this throw. This was the only case in the tree covering mock
       * validation, so it was moved rather than deleted.
       */
      await expect(createEntity("mocks", { ...baseMock, urlPattern: "   " })).rejects.toThrow(/urlPattern/);
    });

    it("prepends the new mock to the front of the list", async () => {
      // The pre-existing mock is given a **distinct name on purpose.** It used to be `{...baseMock}`,
      // i.e. also called "Test Mock", so `mocks[0].name` read "Test Mock" whether the new mock was
      // prepended *or* appended — the assertion passed on a `push`. Asserting by `id` (and checking
      // index 1) is what makes it a real pin on the prepend branch, which is the branch the engine
      // now selects by `opts.kind` rather than by `opts.ipcPrefix` (P6 step 3, 2026-09-20).
      currentConfig.mocks = [{ ...baseMock, id: "existing", name: "Existing", createdAt: 1 }];
      const created = await createEntity("mocks", baseMock);
      expect(currentConfig.mocks[0].id).toBe(created.id);
      expect(currentConfig.mocks[1].id).toBe("existing");
    });

    it("disables existing mocks with the same signature when new mock is enabled", async () => {
      const existing: MockRule = {
        ...baseMock,
        id: "old",
        createdAt: 1,
        enabled: true,
      };
      currentConfig.mocks = [existing];

      await createEntity("mocks", { ...baseMock, enabled: true });

      // The old mock should be disabled because it has the same signature
      expect(currentConfig.mocks.find((m) => m.id === "old")?.enabled).toBe(false);
    });

    it("does not disable existing mocks when new mock is disabled", async () => {
      const existing: MockRule = { ...baseMock, id: "old", createdAt: 1, enabled: true };
      currentConfig.mocks = [existing];

      await createEntity("mocks", { ...baseMock, enabled: false });

      expect(currentConfig.mocks.find((m) => m.id === "old")?.enabled).toBe(true);
    });
  });

  // ── mock:update ───────────────────────────────────────────────────────

  describe("entity.update — mocks", () => {
    it("updates an existing mock", async () => {
      const mock: MockRule = {
        id: "m1", name: "old name", method: "GET", urlPattern: "http://x.com",
        useRegex: false, enabled: true, capturedHeaders: {}, capturedBody: "",
        responseStatus: 200, responseHeaders: {}, responseBody: "{}", createdAt: 1,
      };
      currentConfig.mocks = [{ ...mock }];

      await updateEntity("mocks", { ...mock, name: "new name" });

      expect(currentConfig.mocks[0].name).toBe("new name");
    });

    it("returns { ok: true }", async () => {
      const mock: MockRule = {
        id: "m1", name: "m", method: "GET", urlPattern: "http://x.com",
        useRegex: false, enabled: true, capturedHeaders: {}, capturedBody: "",
        responseStatus: 200, responseHeaders: {}, responseBody: "{}", createdAt: 1,
      };
      currentConfig.mocks = [mock];
      const result = await updateEntity("mocks", mock);
      expect(result).toEqual({ ok: true });
    });

    it("writes the updated mock entity to disk", async () => {
      const { writeEntity } = await import("@bifurc/engine/workspace/workspaceFs");
      const mock: MockRule = {
        id: "m1", name: "m", method: "GET", urlPattern: "http://x.com",
        useRegex: false, enabled: true, capturedHeaders: {}, capturedBody: "",
        responseStatus: 200, responseHeaders: {}, responseBody: "{}", createdAt: 1,
        workspaceId: "default",
      };
      currentConfig.mocks = [{ ...mock }];
      await updateEntity("mocks", mock);
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
      await updateEntity("mocks", { ...mock, responseStatus: 201 });
      expect(commitMutation).not.toHaveBeenCalled();
    });
  });

  // ── mock:delete ───────────────────────────────────────────────────────

  describe("entity.delete — mocks", () => {
    it("deletes the entity file for the given id", async () => {
      const { deleteEntityFile } = await import("@bifurc/engine/workspace/workspaceFs");
      currentConfig.mocks = [
        { id: "m1", name: "a", method: "GET", urlPattern: "http://a.com", useRegex: false, enabled: true, capturedHeaders: {}, capturedBody: "", responseStatus: 200, responseHeaders: {}, responseBody: "{}", createdAt: 1, workspaceId: "default" },
        { id: "m2", name: "b", method: "POST", urlPattern: "http://b.com", useRegex: false, enabled: true, capturedHeaders: {}, capturedBody: "", responseStatus: 201, responseHeaders: {}, responseBody: "{}", createdAt: 2, workspaceId: "default" },
      ];

      await deleteEntity("mocks", "m1");

      expect(deleteEntityFile).toHaveBeenCalledWith("default", "mocks", "m1");
    });

    it("returns { ok: true }", async () => {
      currentConfig.mocks = [
        { id: "m1", name: "m", method: "GET", urlPattern: "http://x.com", useRegex: false, enabled: true, capturedHeaders: {}, capturedBody: "", responseStatus: 200, responseHeaders: {}, responseBody: "{}", createdAt: 1, workspaceId: "default" },
      ];
      const result = await deleteEntity("mocks", "m1");
      expect(result).toEqual({ ok: true });
    });

    it("does not auto-commit on delete (publish-on-demand model)", async () => {
      currentConfig.mocks = [
        { id: "m1", name: "My Mock", method: "GET", urlPattern: "http://x.com", useRegex: false, enabled: true, capturedHeaders: {}, capturedBody: "", responseStatus: 200, responseHeaders: {}, responseBody: "{}", createdAt: 1, workspaceId: "default" },
      ];
      await deleteEntity("mocks", "m1");
      expect(commitMutation).not.toHaveBeenCalled();
    });
  });

  // ── request:add ───────────────────────────────────────────────────────

  describe("entity.create — requests", () => {
    const baseReq: Omit<SavedRequest, "id" | "createdAt"> = {
      name: "My Request",
      method: "POST",
      url: "http://api.example.com/data",
      headers: { "content-type": "application/json" },
      body: '{"key":"value"}',
      folderId: null,
    };

    it("adds a request and returns it with generated id and createdAt", async () => {
      const result = await createEntity("requests", baseReq);
      expect(result.id).toBeTruthy();
      expect(result.createdAt).toBeTypeOf("number");
      expect(result.name).toBe("My Request");
    });

    it("writes the new request to disk via writeEntity", async () => {
      const { writeEntity } = await import("@bifurc/engine/workspace/workspaceFs");
      await createEntity("requests", baseReq);
      expect(writeEntity).toHaveBeenCalled();
    });

    it("appends the new request — `requests` is deliberately not on the prepend branch", async () => {
      // The other half of the branch. Without this, "prepends" could be satisfied by a rule that
      // prepends everything, and the `kind` predicate would be untestable from the passing side.
      currentConfig.requests = [{ ...baseReq, id: "existing", name: "Existing", createdAt: 1 }];
      const created = await createEntity("requests", baseReq);
      expect(currentConfig.requests[0].id).toBe("existing");
      expect(currentConfig.requests[1].id).toBe(created.id);
    });
  });

  // ── request:update ────────────────────────────────────────────────────

  describe("entity.update — requests", () => {
    it("writes the updated request to disk via writeEntity", async () => {
      const req: SavedRequest = { id: "r1", name: "old", method: "GET", url: "http://x.com", headers: {}, body: "", createdAt: 1 };
      const { writeEntity } = await import("@bifurc/engine/workspace/workspaceFs");
      await updateEntity("requests", { ...req, name: "updated" });
      expect(writeEntity).toHaveBeenCalled();
    });

    it("returns { ok: true }", async () => {
      const req: SavedRequest = { id: "r1", name: "r", method: "GET", url: "http://x.com", headers: {}, body: "", createdAt: 1 };
      const result = await updateEntity("requests", req);
      expect(result).toEqual({ ok: true });
    });
  });

  // ── request:delete ────────────────────────────────────────────────────

  describe("entity.delete — requests", () => {
    it("deletes the request file via deleteEntityFile", async () => {
      currentConfig.requests = [
        { id: "r1", name: "a", method: "GET", url: "http://a.com", headers: {}, body: "", createdAt: 1, workspaceId: "default" } as SavedRequest,
      ];
      const { deleteEntityFile } = await import("@bifurc/engine/workspace/workspaceFs");

      await deleteEntity("requests", "r1");

      expect(deleteEntityFile).toHaveBeenCalledWith("default", "requests", "r1");
    });

    it("returns { ok: true }", async () => {
      currentConfig.requests = [
        { id: "r1", name: "a", method: "GET", url: "http://a.com", headers: {}, body: "", createdAt: 1, workspaceId: "default" } as SavedRequest,
      ];
      const result = await deleteEntity("requests", "r1");
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
      const result = await folderAdd("mock", baseFolder);
      expect(result.id).toBeTruthy();
      expect(result.name).toBe("My Folder");
      expect(currentConfig.mockFolders).toHaveLength(1);
    });

    it("adds a request folder to requestFolders", async () => {
      await folderAdd("request", baseFolder);
      expect(currentConfig.requestFolders).toHaveLength(1);
    });
  });

  // ── folder:rename ─────────────────────────────────────────────────────

  describe("folder:rename handler", () => {
    it("renames a mock folder", () => {
      currentConfig.mockFolders = [{ id: "f1", name: "Old Name", parentId: null, createdAt: 1 }];

      folderRename("mock", "f1", "New Name");

      expect(currentConfig.mockFolders[0].name).toBe("New Name");
    });

    it("renames a request folder", () => {
      currentConfig.requestFolders = [{ id: "f2", name: "Old", parentId: null, createdAt: 1 }];

      folderRename("request", "f2", "New");

      expect(currentConfig.requestFolders[0].name).toBe("New");
    });

    it("returns { ok: true }", async () => {
      currentConfig.mockFolders = [{ id: "f1", name: "X", parentId: null, createdAt: 1 }];
      const result = await folderRename("mock", "f1", "Y");
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

      folderDelete("mock", "f1");

      expect(currentConfig.mockFolders).toHaveLength(0);
      expect(currentConfig.mocks).toHaveLength(0);
    });

    it("removes a request folder and deletes all contained requests (cascade delete)", () => {
      currentConfig.requestFolders = [{ id: "f2", name: "F", parentId: null, createdAt: 1 }];
      currentConfig.requests = [
        { id: "r1", name: "req", method: "GET", url: "http://x.com", headers: {}, body: "", createdAt: 1, folderId: "f2" },
      ];

      folderDelete("request", "f2");

      expect(currentConfig.requestFolders).toHaveLength(0);
      expect(currentConfig.requests).toHaveLength(0);
    });

    it("returns { ok: true }", async () => {
      currentConfig.mockFolders = [{ id: "f1", name: "F", parentId: null, createdAt: 1 }];
      const result = await folderDelete("mock", "f1");
      expect(result).toEqual({ ok: true });
    });
  });

  // ── env:add ───────────────────────────────────────────────────────────

  describe("entity.create — environments", () => {
    it("adds an environment and returns it with generated id", async () => {
      const input: Omit<Environment, "id" | "createdAt"> = {
        name: "Development",
        variables: [{ id: "v1", key: "BASE_URL", value: "http://localhost:3000" }],
      };

      const result = await createEntity("environments", input);

      expect(result.id).toBeTruthy();
      expect(result.name).toBe("Development");
      expect(currentConfig.environments).toHaveLength(1);
    });
  });

  // ── env:update ────────────────────────────────────────────────────────

  describe("entity.update — environments", () => {
    it("updates an existing environment", () => {
      const env: Environment = { id: "e1", name: "old", variables: [], createdAt: 1 };
      currentConfig.environments = [{ ...env }];

      updateEntity("environments", { ...env, name: "updated" });

      expect(currentConfig.environments[0].name).toBe("updated");
    });

    it("returns { ok: true }", async () => {
      const env: Environment = { id: "e1", name: "env", variables: [], createdAt: 1 };
      currentConfig.environments = [env];
      const result = await updateEntity("environments", env);
      expect(result).toEqual({ ok: true });
    });
  });

  // ── env:delete ────────────────────────────────────────────────────────

  describe("entity.delete — environments", () => {
    it("removes the environment with the given id", () => {
      currentConfig.environments = [
        { id: "e1", name: "dev", variables: [], createdAt: 1 },
        { id: "e2", name: "prod", variables: [], createdAt: 2 },
      ];

      deleteEntity("environments", "e1");

      expect(currentConfig.environments).toHaveLength(1);
      expect(currentConfig.environments[0].id).toBe("e2");
    });

    it("clears activeEnvironmentId when the active environment is deleted", () => {
      currentConfig.environments = [{ id: "e1", name: "dev", variables: [], createdAt: 1 }];
      currentConfig.activeEnvironmentId = "e1";

      deleteEntity("environments", "e1");

      expect(currentConfig.activeEnvironmentId).toBeNull();
    });

    it("does not clear activeEnvironmentId when a different environment is deleted", () => {
      currentConfig.environments = [
        { id: "e1", name: "dev", variables: [], createdAt: 1 },
        { id: "e2", name: "prod", variables: [], createdAt: 2 },
      ];
      currentConfig.activeEnvironmentId = "e2";

      deleteEntity("environments", "e1");

      expect(currentConfig.activeEnvironmentId).toBe("e2");
    });

    it("returns { ok: true }", async () => {
      currentConfig.environments = [{ id: "e1", name: "e", variables: [], createdAt: 1 }];
      const result = await deleteEntity("environments", "e1");
      expect(result).toEqual({ ok: true });
    });
  });

  // ── env:setActive ─────────────────────────────────────────────────────

  describe("env.setActive command (the deleted `env:setActive` channel)", () => {
    it("sets activeEnvironmentId to the given id", () => {
      envSetActive("env-123");
      expect(currentConfig.activeEnvironmentId).toBe("env-123");
    });

    it("clears activeEnvironmentId when null is passed", () => {
      currentConfig.activeEnvironmentId = "env-123";
      envSetActive(null);
      expect(currentConfig.activeEnvironmentId).toBeNull();
    });

    it("returns { ok: true }", () => {
      const result = envSetActive("env-1");
      expect(result).toEqual({ ok: true });
    });
  });

  // ── server:status / proxy:status / request:replay — cases deleted ─────
  //
  // Three `describe`s stood here, one case each. All three were already asserted by the "delivers the
  // 21 moved commands through the registry" block, against the same mocks, which is why none was
  // re-pointed:
  //
  //  - `server:status` — the block asserts `{running, port, error}` exactly, with the same
  //    `isRunning`/`getPort`/`getServerError` mocks. (The block also pins the *pair*: `proxy.status` is
  //    the same `isRunning()` with a different payload, and collapsing the two would silently change
  //    one of them.)
  //  - `proxy:status` — `toEqual({ running: true })`, the same assertion.
  //  - `request:replay` — the channel case asserted `replayRequest` was called with the four positional
  //    slots *and* that its result was returned. The block asserted the slots; the result assertion was
  //    the residue, so it was **folded into the block** rather than dropped — the same fold as
  //    `config:save`'s bus emission and `services:discover`'s pass-through.

  // ── server:restart / server:stop / server:start — cases deleted ──────
  //
  // Three `describe`s stood here, six cases in all: for each channel one `it` asserting
  // `registeredHandlers.has(channel)` and one driving `getHandler(channel)`. The reduction's fifth
  // group deleted the channels (`plan/07` §5b-sedecies), and **all six were already covered
  // elsewhere**, which is why none was re-pointed:
  //
  //  - the three `has()` cases asserted what `expectedChannels` generated a case for — the same
  //    assertion twice over, and the generated one is the list that exists to check it. Registration
  //    of `server.start` / `server.stop` / `server.restart` is *also* asserted by the "registry
  //    coverage" block, which iterates every protocol command.
  //  - the three behaviour cases asserted `startServer` / `stopServer` were called and `{ok:true}`
  //    returned — which the "delivers the 21 moved commands" block above asserts against the same
  //    mocks, and asserts *more* (the stop-before-start order). The one assertion that block was
  //    missing — `server.restart` starting on `currentConfig.port` — was folded into it rather than
  //    left behind here.
  //
  // Deleting rather than re-pointing follows `plan/07` §5b-septies' rule: re-point the cases whose
  // assertions live only in the channel, delete the ones a suite already makes. Re-pointing these
  // would have produced three literal duplicates of a block 900 lines above them.

  // ── rule:update — not-found branch ──────────────────────────────────

  describe("entity.update — proxyRules, not-found branch", () => {
    it("does nothing when the rule id does not exist", () => {
      currentConfig.proxyRules = [];
      const rule: ProxyRule = { id: "nonexistent", name: "r", pattern: ".*", targetMappingId: "m1", enabled: true };
      expect(() => updateEntity("proxyRules", rule)).not.toThrow();
      expect(currentConfig.proxyRules).toHaveLength(0);
    });
  });

  // ── env:update — not-found branch ────────────────────────────────────

  describe("entity.update — environments, not-found branch", () => {
    it("does nothing when the env id does not exist", () => {
      currentConfig.environments = [];
      const env: Environment = { id: "nonexistent", name: "e", variables: [], createdAt: 1 };
      expect(() => updateEntity("environments", env)).not.toThrow();
      expect(currentConfig.environments).toHaveLength(0);
    });
  });

  // ── folder:rename — not-found branch ─────────────────────────────────

  describe("folder:rename handler — not-found branch", () => {
    it("does nothing when the folder id does not exist", () => {
      currentConfig.mockFolders = [];
      expect(() => folderRename("mock", "nonexistent", "New Name")).not.toThrow();
      expect(currentConfig.mockFolders).toHaveLength(0);
    });
  });

  // ── folder:add — null array initialization ────────────────────────────

  describe("folder:add handler — null array initialization", () => {
    it("initializes mockFolders when it is null before adding", async () => {
      currentConfig.mockFolders = null as any;
      const result = await folderAdd("mock", { name: "F", parentId: null });
      expect(result.name).toBe("F");
      expect(Array.isArray(currentConfig.mockFolders)).toBe(true);
    });

    it("initializes requestFolders when it is null before adding", async () => {
      currentConfig.requestFolders = null as any;
      const result = await folderAdd("request", { name: "F", parentId: null });
      expect(result.name).toBe("F");
      expect(Array.isArray(currentConfig.requestFolders)).toBe(true);
    });
  });

  // ── mock:update — conflict disabling ─────────────────────────────────

  describe("entity.update — mocks, disableConflicts", () => {
    const baseMock: MockRule = {
      id: "m1", name: "Mock A", method: "GET", urlPattern: "http://api.example.com/data",
      useRegex: false, enabled: true, capturedHeaders: {}, capturedBody: "",
      responseStatus: 200, responseHeaders: {}, responseBody: "{}", createdAt: 1,
    };

    it("does not disable conflicting mocks (conflict resolution moved to entity:setEnabled)", async () => {
      const conflict: MockRule = { ...baseMock, id: "m2", name: "Mock B" };
      currentConfig.mocks = [{ ...baseMock }, conflict];

      await updateEntity("mocks", { ...baseMock, enabled: true });

      // mock:update no longer handles conflict resolution — that's done by entity:setEnabled
      expect(currentConfig.mocks.find((m) => m.id === "m2")?.enabled).toBe(true);
    });

    it("does not disable other mocks when updated mock is disabled", async () => {
      const other: MockRule = { ...baseMock, id: "m2", name: "Mock B" };
      currentConfig.mocks = [{ ...baseMock }, other];

      await updateEntity("mocks", { ...baseMock, enabled: false });

      expect(currentConfig.mocks.find((m) => m.id === "m2")?.enabled).toBe(true);
    });
  });

  // ── mock:update — not-found branch ───────────────────────────────────

  describe("entity.update — mocks, not-found branch", () => {
    it("does nothing when the mock id does not exist", async () => {
      currentConfig.mocks = [];
      const mock: MockRule = {
        id: "nonexistent", name: "m", method: "GET", urlPattern: "http://x.com",
        useRegex: false, enabled: true, capturedHeaders: {}, capturedBody: "",
        responseStatus: 200, responseHeaders: {}, responseBody: "{}", createdAt: 1,
      };
      await expect(updateEntity("mocks", mock)).resolves.not.toThrow();
      expect(currentConfig.mocks).toHaveLength(0);
    });
  });

  // ── request:update — not-found branch ────────────────────────────────

  describe("entity.update — requests, not-found branch", () => {
    it("does nothing when the request id does not exist", () => {
      currentConfig.requests = [];
      const req: SavedRequest = { id: "nonexistent", name: "r", method: "GET", url: "http://x.com", headers: {}, body: "", createdAt: 1 };
      expect(() => updateEntity("requests", req)).not.toThrow();
      expect(currentConfig.requests).toHaveLength(0);
    });
  });

  // ── ws:add / ws:update / ws:delete ───────────────────────────────────

  describe("entity.create — wsConnections", () => {
    const baseConn: Omit<SavedWsConnection, "id" | "createdAt"> = {
      name: "Local WS",
      url: "ws://localhost:3000",
      headers: {},
      folderId: null,
      workspaceId: "default",
    };

    it("adds a ws connection and returns it with generated id", async () => {
      const result = await createEntity("wsConnections", baseConn);
      expect(result.id).toBeTruthy();
      expect(result.url).toBe("ws://localhost:3000");
    });

    it("writes the new ws connection to disk via writeEntity", async () => {
      const { writeEntity } = await import("@bifurc/engine/workspace/workspaceFs");
      await createEntity("wsConnections", baseConn);
      expect(writeEntity).toHaveBeenCalled();
    });
  });

  describe("entity.update — wsConnections", () => {
    it("writes the updated ws connection to disk via writeEntity", async () => {
      const conn: SavedWsConnection = { id: "c1", name: "old", url: "ws://localhost:1", headers: {}, createdAt: 1, workspaceId: "default" };
      const { writeEntity } = await import("@bifurc/engine/workspace/workspaceFs");
      await updateEntity("wsConnections", { ...conn, name: "updated" });
      expect(writeEntity).toHaveBeenCalled();
    });

    it("returns { ok: true }", async () => {
      const conn: SavedWsConnection = { id: "c1", name: "c", url: "ws://x", headers: {}, createdAt: 1, workspaceId: "default" };
      const result = await updateEntity("wsConnections", conn);
      expect(result).toEqual({ ok: true });
    });
  });

  describe("entity.delete — wsConnections", () => {
    it("deletes the ws connection file via deleteEntityFile", async () => {
      currentConfig.wsConnections = [
        { id: "c1", name: "a", url: "ws://a", headers: {}, createdAt: 1, workspaceId: "default" } as SavedWsConnection,
      ];
      const { deleteEntityFile } = await import("@bifurc/engine/workspace/workspaceFs");
      await deleteEntity("wsConnections", "c1");
      expect(deleteEntityFile).toHaveBeenCalledWith("default", "sockets", "c1");
    });

    it("returns { ok: true }", async () => {
      currentConfig.wsConnections = [
        { id: "c1", name: "a", url: "ws://a", headers: {}, createdAt: 1, workspaceId: "default" } as SavedWsConnection,
      ];
      const result = await deleteEntity("wsConnections", "c1");
      expect(result).toEqual({ ok: true });
    });
  });

  // ── folder:add — ws kind ─────────────────────────────────────────────

  describe("folder:add handler — ws kind", () => {
    it("adds a ws folder and returns it with generated id", async () => {
      const result = await folderAdd("ws", { name: "WS Folder", parentId: null });
      expect(result.id).toBeTruthy();
      expect(result.name).toBe("WS Folder");
      expect(currentConfig.wsFolders).toHaveLength(1);
    });
  });

  describe("folder:rename handler — ws kind", () => {
    it("renames a ws folder", () => {
      currentConfig.wsFolders = [{ id: "f1", name: "Old", parentId: null, createdAt: 1, workspaceId: "default" }];
      folderRename("ws", "f1", "New");
      expect(currentConfig.wsFolders[0].name).toBe("New");
    });
  });

  describe("folder:delete handler — ws kind", () => {
    it("removes ws folder and deletes all contained ws connections (cascade delete)", () => {
      currentConfig.wsFolders = [{ id: "f1", name: "F", parentId: null, createdAt: 1, workspaceId: "default" }];
      currentConfig.wsConnections = [{ id: "c1", name: "c", url: "ws://x", headers: {}, createdAt: 1, folderId: "f1", workspaceId: "default" }];
      folderDelete("ws", "f1");
      expect(currentConfig.wsFolders).toHaveLength(0);
      expect(currentConfig.wsConnections).toHaveLength(0);
    });
  });

  // ── workspace.add — the two cases the "21 moved commands" block does not make ────────────────

  describe("workspace.add command (the deleted `workspace:add` channel)", () => {
    // "adds a workspace and returns it with a generated id" and "trims the workspace name" stood here
    // too, and both were deleted: the block asserts the trim, the id, `activeEnvironmentId` being
    // `null`, and the count growing by one — against the same mocks. What it does **not** make is the
    // empty-name branch, so that case was re-pointed rather than dropped.
    it("generates a random name when name is empty", async () => {
      const result = (await commandRegistry.invoke("workspace.add", { name: "" }, ENTITY_CTX)) as {
        name: string;
      };
      expect(result.name).toMatch(/^[a-z]+-[a-z]+$/);
    });
  });

  // ── workspace.rename — the case the block does not make ──────────────────

  describe("workspace.rename command (the deleted `workspace:rename` channel)", () => {
    // "renames the workspace" and "returns { ok: true }" were deleted — the block asserts both, plus
    // the whitespace-only fallback, which is the sharper form of the same contract.
    it("does nothing when workspace id is not found", async () => {
      // Strengthened while re-pointing: the channel case was
      // `expect(() => getHandler("workspace:rename")(…)).not.toThrow()`, which an `async` handler
      // satisfies vacuously — it can only reject, never throw synchronously. Asserting the config is
      // unchanged is what the case was trying to say.
      const before = currentConfig.workspaces.map((w) => w.name);
      await commandRegistry.invoke("workspace.rename", { id: "nonexistent", name: "X" }, ENTITY_CTX);
      expect(currentConfig.workspaces.map((w) => w.name)).toEqual(before);
    });
  });

  // ── workspace.delete — the active-workspace fallback ─────────────────────

  describe("workspace.delete command (the deleted `workspace:delete` channel)", () => {
    beforeEach(() => {
      currentConfig.workspaces = [
        { id: "ws1", name: "One", createdAt: 0, activeEnvironmentId: null },
        { id: "ws2", name: "Two", createdAt: 0, activeEnvironmentId: null },
      ];
      currentConfig.activeWorkspaceId = "ws1";
    });

    // "removes the workspace" and "returns { ok: true }" were deleted — the block asserts both, and
    // deliberately against the *inactive* workspace so that this branch is what stays under test.
    it("falls back to first remaining workspace when active workspace is deleted", async () => {
      await commandRegistry.invoke("workspace.delete", { id: "ws1" }, ENTITY_CTX);
      expect(currentConfig.activeWorkspaceId).toBe("ws2");
    });
  });

  // ── workspace.setActive ───────────────────────────────────────────────

  describe("workspace.setActive command (the deleted `workspace:setActive` channel)", () => {
    beforeEach(() => {
      currentConfig.workspaces = [
        { id: "ws1", name: "One", createdAt: 0, activeEnvironmentId: "env-1" },
        { id: "ws2", name: "Two", createdAt: 0, activeEnvironmentId: null },
      ];
      currentConfig.activeWorkspaceId = "ws1";
    });

    it("sets the active workspace and loads its environment", () => {
      const result = workspaceSetActive("ws2");
      expect(result.ok).toBe(true);
      expect(currentConfig.activeWorkspaceId).toBe("ws2");
      expect(currentConfig.activeEnvironmentId).toBeNull();
    });

    it("restores activeEnvironmentId from the workspace", () => {
      workspaceSetActive("ws1");
      expect(currentConfig.activeEnvironmentId).toBe("env-1");
    });

    it("returns { ok: false } when workspace id is not found", () => {
      const result = workspaceSetActive("nonexistent");
      expect(result.ok).toBe(false);
    });

    it("returns the full config on success", () => {
      const result = workspaceSetActive("ws2");
      expect(result.config).toBeDefined();
      expect(result.config.activeWorkspaceId).toBe("ws2");
    });

    /**
     * `plan/17` §5 work item 3, 2026-09-25: switching the active workspace **does** re-point the proxy.
     *
     * ## This reverses P14 WI 4's rule on purpose, and the reversal is the point of the work item
     *
     * P14 made the pointer and the proxies independent because there were **N** proxies, one per workspace:
     * the port belonged to the *workspace*, not to the selection, so switching the selection must not move any
     * of them. `plan/17` §1 deletes that model. There is now **one** host-level forward proxy, and its whole
     * job is *"which workspace am I looking at?"* — it loads the active workspace's rules and mocks **at
     * start** and **swaps its in-memory config when the active workspace changes**, rather than resolving them
     * per request (`plan/17` §5 work item 3, verbatim: *"Changing workspace changes the point to workspace."*).
     *
     * So the swap has to happen somewhere, and this command is where. **Without it the proxy would keep
     * serving the previous workspace's rules until the next restart** — a stale-config failure that every
     * other case in this block would miss, because they only look at the pointer. Hence the assertion is on
     * the *presence* of the call, which is the same instrument P14 used for its own (opposite) rule and for
     * the same reason: the call is the half that is invisible to a value check.
     *
     * The contrast with `env.setActive` below is now **symmetry**, not asymmetry — see that case.
     */
    it("DOES reload the proxy — one host-level proxy follows the selection (plan/17 §1)", () => {
      vi.mocked(notifyConfigChanged).mockClear();

      const result = workspaceSetActive("ws2");

      expect(result.ok).toBe(true);
      expect(currentConfig.activeWorkspaceId).toBe("ws2");
      expect(notifyConfigChanged).toHaveBeenCalled();
    });

    /**
     * `env.setActive` also reloads — and the two commands are now **symmetric**, where they used to be a
     * deliberate contrast.
     *
     * An environment is proxy state (which rules and enabled sets are live), so changing it has always had to
     * re-point the proxy. P14's `workspace.setActive` was the *exception*: with N proxies, one per workspace,
     * the selection moved nothing. `plan/17` §1 removed the N, so "the selection is proxy state" is true of
     * the workspace too — and this pair is kept as a pair because they are the two commands that swap the
     * proxy's config, and a future change that drops one of the two calls should be caught in both places.
     */
    it("env.setActive DOES reload the proxy — environments are proxy state", () => {
      vi.mocked(notifyConfigChanged).mockClear();

      commandRegistry.invoke("env.setActive", { id: "env-1" }, ENTITY_CTX);

      expect(notifyConfigChanged).toHaveBeenCalled();
    });
  });

  // ── env.setActive — workspace activeEnvironmentId sync ───────────────

  describe("env.setActive — workspace activeEnvironmentId sync", () => {
    it("updates activeEnvironmentId on the active workspace", () => {
      envSetActive("env-abc");
      const ws = currentConfig.workspaces.find((w) => w.id === "default");
      expect(ws?.activeEnvironmentId).toBe("env-abc");
    });

    it("sets workspace activeEnvironmentId to null when clearing", () => {
      currentConfig.activeEnvironmentId = "env-abc";
      envSetActive(null);
      const ws = currentConfig.workspaces.find((w) => w.id === "default");
      expect(ws?.activeEnvironmentId).toBeNull();
    });
  });

  // ── config.get — returns config as-is ────────────────────────────────

  describe("config.get command — the loaded config is returned as-is", () => {
    it("returns the loaded config directly", () => {
      const result = getConfig();
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

  // ── history.list ──────────────────────────────────────────────────────

  describe("history.list command (the deleted `history:list` channel)", () => {
    it("delegates to queryLog with the provided filePath", async () => {
      const mockEntries = [{ commitHash: "abc1234", action: "update", entity: "mock", entityId: "m1", entityName: "My Mock", actor: "local", ts: 1000, workspaceId: "default" }];
      vi.mocked(queryLog).mockResolvedValue({ entries: mockEntries as any, total: 1 });

      const result = await historyList({ filePath: "mocks/m1.json", workspaceId: "default", limit: 50, offset: 0 });

      expect(queryLog).toHaveBeenCalledWith(
        expect.objectContaining({ filePath: "mocks/m1.json", workspaceId: "default", limit: 50, offset: 0 }),
      );
      expect(result.entries).toHaveLength(1);
      expect(result.total).toBe(1);
    });

    it("uses active workspace when workspaceId is omitted", async () => {
      vi.mocked(queryLog).mockResolvedValue({ entries: [], total: 0 });
      currentConfig.activeWorkspaceId = "default";

      await historyList({ filePath: "mappings/m1.json" });

      expect(queryLog).toHaveBeenCalledWith(
        expect.objectContaining({ workspaceId: "default", filePath: "mappings/m1.json" }),
      );
    });

    it("defaults limit to 100 and offset to 0 when not provided", async () => {
      vi.mocked(queryLog).mockResolvedValue({ entries: [], total: 0 });

      await historyList({ filePath: "rules/r1.json", workspaceId: "default" });

      expect(queryLog).toHaveBeenCalledWith(
        expect.objectContaining({ limit: 100, offset: 0 }),
      );
    });
  });

  // ── history.diff ──────────────────────────────────────────────────────

  describe("history.diff command (the deleted `history:diff` channel)", () => {
    it("returns before and after states for a commit", async () => {
      const beforeState = { id: "m1", name: "Old Name" };
      const afterState = { id: "m1", name: "New Name" };
      vi.mocked(getEntityAtCommit)
        .mockResolvedValueOnce(afterState)
        .mockResolvedValueOnce(beforeState);

      const result = await historyDiff("abc1234", "mocks/m1.json", "default");

      expect(getEntityAtCommit).toHaveBeenCalledWith("abc1234", "default", "mocks/m1.json");
      expect(getEntityAtCommit).toHaveBeenCalledWith("abc1234~1", "default", "mocks/m1.json");
      expect(result.after).toEqual(afterState);
      expect(result.before).toEqual(beforeState);
    });

    it("returns null before/after when entity does not exist at commit", async () => {
      vi.mocked(getEntityAtCommit).mockResolvedValue(null);

      const result = await historyDiff("deadbeef", "mocks/missing.json", "default");

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
  // These cases call the collapsed `entity.create`/`entity.update`/`entity.delete`/`entity.list`
  // commands directly. They used to reach them through the per-kind channels (`rule:add`, `ws:add`, …),
  // which were one-line adapters; the channels are the step-3 reduction's target (`plan/07` §5b-ter) and
  // these cases no longer depend on them.
  //
  // `EntityKind`'s wire vocabulary renames two of the twelve kinds — engine-internal "rules"/"sockets"
  // become protocol "proxyRules"/"wsConnections" — so these tests exist specifically to pin that
  // translation: without it, `commandRegistry.invoke()` would throw a schema-validation error the moment
  // a rule or websocket connection was created, updated, deleted, or loaded.
  //
  // **What changed on 2026-09-20, and then again on 2026-09-21.** The translation is two-sided: the
  // channel applied `toProtocolKind()` on the way in, and the engine applies `toEngineKind()` on the way
  // out. Re-pointing these cases at the commands directly removes the *first* half from the test's path,
  // so what they pin now is `toEngineKind()` alone. That was correct in 2026-09-20's words — "the half
  // that survives, since `toProtocolKind()` lives in the factory and dies with the channels" — **and it
  // was wrong about what dying would cost.** `toProtocolKind()` was not dead weight: two renderer panels
  // pass the engine-internal `"rules"` / `"sockets"` names and the client twin never applied the map, so
  // the reduction's sixth group had to move that half into `packages/client/src/index.ts` **before** it
  // deleted the channel (`174a01d`). The prediction "dies with the channels" is the shape of bug this
  // reduction exists to catch, and it was written in this comment.

  // ── entity.changed: which kinds ping the bus ──────────────────────────

  describe("entity.create — the `entity.changed` ping", () => {
    it("fires for `mocks` and `requests` only, and never for another kind", async () => {
      // Site 2 of the three the engine used to select by `opts.ipcPrefix` (`["mock","request"]`).
      // The companion server's suite pins the `mocks` side end-to-end; what is pinned **here** is the
      // boundary, because a translation that widened the set — or that fired for everything — would
      // still satisfy that one. `proxyRules` is the control: it is the kind that would most plausibly
      // be swept in by a careless rewrite, since it shares the *other* branch (the prepend).
      const fired: { kind: string; action: string }[] = [];
      const onChange = (p: any) => fired.push(p);
      bus.onTyped("entity.changed", onChange);
      try {
        await createEntity("mocks", {
          name: "m", method: "GET", urlPattern: "http://x.com", useRegex: false, enabled: true,
          capturedHeaders: {}, capturedBody: "", responseStatus: 200, responseHeaders: {},
          responseBody: "{}", folderId: null,
        });
        await createEntity("requests", {
          name: "r", method: "GET", url: "http://x.com", headers: {}, body: "", folderId: null,
        });
        await createEntity("proxyRules", {
          name: "rule", pattern: ".*", targetMappingId: "m1", enabled: true,
        });

        expect(fired.map((e) => e.kind)).toEqual(["mocks", "requests"]);
        expect(fired.every((e) => e.action === "created")).toBe(true);
      } finally {
        // `bus` is the process-wide singleton and vitest shares a module registry across files in a
        // worker — a leaked listener would fire into whatever runs next.
        bus.offTyped("entity.changed", onChange);
      }
    });
  });

  describe("entity.* CommandRegistry collapse", () => {
    it("registers entity.create, entity.update, entity.delete, entity.load, entity.list and entity.setEnabled", () => {
      for (const action of ["entity.create", "entity.update", "entity.delete", "entity.load", "entity.list", "entity.setEnabled"]) {
        expect(commandRegistry.isRegistered(action)).toBe(true);
      }
    });

    it("entity.load accepts the protocol's \"proxyRules\" kind — the name the client sends after translating \"rules\"", async () => {
      // `loadEntity` is mocked to always return null in this file — the meaningful assertion is that
      // this resolves at all instead of rejecting on Zod schema validation, which is what the deleted
      // channel case was checking. The *translation* half now lives in the client, where it belongs and
      // where the caller's `"rules"` actually arrives:
      // `packages/client/tests/client.test.ts`'s "loadEntity translates the two engine-internal kinds
      // its callers pass — the live bug, 2026-09-21" asserts the payload. Pinning it here too would be
      // pinning it in the one place the bug cannot occur.
      const result = await entityLoad("default", "proxyRules", "r1");
      expect(result).toEqual({ ok: false });
    });

    it("entity.load accepts the protocol's \"wsConnections\" kind — the name the client sends after translating \"sockets\"", async () => {
      const result = await entityLoad("default", "wsConnections", "ws1");
      expect(result).toEqual({ ok: false });
    });

    it("a rule is created with kind \"proxyRules\" and still stores under \"rules\"", async () => {
      const { writeEntity } = await import("@bifurc/engine/workspace/workspaceFs");
      const input: Omit<ProxyRule, "id"> = {
        name: "API rule", pattern: ".*\\.api\\.com.*", targetMappingId: "m1", enabled: true,
      };
      await createEntity("proxyRules", input);
      expect(vi.mocked(writeEntity).mock.calls[0][1]).toBe("rules");
    });

    it("a websocket connection is created with kind \"wsConnections\" and still stores under \"sockets\"", async () => {
      const { writeEntity } = await import("@bifurc/engine/workspace/workspaceFs");
      const input: Omit<SavedWsConnection, "id"> = { name: "conn", url: "ws://localhost" } as any;
      await createEntity("wsConnections", input);
      expect(vi.mocked(writeEntity).mock.calls[0][1]).toBe("sockets");
    });

    it("graphql schemas round-trip through entity.create / entity.delete / entity.list", async () => {
      const { writeEntity, deleteEntityFile, readAllEntities } = await import("@bifurc/engine/workspace/workspaceFs");
      await createEntity("graphqlSchemas", { name: "s", content: "type Query {}" } as any);
      expect(vi.mocked(writeEntity).mock.calls[0][1]).toBe("graphqlSchemas");

      await deleteEntity("graphqlSchemas", "s1");
      expect(vi.mocked(deleteEntityFile)).toHaveBeenCalledWith("default", "graphqlSchemas", "s1");

      const list = await listEntities("graphqlSchemas");
      expect(vi.mocked(readAllEntities)).toHaveBeenCalledWith("default", "graphqlSchemas");
      expect(list).toEqual([]);
    });

    // `environments` is the thirteenth (and last) `EntityKind` to join the collapse — unlike
    // the twelve original CRUD-factory kinds, it carries a create-gate check (`gateKind`) and
    // two delete-time quirks (the "__global__" guard, the active-environment reset). These
    // pin that it now goes through `entity.create`/`entity.update`/`entity.delete` exactly
    // like every other kind, with those quirks preserved.

    it("an environment is created with kind \"environments\" and stores flat", async () => {
      const { writeFlatEntity } = await import("@bifurc/engine/workspace/workspaceFs");
      const input = { name: "Dev", variables: [] };
      const result = await createEntity("environments", input);
      expect(result.id).toBeTruthy();
      expect(result.name).toBe("Dev");
      expect(vi.mocked(writeFlatEntity).mock.calls[0][1]).toBe("environments");
    });

    it("a gated create returns { error: \"limit_reached\", ... } and creates nothing", async () => {
      const { gateCreate } = await import("@bifurc/engine/workspace/entityCount");
      vi.mocked(gateCreate).mockReturnValueOnce({ allowed: false, current: 1, limit: 1 });

      currentConfig.environments = [];
      const result = await createEntity("environments", { name: "Dev", variables: [] });

      expect(result).toEqual({ error: "limit_reached", allowed: false, current: 1, limit: 1 });
      expect(currentConfig.environments).toHaveLength(0);
      expect(gateCreate).toHaveBeenCalledWith(expect.any(String), "environment");
    });

    it("an environment delete refuses \"__global__\"", async () => {
      currentConfig.environments = [{ id: "__global__", name: "Global", variables: [], createdAt: 1 }];
      const result = await deleteEntity("environments", "__global__");
      expect(result).toEqual({ ok: false, error: "cannot_delete_global" });
      expect(currentConfig.environments).toHaveLength(1);
    });
  });
});
