/**
 * P4 work item 3's second half — the engine-side event pump: coalescing and the drop policy.
 *
 * ## The problem this solves, stated as the plan states it
 *
 * `event.log.entry` fires **per proxied request**. A capture session against a busy site produces
 * thousands of them a minute, and `plan/02`'s decision record says what to do about it:
 *
 * > `log:entry`: **coalesce into batches** (e.g. up to 100 entries or 250 ms, whichever first).
 *
 * `@bifurc/protocol` already carries the shape that decision implies — `LogEntryBatchEventSchema`,
 * `{ entries: LogEntryEvent[] }` — and it has been declared-but-unused since P1. This module is what
 * finally uses it.
 *
 * ## Decision: the batch is what the *client* receives, not an invisible wire trick
 *
 * There were two ways to spend the batch, and the choice is the load-bearing one in this file.
 *
 * **(a) Expand it at the client.** The wire carries `{entries}`, the client session unwraps it and
 * calls each subscriber once per entry. `Transport.subscribe` keeps meaning "one `EngineEvent` per
 * bus event" everywhere, and nothing above the transport ever learns that batching exists.
 *
 * **(b) Deliver it as-is.** One envelope is one `EngineEvent`, and a subscriber to `event.log.entry`
 * receives `payload: { entries: [...] }`. A single entry is a batch of one.
 *
 * This module implements **(b)**, for four reasons, in descending order of force:
 *
 * 1. **Under (a), coalescing is unobservable through the `Transport` interface, and the acceptance
 *    criterion is that it be *verified*.** A client under (a) sees N `EngineEvent`s whether the
 *    engine sent one frame or N — that is the point of (a) — so no case written against `Transport`
 *    can distinguish a coalescing engine from one that forwards every entry. Verifying it would need
 *    a frame counter handed to the suite, i.e. a testing hook in a production contract, which the
 *    conformance suite's own header rules out in the same words it uses for `barrier()`.
 * 2. **`LogEntryBatchEventSchema` is exported from the frozen protocol, and that package's job is to
 *    describe what a client receives in `EventEnvelope.payload`.** Under (a) no client could ever
 *    receive it; it would be an internal wire detail that had no business being frozen as public API.
 *    Its existence is the protocol saying which of (a) and (b) was intended.
 * 3. **One event name must have one payload shape.** Under (a) the shape is single-entry and the
 *    batch is hidden; under a hybrid — batch on the wire, single in `in-process` — a subscriber would
 *    have to handle both, which is strictly worse than either.
 * 4. **The batch is the unit of sequencing.** One envelope carries one `seq`, so the envelope is what
 *    `lastSeq` refers to. See "Why the batch's `seq` is its *last* entry's" below.
 *
 * ## Where the shape is applied, and why that is three call sites rather than one
 *
 * `toClientEvent`/`toLogEntryBatch` are called by `inProcess.ts` (a client-facing transport in its own
 * right), by the auth decorator's replay-backlog path (which synthesises events itself instead of going
 * through the inner transport), and by this module's pump (the wire for `stdio`, `ws` and `socket`).
 *
 * Three sites is more than one, and the reason is architectural rather than sloppy: there is no single
 * chokepoint today, because `ws`/`socket` compose a client-facing transport (`in-process`) *inside*
 * their own server, so their pump is downstream of something that has already applied the shape. The
 * first version of this change applied the shape in two of those places and produced a batch of 100
 * **empty** log rows over `ws` and nothing wrong over `stdio` — a double application, because the pump
 * projected an already-shaped payload. `toLogEntryBatch`'s idempotence is the fix, and it is asserted
 * rather than assumed.
 *
 * The cost, stated rather than hidden: `window.api`'s `onLogEntry` is per-entry and stays that way
 * (it is a frozen P1 surface and the shell's bridge is what expands a batch into per-entry
 * broadcasts), so the expansion that (a) would have done inside the transport happens one layer
 * higher, in the shell. `src/ipc/eventBridge.ts` is where that will land when P5/P6 migrate the shell
 * onto a transport; today the shell is still in-process and unaffected.
 *
 * ## Only `log.entry` coalesces, and that is the whole policy table
 *
 * `plan/05`'s table, implemented exactly:
 *
 * | Event | Policy |
 * |---|---|
 * | `log.entry` | **coalesce** — up to `maxBatch` entries or `batchWindowMs`, whichever is first |
 * | `log.chunk`, `process.output` | stream immediately (their flow-control window is still missing) |
 * | `sync.status`, `entity.changed`, `sync.entityStatus`, `webhook.payload`, `settings.changed` | immediate |
 * | `server.error` | immediate, **never dropped** |
 *
 * `log.chunk` and `process.output` are deliberately **not** batched. The plan's row for them is
 * "stream, with a flow-control window so the engine can pause a chatty child", and the window is a
 * different mechanism (a client→engine credit, which is a protocol addition) that is still not
 * implemented. Batching them instead would be a substitution the plan did not ask for: a chunk is a
 * *stream* whose whole value is arriving as it is produced, so holding it for 250 ms would trade the
 * feature for the frame count.
 *
 * ## FIFO is not negotiable, so an immediate event flushes the batch in hand
 *
 * Coalescing introduces a delay, and a delay in front of a queue is how reordering bugs get in. If a
 * pending batch were left to its timer while `server.error` overtook it, the wire order would no
 * longer be emission order — and the conformance suite's delivery barrier depends on FIFO *being*
 * that order ("a sentinel arriving proves every event emitted before it has already been delivered").
 * So every immediate event **flushes the pending batch first**, then sends itself. The batch's
 * entries were produced earlier, so they go first; the invariant "wire order == emission order" holds
 * with no exceptions to remember.
 *
 * ## Why the batch's `seq` is its *last* entry's
 *
 * The envelope's `seq` is the highest `seq` among the entries it carries, not the lowest, and this is
 * the difference between replay working and replay duplicating.
 *
 * A client's `lastSeq` is the last envelope `seq` it saw, and the engine replays everything with
 * `seq > lastSeq`. If a batch carried its *first* entry's `seq`, the client would resume from a point
 * **before** the rest of its own batch and the engine would replay them again — every entry after the
 * first, once per reconnect. Carrying the last one means the whole batch is behind the resume point
 * the moment it arrives, which is true, because a frame is atomic: the client either got all 100
 * entries or got a broken connection.
 *
 * ## The drop policy needs no drop mechanism, and that is the finding
 *
 * `plan/05` says "under sustained overload, drop `log.entry` before `server.error`". Read as a
 * requirement to write dropping code, that reads as an unbounded-queue problem. It is not, and no
 * drop path is implemented here, for two structural reasons:
 *
 * - **`server.error` is never queued.** It takes the immediate path, which sends it on the spot. A
 *   value that is never held cannot be dropped in favour of something else, so "never dropped" is a
 *   property of the code's shape rather than of a priority comparison that has to be got right.
 * - **The queue cannot grow without bound.** A batch is flushed the moment it reaches `maxBatch`, and
 *   `send` is synchronous, so the pending queue is capped at `maxBatch` entries *by construction*.
 *   There is no sustained-overload window in which a backlog accumulates to be triaged — the batch
 *   leaves as soon as it is full.
 *
 * `tests/transport/eventPump.test.ts` asserts the boundedness rather than trusting this paragraph: it
 * pushes far more than `maxBatch` entries and checks that no batch ever exceeds the cap. If a future
 * change makes `send` asynchronous or removes the cap-triggered flush, that test fails and the drop
 * policy becomes a real question again — which is the point of asserting it.
 *
 * One honest limitation: because nothing is ever dropped, a *dropped* entry has no reporter. If the
 * boundedness argument above is ever broken and a bound is added, dropping entries would be silent
 * data loss on the log stream, and telling the client about it needs a protocol addition that does not
 * exist today (`ResyncRequiredEvent` is about replay, not about loss). That is a P9/P12 question, and
 * it is recorded here rather than left to be discovered.
 */
import type { EventEnvelope, LogEntryBatchEvent, LogEntryEvent } from "@bifurc/protocol";
import type { RequestLogEntry } from "../proxy/logEmitter";
import type { EngineEvent } from "./types";

/**
 * The one wire name that is coalesced. Named rather than inlined because the comparison in `push()`
 * is the whole policy table's hinge, and because a second coalesced event would have to be a
 * deliberate edit here rather than a literal that happened to match.
 */
export const COALESCED_EVENT = "event.log.entry";

/** `plan/02`'s "up to 100 entries", and `events.ts`'s "batches of up to 100". */
export const DEFAULT_MAX_BATCH = 100;

/** `plan/02`'s "or 250 ms, whichever first". */
export const DEFAULT_BATCH_WINDOW_MS = 250;

export interface EventPumpOptions {
  /**
   * Write one wire envelope. Called with a fully-formed `{event, seq, payload}`.
   *
   * **Synchronous**, and the pump depends on that: the cap-triggered flush is what bounds the queue
   * (see the header's drop-policy section), so a `send` that deferred its work would reintroduce the
   * unbounded backlog the bound is there to rule out.
   */
  send: (envelope: EventEnvelope) => void;
  /** Batch cap. Defaults to `DEFAULT_MAX_BATCH`. */
  maxBatch?: number;
  /** Batch window in milliseconds. Defaults to `DEFAULT_BATCH_WINDOW_MS`. */
  batchWindowMs?: number;
  /**
   * Start a timer, returning a canceller. Injectable for the same reason `EventLogOptions.now` is:
   * the window is a *time* behaviour, and a test that sleeps 250 ms is a slow test that is also a
   * flaky one. The unit suite passes a manual scheduler and drives the window by hand.
   */
  schedule?: (fn: () => void, ms: number) => () => void;
}

export interface EventPump {
  /** Offer one event. Coalesced events are held; everything else is sent on the spot. */
  push(event: EngineEvent): void;
  /**
   * Deliver the pending batch **best-effort**, then stop. Idempotent, and never throws.
   *
   * Flushing rather than discarding, because discarding is only recoverable on some transports. A
   * `ws`/`socket` client can reconnect and present a `lastSeq`, and the engine's `EventLog` still holds
   * the entries — so dropping them there costs a round trip and nothing else. `stdio`'s server has **no
   * `EventLog`** (a pipe has exactly one peer and a session-less `seq` counter), so an entry dropped at
   * close is gone permanently. One rule that is safe on all three beats a rule that is only safe where
   * a log happens to exist.
   *
   * The cost is that a caller whose channel is already dead must not expect the write to land. It
   * cannot be helped — and it is why this is called *best-effort*: `send` is the caller's, and both
   * `stdio`'s and `ws`'s implementations already swallow a write failure rather than letting it escape
   * into a teardown path. A `send` that throws anyway is caught here, because `close()` runs where
   * there is nobody left to report a failure to.
   */
  close(): void;
}

/** `setTimeout`, deliberately **not** `unref`'d — see the note on the returned canceller below. */
function defaultSchedule(fn: () => void, ms: number): () => void {
  /**
   * Not `unref()`'d, unlike the blob store's TTL sweeper, and the difference is what the timer means.
   * The sweeper is housekeeping the engine would rather not be kept alive for; this timer is a
   * **promise already made** to a client — 100 log entries have been accepted and not yet written.
   * An `unref`'d timer lets the process exit with that batch still pending, which is silent loss of
   * exactly the kind `plan/05` is built to prevent. The cost of keeping it is bounded and tiny: the
   * handle exists only while a batch is pending, so it delays a process exit by at most
   * `batchWindowMs`, and `close()` cancels it outright.
   */
  const timer = setTimeout(fn, ms);
  return () => clearTimeout(timer);
}

/**
 * Project an engine `log.entry` payload onto the protocol's wire shape.
 *
 * ## Why a projection exists at all, and why it is part of this work rather than a tidy-up
 *
 * The bus emits `RequestLogEntry`, which is the engine's **internal** shape: it carries the capture
 * bodies (`reqBody`, `resBody` — base64, the response capped at 512 KB), plus `host`, `via` and
 * `target`. `@bifurc/protocol`'s `LogEntryEventSchema` is a much smaller thing — `{id, timestamp,
 * method, url, status, durationMs}` — and it renames the timestamp (`ts` on the bus, `timestamp` on
 * the wire). Both schemas have been declared-but-unused since P1, waiting for exactly this.
 *
 * Coalescing is what makes the projection **load-bearing rather than cosmetic**. One entry with a
 * 512 KB response body is a large frame; a batch of 100 of them is up to ~50 MB in a single frame,
 * which is not a log stream, it is an outage. So batching is only viable because the protocol already
 * decided the wire carries a light entry, and a pump that batched the internal shape would be
 * shipping the defect rather than the feature.
 *
 * The type import is `RequestLogEntry` on purpose: it makes this projection compile-checked against
 * the bus's real shape, so renaming `ts` there fails the build here instead of silently emitting
 * `timestamp: 0` to every client. It is type-only, so nothing about the proxy module is loaded.
 *
 * The fallbacks are defensive rather than expected — the bus is typed, and a payload that reaches here
 * with the wrong shape means something upstream bypassed that type. Producing a zeroed entry keeps the
 * client's log list intact and one row meaningless, which is better than a frame the peer rejects.
 */
export function toLogEntryEvent(payload: unknown): LogEntryEvent {
  // A non-object payload (`undefined`, a string) would throw on the first property read, and this runs
  // inside a bus listener — where a throw unwinds into machinery that is not expecting it. So the
  // shape is checked rather than asserted.
  const e = (typeof payload === "object" && payload !== null ? payload : {}) as Partial<RequestLogEntry>;
  return {
    id: typeof e.id === "string" ? e.id : "",
    timestamp: typeof e.ts === "number" ? e.ts : 0,
    method: typeof e.method === "string" ? e.method : "",
    url: typeof e.url === "string" ? e.url : "",
    status: typeof e.status === "number" ? e.status : null,
    durationMs: typeof e.durationMs === "number" ? e.durationMs : null,
  };
}

/**
 * Is this payload already the protocol's batch shape?
 *
 * Unambiguous, and worth saying why: `RequestLogEntry` — the engine's own payload for this event — has
 * no `entries` field, so an array under that key can only be a `LogEntryBatchEvent`. A future bus field
 * named `entries` would make this ambiguous, which is the reason the check is written down here rather
 * than left implicit in a truthiness test.
 */
function isLogEntryBatch(payload: unknown): payload is LogEntryBatchEvent {
  return (
    typeof payload === "object" &&
    payload !== null &&
    Array.isArray((payload as { entries?: unknown }).entries)
  );
}

/**
 * The wire entries for one `log.entry` payload. **Idempotent** — a batch is a batch, and a raw entry is
 * a batch of one.
 *
 * ## Why a normaliser rather than a projection, and why the two input shapes are both legitimate
 *
 * Two shapes genuinely arrive here, and neither is an accident:
 *
 *  - **A raw `RequestLogEntry`** — the bus's payload. This is what `stdio`'s server sees, because it
 *    subscribes to the bus directly, and what the auth decorator's replay backlog synthesises from a
 *    `LoggedEvent`.
 *  - **An already-shaped `LogEntryBatchEvent`** — what `ws` and `socket` see. Their event stream comes
 *    from the client-facing transport underneath them (`in-process`, wrapped in the auth decorator),
 *    and *that* is a transport in its own right, so it applies the shape. `ws`/`socket` sit downstream
 *    of it and must not shape a second time.
 *
 * So this function has to recognise its own output, and making it idempotent is what lets one pump
 * serve both sources **without a flag** saying which kind of input it is being fed. A flag would be the
 * "helper you must call in the right order" smell this layer keeps running into: a future caller sets
 * it wrong, and the symptom is a log stream of empty rows.
 *
 * Idempotence is asserted in `tests/transport/eventPump.test.ts`, because it is load-bearing rather
 * than incidental.
 */
export function toLogEntryBatch(payload: unknown): LogEntryBatchEvent {
  if (isLogEntryBatch(payload)) return payload;
  return { entries: [toLogEntryEvent(payload)] };
}

/**
 * An engine event **as a client receives it**.
 *
 * The single place the client-facing payload shape is applied, and it is applied at every boundary
 * where an event becomes client-visible: `inProcess.ts`'s subscription, the auth decorator's replay
 * backlog (which synthesises events itself rather than going through the inner transport, so it would
 * otherwise be the one path delivering a different shape from the live one), and the pump for the three
 * transports with a wire.
 *
 * That is three call sites, which is more than one, and the honest reason is architectural: there is no
 * single chokepoint today because `ws`/`socket` compose a client-facing transport (`in-process`)
 * *inside* their own server. `toLogEntryBatch`'s idempotence is what makes the repetition safe rather
 * than a double-application bug — which is exactly the bug the first version of this change had, and it
 * surfaced as a batch of 100 empty log rows over `ws` and nothing at all over `stdio`.
 */
export function toClientEvent(event: EngineEvent): EngineEvent {
  if (event.event !== COALESCED_EVENT) return event;
  return { ...event, payload: toLogEntryBatch(event.payload) };
}

/**
 * Create the per-session pump.
 *
 * **Per session, not per engine**, unlike `EventLog`. The log is shared because `seq` has to mean one
 * thing across sessions; a pump is not, because its queue holds frames destined for *one* channel —
 * batching two clients' entries together would send each of them the other's traffic.
 */
export function createEventPump(opts: EventPumpOptions): EventPump {
  const maxBatch = opts.maxBatch ?? DEFAULT_MAX_BATCH;
  const batchWindowMs = opts.batchWindowMs ?? DEFAULT_BATCH_WINDOW_MS;
  const schedule = opts.schedule ?? defaultSchedule;

  /** Entries held for the next batch, oldest first. Bounded by `maxBatch` — see the header. */
  let pending: LogEntryEvent[] = [];
  /** The highest `seq` among `pending`. Becomes the batch envelope's `seq` — see the header. */
  let pendingSeq = 0;
  /** Cancel for the window timer, or `undefined` when no timer is armed. */
  let cancelTimer: (() => void) | undefined;
  let closed = false;

  function cancel(): void {
    if (cancelTimer === undefined) return;
    const cancelFn = cancelTimer;
    // Cleared *before* the call, so a canceller that re-enters (or a timer that has already fired)
    // cannot leave a stale handle behind that a later `cancel()` would try to cancel twice.
    cancelTimer = undefined;
    cancelFn();
  }

  /** Write the pending batch, if any, and disarm the window. */
  function flush(): void {
    cancel();
    if (pending.length === 0) return;
    const entries = pending;
    const seq = pendingSeq;
    pending = [];
    pendingSeq = 0;
    opts.send({ event: COALESCED_EVENT, seq, payload: { entries } });
  }

  return {
    push(event: EngineEvent): void {
      if (closed) return;

      if (event.event !== COALESCED_EVENT) {
        // Immediate. The batch in hand was produced *earlier*, so it is written first — this is the
        // FIFO guarantee the header describes, and it is why `server.error` cannot overtake it.
        flush();
        opts.send({ event: event.event, seq: event.seq, payload: event.payload });
        return;
      }

      // Normalised on the way in, not on the way out: the batch holds wire entries, so the memory it
      // holds is the wire's, which is the whole reason a batch of 100 is affordable — see
      // `toLogEntryEvent`. The spread is at most one entry per push in every configuration that exists
      // today (a raw payload, or a batch of one from the client-facing transport underneath), so the
      // `maxBatch` check below still bounds the queue exactly — the boundedness the drop policy rests
      // on. A source that could hand over a whole batch at once would need the cap re-checked inside
      // the loop rather than after it.
      pending.push(...toLogEntryBatch(event.payload).entries);
      // `Math.max` rather than assignment: the sources are monotonic today (the `EventLog`'s counter,
      // and `stdio`'s local one), but "the highest seq in this batch" is what replay correctness
      // depends on, so it is computed from the batch rather than assumed from arrival order.
      pendingSeq = Math.max(pendingSeq, event.seq);

      if (pending.length >= maxBatch) {
        flush();
        return;
      }
      // Armed once per batch, on the first entry — not refreshed per entry, which is the difference
      // between "250 ms since the batch opened" and "250 ms since the last entry". The latter starves
      // under a steady stream that never quite reaches the cap, which is precisely the load this
      // exists for.
      if (cancelTimer === undefined) {
        cancelTimer = schedule(() => {
          // Disarmed first: the timer has fired, so there is nothing left to cancel, and leaving the
          // handle set would make the next `cancel()` call a canceller twice.
          cancelTimer = undefined;
          flush();
        }, batchWindowMs);
      }
    },

    close(): void {
      if (closed) return;
      closed = true;
      try {
        flush();
      } catch {
        // A dead channel, or a caller's `send` that threw. Nothing to report and nowhere to report it.
      }
      pending = [];
      pendingSeq = 0;
    },
  };
}
