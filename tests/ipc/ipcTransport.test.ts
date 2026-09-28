/**
 * `src/ipcTransport.ts` — the preload half of the P6 seam.
 *
 * This is the mirror of `tests/ipc/rpcBridge.test.ts`, and the pair is what makes the boundary
 * testable at all: one side is asserted to *produce* a survivable failure shape, the other to
 * *consume* it. Neither test can prove the round trip on its own — that is what the e2e suite is
 * for — but together they pin both ends of the contract that the round trip has to honour.
 *
 * The case that matters most is **"rebuilds an EngineError, preserving the code"**. It is the reason
 * the frame is discriminated rather than a rejection: Electron flattens a rejection crossing
 * `ipcMain.handle` → `ipcRenderer.invoke` and drops `EngineError.code`, and
 * `@bifurc/client`'s retry policy branches on exactly that code.
 */
import { describe, it, expect, vi, beforeEach } from "vitest";

// ── Module mocks (hoisted) ────────────────────────────────────────────────────

const { transportCalls, responder, mainListeners, emitFromMain, liveChannelCount } = vi.hoisted(() => {
  const calls: { channel: string; frame: any }[] = [];
  const r: { impl: (channel: string, frame: any) => Promise<any> } = {
    impl: async () => ({ ok: true, value: undefined }),
  };

  /**
   * The push half of the mock. `subscribe()` attaches one `ipcRenderer.on(EVENT_CHANNEL, …)`, so a
   * mock with only `invoke` would fail with `ipcRenderer.on is not a function` — which is what
   * happened the first time this suite ran against the step-3 transport, and it read as a transport
   * bug rather than an incomplete mock.
   */
  const listeners = new Map<string, Set<(e: unknown, payload: unknown) => void>>();

  return {
    transportCalls: calls,
    responder: r,
    mainListeners: listeners,
    /** Deliver a frame the way main's `webContents.send` would. */
    emitFromMain: (channel: string, payload: unknown): void => {
      for (const cb of [...(listeners.get(channel) ?? [])]) cb({}, payload);
    },
    /** How many channels still have a live listener — the leak instrument for `close()`. */
    liveChannelCount: (): number => [...listeners.values()].filter((s) => s.size > 0).length,
  };
});

vi.mock("electron", () => ({
  ipcRenderer: {
    invoke: (channel: string, frame: any) => {
      transportCalls.push({ channel, frame });
      return responder.impl(channel, frame);
    },
    on: (channel: string, cb: (e: unknown, payload: unknown) => void) => {
      let set = mainListeners.get(channel);
      if (!set) {
        set = new Set();
        mainListeners.set(channel, set);
      }
      set.add(cb);
    },
    removeAllListeners: (channel: string) => {
      mainListeners.delete(channel);
    },
  },
}));

import { EngineError } from "@bifurc/protocol";
import { createIpcTransport } from "@/ipcTransport";
import { EVENT_CHANNEL, RPC_CHANNEL } from "@/ipc/rpcContract";

beforeEach(() => {
  transportCalls.length = 0;
  mainListeners.clear();
  responder.impl = async () => ({ ok: true, value: undefined });
});

describe("src/ipcTransport.ts", () => {
  it("reports itself as the ipc kind, not in-process", () => {
    // Dispatch is identical to `in-process`; the trust model is not. The renderer is a separate
    // process behind `contextIsolation: true`, so anything keying off the kind must not be told
    // this is a direct function call.
    expect(createIpcTransport().kind).toBe("ipc");
  });

  it("sends {cmd, payload} on the shared RPC channel", async () => {
    const transport = createIpcTransport();
    await transport.request("config.get", { workspaceId: "ws-1" });

    expect(transportCalls).toHaveLength(1);
    expect(transportCalls[0].channel).toBe(RPC_CHANNEL);
    expect(transportCalls[0].frame).toEqual({ cmd: "config.get", payload: { workspaceId: "ws-1" } });
  });

  it("unwraps a successful frame and returns the value", async () => {
    responder.impl = async () => ({ ok: true, value: { activeWorkspaceId: "ws-1" } });

    await expect(createIpcTransport().request("config.get", {})).resolves.toEqual({
      activeWorkspaceId: "ws-1",
    });
  });

  it("rebuilds an EngineError, preserving the code", async () => {
    responder.impl = async () => ({
      ok: false,
      error: { code: "CONFLICT", message: "an unnamed entity would be affected" },
    });

    const err = await createIpcTransport()
      .request("entity.setEnabled", {})
      .then(() => null)
      .catch((e: unknown) => e);

    expect(err).toBeInstanceOf(EngineError);
    // Not ENGINE_ERROR. This single assertion is what keeps `isRetryable(err.code)` meaningful on
    // the far side of the hop.
    expect((err as EngineError).code).toBe("CONFLICT");
    expect((err as EngineError).message).toBe("an unnamed entity would be affected");
  });

  it("passes a resolved {ok:false} value through untouched", async () => {
    // The two `ok` flags are different questions: the outer one is the IPC hop, the inner one is the
    // handler's own report. A handler that resolves `{ok:false}` is an envelope-level SUCCESS, so its
    // value must arrive as data rather than as a rejection.
    responder.impl = async () => ({ ok: true, value: { ok: false, error: "no active workspace" } });

    await expect(createIpcTransport().request("config.get", {})).resolves.toEqual({
      ok: false,
      error: "no active workspace",
    });
  });

  it("classifies an unrecognisable failure payload as ENGINE_ERROR", async () => {
    responder.impl = async () => ({ ok: false, error: "something went wrong" });

    const err = await createIpcTransport()
      .request("config.get", {})
      .then(() => null)
      .catch((e: unknown) => e);

    expect(err).toBeInstanceOf(EngineError);
    expect((err as EngineError).code).toBe("ENGINE_ERROR");
  });

  it("classifies a failed hop as ENGINE_ERROR, naming the bridge", async () => {
    // What actually happens when the channel has no handler — the bridge was never started, or this
    // preload is talking to an older main process. Electron's own rejection is untyped, so it has to
    // be reclassified: reaching the retry policy with no code at all is the failure this whole
    // result shape exists to prevent.
    responder.impl = async () => {
      throw new Error("No handler registered for 'engine:rpc'");
    };

    const err = await createIpcTransport()
      .request("config.get", {})
      .then(() => null)
      .catch((e: unknown) => e);

    expect(err).toBeInstanceOf(EngineError);
    expect((err as EngineError).code).toBe("ENGINE_ERROR");
    expect((err as EngineError).message).toContain("RPC bridge is not reachable");
    expect((err as EngineError).message).toContain("No handler registered");
  });

  it("refuses an unknown wire name synchronously, with the contract's code", () => {
    // The transport contract is explicit that `subscribe()` throws **synchronously** rather than
    // accepting a subscription it can never fire — a silent no-op is indistinguishable from "nothing
    // has happened yet" and gets debugged as a broken engine.
    //
    // This is the one failure that *can* be synchronous here, because it is decided locally against
    // `BUS_NAME_BY_WIRE_NAME` — the same table `inProcess.ts` uses. The wording and the code are
    // deliberately identical to that transport's, so a caller cannot tell which transport it holds
    // from the error alone.
    const transport = createIpcTransport();

    expect(() => transport.subscribe(["not.an.event"], () => {})).toThrowError(EngineError);
    try {
      transport.subscribe(["not.an.event"], () => {});
    } catch (e) {
      expect((e as EngineError).code).toBe("UNSUPPORTED");
    }
    // Refused before anything was sent, so the main half never hears about a name it cannot serve.
    expect(transportCalls).toHaveLength(0);
  });

  it("subscribes on the main half, and delivers a pushed envelope to the listener", () => {
    // The whole point of step 3: events now have a path. `ipcRenderer.invoke` is request/response and
    // cannot carry them, so the frame arrives on `EVENT_CHANNEL` via `webContents.send`.
    const transport = createIpcTransport();
    const seen: unknown[] = [];
    transport.subscribe(["event.sync.status"], (e) => seen.push(e));

    expect(transportCalls).toHaveLength(1);
    expect(transportCalls[0].channel).toBe(RPC_CHANNEL);
    expect(transportCalls[0].frame).toEqual({
      cmd: "subscribe",
      payload: { events: ["event.sync.status"] },
    });

    emitFromMain(EVENT_CHANNEL, { event: "event.sync.status", seq: 7, payload: { wsId: "ws-1" } });

    expect(seen).toHaveLength(1);
    expect(seen[0]).toEqual({ event: "event.sync.status", seq: 7, payload: { wsId: "ws-1" } });
  });

  it("drops an envelope for a name nothing is listening to", () => {
    // One channel carries every event, and main broadcasts to *every* window, so a renderer that did
    // not ask for a name receives its frames anyway. Dropping them here is what makes that
    // broadcast-to-all design safe rather than leaky — and it must not throw, because this runs
    // inside an Electron event handler where a throw is swallowed and reads as a renderer crash.
    const transport = createIpcTransport();
    const seen: unknown[] = [];
    transport.subscribe(["event.sync.status"], (e) => seen.push(e));

    expect(() => emitFromMain(EVENT_CHANNEL, { event: "event.log.chunk", seq: 8, payload: {} })).not.toThrow();
    expect(() => emitFromMain(EVENT_CHANNEL, undefined)).not.toThrow();
    expect(() => emitFromMain(EVENT_CHANNEL, { seq: 9, payload: {} })).not.toThrow();
    expect(seen).toHaveLength(0);
  });

  it("sends `subscribe` once for a name, however many callbacks want it", () => {
    // The main half counts *renderers*, so two callbacks in one renderer are one subscription. If
    // this regressed, the count would inflate and the first detach would not unsubscribe — the
    // engine would keep delivering a name nobody wanted.
    const transport = createIpcTransport();
    const a = () => {};
    const b = () => {};
    transport.subscribe(["event.sync.status"], a);
    transport.subscribe(["event.sync.status"], b);

    expect(transportCalls.filter((c) => c.frame.cmd === "subscribe")).toHaveLength(1);
  });

  it("unsubscribes on the main half only when the last callback for a name detaches", () => {
    const transport = createIpcTransport();
    const offA = transport.subscribe(["event.sync.status"], () => {});
    const offB = transport.subscribe(["event.sync.status"], () => {});

    offA();
    expect(transportCalls.filter((c) => c.frame.cmd === "unsubscribe")).toHaveLength(0);

    offB();
    expect(transportCalls.filter((c) => c.frame.cmd === "unsubscribe")).toHaveLength(1);
    expect(transportCalls.at(-1)!.frame).toEqual({
      cmd: "unsubscribe",
      payload: { events: ["event.sync.status"] },
    });
  });

  it("stops delivering after the detacher runs, and the detacher is idempotent", () => {
    const transport = createIpcTransport();
    const seen: unknown[] = [];
    const off = transport.subscribe(["event.sync.status"], (e) => seen.push(e));

    emitFromMain(EVENT_CHANNEL, { event: "event.sync.status", seq: 1, payload: {} });
    off();
    off(); // second call must not send a second unsubscribe
    emitFromMain(EVENT_CHANNEL, { event: "event.sync.status", seq: 2, payload: {} });

    expect(seen).toHaveLength(1);
    expect(transportCalls.filter((c) => c.frame.cmd === "unsubscribe")).toHaveLength(1);
  });

  it("detaches the channel listener on close(), so a closed transport leaks nothing", async () => {
    // A window torn down with live subscriptions would otherwise pin engine listeners for the life of
    // the process — the leak class `socket.ts` records for closed sessions. Both halves are asserted:
    // the main half is told to release, and the renderer stops listening.
    const transport = createIpcTransport();
    transport.subscribe(["event.sync.status"], () => {});
    expect(liveChannelCount()).toBe(1);

    await transport.close();

    expect(liveChannelCount()).toBe(0);
    expect(transportCalls.filter((c) => c.frame.cmd === "unsubscribe")).toHaveLength(1);
    // And a later frame cannot reach a listener of a closed transport.
    expect(() => emitFromMain(EVENT_CHANNEL, { event: "event.sync.status", seq: 1, payload: {} })).not.toThrow();
  });

  it("refuses to subscribe after close()", async () => {
    const transport = createIpcTransport();
    await transport.close();

    try {
      transport.subscribe(["event.sync.status"], () => {});
      throw new Error("expected subscribe() to throw");
    } catch (e) {
      expect((e as EngineError).code).toBe("ENGINE_ERROR");
    }
  });

  it("rejects requests after close(), and close() is idempotent", async () => {
    const transport = createIpcTransport();
    await transport.close();
    await transport.close(); // idempotent

    const err = await transport
      .request("config.get", {})
      .then(() => null)
      .catch((e: unknown) => e);

    expect(err).toBeInstanceOf(EngineError);
    // `ENGINE_ERROR` is the *retryable* code, which is exactly why `retry.ts` also checks the
    // transport is open — a closed transport would otherwise be retried three times.
    expect((err as EngineError).code).toBe("ENGINE_ERROR");
    expect(transportCalls).toHaveLength(0);
  });
});
