/**
 * `src/ipc/rpcBridge.ts`'s **push half** — the P6 step 3 event channel.
 *
 * ## What this file is for
 *
 * Step 2 proved commands survive the IPC hop. Events are the other direction, and they are a
 * different mechanism: `ipcMain.handle`/`ipcRenderer.invoke` is request/response, so there is nothing
 * to hang an event on. Main pushes with `webContents.send`, the renderer listens with
 * `ipcRenderer.on`, and the two are joined by `EVENT_CHANNEL`.
 *
 * The assertions here are therefore about **traffic and accounting**, not about any event's payload:
 *
 *  - does an event emitted on the engine's bus actually reach a window?
 *  - does the per-name reference count do what it claims, so one window closing cannot unsubscribe
 *    another?
 *  - is a batch of names all-or-nothing, so a bad name cannot leave a live subscription with no
 *    handle to release it?
 *
 * ## The case that matters most
 *
 * *"carries `log.entry` end to end, which is what finding 1 was for"*. That one spans three layers —
 * `logEmitter` → `wireLogEventsToBus()` → `EventLog` → the pump-free bridge → `EVENT_CHANNEL` — and it
 * is the only test anywhere that exercises finding 1's fix through a consumer. Before it, the three
 * log events were declared, mapped and conformance-tested while **nothing emitted them**, which is
 * how the defect survived four phases with every suite green.
 */
import { describe, it, expect, vi, beforeEach } from "vitest";

// ── Module mocks (hoisted) ────────────────────────────────────────────────────

const { mockIpcMain, registeredHandlers, windows } = vi.hoisted(() => {
  const handlers = new Map<string, (...args: any[]) => any>();

  /** Windows created for a test. Each records what main pushed to it. */
  const wins: { sent: { channel: string; payload: unknown }[]; destroyed: boolean }[] = [];

  const ipcMain = {
    handle: (channel: string, handler: (...args: any[]) => any) => {
      handlers.set(channel, handler);
    },
  };

  return { mockIpcMain: ipcMain, registeredHandlers: handlers, windows: wins };
});

vi.mock("electron", () => ({
  ipcMain: mockIpcMain,
  BrowserWindow: {
    getAllWindows: () =>
      windows.map((w) => ({
        isDestroyed: () => w.destroyed,
        webContents: { send: (channel: string, payload: unknown) => w.sent.push({ channel, payload }) },
      })),
  },
}));

import { RPC_CHANNEL, EVENT_CHANNEL, SUBSCRIBE_ACTION, UNSUBSCRIBE_ACTION } from "@/ipc/rpcContract";

const EVENT = {} as never;

function addWindow(destroyed = false) {
  const w = { sent: [] as { channel: string; payload: unknown }[], destroyed };
  windows.push(w);
  return w;
}

/**
 * Build a bridge over a **fresh** module graph, and hand back the pieces the test needs.
 *
 * `vi.resetModules()` is load-bearing for the same reason `rpcBridge.test.ts` documents:
 * `commandRegistry` is a process-wide singleton whose `register()` throws on a duplicate, and
 * `registerRpcBridge()` sets a module-level flag. It also means the `bus`, `logEmitter` and
 * `EngineError` returned here are the ones **this** graph's bridge is actually using — see the trap
 * documented in that file, where an `EngineError` from the test's copy was silently degraded to
 * `ENGINE_ERROR` because `toRpcError`'s `instanceof` check failed against the bridge's copy.
 */
async function loadBridge() {
  vi.resetModules();
  const engineBus = await import("@bifurc/engine/eventBus");
  const registryModule = await import("@bifurc/engine/commands/registry");
  const logModule = await import("@bifurc/engine/proxy/logEmitter");
  const protocolModule = await import("@bifurc/protocol");
  const bridgeModule = await import("@/ipc/rpcBridge");

  bridgeModule.registerRpcBridge();

  const handler = registeredHandlers.get(RPC_CHANNEL);
  if (!handler) throw new Error(`the bridge did not register a handler on "${RPC_CHANNEL}"`);

  return {
    bus: engineBus.bus,
    /**
     * The engine's own wiring, called here exactly as `registerIpcHandlers()` calls it.
     *
     * Reproduced rather than imported because `src/ipc/handlers.ts` registers every Electron handler
     * the shell has (~136 of them) and mocking all of that would make this file about the shell
     * rather than about the channel. The consequence, stated rather than hidden: this asserts that
     * the *wiring plus the channel* deliver, not that `handlers.ts` contains the call. That second
     * claim is `wireLogEventsToBus()`'s own suite plus the fact that the shell has exactly one
     * startup path.
     */
    wireLogEventsToBus: engineBus.wireLogEventsToBus,
    registry: registryModule.commandRegistry,
    logEmitter: logModule.logEmitter,
    EngineError: protocolModule.EngineError,
    reset: bridgeModule.resetRpcBridgeForTests,
    /** Call the registered handler exactly as Electron would. */
    call: (frame: unknown) => handler(EVENT, frame),
  };
}

beforeEach(() => {
  windows.length = 0;
  registeredHandlers.clear();
});

describe("the IPC event channel (P6 step 3)", () => {
  it("broadcasts a bus event to every live window", async () => {
    const { bus, call } = await loadBridge();
    const a = addWindow();
    const b = addWindow();

    const res = await call({ cmd: SUBSCRIBE_ACTION, payload: { events: ["event.sync.status"] } });
    expect(res).toEqual({ ok: true, value: { events: ["event.sync.status"] } });

    bus.emitTyped("sync.status", { wsId: "ws-1", status: "syncing" });

    for (const w of [a, b]) {
      expect(w.sent).toHaveLength(1);
      expect(w.sent[0].channel).toBe(EVENT_CHANNEL);
      expect(w.sent[0].payload).toMatchObject({
        event: "event.sync.status",
        payload: { wsId: "ws-1", status: "syncing" },
      });
    }
  });

  it("carries `log.entry` end to end, which is what finding 1 was for", async () => {
    // `logEmitter` → `wireLogEventsToBus()` → `EventLog` → `EVENT_CHANNEL`. Nothing else in the repo
    // exercises finding 1's fix through a consumer, and before the fix this test fails with an empty
    // `sent` while every mapping test in the suite still passes — the exact shape of the defect.
    const { call, logEmitter, wireLogEventsToBus } = await loadBridge();
    const w = addWindow();

    wireLogEventsToBus();
    await call({ cmd: SUBSCRIBE_ACTION, payload: { events: ["event.log.entry"] } });

    logEmitter.emit("request", {
      id: "log-1",
      ts: 1234,
      method: "GET",
      url: "http://example.test/",
      host: "example.test",
      status: 200,
      via: "proxy",
      target: null,
      durationMs: 5,
      reqHeaders: {},
      reqBody: "",
      resHeaders: {},
      resBody: "",
      resStatus: 200,
    });

    expect(w.sent).toHaveLength(1);
    const envelope = w.sent[0].payload as { event: string; seq: number; payload: { entries: unknown[] } };
    expect(envelope.event).toBe("event.log.entry");
    // A **batch of one**, not a bare entry: the batch shape belongs to `@bifurc/protocol`, so a
    // subscriber reading `{entries}` over a socket and a bare entry here would be reading two
    // contracts for one event name. Coalescing is what this transport lacks, not the envelope.
    expect(envelope.payload.entries).toHaveLength(1);
    // The wire projection renames `ts` → `timestamp` and drops the capture bodies. Asserted here
    // because the *push* path is where a missing projection would first be visible to a client.
    expect(envelope.payload.entries[0]).toMatchObject({ id: "log-1", timestamp: 1234, method: "GET" });
    expect(envelope.payload.entries[0]).not.toHaveProperty("reqBody");
  });

  it("skips a destroyed window rather than throwing inside the broadcast", async () => {
    // A window can be torn down between the subscription and the event, and `webContents.send()` on a
    // destroyed window throws — inside a bus listener, where a throw unwinds into machinery that is
    // not expecting it. The `eventBridge.ts` this replaced guarded it the same way.
    const { bus, call } = await loadBridge();
    const live = addWindow();
    const dead = addWindow(true);

    await call({ cmd: SUBSCRIBE_ACTION, payload: { events: ["event.sync.status"] } });
    expect(() => bus.emitTyped("sync.status", { wsId: "w", status: "s" })).not.toThrow();

    expect(live.sent).toHaveLength(1);
    expect(dead.sent).toHaveLength(0);
  });

  it("subscribes to the engine once for a name, however many times it is requested", async () => {
    // The count is *renderers*, not callbacks, so a re-subscribe is a reference bump and must not
    // attach a second engine listener. A double subscription would deliver every event twice, which
    // surfaces as duplicated rows in the capture panel rather than as anything that reads as a
    // wiring bug.
    const { bus, call } = await loadBridge();
    const w = addWindow();

    await call({ cmd: SUBSCRIBE_ACTION, payload: { events: ["event.sync.status"] } });
    await call({ cmd: SUBSCRIBE_ACTION, payload: { events: ["event.sync.status"] } });

    bus.emitTyped("sync.status", { wsId: "w", status: "s" });

    expect(w.sent).toHaveLength(1);
  });

  it("keeps delivering until the last renderer releases the name", async () => {
    // The reason for a count rather than a `Set` keyed by name: one window closing must not
    // unsubscribe another that is still listening.
    const { bus, call } = await loadBridge();
    const w = addWindow();

    await call({ cmd: SUBSCRIBE_ACTION, payload: { events: ["event.sync.status"] } });
    await call({ cmd: SUBSCRIBE_ACTION, payload: { events: ["event.sync.status"] } });

    await call({ cmd: UNSUBSCRIBE_ACTION, payload: { events: ["event.sync.status"] } });
    bus.emitTyped("sync.status", { wsId: "w", status: "first" });
    expect(w.sent).toHaveLength(1);

    await call({ cmd: UNSUBSCRIBE_ACTION, payload: { events: ["event.sync.status"] } });
    bus.emitTyped("sync.status", { wsId: "w", status: "second" });
    expect(w.sent).toHaveLength(1);
  });

  it("treats an unsubscribe for a name nobody holds as a no-op", async () => {
    // A caller that unsubscribes twice must not fail the second time — the same rule every other
    // `unsubscribe` in this layer follows.
    const { call } = await loadBridge();
    const res = await call({ cmd: UNSUBSCRIBE_ACTION, payload: { events: ["event.sync.status"] } });
    expect(res).toEqual({ ok: true, value: { events: ["event.sync.status"] } });
  });

  it("classifies an unknown wire name as UNSUPPORTED, and subscribes nothing", async () => {
    const { bus, call } = await loadBridge();
    const w = addWindow();

    const res = (await call({ cmd: SUBSCRIBE_ACTION, payload: { events: ["not.an.event"] } })) as {
      ok: boolean;
      error: { code: string };
    };

    expect(res.ok).toBe(false);
    expect(res.error.code).toBe("UNSUPPORTED");

    // Nothing was retained, so a later event finds no subscription to deliver through.
    bus.emitTyped("sync.status", { wsId: "w", status: "s" });
    expect(w.sent).toHaveLength(0);
  });

  it("is all-or-nothing: a bad name in a batch leaves no subscription behind", async () => {
    // Without the pre-flight this would subscribe the first name, fail on the second, and return a
    // rejected request — leaving the caller with a live subscription and **no handle to release it**,
    // since the detacher is only ever returned on success. That is a leak, not an inconvenience.
    const { bus, call } = await loadBridge();
    const w = addWindow();

    const res = (await call({
      cmd: SUBSCRIBE_ACTION,
      payload: { events: ["event.sync.status", "not.an.event"] },
    })) as { ok: boolean; error: { code: string } };

    expect(res.ok).toBe(false);
    expect(res.error.code).toBe("UNSUPPORTED");

    bus.emitTyped("sync.status", { wsId: "w", status: "s" });
    expect(w.sent).toHaveLength(0);
  });

  it("classifies a malformed subscribe payload as BAD_REQUEST", async () => {
    // The frame comes from the renderer, which is the untrusted side of this boundary — the same
    // reason `socket.ts` parses these payloads with the protocol's schemas instead of destructuring.
    // A payload that reached the subscription code as `undefined` would throw a `TypeError` and come
    // back as an unclassifiable failure.
    const { call } = await loadBridge();

    for (const payload of [undefined, null, {}, { events: "event.sync.status" }, { events: [1, 2] }]) {
      const res = (await call({ cmd: SUBSCRIBE_ACTION, payload })) as { ok: boolean; error: { code: string } };
      expect(res.ok).toBe(false);
      expect(res.error.code).toBe("BAD_REQUEST");
    }
  });

  it("treats a control action as control, not as a command", async () => {
    // `subscribe`/`unsubscribe` are the protocol's `RESERVED_ACTIONS`, and that module refuses to
    // load if the protocol ever grows a real command by either name. So this branch cannot shadow
    // anything — but it *would* be answered `UNKNOWN_COMMAND` if the branch were missing, which is
    // what this pins.
    const { call } = await loadBridge();
    const res = (await call({ cmd: SUBSCRIBE_ACTION, payload: { events: [] } })) as {
      ok: boolean;
      value?: unknown;
      error?: { code: string };
    };
    expect(res.ok).toBe(true);
    expect(res.error).toBeUndefined();
  });

  it("still dispatches ordinary commands through the registry", async () => {
    // The push channel must not have displaced the request path — the same handler serves both, and
    // the control-action branch is checked *before* the registry, so a bug there would swallow real
    // commands. A stub is registered rather than the real `config.get` so this stays a test of the
    // dispatch rather than of the config store.
    const { call, registry } = await loadBridge();
    registry.register("config.get" as never, () => ({ activeWorkspaceId: "ws-1" }));

    const res = (await call({ cmd: "config.get", payload: {} })) as { ok: boolean; value: unknown };
    expect(res.ok).toBe(true);
    expect(res.value).toEqual({ activeWorkspaceId: "ws-1" });
  });

  it("agrees with the engine's own reserved-action names", async () => {
    // The contract duplicates these two strings rather than importing them, because the preload reads
    // that module and a value import would put a second copy of the wire-name table in the renderer
    // bundle. This is the guard that keeps the duplication honest: if `RESERVED_ACTIONS` is renamed,
    // the bridge stops recognising control frames and this fails rather than the app quietly
    // answering `UNKNOWN_COMMAND` to every subscription.
    const { RESERVED_ACTIONS } = await import("@bifurc/engine/transport/types");
    expect([...RESERVED_ACTIONS].sort()).toEqual([SUBSCRIBE_ACTION, UNSUBSCRIBE_ACTION].sort());
  });
});
