/**
 * P4 work item 3 — the event log, tested directly.
 *
 * The log has no `Transport` to run against, the same way `framing.ts` has no `Transport` to run
 * against: it is a component the transports use, not one of them. The conformance suite covers the
 * replay *behaviour* through a real transport; this file covers the log's own contract, including the
 * bounds, the boundaries, and the two properties that are the reason it exists:
 *
 * 1. **`seq` is assigned even when nobody is subscribed**, so a client that reconnects after a blip
 *    can be caught up.
 * 2. **`lastSeq` outside the retained window is a resync, never a silent partial replay.**
 */
import { describe, expect, it } from "vitest";
import { EngineEventBus } from "../../src/eventBus";
import { EventLog } from "../../src/transport/eventLog";
import type { EngineEvent } from "../../src/transport/types";

const ERROR = "event.server.error";
const CHUNK = "event.log.chunk";

function makeLog(opts: Partial<ConstructorParameters<typeof EventLog>[0]> = {}): {
  log: EventLog;
  bus: EngineEventBus;
  setNow: (t: number) => void;
} {
  const bus = new EngineEventBus();
  let clock = 1_000;
  const log = new EventLog({ bus, now: () => clock, ...opts });
  return { log, bus, setNow: (t) => (clock = t) };
}

const chunk = (text: string) => ({ logId: "l1", chunk: text, done: false });

// ── seq ──────────────────────────────────────────────────────────────────────

describe("seq", () => {
  it("starts at 1 and increases by one per retained event", () => {
    const { log, bus } = makeLog();
    log.subscribe([ERROR], () => {});
    bus.emitTyped("server.error", "a");
    bus.emitTyped("server.error", "b");
    expect(log.newestSeq).toBe(2);
  });

  it("advances even with no subscribers — the property replay depends on", () => {
    // Without this, a blip would produce a replay with a hole in it: the events that occurred while
    // nothing was subscribed would never have been numbered, and the client would be caught up to a
    // state that never existed. It is the reason retention outlives its subscribers.
    const { log, bus } = makeLog();
    log.subscribe([ERROR], () => {});
    bus.emitTyped("server.error", "while-someone-listened");
    log.subscribe([ERROR], () => {})(); // subscribe and immediately leave
    bus.emitTyped("server.error", "while-nobody-listened");
    expect(log.newestSeq).toBe(2);
    expect(log.size).toBe(2);
  });

  it("is shared across subscriptions, so the log has one order", () => {
    const { log, bus } = makeLog();
    const a: EngineEvent[] = [];
    const b: EngineEvent[] = [];
    log.subscribe([ERROR], (e) => a.push(e));
    log.subscribe([CHUNK], (e) => b.push(e));

    bus.emitTyped("server.error", "x");
    bus.emitTyped("log.chunk", chunk("hi"));
    bus.emitTyped("server.error", "y");

    expect(a.map((e) => e.seq)).toEqual([1, 3]);
    expect(b.map((e) => e.seq)).toEqual([2]);
  });

  it("leaves gaps for a session that subscribed to only some events", () => {
    // The documented trade of an engine-scoped counter: a gap can no longer be read as "events were
    // lost". Asserted rather than merely commented, so the trade is visible if someone revisits it.
    //
    // Retention is *lazy*, so a gap is only observable among **retained** events: `log.chunk` has no
    // bus listener until something subscribes to it, and an event nobody listens for is never
    // numbered. Attaching retention and immediately leaving reproduces exactly what a blip does —
    // the event is numbered while the session is gone.
    const { log, bus } = makeLog();
    const seen: EngineEvent[] = [];
    log.subscribe([ERROR], (e) => seen.push(e));
    log.subscribe([CHUNK], () => {})();

    bus.emitTyped("log.chunk", chunk("ignored"));
    bus.emitTyped("server.error", "seen");
    expect(seen.map((e) => e.seq)).toEqual([2]);
  });
});

// ── retention ────────────────────────────────────────────────────────────────

describe("retention", () => {
  it("retains events that occur while nothing is subscribed", () => {
    // The blip case. A client subscribes, the connection drops, events keep happening, the client
    // reconnects — and the events from the gap must still be there.
    const { log, bus } = makeLog();
    const off = log.subscribe([ERROR], () => {});
    bus.emitTyped("server.error", "before");
    off();
    bus.emitTyped("server.error", "during-the-blip");
    bus.emitTyped("server.error", "still-during");

    const outcome = log.replayFrom(1, [ERROR]);
    expect(outcome.kind).toBe("replay");
    expect(outcome.kind === "replay" && outcome.events.map((e) => e.payload)).toEqual([
      "during-the-blip",
      "still-during",
    ]);
  });

  it("attaches no bus listener until an event is first subscribed to", () => {
    // Lazy, so an engine that never uses an event does not hold a listener for it. Once attached the
    // listener is kept — see the previous case — so this is about the *first* attach, not release.
    const { log, bus } = makeLog();
    expect(bus.eventNames()).toHaveLength(0);
    expect(log.retainedNames()).toEqual([]);

    const off = log.subscribe([ERROR], () => {});
    expect(bus.listenerCount("server.error")).toBe(1);
    expect(log.retainedNames()).toEqual([ERROR]);

    off();
    expect(bus.listenerCount("server.error")).toBe(1);
    expect(log.retainedNames()).toEqual([ERROR]);
  });

  it("does not attach a second listener for a second subscriber", () => {
    const { log, bus } = makeLog();
    log.subscribe([ERROR], () => {});
    log.subscribe([ERROR], () => {});
    expect(bus.listenerCount("server.error")).toBe(1);
  });

  it("detaches everything on close", () => {
    const { log, bus } = makeLog();
    log.subscribe([ERROR, CHUNK], () => {});
    log.close();
    expect(bus.listenerCount("server.error")).toBe(0);
    expect(bus.listenerCount("log.chunk")).toBe(0);
    expect(log.retainedNames()).toEqual([]);
  });

  it("throws rather than silently doing nothing when closed", () => {
    const { log } = makeLog();
    log.close();
    expect(() => log.subscribe([ERROR], () => {})).toThrow(/closed/i);
  });

  it("closes idempotently", () => {
    const { log } = makeLog();
    log.close();
    expect(() => log.close()).not.toThrow();
  });
});

// ── subscribe ────────────────────────────────────────────────────────────────

describe("subscribe", () => {
  it("delivers the wire name, the seq and the payload", () => {
    const { log, bus } = makeLog();
    const seen: EngineEvent[] = [];
    log.subscribe([ERROR], (e) => seen.push(e));
    bus.emitTyped("server.error", "upstream died");
    expect(seen).toEqual([{ event: ERROR, seq: 1, payload: "upstream died" }]);
  });

  it("does not deliver an event the caller did not subscribe to", () => {
    const { log, bus } = makeLog();
    const seen: EngineEvent[] = [];
    log.subscribe([ERROR], (e) => seen.push(e));
    bus.emitTyped("log.chunk", chunk("hi"));
    expect(seen).toHaveLength(0);
  });

  it("attaches nothing when one name in the batch is bad", () => {
    // All-or-nothing. Attaching as we go would leave the earlier names buffering events for a caller
    // that caught the throw and retried, so the retry would double up.
    const { log, bus } = makeLog();
    expect(() => log.subscribe([ERROR, "event.not.real"], () => {})).toThrow(/cannot deliver/i);
    expect(log.retainedNames()).toEqual([]);
    expect(bus.eventNames()).toHaveLength(0);
  });

  it("treats an empty list as a no-op", () => {
    const { log, bus } = makeLog();
    const off = log.subscribe([], () => {});
    expect(typeof off).toBe("function");
    expect(bus.eventNames()).toHaveLength(0);
    expect(() => off()).not.toThrow();
  });

  it("fires two identical subscriptions independently", () => {
    const { log, bus } = makeLog();
    const a: EngineEvent[] = [];
    const b: EngineEvent[] = [];
    const offA = log.subscribe([ERROR], (e) => a.push(e));
    log.subscribe([ERROR], (e) => b.push(e));

    offA();
    bus.emitTyped("server.error", "x");

    expect(a).toHaveLength(0);
    expect(b).toHaveLength(1);
  });

  it("counts a duplicated name in one call as one subscription", () => {
    // `subscribe(["a", "a"])` with the same callback must not deliver twice, and a Set of callbacks
    // would not catch it because both entries are the same function.
    const { log, bus } = makeLog();
    const seen: EngineEvent[] = [];
    log.subscribe([ERROR, ERROR], (e) => seen.push(e));
    bus.emitTyped("server.error", "x");
    expect(seen).toHaveLength(1);
    expect(log.subscriberCount(ERROR)).toBe(1);
  });

  it("reports subscriber counts per name, and an unsubscribe is idempotent", () => {
    const { log } = makeLog();
    const off = log.subscribe([ERROR, CHUNK], () => {});
    expect(log.subscriberCount(ERROR)).toBe(1);
    expect(log.subscriberCount(CHUNK)).toBe(1);
    off();
    off();
    expect(log.subscriberCount(ERROR)).toBe(0);
    expect(log.subscriberCount(CHUNK)).toBe(0);
  });
});

// ── replayFrom ───────────────────────────────────────────────────────────────

describe("replayFrom", () => {
  it("replays only what the client has not seen", () => {
    const { log, bus } = makeLog();
    log.subscribe([ERROR], () => {});
    bus.emitTyped("server.error", "one");
    bus.emitTyped("server.error", "two");
    bus.emitTyped("server.error", "three");

    const outcome = log.replayFrom(1, [ERROR]);
    expect(outcome.kind).toBe("replay");
    expect(outcome.kind === "replay" && outcome.events.map((e) => e.payload)).toEqual(["two", "three"]);
  });

  it("replays nothing when the client is already current", () => {
    const { log, bus } = makeLog();
    log.subscribe([ERROR], () => {});
    bus.emitTyped("server.error", "one");
    const outcome = log.replayFrom(1, [ERROR]);
    expect(outcome.kind === "replay" && outcome.events).toEqual([]);
  });

  it("filters to the names the client asked for", () => {
    const { log, bus } = makeLog();
    log.subscribe([ERROR, CHUNK], () => {});
    bus.emitTyped("server.error", "e");
    bus.emitTyped("log.chunk", chunk("c"));

    const onlyChunks = log.replayFrom(0, [CHUNK]);
    expect(onlyChunks.kind === "replay" && onlyChunks.events.map((e) => e.event)).toEqual([CHUNK]);

    const both = log.replayFrom(0, [ERROR, CHUNK]);
    expect(both.kind === "replay" && both.events).toHaveLength(2);
  });

  it("treats the oldest retained seq minus one as still replayable", () => {
    // The inclusive boundary: a client at `oldest - 1` has seen exactly the events before the window,
    // so nothing it needs has been dropped.
    const { log, bus } = makeLog({ maxEvents: 2 });
    log.subscribe([ERROR], () => {});
    for (const p of ["1", "2", "3"]) bus.emitTyped("server.error", p);

    expect(log.oldestSeq).toBe(2);
    expect(log.replayFrom(1, [ERROR]).kind).toBe("replay");
    // One further back and the event it needs (seq 1) is gone.
    const tooOld = log.replayFrom(0, [ERROR]);
    expect(tooOld.kind).toBe("resync");
    expect(tooOld.kind === "resync" && tooOld.reason).toMatch(/older than the oldest retained/);
  });

  it("resyncs rather than replaying partially once the window has moved past the client", () => {
    const { log, bus } = makeLog({ maxEvents: 2 });
    log.subscribe([ERROR], () => {});
    for (const p of ["1", "2", "3", "4"]) bus.emitTyped("server.error", p);

    const outcome = log.replayFrom(1, [ERROR]);
    expect(outcome.kind).toBe("resync");
    // The point of the resync: a partial replay would leave the client believing it is current.
    expect(outcome.kind === "resync" && outcome.reason).toMatch(/cannot be replayed/);
  });

  it("resyncs when the client claims a seq ahead of anything the engine sent", () => {
    // Replaying nothing would leave the client believing it is up to date — a silent staleness bug,
    // which is the failure mode `plan/05` says nobody can reproduce.
    const { log, bus } = makeLog();
    log.subscribe([ERROR], () => {});
    bus.emitTyped("server.error", "one");

    const outcome = log.replayFrom(99, [ERROR]);
    expect(outcome.kind).toBe("resync");
    expect(outcome.kind === "resync" && outcome.reason).toMatch(/ahead of this engine/);
  });

  it("replays an empty history for a client that has seen nothing", () => {
    const { log } = makeLog();
    const outcome = log.replayFrom(0, [ERROR]);
    expect(outcome.kind).toBe("replay");
    expect(outcome.kind === "replay" && outcome.events).toEqual([]);
  });

  it("resyncs a fresh log against a client that claims a non-zero seq", () => {
    const { log } = makeLog();
    const outcome = log.replayFrom(5, [ERROR]);
    expect(outcome.kind).toBe("resync");
    expect(outcome.kind === "resync" && outcome.reason).toMatch(/no events are retained/);
  });

  it("answers the yes/no question without names, which is all the handshake needs", () => {
    const { log, bus } = makeLog();
    log.subscribe([ERROR], () => {});
    bus.emitTyped("server.error", "one");
    expect(log.replayFrom(0).kind).toBe("replay");
  });
});

// ── the bounds ───────────────────────────────────────────────────────────────

describe("bounds", () => {
  it("evicts the oldest events past maxEvents and keeps the newest", () => {
    const { log, bus } = makeLog({ maxEvents: 3 });
    log.subscribe([ERROR], () => {});
    for (const p of ["1", "2", "3", "4", "5"]) bus.emitTyped("server.error", p);

    expect(log.size).toBe(3);
    expect(log.oldestSeq).toBe(3);
    expect(log.newestSeq).toBe(5);
    const outcome = log.replayFrom(2, [ERROR]);
    expect(outcome.kind === "replay" && outcome.events.map((e) => e.payload)).toEqual(["3", "4", "5"]);
  });

  it("evicts events past maxAgeMs, on an injected clock", () => {
    // A time behaviour, tested without sleeping — a test that sleeps is a flaky test.
    //
    // The clock starts at 1_000, so the second event must be stamped *past* `1_000 + maxAgeMs` for
    // the first to fall outside the window. Advancing to 1_500 would leave both live, and the
    // eviction this case is about would never have been exercised.
    const { log, bus, setNow } = makeLog({ maxAgeMs: 1_000 });
    log.subscribe([ERROR], () => {});
    bus.emitTyped("server.error", "old");
    setNow(2_100);
    bus.emitTyped("server.error", "new");

    expect(log.size).toBe(1);
    expect(log.oldestSeq).toBe(2);
  });

  it("expires on read as well as on write, because the clock moves without events", () => {
    const { log, bus, setNow } = makeLog({ maxAgeMs: 1_000 });
    log.subscribe([ERROR], () => {});
    bus.emitTyped("server.error", "old");
    expect(log.size).toBe(1);

    setNow(5_000);
    expect(log.size).toBe(0);
    // And the expiry is visible to a reconnecting client rather than looking like an empty history.
    const outcome = log.replayFrom(0, [ERROR]);
    expect(outcome.kind).toBe("resync");
  });

  it("keeps everything when the age bound is disabled", () => {
    const { log, bus, setNow } = makeLog({ maxAgeMs: 0 });
    log.subscribe([ERROR], () => {});
    bus.emitTyped("server.error", "old");
    setNow(1e12);
    expect(log.size).toBe(1);
  });
});
