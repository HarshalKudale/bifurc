/**
 * P4 item 1 — conflict resolution is a privilege, and the companion scope does not hold it.
 *
 * WHY THIS EXISTS
 * ---------------
 * `entity.create` and `entity.setEnabled` both do something a plain `write` does not: they **disable
 * other entities the caller never named**, discovered by scanning config for a matching signature
 * (`method|urlPattern|capturedBody` for mocks, `useRegex|pattern` for rules). The companion browser
 * extension is a lower-trust caller, and `V1_COMPANION_ACTIONS`' own test documents that its surface
 * must stay **additive** — a page in the user's browser must not be able to switch off a mock the user
 * set up. So both sites are gated on `mayAffectUnnamedEntities()`, i.e. `admin`, which
 * `COMPANION_SCOPES` (`{read, write}`) does not include.
 *
 * What this file pins is the **pair**, because either half alone is worthless:
 *   - a session-less context — what every legacy `ipcMain.handle` adapter passes — still resolves
 *     conflicts, so the app's own behaviour is unchanged;
 *   - a `COMPANION_SCOPES` session does not, and the pre-existing mock stays enabled.
 * The second half alone would pass if the gate had simply switched conflict resolution off for
 * everyone, which would be a silent behaviour change for the shell. The first half is what rules that
 * out, so neither is optional.
 *
 * TWO THINGS THIS FILE HAS TO KNOW ABOUT THE CODE IT TESTS
 * -------------------------------------------------------
 * 1. `createEntityCore()` **regenerates the id** (`{...entity, id: generateId()}`), so an id passed in
 *    the payload is not the id that lands on disk. Entities are located by `name` here.
 * 2. Assertions read `loadConfig()` — real disk — not a mocked in-memory config. That is deliberate,
 *    and it is what surfaced the sibling-persistence bug described in `createEntityCore()`: `enabled`
 *    is stripped by `writeEntity()` and lives in `enabled.json`, so a conflict rule that only mutates
 *    `cfg` changes nothing durable. A test asserting on an in-memory config object cannot see that.
 *
 * Real workspace, real `app.json`, real `entityCrudRegistry`, real `loadConfig()`. Only Electron and
 * the proxy's `reloadConfig` are mocked.
 */

import { describe, it, expect, beforeAll, beforeEach, afterEach, vi } from "vitest";

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

vi.mock("@bifurc/engine/proxy/server", () => ({
  reloadConfig: vi.fn(),
  startServer: vi.fn(),
  stopServer: vi.fn(),
  isRunning: vi.fn(() => false),
  getPort: vi.fn(() => 80),
  getServerError: vi.fn(() => null),
}));

import { createWorkspace, mockFixture, mappingFixture, TEST_WS, type WorkspaceFixture } from "./proxyHarness";
import { loadConfig } from "@bifurc/engine/store/config";
import { bus } from "@bifurc/engine/eventBus";
import { commandRegistry, type CommandContext, type Scope } from "@bifurc/engine/commands/registry";
import { COMPANION_SCOPES, SHELL_SCOPES } from "@bifurc/engine/transport/auth/scopes";
import { registerCrudHandlers } from "@/ipc/handlers/crudHandlers";
// Side-effect import: registering `entity.setEnabled` happens at this module's load, not inside
// `registerCoreHandlers()`.
import "@/ipc/handlers/coreHandlers";

/** The conflict signature both sites compare on, for mocks. */
const SIG = { method: "GET", urlPattern: "/api/conflict", capturedBody: "" };

/** What every legacy `ipcMain.handle` adapter passes: the real bus, and no session. */
const engineCtx = (): CommandContext => ({ bus });

function sessionCtx(scopes: ReadonlySet<Scope>): CommandContext {
  return {
    bus,
    session: { sessionId: "s-1", clientName: "companion", clientVersion: "1.0.0", scopes },
  };
}

const companionCtx = (): CommandContext => sessionCtx(COMPANION_SCOPES);
const shellCtx = (): CommandContext => sessionCtx(SHELL_SCOPES);

const mocks = () => loadConfig().mocks;
const enabledOf = (id: string): boolean | undefined => mocks().find((m) => m.id === id)?.enabled;
/** `createEntityCore()` assigns its own id, so entities are found by the name that was asked for. */
const hasMockNamed = (name: string): boolean => mocks().some((m) => m.name === name);

const createMock = (ctx: CommandContext, name = "Incoming") =>
  commandRegistry.invoke(
    "entity.create",
    { kind: "mocks", entity: mockFixture({ name, ...SIG }), workspaceId: TEST_WS },
    ctx,
  );

let ws: WorkspaceFixture;

beforeAll(() => {
  registerCrudHandlers();
});

beforeEach(() => {
  ws = createWorkspace({
    mocks: [mockFixture({ id: "existing", name: "Existing", ...SIG, enabled: true })],
  });
});

afterEach(() => {
  ws.cleanup();
});

describe("entity.create — the incoming mock is always stored", () => {
  it.each([
    ["a session-less engine context", engineCtx],
    ["a companion {read, write} session", companionCtx],
    ["a shell session", shellCtx],
  ])("stores the new mock for %s", async (_label, makeCtx) => {
    const before = mocks().length;

    await createMock(makeCtx());

    expect(mocks().length).toBe(before + 1);
    expect(hasMockNamed("Incoming")).toBe(true);
  });
});

describe("entity.create — conflict resolution follows the caller's authority", () => {
  it("disables the existing same-signature mock for the engine's own caller", async () => {
    expect(enabledOf("existing")).toBe(true);

    await createMock(engineCtx());

    // This is also the regression test for the sibling-persistence fix in `createEntityCore()`: before
    // it, the sibling was disabled only in the in-memory `cfg` and `saveConfig()` dropped the flag,
    // because `writeEntity()` strips `enabled`. So this assertion failed on real disk while the
    // mocked-config suite in `tests/ipc/handlers.test.ts` stayed green.
    expect(enabledOf("existing")).toBe(false);
  });

  it("disables it for a shell session too, which holds admin", async () => {
    await createMock(shellCtx());

    expect(enabledOf("existing")).toBe(false);
  });

  it("leaves it enabled for the companion — the additive-only guarantee", async () => {
    await createMock(companionCtx());

    // The page added its mock; the user's mock is untouched. Checked together with the "was stored"
    // assertion so that a gate rejecting the *command* rather than the *side effect* cannot pass.
    expect(enabledOf("existing")).toBe(true);
    expect(hasMockNamed("Incoming")).toBe(true);
  });

  it("does not consult the gate for a kind that has no conflict rule", async () => {
    // Only `mocks` and `rules` set `onAddConflict`, so the flag is inert elsewhere — a regression that
    // applied it to every kind would show up as a mapping refusing to be created.
    const before = loadConfig().mappings.length;

    await commandRegistry.invoke(
      "entity.create",
      { kind: "mappings", entity: mappingFixture({ domain: "new.localhost" }), workspaceId: TEST_WS },
      companionCtx(),
    );

    expect(loadConfig().mappings.length).toBe(before + 1);
  });
});

describe("entity.setEnabled — the same privilege, the same gate", () => {
  beforeEach(() => {
    // `a` is enabled, `b` is the same signature but disabled — enabling `b` is what would disable `a`.
    ws.cleanup();
    ws = createWorkspace({
      mocks: [
        mockFixture({ id: "a", name: "A", ...SIG, enabled: true }),
        mockFixture({ id: "b", name: "B", ...SIG, enabled: true }),
      ],
      disabledMocks: ["b"],
    });
  });

  const enable = (ctx: CommandContext) =>
    commandRegistry.invoke(
      "entity.setEnabled",
      { workspaceId: TEST_WS, kind: "mocks", id: "b", enabled: true },
      ctx,
    );

  it("disables the conflicting sibling for the engine's own caller", async () => {
    expect(enabledOf("a")).toBe(true);
    expect(enabledOf("b")).toBe(false);

    await enable(engineCtx());

    expect(enabledOf("b")).toBe(true);
    expect(enabledOf("a")).toBe(false);
  });

  it("enables the entity but leaves the sibling alone for the companion", async () => {
    await enable(companionCtx());

    // The toggle itself is unconditional — only the sibling-disabling half is narrowed.
    expect(enabledOf("b")).toBe(true);
    expect(enabledOf("a")).toBe(true);
  });
});
