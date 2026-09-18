/**
 * The subscription multiplexer's contract — P5 work item 6.
 *
 * Each case here corresponds to one of the four properties the module's header lists, and the
 * first one is the reason the module exists at all: **two listeners must produce one transport
 * subscription**. That is asserted against the transport itself (a counting fake) rather than
 * inferred from delivery counts, because a hub that opened two subscriptions would still deliver
 * each event twice and a delivery-count assertion would pass for the wrong reason.
 */

import { describe, expect, it, vi } from "vitest";
import type { Transport, EngineEvent } from "@bifurc/engine/transport/types";
import { createSubscriptionHub } from "../src/subscriptions";

/** A transport that records its subscriptions and lets a test deliver events by hand. */
function makeFakeTransport() {
  const subs = new Map<number, { events: string[]; cb: (e: EngineEvent) => void }>();
  let nextId = 1;

  const transport = {
    kind: "in-process",
    request: vi.fn(async () => ({})),
    subscribe(events: string[], cb: (e: EngineEvent) => void) {
      const id = nextId++;
      subs.set(id, { events, cb });
      let closed = false;
      return () => {
        if (closed) throw new Error(`transport unsubscribe called twice for subscription ${id}`);
        closed = true;
        subs.delete(id);
      };
    },
    close: vi.fn(async () => {}),
  } as unknown as Transport;

  return {
    transport,
    /** How many subscriptions the hub actually opened on the transport. */
    openCount: () => subs.size,
    /** Deliver an event to every open subscription that asked for it. */
    emit(event: string, payload: unknown) {
      for (const { events, cb } of [...subs.values()]) {
        if (events.includes(event)) cb({ event: event as EngineEvent["event"], seq: 1, payload });
      }
    },
  };
}

describe("createSubscriptionHub", () => {
  it("opens exactly one transport subscription for two listeners on the same event", () => {
    const fake = makeFakeTransport();
    const hub = createSubscriptionHub(fake.transport);

    const a = vi.fn();
    const b = vi.fn();
    hub.subscribe("event.log.entry", a);
    hub.subscribe("event.log.entry", b);

    expect(hub.transportSubscriptionCount()).toBe(1);
    expect(fake.openCount()).toBe(1);

    fake.emit("event.log.entry", { message: "hi" });
    expect(a).toHaveBeenCalledTimes(1);
    expect(b).toHaveBeenCalledTimes(1);
    // Both listeners see the *payload*, not the envelope — `window.api.onLogEntry` takes an entry.
    expect(a).toHaveBeenCalledWith({ message: "hi" });
  });

  it("opens one subscription per distinct event", () => {
    const fake = makeFakeTransport();
    const hub = createSubscriptionHub(fake.transport);

    hub.subscribe("event.log.entry", vi.fn());
    hub.subscribe("event.log.chunk", vi.fn());

    expect(hub.transportSubscriptionCount()).toBe(2);
    expect(fake.openCount()).toBe(2);
  });

  it("keeps the shared subscription alive while one of two listeners remains", () => {
    const fake = makeFakeTransport();
    const hub = createSubscriptionHub(fake.transport);

    const a = vi.fn();
    const b = vi.fn();
    const offA = hub.subscribe("event.log.entry", a);
    hub.subscribe("event.log.entry", b);

    offA();

    // The survivor must still be subscribed — this is the case a naive ref-count gets wrong.
    expect(hub.transportSubscriptionCount()).toBe(1);
    expect(fake.openCount()).toBe(1);

    fake.emit("event.log.entry", 1);
    expect(a).not.toHaveBeenCalled();
    expect(b).toHaveBeenCalledTimes(1);
  });

  it("tears the transport subscription down when the last listener leaves", () => {
    const fake = makeFakeTransport();
    const hub = createSubscriptionHub(fake.transport);

    const offA = hub.subscribe("event.log.entry", vi.fn());
    const offB = hub.subscribe("event.log.entry", vi.fn());

    offA();
    expect(fake.openCount()).toBe(1);

    offB();
    expect(hub.transportSubscriptionCount()).toBe(0);
    expect(fake.openCount()).toBe(0);
  });

  it("returns an idempotent unsubscribe, because React StrictMode calls cleanup twice", () => {
    const fake = makeFakeTransport();
    const hub = createSubscriptionHub(fake.transport);

    const off = hub.subscribe("event.log.entry", vi.fn());

    // The fake throws if the transport's own unsubscribe is called twice, so this passing is
    // the assertion: the hub must not forward the second call.
    expect(() => {
      off();
      off();
    }).not.toThrow();

    expect(hub.transportSubscriptionCount()).toBe(0);
  });

  it("returns a function from every subscribe, since renderer effects call it from cleanup", () => {
    const fake = makeFakeTransport();
    const hub = createSubscriptionHub(fake.transport);

    const off = hub.subscribe("event.log.entry", vi.fn());
    expect(typeof off).toBe("function");
  });

  it("keeps delivering to the other listeners when one listener throws", () => {
    const fake = makeFakeTransport();
    const hub = createSubscriptionHub(fake.transport);

    const bad = vi.fn(() => {
      throw new Error("panel blew up");
    });
    const good = vi.fn();

    hub.subscribe("event.log.entry", bad);
    hub.subscribe("event.log.entry", good);

    expect(() => fake.emit("event.log.entry", 1)).not.toThrow();
    expect(bad).toHaveBeenCalledTimes(1);
    expect(good).toHaveBeenCalledTimes(1);
  });

  it("still delivers to a listener that unsubscribes another listener mid-dispatch", () => {
    const fake = makeFakeTransport();
    const hub = createSubscriptionHub(fake.transport);

    const second = vi.fn();
    let offSecond: () => void = () => {};

    const first = vi.fn(() => {
      offSecond();
    });

    hub.subscribe("event.log.entry", first);
    offSecond = hub.subscribe("event.log.entry", second);

    fake.emit("event.log.entry", 1);

    // `first` ran and removed `second` before `second`'s turn. The dispatch set is copied, so
    // `second` is still called this round; what must not happen is the loop skipping it.
    expect(first).toHaveBeenCalledTimes(1);
    expect(second).toHaveBeenCalledTimes(1);
    expect(hub.transportSubscriptionCount()).toBe(1); // `first` is still there
  });

  it("close() tears everything down and is idempotent", () => {
    const fake = makeFakeTransport();
    const hub = createSubscriptionHub(fake.transport);

    hub.subscribe("event.log.entry", vi.fn());
    hub.subscribe("event.log.chunk", vi.fn());
    expect(fake.openCount()).toBe(2);

    hub.close();
    expect(hub.transportSubscriptionCount()).toBe(0);
    expect(fake.openCount()).toBe(0);

    expect(() => hub.close()).not.toThrow();
  });
});
