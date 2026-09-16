import { describe, it, expect, vi, beforeEach } from "vitest";

// Leaf modules, fully stubbed — this file tests the *sequence and idempotency* of engine
// shutdown, not any subsystem's own teardown.
vi.mock("@bifurc/engine/applications/processSpawner", () => ({
  processSpawner: { stopAll: vi.fn() },
}));
vi.mock("@bifurc/engine/sync/autoSync", () => ({ stopAllAutoSync: vi.fn() }));
vi.mock("@bifurc/engine/companion/companionServer", () => ({ stopCompanionServer: vi.fn() }));
vi.mock("@bifurc/engine/proxy/server", () => ({ stopServer: vi.fn() }));

type MockFn = ReturnType<typeof vi.fn>;

interface Harness {
  shutdownEngine: () => Promise<void>;
  isShuttingDown: () => boolean;
  stopAll: MockFn;
  stopAllAutoSync: MockFn;
  stopCompanionServer: MockFn;
  stopServer: MockFn;
}

/**
 * `shutdownEngine()` memoises its teardown for the lifetime of the module — that memoisation *is*
 * the idempotency guarantee under test. So each test needs a fresh module registry, otherwise the
 * first test's shutdown would satisfy every later one and the assertions would be vacuous.
 */
async function harness(): Promise<Harness> {
  vi.resetModules();
  const spawner = await import("@bifurc/engine/applications/processSpawner");
  const autoSync = await import("@bifurc/engine/sync/autoSync");
  const companion = await import("@bifurc/engine/companion/companionServer");
  const proxy = await import("@bifurc/engine/proxy/server");
  const shutdown = await import("@/shutdown");
  return {
    shutdownEngine: shutdown.shutdownEngine,
    isShuttingDown: shutdown.isShuttingDown,
    stopAll: spawner.processSpawner.stopAll as unknown as MockFn,
    stopAllAutoSync: autoSync.stopAllAutoSync as unknown as MockFn,
    stopCompanionServer: companion.stopCompanionServer as unknown as MockFn,
    stopServer: proxy.stopServer as unknown as MockFn,
  };
}

describe("shutdown/shutdownEngine", () => {
  // `vi.resetModules()` clears the module registry but *not* the mock registry, so the same
  // `vi.fn()` instances are reused by every test in this file. Without this, call counts
  // accumulate across tests and the idempotency assertions below read as false failures.
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("tears down every subsystem exactly once", async () => {
    const h = await harness();

    await h.shutdownEngine();

    expect(h.stopAll).toHaveBeenCalledTimes(1);
    expect(h.stopAllAutoSync).toHaveBeenCalledTimes(1);
    expect(h.stopCompanionServer).toHaveBeenCalledTimes(1);
    expect(h.stopServer).toHaveBeenCalledTimes(1);
  });

  it("stops child processes and pollers before the servers", async () => {
    const h = await harness();

    await h.shutdownEngine();

    const order = (m: MockFn) => m.mock.invocationCallOrder[0];
    // Otherwise a server still accepting traffic can spawn work after we tore the spawner down.
    expect(order(h.stopAll)).toBeLessThan(order(h.stopCompanionServer));
    expect(order(h.stopAll)).toBeLessThan(order(h.stopServer));
    expect(order(h.stopAllAutoSync)).toBeLessThan(order(h.stopServer));
  });

  it("is idempotent — repeated calls do not re-run the teardown", async () => {
    const h = await harness();

    await h.shutdownEngine();
    await h.shutdownEngine();
    await h.shutdownEngine();

    expect(h.stopAll).toHaveBeenCalledTimes(1);
    expect(h.stopAllAutoSync).toHaveBeenCalledTimes(1);
    expect(h.stopCompanionServer).toHaveBeenCalledTimes(1);
    expect(h.stopServer).toHaveBeenCalledTimes(1);
  });

  it("shares a single teardown between concurrent callers", async () => {
    const h = await harness();

    // The shell quitting, a supervisor SIGTERM and a crash handler can all land in the same tick.
    await Promise.all([h.shutdownEngine(), h.shutdownEngine(), h.shutdownEngine()]);

    expect(h.stopAll).toHaveBeenCalledTimes(1);
    expect(h.stopServer).toHaveBeenCalledTimes(1);
  });

  it("hands every caller the same promise", async () => {
    const h = await harness();

    const a = h.shutdownEngine();
    const b = h.shutdownEngine();

    expect(a).toBe(b);
    await a;
  });

  it("reports shutdown state through isShuttingDown()", async () => {
    const h = await harness();

    expect(h.isShuttingDown()).toBe(false);

    const pending = h.shutdownEngine();
    expect(h.isShuttingDown()).toBe(true);

    await pending;
    // Still true after completion — a late caller must not restart the engine's subsystems.
    expect(h.isShuttingDown()).toBe(true);
  });
});
