/**
 * P6 finding 1 — `wireLogEventsToBus()`.
 *
 * ## Why this file exists
 *
 * Three events (`log.entry`, `log.chunk`, `server.error`) were declared in `EngineEvents`, listed in
 * `ENGINE_EVENT_NAMES`, mapped by the bus→wire bridge, asserted total at import time, and given
 * conformance cases — and **nothing ever emitted them**. They travelled only through `logEmitter`,
 * which the shell's `eventBridge.ts` subscribed to directly. Every test above stayed green, because
 * every test above checks the bridge's *mapping* rather than its *traffic*.
 *
 * So the assertions here are deliberately about **traffic**: emit on the source, and require the event
 * to arrive on the bus. A mapping test cannot fail when nothing flows, which is exactly how this
 * survived four phases.
 *
 * ## Isolation
 *
 * Both `bus` and `logEmitter` are process-wide singletons, and Vitest shares a module registry across
 * every test file in a worker. A test using the defaults would leave three listeners attached for
 * whatever runs next — the leak `EngineEventBus`'s own comment warns about, one layer down. Hence the
 * injectable `target`/`source` pair, and a fresh instance of each per test.
 */
import { EventEmitter } from "events";
import { describe, expect, it } from "vitest";
import { EngineEventBus, bus, wireLogEventsToBus } from "../src/eventBus";
import { logEmitter, type RequestLogEntry } from "../src/proxy/logEmitter";

function entry(overrides: Partial<RequestLogEntry> = {}): RequestLogEntry {
  return {
    id: "log-1",
    ts: 1,
    method: "GET",
    url: "http://example.test/",
    host: "example.test",
    status: 200,
    via: "proxy",
    target: null,
    durationMs: 3,
    reqHeaders: {},
    reqBody: "",
    resHeaders: {},
    resBody: "",
    resStatus: 200,
    ...overrides,
  };
}

/** A fresh bus and a fresh source, so nothing touches the process-wide singletons. */
function harness() {
  const bus = new EngineEventBus();
  const source = new EventEmitter();

  const entries: RequestLogEntry[] = [];
  const chunks: { logId: string; chunk: string; done: boolean }[] = [];
  const errors: string[] = [];

  bus.onTyped("log.entry", (e) => entries.push(e));
  bus.onTyped("log.chunk", (c) => chunks.push(c));
  bus.onTyped("server.error", (m) => errors.push(m));

  return { bus, source, entries, chunks, errors };
}

describe("wireLogEventsToBus", () => {
  it("puts a log entry on the bus as log.entry", () => {
    const { bus, source, entries } = harness();
    wireLogEventsToBus(bus, source);

    const e = entry({ id: "log-42", method: "POST" });
    source.emit("request", e);

    expect(entries).toHaveLength(1);
    expect(entries[0]).toBe(e);
    expect(entries[0].id).toBe("log-42");
  });

  it("puts a streamed chunk on the bus as log.chunk", () => {
    const { bus, source, chunks } = harness();
    wireLogEventsToBus(bus, source);

    const c = { logId: "log-7", chunk: "aGVsbG8=", done: false };
    source.emit("chunk", c);

    expect(chunks).toEqual([c]);
  });

  it("puts a bind failure on the bus as server.error", () => {
    const { bus, source, errors } = harness();
    wireLogEventsToBus(bus, source);

    source.emit("server-error", "Port 4545 is already in use");

    expect(errors).toEqual(["Port 4545 is already in use"]);
  });

  it("carries ONE entry, not a {entries:[…]} batch", () => {
    // The batch shape belongs to the *wire*, not the bus: `eventPump.ts`'s `toClientEvent` applies it
    // in every serialising transport. Putting it on the bus here would hand every in-process
    // subscriber an envelope that no engine code produces — and the conformance suite's batch case
    // would then be asserting a shape the engine never emits.
    const { bus, source, entries } = harness();
    wireLogEventsToBus(bus, source);

    source.emit("request", entry());

    expect(entries).toHaveLength(1);
    expect(entries[0]).not.toHaveProperty("entries");
    expect(entries[0]).toHaveProperty("method");
  });

  it("is idempotent per bus — wiring twice does not double-deliver", () => {
    // The bus is a process-wide singleton and two callers are expected to wire it (`createEngine()`
    // and the shell's `registerIpcHandlers()`). Double-delivery would show up as duplicated rows in
    // the capture panel, which reads as a UI bug rather than a wiring one.
    const { bus, source, entries, chunks, errors } = harness();
    wireLogEventsToBus(bus, source);
    wireLogEventsToBus(bus, source);

    source.emit("request", entry());
    source.emit("chunk", { logId: "l", chunk: "", done: true });
    source.emit("server-error", "boom");

    expect(entries).toHaveLength(1);
    expect(chunks).toHaveLength(1);
    expect(errors).toHaveLength(1);
  });

  it("stops delivering once detached, and the detacher is safe to call twice", () => {
    const { bus, source, entries } = harness();
    const detach = wireLogEventsToBus(bus, source);

    source.emit("request", entry({ id: "before" }));
    detach();
    detach();
    source.emit("request", entry({ id: "after" }));

    expect(entries.map((e) => e.id)).toEqual(["before"]);
  });

  it("can be wired again after being detached", () => {
    // The flag is cleared on detach rather than being one-shot, so `stop()` followed by a fresh
    // engine in the same process works. A one-shot flag would silently stop all logging forever.
    const { bus, source, entries } = harness();
    const detach = wireLogEventsToBus(bus, source);
    detach();

    wireLogEventsToBus(bus, source);
    source.emit("request", entry({ id: "second-life" }));

    expect(entries.map((e) => e.id)).toEqual(["second-life"]);
  });

  it("wires each bus independently", () => {
    // The guard keys on the target bus, so wiring one bus must not suppress wiring another — which is
    // what a module-level boolean instead of a WeakSet would have done.
    const a = harness();
    const b = harness();
    const shared = new EventEmitter();

    wireLogEventsToBus(a.bus, shared);
    wireLogEventsToBus(b.bus, shared);

    shared.emit("request", entry({ id: "both" }));

    expect(a.entries.map((e) => e.id)).toEqual(["both"]);
    expect(b.entries.map((e) => e.id)).toEqual(["both"]);
  });

  it("defaults to the process-wide bus and logEmitter", () => {
    // The default path is the one both callers actually use (`createEngine()` and
    // `registerIpcHandlers()`), so it is asserted by **traffic on the singletons** rather than left to
    // production. Detached in `finally` so it cannot leak into another file in this worker.
    //
    // `wireLogEventsToBus()` is idempotent per bus, so if a previous file already wired the singleton
    // and did not detach, this call is a no-op and its own wiring still delivers — the assertion holds
    // either way, which is the correct behaviour to pin.
    const seen: RequestLogEntry[] = [];
    const listener = (e: RequestLogEntry): void => void seen.push(e);
    bus.onTyped("log.entry", listener);

    const detach = wireLogEventsToBus();
    try {
      const e = entry({ id: "singleton-path" });
      logEmitter.emit("request", e);
      expect(seen.map((x) => x.id)).toContain("singleton-path");
    } finally {
      detach();
      bus.offTyped("log.entry", listener);
    }
  });
});
