/**
 * `session.ts`'s own cases — the shared client core, over a **fake channel**.
 *
 * ## Why this file exists at all, when three transports already run 270 conformance cases
 *
 * `session.ts` was extracted from `stdio.ts` and `ws.ts` so that `socket.ts` would not be a third copy
 * of the same state machine. That makes it the one module in this directory whose contract is
 * **internal** rather than a `Transport`: the conformance suite hands it a real channel and can only
 * observe what comes out of the far end, so several of its rules are not directly assertable there.
 *
 * Three in particular:
 *
 *  - **`opened()` must replay the outbox verbatim.** The first version called `sendFrame()` on entries
 *    that were already encoded, and `encodeFrame` does `JSON.stringify` — so a `Buffer` would have gone
 *    out as `{"type":"Buffer","data":[…]}` and the peer would have rejected it as `not_an_object`. Over
 *    a real transport this is *indirectly* covered (every queued subscription would be lost and nine
 *    conformance cases would fail), but the failure it produces is "the subscription never fires", which
 *    is the hardest class in this layer to attribute. A fake channel that records the bytes makes the
 *    double-encode visible as itself.
 *  - **The envelope rule on the way back.** A handler that *resolved* with `{ok:false}` is an
 *    envelope-level success and must reach the caller as **data**. A real registry can only produce
 *    that shape by having a handler that returns it, so testing it through a transport tests the
 *    handler, not the rule.
 *  - **The channel contract itself** — that a `write()` failure tears the session down rather than
 *    escaping, that `dispose()` is called exactly once from every teardown path, and that a
 *    caller-initiated `close()` does not report a fault. Those are statements about `ClientChannel`,
 *    and a fake channel is the only way to make a channel misbehave on purpose.
 *
 * The fake channel encodes with the real `framing.ts` codec rather than a stub, so "the bytes on the
 * wire" in these assertions are the bytes a `stdio` or `socket` peer would actually receive.
 */
import { describe, expect, it, vi } from "vitest";
import { createFrameDecoder, encodeFrame } from "../../src/transport/framing";
import {
  createClientSession,
  type ClientChannel,
  type ClientSession,
} from "../../src/transport/session";
import type { EngineEvent, Transport } from "../../src/transport/types";

/** Decode everything the fake channel was handed, so a double-encode is an assertion failure. */
function decodeAll(buffers: readonly Buffer[]): Record<string, unknown>[] {
  const frames: Record<string, unknown>[] = [];
  const decoder = createFrameDecoder({
    onFrame: (frame) => frames.push(frame as Record<string, unknown>),
    onError: (err) => {
      throw err;
    },
  });
  for (const buffer of buffers) decoder.push(buffer);
  return frames;
}

interface Harness {
  session: ClientSession;
  transport: Transport;
  /** Encoded bytes handed to `channel.write()`. */
  written: Buffer[];
  fatals: Error[];
  /** Flip to make the next `write()` throw, as a dead peer would. */
  state: { failWrites: boolean; disposals: number; closes: number };
  /** Everything written, decoded. */
  sent(): Record<string, unknown>[];
  /** The last frame written. */
  last(): Record<string, unknown>;
  /**
   * Issue a request and wait until its frame has been written.
   *
   * `request()` is `async` — it awaits `ensureReady()` before it writes anything — so the frame is
   * *not* on the wire by the time the call returns. Every case that needs to answer a request needs
   * its `id`, and this is the only place that has to know about the microtask.
   */
  request(cmd?: string, payload?: unknown): Promise<{ pending: Promise<unknown>; id: string }>;
  /** The channel became writable. */
  open(): void;
}

function harness(overrides: Partial<ClientChannel> = {}): Harness {
  const written: Buffer[] = [];
  const fatals: Error[] = [];
  const state = { failWrites: false, disposals: 0, closes: 0 };
  let ready = false;

  const channel: ClientChannel = {
    encode: (frame) => encodeFrame(frame),
    write: (frame) => {
      // The contract says `write()` is only called when `isReady()` is true, and that it must not
      // throw for a dead peer in a way that escapes — `session.ts` turns the throw into a teardown.
      if (state.failWrites) throw new Error("EPIPE");
      written.push(frame);
    },
    isReady: () => ready,
    ensureReady: () => Promise.resolve(),
    dispose: () => {
      state.disposals += 1;
    },
    onClose: () => {
      state.closes += 1;
    },
    onFatal: (error) => fatals.push(error),
    ...overrides,
  };

  const session = createClientSession({ kind: "in-process", channel });

  return {
    session,
    transport: session.transport,
    written,
    fatals,
    state,
    sent: () => decodeAll(written),
    last: () => decodeAll(written).at(-1)!,
    async request(cmd = "config.get", payload: unknown = {}) {
      const before = written.length;
      const pending = session.transport.request(cmd, payload);
      await vi.waitFor(() => expect(written.length).toBeGreaterThan(before));
      return { pending, id: String(decodeAll(written).at(-1)!.id) };
    },
    open() {
      ready = true;
      session.opened();
    },
  };
}

describe("session — the channel contract", () => {
  it("queues frames written before the channel is ready, and replays them VERBATIM on opened()", () => {
    // The bug this case was written for. `opened()` must not route queued bytes back through
    // `sendFrame()`, because `encodeFrame` would `JSON.stringify` an already-encoded `Buffer` and the
    // peer would reject `{"type":"Buffer","data":[…]}` as `not_an_object` — silently losing every
    // subscription queued before the channel came up.
    const h = harness();

    h.transport.subscribe(["event.log.entry"], () => {});
    // Queued, not written: `isReady()` is false, so nothing has reached the channel.
    expect(h.written).toHaveLength(0);

    h.open();
    expect(h.written).toHaveLength(1);
    // The frame decodes as the control frame it was meant to be — not as a serialised Buffer.
    expect(h.sent()).toEqual([
      { id: expect.any(String), action: "subscribe", payload: { events: ["event.log.entry"] } },
    ]);
  });

  it("awaits ensureReady() before the first request, and re-checks after it", async () => {
    // A connect can fail, and it tears the session down — so a caller must not be handed a promise
    // from a session that is already dead.
    let release: () => void = () => {};
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    const h = harness({ ensureReady: () => gate });
    // The channel is writable, so the *only* reason nothing goes out below is the gate — which is
    // what makes this an assertion about `ensureReady()` rather than about the outbox.
    h.open();

    const pending = h.transport.request("config.get", {});
    expect(h.written).toHaveLength(0);

    release();
    await vi.waitFor(() => expect(h.written).toHaveLength(1));
    expect(h.last()).toMatchObject({ action: "config.get" });

    // Settle it so the case does not end with a promise in flight.
    h.session.receive({ id: h.last().id, ok: true, data: null });
    await expect(pending).resolves.toBeNull();
  });

  it("tears the session down when a write fails, rather than letting the throw escape", async () => {
    // `write()` is called from inside a bus listener or a stream's `data` handler. A throw from there
    // unwinds into machinery that is not expecting it, and the resulting unhandled `error` event on a
    // socket nobody is watching is far harder to diagnose than a clean teardown.
    const h = harness();
    h.open();

    const { pending } = await h.request();
    h.state.failWrites = true;
    // A fresh name, so this actually writes — and throws.
    h.transport.subscribe(["event.server.error"], () => {});

    await expect(pending).rejects.toThrow(/Send failed/);
    expect(h.fatals).toHaveLength(1);
    expect(h.state.disposals).toBe(1);
  });

  it("calls dispose() once from a graceful close, and does not report it as a fault", async () => {
    const h = harness();
    h.open();
    await h.transport.close();

    expect(h.state.disposals).toBe(1);
    expect(h.state.closes).toBe(1);
    // A clean shutdown reported as a fault would make every orderly exit look like a crash.
    expect(h.fatals).toHaveLength(0);

    // Idempotent, and the second call must not dispose again.
    await h.transport.close();
    expect(h.state.disposals).toBe(1);
  });

  it("calls onFatal for a fault and does not call onClose", () => {
    const h = harness();
    h.session.fail(new Error("the peer went away"), true);
    expect(h.fatals).toHaveLength(1);
    // `onClose` is the graceful path only — `stdio` uses it to end a spawned engine's stdin, which
    // must not happen when the session died for a reason nobody asked for.
    expect(h.state.closes).toBe(0);
    expect(h.state.disposals).toBe(1);
  });

  it("refuses a request or a subscribe once the session has ended", async () => {
    const h = harness();
    await h.transport.close();
    await expect(h.transport.request("config.get", {})).rejects.toThrow(/Cannot request\(\)/);
    expect(() => h.transport.subscribe(["event.log.entry"], () => {})).toThrow(/Cannot subscribe\(\)/);
  });
});

describe("session — subscriptions", () => {
  it("rejects an unknown event name synchronously and attaches NOTHING", () => {
    // `subscribe()` is not async, so a bad name has to be caught here rather than awaited — and the
    // all-or-nothing rule means a list with one bad name must not leave the good ones live. A caller
    // that caught the refusal and retried would otherwise accumulate duplicates it could not see.
    const h = harness();
    h.open();

    expect(() => h.transport.subscribe(["event.log.entry", "nope.not.an.event"], () => {})).toThrow(
      /cannot deliver "nope.not.an.event"/,
    );

    // Nothing was sent, and the good name is not live either.
    expect(h.written).toHaveLength(0);
    const seen: EngineEvent[] = [];
    h.transport.subscribe(["event.log.entry"], (e) => seen.push(e));
    h.session.receive({ event: "event.log.entry", seq: 1, payload: "a" });
    // Exactly one delivery: the refused subscribe attached nothing.
    expect(seen).toHaveLength(1);
  });

  it("refcounts by NAME: two subscribers share one control frame, and each off() removes only its own", () => {
    const h = harness();
    h.open();

    const seen: string[] = [];
    const cb = (e: EngineEvent): void => {
      seen.push(String(e.payload));
    };
    const off1 = h.transport.subscribe(["event.log.entry"], cb);
    const off2 = h.transport.subscribe(["event.log.entry"], cb);

    // One wire subscription for two callers.
    expect(h.sent().filter((f) => f.action === "subscribe")).toHaveLength(1);

    // A set of tokens, not a set of callbacks: the same function twice fires twice.
    h.session.receive({ event: "event.log.entry", seq: 1, payload: "a" });
    expect(seen).toEqual(["a", "a"]);

    off1();
    // Still subscribed — and no `unsubscribe` was sent, because a token remains.
    h.session.receive({ event: "event.log.entry", seq: 2, payload: "b" });
    expect(seen).toEqual(["a", "a", "b"]);
    expect(h.sent().filter((f) => f.action === "unsubscribe")).toHaveLength(0);

    off2();
    expect(h.sent().filter((f) => f.action === "unsubscribe")).toHaveLength(1);

    // And calling an unsubscribe twice sends nothing extra.
    off2();
    expect(h.sent().filter((f) => f.action === "unsubscribe")).toHaveLength(1);
  });

  it("carries the ENGINE's seq, so a subset subscription sees gaps", () => {
    // `seq` is engine-scoped, so events the session did not subscribe to still consume a number. A
    // client-side recount would number by arrival and produce `[1, 2]`.
    const h = harness();
    h.open();
    const seen: number[] = [];
    h.transport.subscribe(["event.server.error"], (e) => seen.push(e.seq));

    h.session.receive({ event: "event.server.error", seq: 41, payload: "a" });
    h.session.receive({ event: "event.log.chunk", seq: 42, payload: {} });
    h.session.receive({ event: "event.server.error", seq: 43, payload: "b" });

    expect(seen).toEqual([41, 43]);
  });

  it("says goodbye for every live name before tearing down", async () => {
    // Without this, closing an **attached** session leaves the engine's event-log subscriptions
    // attached with nobody to receive them, and the channel may outlive us by hours — one leaked
    // listener per closed session until Node's 11-listener warning fires.
    const h = harness();
    h.open();
    h.transport.subscribe(["event.log.entry"], () => {});
    h.transport.subscribe(["event.server.error"], () => {});
    h.written.length = 0;

    await h.transport.close();

    const goodbye = h.sent().find((f) => f.action === "unsubscribe");
    expect(goodbye).toBeDefined();
    expect((goodbye!.payload as { events: string[] }).events.sort()).toEqual([
      "event.log.entry",
      "event.server.error",
    ]);
  });
});

describe("session — the envelope rule", () => {
  it("resolves with an inner {ok:false} UNINSPECTED, because the handler did not throw", async () => {
    // The rule that keeps `window.api` byte-identical: a handler that *resolved* has succeeded as far
    // as the envelope is concerned, whatever it resolved with. Most legacy handlers report failure
    // this way, so conflating the two flags would break all of them.
    const h = harness();
    h.open();
    const { pending, id } = await h.request();
    h.session.receive({ id, ok: true, data: { ok: false, error: "the handler said no" } });
    await expect(pending).resolves.toEqual({ ok: false, error: "the handler said no" });
  });

  it("rejects when the OUTER ok is false, with the peer's code", async () => {
    const h = harness();
    h.open();
    const { pending, id } = await h.request();
    h.session.receive({ id, ok: false, error: { code: "FORBIDDEN", message: "out of scope" } });
    await expect(pending).rejects.toMatchObject({ code: "FORBIDDEN" });
  });

  it("ignores a response to an id nobody is waiting on", async () => {
    // A duplicate, or a late answer to a request whose session has moved on. There is no caller to
    // tell, and killing a working session over it would be worse than ignoring it.
    const h = harness();
    h.open();
    h.session.receive({ id: "not-a-pending-id", ok: true, data: null });
    expect(h.fatals).toHaveLength(0);

    const { pending, id } = await h.request();
    h.session.receive({ id, ok: true, data: "still working" });
    await expect(pending).resolves.toBe("still working");
  });

  it("treats a refused CONTROL frame as fatal, because the caller has nowhere else to hear it", async () => {
    // `subscribe()` is synchronous and returns only an unsubscribe function, so a late refusal cannot
    // be reported to it. A caller that believed it was subscribed and is not would wait forever for
    // events that never arrive — precisely the "silently never fires" defect the contract forbids.
    const h = harness();
    h.open();
    h.transport.subscribe(["event.log.entry"], () => {});
    h.session.receive({
      id: h.last().id,
      ok: false,
      error: { code: "CONFLICT", message: "the replay window moved" },
    });
    expect(h.fatals).toHaveLength(1);
    expect(h.fatals[0].message).toMatch(/refused a transport control frame/);
  });

  it("ends the session on a frame that is neither a response nor an event", () => {
    const h = harness();
    h.open();
    h.session.receive({ nothing: "useful" });
    expect(h.fatals).toHaveLength(1);
    expect(h.fatals[0].message).toMatch(/neither a response nor an event/);
  });

  it("ends the session on a frame that is not an object at all", () => {
    const h = harness();
    h.open();
    h.session.receive(null);
    expect(h.fatals).toHaveLength(1);
    expect(h.fatals[0].message).toMatch(/not an object/);
  });

  it("drops a payload the wire cannot represent WITHOUT killing a healthy session", async () => {
    // A `BigInt` is a caller error, not a dead peer. Conflating the two would turn "you passed a
    // BigInt" into a dropped connection — which is why `encode()` is called *outside* the write's
    // `try` in `session.ts`.
    const h = harness();
    h.open();
    await expect(h.transport.request("config.get", { n: 1n })).rejects.toMatchObject({
      code: "BAD_REQUEST",
    });

    // Nothing was written for the failed call, no teardown happened, and the session still works.
    expect(h.written).toHaveLength(0);
    expect(h.fatals).toHaveLength(0);

    const { pending, id } = await h.request();
    h.session.receive({ id, ok: true, data: "still healthy" });
    await expect(pending).resolves.toBe("still healthy");
  });
});
