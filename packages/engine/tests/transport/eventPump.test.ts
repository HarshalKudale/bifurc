/**
 * `eventPump.ts` — P4 work item 3's coalescing half, tested directly.
 *
 * ## Why this suite exists *beside* the conformance cases rather than instead of them
 *
 * The shared suite verifies the pump through the `Transport` interface, which is the contract that
 * matters — and that is exactly why it cannot see most of this file. Three of the pump's properties are
 * invisible from above:
 *
 * - **The window.** A client cannot tell "batched after 250 ms" from "batched immediately"; both
 *   deliver the same events. The timing is only observable at the pump, and here it is observable
 *   *deterministically*, because the scheduler is injected. The suite's counterpart emits a flood and
 *   waits — it proves the batch happens, not when.
 * - **The bound.** That the pending queue never exceeds `maxBatch` is what makes `plan/05`'s drop
 *   policy structural rather than a mechanism (`eventPump.ts`'s header works this through). A client
 *   cannot see a queue that was never allowed to grow; this suite pushes 10,000 entries and checks the
 *   cap held.
 * - **The projection.** What the wire carries for one log entry — a light `LogEntryEvent`, not the
 *   engine's `RequestLogEntry` with its capture bodies — is a shape a client can see but cannot
 *   attribute to this layer. Here it is an assertion on one function.
 *
 * The split is the same one the layer already uses for `EventLog`: `eventLog.test.ts` (30 cases) drives
 * the mechanism, and the conformance suite drives replay through the interface.
 *
 * ## The scheduler is injected, and that is not a testing convenience
 *
 * `EventLogOptions.now` set the precedent: the age bound is a *time* behaviour, and a test that sleeps
 * is a test that is both slow and flaky. `EventPumpOptions.schedule` is the same seam for the same
 * reason, so every case here is synchronous and none of them waits 250 ms.
 */
import { describe, expect, it, vi } from "vitest";
import { LogEntryBatchEventSchema, type EventEnvelope, type LogEntryBatchEvent } from "@bifurc/protocol";
import {
  COALESCED_EVENT,
  createEventPump,
  DEFAULT_BATCH_WINDOW_MS,
  DEFAULT_MAX_BATCH,
  toClientEvent,
  toLogEntryBatch,
  toLogEntryEvent,
} from "../../src/transport/eventPump";
import type { EngineEvent } from "../../src/transport/types";

// ── helpers ──────────────────────────────────────────────────────────────────

/** A scheduler that arms nothing: the window can only be fired by hand. */
function manualClock(): {
  schedule: (fn: () => void, ms: number) => () => void;
  armedDelays: () => number[];
  armedCount: () => number;
  fire: () => void;
  cancelled: () => number;
} {
  let nextId = 1;
  const timers = new Map<number, { fn: () => void; ms: number }>();
  let cancelledCount = 0;

  return {
    schedule(fn, ms) {
      const id = nextId++;
      timers.set(id, { fn, ms });
      return () => {
        if (timers.delete(id)) cancelledCount += 1;
      };
    },
    armedDelays: () => [...timers.values()].map((t) => t.ms),
    armedCount: () => timers.size,
    /** Run the only armed timer, as the event loop eventually would. */
    fire() {
      const [first] = [...timers.entries()];
      if (first === undefined) throw new Error("no timer is armed — nothing to fire");
      const [id, timer] = first;
      timers.delete(id);
      timer.fn();
    },
    cancelled: () => cancelledCount,
  };
}

/** Collect every envelope the pump writes. */
function recorder(): { frames: EventEnvelope[]; send: (e: EventEnvelope) => void } {
  const frames: EventEnvelope[] = [];
  return { frames, send: (e) => void frames.push(e) };
}

/** The capture body the bus really carries, and the reason a batch must be projected before it ships. */
const CAPTURE_BODY = "captured-body-must-not-reach-the-wire";

/**
 * A `log.entry` event as the **bus** emits it: the engine's internal `RequestLogEntry`, complete with
 * the capture bodies and a `ts` field. Deliberately the heavy shape — if the pump batched this, the
 * cases below would be measuring the defect.
 */
function entry(seq: number): EngineEvent {
  return {
    event: COALESCED_EVENT as EngineEvent["event"],
    seq,
    payload: {
      id: `req-${seq}`,
      ts: 1_000 + seq,
      method: "GET",
      url: "https://example.test/",
      host: "example.test",
      status: 200,
      via: "proxy",
      target: null,
      durationMs: 3,
      reqHeaders: {},
      reqBody: "",
      resHeaders: {},
      resBody: CAPTURE_BODY,
    },
  };
}

/**
 * The **wire** entry for `seq`, written out independently of the implementation.
 *
 * Not derived from `toLogEntryEvent` on purpose: a test that asks the code what the answer is cannot
 * disagree with it. This is the contract as `LogEntryEventSchema` states it — six fields, and the
 * timestamp under its wire name.
 */
function wire(seq: number): Record<string, unknown> {
  return {
    id: `req-${seq}`,
    timestamp: 1_000 + seq,
    method: "GET",
    url: "https://example.test/",
    status: 200,
    durationMs: 3,
  };
}

/** An event the policy table sends immediately. */
function immediate(seq: number, event = "event.server.error"): EngineEvent {
  return { event: event as EngineEvent["event"], seq, payload: `failure ${seq}` };
}

/** The entries of a batch frame, for the size and content assertions. */
function batchOf(frame: EventEnvelope): LogEntryBatchEvent {
  return frame.payload as LogEntryBatchEvent;
}

// ── the policy table ─────────────────────────────────────────────────────────

describe("the policy table", () => {
  it("sends a non-coalesced event immediately, without arming a window", () => {
    const clock = manualClock();
    const { frames, send } = recorder();
    const pump = createEventPump({ send, schedule: clock.schedule });

    pump.push(immediate(1));

    expect(frames).toHaveLength(1);
    expect(frames[0]).toMatchObject({ event: "event.server.error", seq: 1 });
    expect(clock.armedCount(), "an immediate event must not start a batch window").toBe(0);
  });

  it("holds a `log.entry` instead of sending it", () => {
    const clock = manualClock();
    const { frames, send } = recorder();
    const pump = createEventPump({ send, schedule: clock.schedule });

    pump.push(entry(1));

    expect(frames, "nothing is written until the cap or the window").toHaveLength(0);
    expect(clock.armedDelays()).toEqual([DEFAULT_BATCH_WINDOW_MS]);
  });

  it("defaults to the protocol's 100 entries and 250 ms", () => {
    // The numbers are `plan/02`'s and `packages/protocol/src/events.ts`'s, restated as constants. A
    // change to either without a matching change here is what this pins.
    expect(DEFAULT_MAX_BATCH).toBe(100);
    expect(DEFAULT_BATCH_WINDOW_MS).toBe(250);
  });

  it("coalesces a burst into ONE frame carrying the protocol's batch payload", () => {
    const clock = manualClock();
    const { frames, send } = recorder();
    const pump = createEventPump({ send, schedule: clock.schedule });

    for (let seq = 1; seq <= 3; seq += 1) pump.push(entry(seq));
    expect(frames).toHaveLength(0);

    clock.fire();

    expect(frames).toHaveLength(1);
    expect(frames[0]!.event).toBe(COALESCED_EVENT);
    // The shape is `LogEntryBatchEventSchema`'s, not a bare array — see `asLogEntryBatch` below.
    expect(frames[0]!.payload).toEqual({ entries: [wire(1), wire(2), wire(3)] });
  });

  it("keeps the capture bodies off the wire, which is what makes a batch affordable", () => {
    /**
     * The reason the projection is load-bearing rather than cosmetic. A bus entry carries the captured
     * request and response bodies (base64, the response capped at 512 KB). One of those is a large
     * frame; a batch of 100 of them is up to ~50 MB in a single frame. Batching is only viable because
     * `LogEntryEventSchema` already decided the wire carries a light entry.
     */
    const clock = manualClock();
    const { frames, send } = recorder();
    const pump = createEventPump({ send, schedule: clock.schedule });

    pump.push(entry(1));
    clock.fire();

    expect(batchOf(frames[0]!).entries[0]).not.toHaveProperty("resBody");
    expect(batchOf(frames[0]!).entries[0]).not.toHaveProperty("reqBody");
    expect(JSON.stringify(frames[0]!.payload)).not.toContain(CAPTURE_BODY);
  });
});

// ── the two bounds ───────────────────────────────────────────────────────────

describe("the batch bounds", () => {
  it("flushes at the cap without waiting for the window", () => {
    const clock = manualClock();
    const { frames, send } = recorder();
    const pump = createEventPump({ send, schedule: clock.schedule });

    for (let seq = 1; seq <= DEFAULT_MAX_BATCH; seq += 1) pump.push(entry(seq));

    expect(frames, "the 100th entry fills the batch and it goes").toHaveLength(1);
    expect(batchOf(frames[0]!).entries).toHaveLength(DEFAULT_MAX_BATCH);
    expect(clock.armedCount(), "the window is disarmed by the flush").toBe(0);
  });

  it("flushes on the window when the cap is never reached", () => {
    const clock = manualClock();
    const { frames, send } = recorder();
    const pump = createEventPump({ send, schedule: clock.schedule });

    pump.push(entry(1));
    pump.push(entry(2));
    expect(frames).toHaveLength(0);

    clock.fire();

    expect(frames).toHaveLength(1);
    expect(batchOf(frames[0]!).entries).toHaveLength(2);
  });

  it("arms the window once per batch, not once per entry", () => {
    // The starvation guard. Re-arming on every entry would make the window "250 ms since the last
    // entry" rather than "since the batch opened" — and a steady stream that never quite reaches the
    // cap would then never flush at all, which is exactly the load this exists for.
    const clock = manualClock();
    const { send } = recorder();
    const pump = createEventPump({ send, schedule: clock.schedule });

    for (let seq = 1; seq <= 50; seq += 1) pump.push(entry(seq));

    expect(clock.armedCount(), "fifty entries, one window").toBe(1);
  });

  it("never lets a batch exceed the cap, however many entries are pushed", () => {
    /**
     * The structural claim `eventPump.ts`'s header makes in prose: the pending queue is bounded by
     * `maxBatch` **by construction**, because the cap-triggered flush is synchronous. That is what
     * makes `plan/05`'s drop policy ("drop `log.entry` before `server.error`") a property of the
     * code's shape rather than a triage mechanism that has to be written and got right.
     *
     * If a future change makes `send` asynchronous, or removes the cap-triggered flush, this fails —
     * and the drop policy becomes a real question again, which is the point of asserting it.
     */
    const clock = manualClock();
    const { frames, send } = recorder();
    const pump = createEventPump({ send, schedule: clock.schedule });

    // A cap-exact flood first: 10,000 is a whole number of batches, so every push either fills a batch
    // or joins one, and **no timer is ever left armed** — the cap-triggered flush alone drains it. That
    // is the bound doing its job without the window's help.
    const exact = 10_000;
    for (let seq = 1; seq <= exact; seq += 1) pump.push(entry(seq));

    expect(frames).toHaveLength(exact / DEFAULT_MAX_BATCH);
    expect(clock.armedCount(), "a cap-exact flood never needs the window").toBe(0);

    // Then a remainder, which is the one case that does need it.
    for (let seq = exact + 1; seq <= exact + 50; seq += 1) pump.push(entry(seq));
    expect(frames, "fifty held back").toHaveLength(exact / DEFAULT_MAX_BATCH);
    clock.fire();

    expect(frames).toHaveLength(exact / DEFAULT_MAX_BATCH + 1);
    for (const frame of frames) {
      expect(batchOf(frame).entries.length).toBeLessThanOrEqual(DEFAULT_MAX_BATCH);
    }
    expect(
      frames.reduce((n, f) => n + batchOf(f).entries.length, 0),
      "and none is lost",
    ).toBe(exact + 50);
  });

  it("honours a custom cap and window", () => {
    const clock = manualClock();
    const { frames, send } = recorder();
    const pump = createEventPump({ send, schedule: clock.schedule, maxBatch: 2, batchWindowMs: 10 });

    pump.push(entry(1));
    pump.push(entry(2));

    expect(frames).toHaveLength(1);
    expect(clock.armedDelays()).toEqual([]);
    // And the window it would have armed is the injected one.
    pump.push(entry(3));
    expect(clock.armedDelays()).toEqual([10]);
  });
});

// ── FIFO, and the `server.error` guarantee ───────────────────────────────────

describe("ordering and the drop policy", () => {
  it("flushes the pending batch BEFORE an immediate event", () => {
    /**
     * The invariant the conformance suite's delivery barrier rests on: wire order == emission order.
     * Without the flush-first rule, `server.error` would overtake log entries that were emitted before
     * it — and a barrier that "proves everything emitted earlier was delivered" would be proving
     * something false.
     */
    const clock = manualClock();
    const { frames, send } = recorder();
    const pump = createEventPump({ send, schedule: clock.schedule });

    pump.push(entry(1));
    pump.push(entry(2));
    pump.push(immediate(3));

    expect(frames.map((f) => f.event)).toEqual([COALESCED_EVENT, "event.server.error"]);
    expect(frames.map((f) => f.seq), "the batch is the earlier of the two").toEqual([2, 3]);
    expect(clock.armedCount(), "the flush disarmed the window").toBe(0);
  });

  it("never holds a `server.error` — it is written on the spot, mid-flood", () => {
    // "Never dropped" is a consequence of "never queued": a value that is not held cannot be
    // displaced by anything, so there is no priority comparison to get wrong. Asserted rather than
    // asserted-about, because the guarantee is the acceptance criterion.
    const clock = manualClock();
    const { frames, send } = recorder();
    const pump = createEventPump({ send, schedule: clock.schedule });

    for (let seq = 1; seq <= 250; seq += 1) {
      pump.push(entry(seq));
      pump.push(immediate(1_000 + seq));
    }

    const errors = frames.filter((f) => f.event === "event.server.error");
    expect(errors).toHaveLength(250);
    expect(errors.map((f) => f.seq)).toEqual(Array.from({ length: 250 }, (_, i) => 1_000 + i + 1));
  });

  it("keeps every event across a flood, in order, with none lost", () => {
    const clock = manualClock();
    const { frames, send } = recorder();
    const pump = createEventPump({ send, schedule: clock.schedule });

    for (let seq = 1; seq <= 250; seq += 1) pump.push(entry(seq));
    clock.fire();
    pump.push(immediate(251));

    // Every batch's `seq` is the highest entry seq it carries, so the envelopes are strictly
    // increasing and the last one is the flood's end. That monotonicity is what `lastSeq` relies on.
    const batchSeqs = frames.filter((f) => f.event === COALESCED_EVENT).map((f) => f.seq);
    expect(batchSeqs).toEqual([100, 200, 250]);
    expect(frames[frames.length - 1]!.seq).toBe(251);
    expect(
      frames.reduce((n, f) => n + (f.event === COALESCED_EVENT ? batchOf(f).entries.length : 1), 0),
      "250 entries plus the one immediate event",
    ).toBe(251);
  });

  it("sets the batch's `seq` to its LAST entry's, so replay cannot duplicate it", () => {
    /**
     * The decision with the most consequence in the file. A client's `lastSeq` is the last envelope
     * `seq` it saw, and the engine replays everything with `seq > lastSeq`. Carrying the *first*
     * entry's seq would put the resume point inside the batch and every entry after the first would
     * be replayed again on the next reconnect.
     */
    const clock = manualClock();
    const { frames, send } = recorder();
    const pump = createEventPump({ send, schedule: clock.schedule });

    // Deliberately gapped, the way engine-scoped `seq` is: other events consumed 3 and 4.
    pump.push(entry(2));
    pump.push(entry(5));
    clock.fire();

    expect(frames[0]!.seq).toBe(5);
  });
});

// ── close ────────────────────────────────────────────────────────────────────

describe("close()", () => {
  it("delivers the pending batch best-effort", () => {
    // Flush rather than discard: `stdio`'s server has no `EventLog`, so an entry dropped at close is
    // gone permanently. The rule has to be safe where the log is absent, not only where it is present.
    const clock = manualClock();
    const { frames, send } = recorder();
    const pump = createEventPump({ send, schedule: clock.schedule });

    pump.push(entry(1));
    pump.push(entry(2));
    pump.close();

    expect(frames).toHaveLength(1);
    expect(batchOf(frames[0]!).entries).toHaveLength(2);
    expect(clock.cancelled(), "the window is cancelled, not left to fire later").toBe(1);
  });

  it("ignores everything pushed after it, and is idempotent", () => {
    const clock = manualClock();
    const { frames, send } = recorder();
    const pump = createEventPump({ send, schedule: clock.schedule });

    pump.close();
    pump.close();
    pump.push(entry(1));
    pump.push(immediate(2));

    expect(frames).toHaveLength(0);
  });

  it("does not throw when the channel is already dead", () => {
    // `close()` runs in teardown paths where there is nobody left to report a failure to, so a
    // caller's `send` that throws must not escape. Every transport's `send` swallows a write failure
    // already; this is the backstop for one that does not.
    const clock = manualClock();
    const boom = vi.fn(() => {
      throw new Error("channel is gone");
    });
    const pump = createEventPump({ send: boom, schedule: clock.schedule });

    pump.push(entry(1));
    expect(() => pump.close()).not.toThrow();
    expect(boom).toHaveBeenCalledTimes(1);
  });
});

// ── the projection, and the single-entry wrap ────────────────────────────────

describe("toLogEntryEvent", () => {
  it("renames `ts` to `timestamp` and keeps the six wire fields", () => {
    // The rename is the protocol's, not a mistake: `RequestLogEntry.ts` is the bus's name and
    // `LogEntryEvent.timestamp` is the wire's. A projection that kept `ts` would produce entries the
    // protocol's own schema silently strips the timestamp from.
    expect(toLogEntryEvent(entry(7).payload)).toEqual(wire(7));
  });

  it("produces a payload the protocol's own schema accepts", () => {
    // The schema is the assertion rather than a structural `toEqual`, because the helper's whole job
    // is to produce *that* shape — and `z.object()` strips unknown keys silently, so a `toEqual`
    // would keep passing while the wire carried fields the contract does not name.
    const parsed = LogEntryBatchEventSchema.safeParse(toLogEntryBatch(entry(1).payload));
    expect(parsed.success, parsed.success ? "" : JSON.stringify(parsed.error.issues)).toBe(true);
    expect(parsed.success && parsed.data.entries[0]).toEqual(wire(1));
  });

  it("is idempotent, so a transport downstream of another can re-apply it", () => {
    /**
     * The property the whole design rests on, and the one the first version of this change lacked.
     *
     * `ws` and `socket` compose a client-facing transport (`in-process`) inside their own server, so
     * their pump is *downstream* of something that has already applied the shape. Without idempotence
     * the pump projects an already-shaped `{entries}` payload as if it were a raw entry, every field
     * falls back to its default, and the client receives a batch of empty rows — which is exactly what
     * happened, over `ws` and `socket` only, while `stdio` was correct.
     */
    const once = toLogEntryBatch(entry(1).payload);
    expect(toLogEntryBatch(once)).toEqual(once);
    expect(toLogEntryBatch(toLogEntryBatch(once))).toEqual(once);
  });

  it("does not mistake a raw entry for a batch", () => {
    // `RequestLogEntry` has no `entries` field, so the discriminator is unambiguous — and this is the
    // case that fails if a future bus payload ever gains one.
    const raw = entry(1).payload as Record<string, unknown>;
    expect(Object.hasOwn(raw, "entries")).toBe(false);
    expect(toLogEntryBatch(raw).entries).toHaveLength(1);
  });

  it("zero-fills a malformed payload rather than emitting a frame the peer rejects", () => {
    // Defensive, not expected: the bus is typed, so this shape means something upstream bypassed the
    // type. One meaningless row in a client's log list beats a frame it cannot parse.
    expect(toLogEntryEvent(undefined)).toEqual({
      id: "",
      timestamp: 0,
      method: "",
      url: "",
      status: null,
      durationMs: null,
    });
    expect(toLogEntryEvent({ id: "r", ts: "not a number", status: "200" })).toMatchObject({
      id: "r",
      timestamp: 0,
      status: null,
    });
  });
});

describe("toClientEvent", () => {
  it("wraps one entry in the protocol's batch shape", () => {
    // Used by `inProcess.ts`, the one transport with no wire. Coalescing is what it lacks; the
    // envelope is not optional, because one event name must have one payload shape on every transport.
    expect(toClientEvent(entry(1))).toEqual({ event: COALESCED_EVENT, seq: 1, payload: { entries: [wire(1)] } });
  });

  it("leaves every other event untouched, by identity", () => {
    // Not a copy: an event this function has nothing to say about must not be rebuilt, or a transport
    // would start allocating per delivery for no reason — and `in-process` passes values by reference
    // by design (the suite's "what this deliberately does not assert" section).
    const other = immediate(3);
    expect(toClientEvent(other)).toBe(other);
  });
});
