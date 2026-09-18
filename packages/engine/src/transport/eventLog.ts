/**
 * P4 work item 3 — the engine's event log: one `seq` authority and one bounded ring buffer.
 *
 * ## Why the log is engine-scoped, not session-scoped
 *
 * `plan/05` says "the engine keeps a bounded ring buffer of recent events **per session**", and its
 * own `hello` sketch contradicts that:
 *
 * ```ts
 * // client reconnects
 * hello { protocolVersion, lastSeq: 48210 }
 * ```
 *
 * A reconnect is a **new session** — the old transport is gone. So if `seq` were counted per session,
 * the `lastSeq` a client presents would be a number from a session that no longer exists, and the same
 * number would mean different events in the new one. There is no session id in the sketch to
 * disambiguate it. Replay across a reconnect is therefore only definable if `seq` is **engine-scoped**,
 * which is what this module implements. `types.ts`'s older "per-session, gap-free" note is corrected
 * accordingly.
 *
 * The cost, stated rather than hidden: a session subscribed to only *some* events now sees `seq` values
 * with gaps, because events it did not subscribe to still consume a number. "Gap-free" is lost, and
 * that is a real trade — a gap can no longer be read as "events were lost". It buys the only thing that
 * makes replay work, and `plan/05` calls replay the most likely source of "works locally, broken
 * remotely" bugs.
 *
 * ## Retention outlives its subscribers, deliberately
 *
 * A bus listener for an event is attached the **first time** anything subscribes to that event and is
 * held until `close()`. It is *not* released when the last subscriber leaves, and that is the whole
 * point: during a network blip nobody is subscribed, and a buffer that only fills while someone is
 * listening would replay an incomplete history — silently, which is the failure mode the plan warns
 * about. Retention starts when an event becomes interesting and continues for the engine's life.
 *
 * The consequence to know about: an `EventLog` holds bus listeners, so `bus.listenerCount(...)` is no
 * longer a proxy for "does this transport leak?". The conformance suite asserts on
 * `subscriberCount()` instead — the same property, measured at the layer that now owns it.
 *
 * ## `seq` is assigned on receipt, before any filtering
 *
 * `seq` increments once per retained event, whether or not anyone is subscribed and whether or not the
 * subscriber wants that name. Assigning it after filtering would make it per-subscriber, which is the
 * design that cannot survive a reconnect.
 */
import type { EventName } from "@bifurc/protocol";
import type { EngineEvents } from "../eventBus";
import { EngineEventBus } from "../eventBus";
import { BUS_NAME_BY_WIRE_NAME, type EngineEvent } from "./types";

/** One retained event. `at` is on the injected clock, so the age bound is testable without sleeping. */
export interface LoggedEvent {
  readonly seq: number;
  /** The **wire** name, matching `EngineEvent`. */
  readonly event: EventName;
  readonly payload: unknown;
  readonly at: number;
}

export interface EventLogOptions {
  bus: EngineEventBus;
  /**
   * Retained events. `plan/05`: "e.g. 1,000 events or 5 minutes". The count bound is the primary one —
   * it is what makes memory use predictable regardless of event rate.
   */
  maxEvents?: number;
  /** Retained age. The secondary bound: a quiet engine should not hold an hour of history. */
  maxAgeMs?: number;
  /** Injectable clock. The age bound is a time behaviour, and a test that sleeps is a flaky test. */
  now?: () => number;
}

/** What the engine can offer a client that presented a `lastSeq`. */
export type ReplayOutcome =
  | { readonly kind: "replay"; readonly from: number; readonly events: readonly LoggedEvent[] }
  | { readonly kind: "resync"; readonly from: number; readonly reason: string };

export const DEFAULT_MAX_EVENTS = 1_000;
export const DEFAULT_MAX_AGE_MS = 5 * 60 * 1_000;

export class EventLog {
  private readonly bus: EngineEventBus;
  private readonly maxEvents: number;
  private readonly maxAgeMs: number;
  private readonly now: () => number;

  /** The ring. Oldest first, so `seq` is monotonically increasing along the array. */
  private buffer: LoggedEvent[] = [];
  private seq = 0;
  private closed = false;

  /**
   * Bus listeners held for retention, keyed by **wire** name. Membership means "attached"; it is never
   * removed except by `close()`, which is the retention guarantee described in the header.
   */
  private readonly retention = new Map<string, (payload: unknown) => void>();

  /** Live subscriptions, per wire name. Tokens, so two identical subscriptions both fire. */
  private readonly subscribers = new Map<string, Set<(e: EngineEvent) => void>>();

  constructor(opts: EventLogOptions) {
    this.bus = opts.bus;
    this.maxEvents = opts.maxEvents ?? DEFAULT_MAX_EVENTS;
    this.maxAgeMs = opts.maxAgeMs ?? DEFAULT_MAX_AGE_MS;
    this.now = opts.now ?? (() => Date.now());
  }

  /** The highest `seq` assigned. `0` before the first event, which is also a valid `lastSeq`. */
  get newestSeq(): number {
    return this.seq;
  }

  /** The lowest `seq` still retained, or `undefined` when nothing is. */
  get oldestSeq(): number | undefined {
    this.prune();
    return this.buffer[0]?.seq;
  }

  get size(): number {
    this.prune();
    return this.buffer.length;
  }

  /** How many live subscriptions a wire name has. The suite's leak check, at the right layer. */
  subscriberCount(wire: string): number {
    return this.subscribers.get(wire)?.size ?? 0;
  }

  /** Wire names currently retained, i.e. those with a bus listener held. */
  retainedNames(): string[] {
    return [...this.retention.keys()];
  }

  /**
   * Subscribe to wire names.
   *
   * Same all-or-nothing rule as every other `subscribe()` in this layer: **every** name is resolved
   * before **any** retention is attached, so a typo in the third name cannot leave the first two
   * buffering events for a caller that then throws and retries.
   */
  subscribe(events: string[], cb: (e: EngineEvent) => void): () => void {
    if (this.closed) {
      throw new Error("Cannot subscribe: this event log has been closed.");
    }
    if (events.length === 0) return () => {};

    for (const wire of events) {
      if (!BUS_NAME_BY_WIRE_NAME.has(wire)) {
        throw new Error(
          `This event log cannot deliver "${wire}". It is either not a known event or not yet on ` +
            `the wire; see BUS_EVENTS_NOT_ON_THE_WIRE in packages/engine/src/transport/types.ts.`,
        );
      }
    }

    const added: string[] = [];
    for (const wire of events) {
      this.retain(wire);
      let set = this.subscribers.get(wire);
      if (set === undefined) {
        set = new Set();
        this.subscribers.set(wire, set);
      }
      // A duplicate name in one call is one subscription, not two: `subscribe(["a", "a"])` should not
      // double-deliver, and a Set of callbacks would not catch it because the callback is the same
      // function either way.
      if (!set.has(cb)) {
        set.add(cb);
        added.push(wire);
      }
    }

    let detached = false;
    return () => {
      if (detached) return;
      detached = true;
      for (const wire of added) this.subscribers.get(wire)?.delete(cb);
    };
  }

  /**
   * Can the engine serve a client that last saw `lastSeq`, and with which events?
   *
   * `names`, when given, filters the returned events — the transport passes the names the client is
   * subscribing to, so it does not replay events the client never asked for. Omit it at handshake time
   * to ask only *whether* replay is possible, which is all `replayedFrom`/`resyncRequired` need.
   */
  replayFrom(lastSeq: number, names?: readonly string[]): ReplayOutcome {
    this.prune();
    const oldest = this.buffer[0]?.seq;

    // Nothing retained. `newest` is 0, so only a client that saw nothing (`lastSeq === 0`) is in
    // range; anything higher is claiming to have seen events this engine has no record of.
    if (oldest === undefined) {
      return lastSeq === this.seq
        ? { kind: "replay", from: lastSeq, events: [] }
        : {
            kind: "resync",
            from: lastSeq,
            reason: `no events are retained, so a client at seq ${lastSeq} cannot be caught up`,
          };
    }

    if (lastSeq < oldest - 1) {
      return {
        kind: "resync",
        from: lastSeq,
        reason:
          `seq ${lastSeq} is older than the oldest retained event (${oldest}); the events in ` +
          `between have been dropped and cannot be replayed`,
      };
    }

    if (lastSeq > this.seq) {
      // The client claims to have seen events the engine never sent. Replaying nothing would leave it
      // believing it is up to date — a silent staleness bug — so this is a resync, not an empty replay.
      return {
        kind: "resync",
        from: lastSeq,
        reason: `seq ${lastSeq} is ahead of this engine's newest (${this.seq})`,
      };
    }

    const wanted = names === undefined ? undefined : new Set(names);
    return {
      kind: "replay",
      from: lastSeq,
      events: this.buffer.filter((e) => e.seq > lastSeq && (wanted === undefined || wanted.has(e.event))),
    };
  }

  /**
   * Detach every retention listener. The log is then dead: `subscribe()` throws and the buffer stops
   * growing. Owned by whoever created the log — a transport that was *given* a log must not close it,
   * because the log is shared.
   */
  close(): void {
    if (this.closed) return;
    this.closed = true;
    for (const [wire, listener] of this.retention) {
      const busName = BUS_NAME_BY_WIRE_NAME.get(wire);
      if (busName !== undefined) {
        this.bus.offTyped(busName as keyof EngineEvents, listener as never);
      }
    }
    this.retention.clear();
    this.subscribers.clear();
  }

  /** Attach retention for a wire name, once. */
  private retain(wire: string): void {
    if (this.retention.has(wire)) return;
    const busName = BUS_NAME_BY_WIRE_NAME.get(wire);
    if (busName === undefined) return;

    const listener = (payload: unknown): void => {
      this.record(wire as EventName, payload);
    };
    this.retention.set(wire, listener);
    this.bus.onTyped(busName as keyof EngineEvents, listener as never);
  }

  /** Assign the seq, retain, and fan out. The single write path into the buffer. */
  private record(event: EventName, payload: unknown): void {
    if (this.closed) return;
    this.seq += 1;
    this.buffer.push({ seq: this.seq, event, payload, at: this.now() });
    this.prune();

    const set = this.subscribers.get(event);
    if (set === undefined) return;
    // Snapshot: a callback may unsubscribe (or subscribe) while we iterate, and mutating the Set
    // mid-iteration is how a delivery gets skipped or repeated.
    for (const cb of [...set]) {
      cb({ event, seq: this.seq, payload });
    }
  }

  /** Enforce both bounds. Called on write and on read, because the age bound moves with the clock. */
  private prune(): void {
    if (this.maxAgeMs > 0 && this.buffer.length > 0) {
      const cutoff = this.now() - this.maxAgeMs;
      // `findIndex` + `splice` rather than a filter: the buffer is ordered by `at` (the clock is
      // monotonic in production), so the expired entries are a prefix.
      const firstLive = this.buffer.findIndex((e) => e.at > cutoff);
      if (firstLive === -1) this.buffer = [];
      else if (firstLive > 0) this.buffer = this.buffer.slice(firstLive);
    }

    if (this.buffer.length > this.maxEvents) {
      this.buffer = this.buffer.slice(this.buffer.length - this.maxEvents);
    }
  }
}
