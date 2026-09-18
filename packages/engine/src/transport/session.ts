/**
 * The **client** half of a serialising transport — shared by `stdio`, `ws` and `socket`.
 *
 * ## Why this file exists
 *
 * `stdio.ts` and `ws.ts` each grew their own copy of the same state machine: a map of pending
 * requests keyed by the `id` we generated, a set of control-frame ids tracked only so a response to
 * one is *recognised*, a map of wire name → subscriber **tokens**, one `teardown()` that snapshots and
 * rejects everything in flight, and the envelope rule applied on the way back. Measured, that is
 * ~150 lines each and ~90% identical; every difference is at the **channel** boundary — how a frame is
 * serialised, how it is written, and how the peer's death is noticed.
 *
 * `plan/05`'s risks table lists "conformance suite written per-transport instead of shared" as a risk,
 * and the same argument applies one layer down. A **third** copy is where "similar" becomes "drifted",
 * and the two existing copies already do disagree — about `seq` (see `deliver()` below). That
 * disagreement was invisible until it was looked for, which is the general shape of this bug class.
 *
 * ## What a channel must supply, and what it gets for free
 *
 * A channel is the six things that are genuinely per-transport. Everything else — correlation, the
 * all-or-nothing subscription rule, refcounting by name, the envelope rule, close semantics, and the
 * rule that a request in flight when the peer dies must **reject rather than hang** — is here.
 *
 * ## Two rules a channel must not break
 *
 * 1. **`write()` must not throw for a dead channel** in a way that escapes. A write failure is a dead
 *    peer, not a caller error, so it is reported through `fail()` — a throw from inside a bus listener
 *    or a stream's `data` handler unwinds into machinery that is not expecting it, and the resulting
 *    unhandled `error` event on a socket nobody is watching is far harder to diagnose than a clean
 *    teardown.
 * 2. **`ensureReady()` must be idempotent and safe to call twice concurrently.** `subscribe()` calls
 *    it fire-and-forget because `subscribe()` is synchronous, and `request()` awaits it; both may
 *    happen on the same tick.
 */
import { randomUUID } from "node:crypto";
import { EngineError } from "@bifurc/protocol";
import {
  BUS_NAME_BY_WIRE_NAME,
  remoteErrorToEngineError,
  type EngineEvent,
  type Transport,
  type TransportKind,
} from "./types";

/**
 * The per-transport half of a client session.
 *
 * Deliberately six members and no more. If a seventh is needed, the question to ask first is whether
 * the behaviour belongs in the shared session instead — that is the whole point of this file.
 */
export interface ClientChannel {
  /**
   * Serialise one frame to **wire bytes**, complete with whatever framing this channel needs.
   *
   * Bytes rather than a string because the two framings differ: `stdio` and `socket` prepend a
   * length prefix (`framing.ts`), while `ws` sends a bare message because the WebSocket protocol
   * already frames it. A `string` return would force one of them to re-encode, and `encodeFrame()`
   * `JSON.stringify`s its argument — so handing it an already-serialised string would double-encode it
   * into `"{\"id\":…}"` and the peer would reject it as `not_an_object`.
   *
   * Throws for a payload the wire cannot represent. That is a **caller** error and must not be
   * confused with a write failure — see `sendFrame()`.
   */
  encode(frame: unknown): Buffer;
  /** Write one already-encoded frame. Only ever called when `isReady()` is true. */
  write(frame: Buffer): void;
  /** Can a frame be written right now? When false, the session queues it until `opened()`. */
  isReady(): boolean;
  /** Bring the channel up, resolving when it can be written to. Idempotent. */
  ensureReady(): Promise<void>;
  /**
   * Release the channel. Called exactly once, from **any** teardown — fatal or caller-initiated — so
   * this is where listeners are detached and a socket is destroyed. Must not throw: it runs inside
   * the teardown path, where there is nobody left to report a failure to.
   */
  dispose(): void;
  /**
   * Cleanup that applies **only** to a graceful `close()`.
   *
   * `stdio` is the reason this exists: ending a spawned engine's stdin is how it learns there will be
   * no more requests, but that must not happen on a *fatal* teardown — and ending a pipe this
   * transport merely attached to would break whatever else is using it. `ws` and `socket` have no
   * such distinction and omit it.
   */
  onClose?(): void;
  /** Notified when the session dies for a reason the caller did not ask for. */
  onFatal?(error: Error): void;
}

export interface ClientSessionOptions {
  kind: TransportKind;
  channel: ClientChannel;
}

export interface ClientSession {
  /** The `Transport` the caller uses. */
  readonly transport: Transport;
  /** A frame arrived from the peer. The channel has already decoded it. */
  receive(frame: unknown): void;
  /** The channel became writable — flush whatever was queued before it came up. */
  opened(): void;
  /** The channel died or ended. `notify: false` for a caller-initiated `close()`. */
  fail(reason: Error, notify: boolean): void;
  readonly closed: boolean;
  readonly fatal: Error | undefined;
}

/** A one-line description of an unknown throwable, for an error message. */
export function describeError(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

export function createClientSession(opts: ClientSessionOptions): ClientSession {
  const channel = opts.channel;

  /** Requests awaiting a response, keyed by the `id` we generated. */
  const pending = new Map<string, { resolve: (v: unknown) => void; reject: (e: Error) => void }>();
  /** `id`s of control frames we sent, tracked only so a response to one is *recognised*. */
  const control = new Set<string>();
  /**
   * Live subscriptions, keyed by wire name.
   *
   * A set of **tokens**, one per `subscribe()` call, not a set of callbacks: subscribing the same
   * function to the same event twice must fire twice, and each unsubscribe must remove only its own.
   * `in-process` behaves that way, and the whole point of the shared conformance suite is that the
   * transports cannot disagree about things like this.
   */
  const listeners = new Map<string, Set<{ cb: (e: EngineEvent) => void }>>();

  /** Frames written before the channel came up. Drained by `opened()`. */
  let outbox: Buffer[] = [];
  let closed = false;
  let fatal: Error | undefined;

  /**
   * The single teardown path.
   *
   * `pending` is snapshotted and cleared **before** anything is rejected, because a rejection handler
   * is arbitrary caller code that could re-enter here and must find nothing to act on. This is also
   * what makes a hung client impossible: a request in flight when the peer dies rejects instead of
   * leaving the caller awaiting a promise nobody will ever settle — the classic serialising-transport
   * bug, and invisible until something crashes.
   */
  function teardown(reason: Error, notify: boolean): void {
    if (closed) return;
    closed = true;
    fatal = reason;

    const waiting = [...pending.values()];
    pending.clear();
    control.clear();
    outbox = [];
    for (const entry of waiting) entry.reject(reason);

    for (const set of listeners.values()) set.clear();
    listeners.clear();

    // Before `onFatal`, so a handler that inspects the channel sees it already released — and so a
    // `close` triggered by `dispose()` cannot re-enter teardown.
    channel.dispose();
    if (notify) channel.onFatal?.(reason);
  }

  function assertOpen(what: string): void {
    if (!closed) return;
    throw fatal
      ? new EngineError("ENGINE_ERROR", `Cannot ${what}: this session ended. ${fatal.message}`)
      : new EngineError("ENGINE_ERROR", `Cannot ${what}: this transport has been closed.`);
  }

  /**
   * Write one already-encoded frame, or queue it if the channel is not up yet.
   *
   * Split from `sendFrame()` deliberately, and the split is load-bearing: `opened()` must replay the
   * outbox **verbatim**. Routing the outbox back through `sendFrame()` would re-encode a `Buffer`, and
   * `JSON.stringify(buffer)` yields `{"type":"Buffer","data":[…]}` — a frame the peer rejects as
   * `not_an_object`. Every subscription queued before the channel opened would be silently lost, which
   * is the "subscription that succeeds and never fires" failure this layer exists to prevent.
   */
  function writeEncoded(encoded: Buffer): void {
    if (closed) return;
    if (!channel.isReady()) {
      outbox.push(encoded);
      return;
    }
    try {
      channel.write(encoded);
    } catch (err) {
      // A write failure is a dead peer, not a caller error. `teardown` rejects everything in flight.
      teardown(new EngineError("ENGINE_ERROR", `Send failed: ${describeError(err)}`), true);
    }
  }

  /**
   * Encode and send one frame.
   *
   * `encode()` is deliberately called **outside** the write's `try`: an unserialisable payload is a
   * *caller* error and must not kill a healthy session, whereas a failed write means the peer is gone.
   * Conflating them would turn "you passed a `BigInt`" into a dropped connection.
   */
  function sendFrame(frame: unknown): void {
    writeEncoded(channel.encode(frame));
  }

  function sendControl(action: string, payload: unknown): void {
    const id = randomUUID();
    control.add(id);
    sendFrame({ id, action, payload });
  }

  function deliver(record: Record<string, unknown>): void {
    const wire = record.event as string;
    const set = listeners.get(wire);
    if (set === undefined || set.size === 0) return;
    const event: EngineEvent = {
      event: wire as EngineEvent["event"],
      /**
       * The engine's own `seq`, never a local recount.
       *
       * `seq` is **engine-scoped** (`eventLog.ts`), so a session subscribed to a subset of events sees
       * gaps — events it did not subscribe to still consume a number. A client-side counter numbers
       * events by *arrival* rather than by *origin*, which hides exactly those gaps, and for a
       * transport with replay it computes a `lastSeq` the engine does not recognise. `stdio`'s client
       * used to do that; it was sound only because `stdio` has no replay, which is a property of the
       * transport rather than of the code, and therefore the wrong thing to depend on.
       */
      seq: typeof record.seq === "number" ? record.seq : 0,
      payload: record.payload,
    };
    // Iterate a copy: a callback may unsubscribe itself, which mutates `set`.
    for (const token of [...set]) token.cb(event);
  }

  function handleResponse(record: Record<string, unknown>): void {
    const id = record.id;
    if (typeof id !== "string") {
      // No `event` and no `id` — nothing to correlate and nothing to deliver. The peer is broken in a
      // way we cannot report to anyone, so the session ends rather than being left half-usable.
      teardown(
        new EngineError("ENGINE_ERROR", "Received a frame that is neither a response nor an event."),
        true,
      );
      return;
    }

    if (control.delete(id)) {
      /**
       * A control acknowledgement. `ok:true` needs nothing further; `ok:false` means the peer refused
       * a subscription the client had already validated against the same frozen `EVENT_NAMES` list —
       * so either the two ends disagree about the protocol, or the engine is withholding something
       * (no handshake yet, or a replay window that moved).
       *
       * All of those are surfaced through `onFatal` rather than swallowed, because the caller has no
       * other channel: `subscribe()` is synchronous and returns only an unsubscribe function, so there
       * is nowhere to report a late refusal. A caller that believed it was subscribed and is not would
       * wait forever for events that never arrive — precisely the "silently never fires" defect the
       * transport contract forbids. Fatal is also consistent with the documented recovery for the
       * `CONFLICT` case: reconnect without `lastSeq` and re-fetch.
       */
      if (record.ok === false) {
        teardown(
          new EngineError(
            "ENGINE_ERROR",
            `The engine refused a transport control frame: ` +
              `${remoteErrorToEngineError(record.error).message}`,
          ),
          true,
        );
      }
      return;
    }

    const entry = pending.get(id);
    if (entry === undefined) {
      // A response to an id we are not waiting on — a duplicate, or a late answer to a request whose
      // session has moved on. There is no caller to tell, and killing a working session over it would
      // be worse than ignoring it.
      return;
    }
    pending.delete(id);

    if (record.ok === true) {
      // UNINSPECTED — the envelope rule. A handler that *resolved* has succeeded as far as the
      // envelope is concerned, whatever it resolved with, so an inner `{ok:false}` reaches the caller
      // as data rather than as a rejection. Most legacy handlers report failure that way.
      entry.resolve(record.data);
      return;
    }
    if (record.ok === false) {
      entry.reject(remoteErrorToEngineError(record.error));
      return;
    }
    entry.reject(new EngineError("ENGINE_ERROR", "Response frame has no boolean `ok` field."));
  }

  const transport: Transport = {
    kind: opts.kind,

    async request(cmd: string, payload: unknown): Promise<unknown> {
      assertOpen("request()");
      // Awaited, then re-checked: `ensureReady()` is where a lazy connect happens, and a connect can
      // fail — which tears the session down. Without the second check the caller would be handed a
      // promise from a session that is already dead.
      await channel.ensureReady();
      assertOpen("request()");

      const id = randomUUID();
      const promise = new Promise<unknown>((resolve, reject) => {
        pending.set(id, { resolve, reject });
      });

      try {
        sendFrame({ id, action: cmd, payload });
      } catch (err) {
        // The payload could not be framed. Drop the pending entry so it cannot leak, and report the
        // caller's error — the session is still healthy.
        pending.delete(id);
        throw err instanceof EngineError
          ? err
          : new EngineError("BAD_REQUEST", describeError(err));
      }

      return promise;
    },

    subscribe(events: string[], cb: (e: EngineEvent) => void): () => void {
      assertOpen("subscribe()");
      if (events.length === 0) return () => {};

      // All-or-nothing, and synchronously — `subscribe()` is not async, so a bad name has to be caught
      // here rather than awaited. Validating against the protocol's own bridge is what makes the
      // engine's answer predictable: both ends read the same frozen table, so a refusal would mean the
      // engine is out of step with the protocol rather than that the caller was wrong.
      const fresh: string[] = [];
      for (const wire of events) {
        if (!BUS_NAME_BY_WIRE_NAME.has(wire)) {
          throw new EngineError(
            "UNSUPPORTED",
            `This transport cannot deliver "${wire}". It is either not a known event or not yet on ` +
              `the wire; see BUS_EVENTS_NOT_ON_THE_WIRE in packages/engine/src/transport/types.ts.`,
          );
        }
        if (!listeners.has(wire)) {
          listeners.set(wire, new Set());
          fresh.push(wire);
        }
      }

      const token = { cb };
      for (const wire of events) listeners.get(wire)!.add(token);

      // Only ask the peer for names it is not already sending us — refcounted by *name*, so two
      // callers subscribing to `event.log.entry` share one wire subscription.
      if (fresh.length > 0) {
        /**
         * Start connecting if nothing has yet.
         *
         * Without this, a client that only ever subscribes would sit with its control frames in the
         * outbox forever: the channel is opened lazily by `request()`, and `subscribe()` — which
         * cannot await — has no other way to trigger it. The symptom is a subscription that succeeds,
         * never fires, and never errors, which is exactly the failure mode the transport contract
         * forbids and the hardest one to diagnose.
         *
         * The rejection is swallowed here and surfaced through `onFatal` (fired by `teardown` inside
         * the channel's `ensureReady`) and through any pending request. A `subscribe()` caller has
         * nowhere to receive it — the method is synchronous and returns only an unsubscribe function.
         */
        void channel.ensureReady().catch(() => undefined);
        sendControl("subscribe", { events: fresh });
      }

      let detached = false;
      return () => {
        if (detached) return;
        detached = true;
        const dropped: string[] = [];
        for (const wire of events) {
          const set = listeners.get(wire);
          if (set === undefined) continue;
          set.delete(token);
          if (set.size === 0) {
            listeners.delete(wire);
            dropped.push(wire);
          }
        }
        // Nothing to send if the session is already gone, and nothing that *can* be sent.
        if (dropped.length > 0 && !closed) sendControl("unsubscribe", { events: dropped });
      };
    },

    async close(): Promise<void> {
      if (closed) return;
      /**
       * Say goodbye before tearing down.
       *
       * Without this, closing an **attached** session leaves the engine's event-log subscriptions
       * attached with nobody to receive them — and the channel may outlive us by hours. On a long-lived
       * engine that is one leaked listener per closed session, accumulating until Node's 11-listener
       * warning fires and the leak is blamed on whatever subscribed last. Best-effort throughout:
       * `close()` must not throw, because a caller cleaning up in a `finally` block has nothing to do
       * with a failure to say goodbye.
       */
      const live = [...listeners.keys()];
      if (live.length > 0) {
        try {
          sendControl("unsubscribe", { events: live });
        } catch {
          // The channel is already gone; there is nobody left to tell.
        }
      }
      // Not `notify: true` — a caller-initiated close is not a fault, and reporting it as one would
      // make every clean shutdown look like a crash to whatever is watching `onFatal`.
      teardown(new EngineError("ENGINE_ERROR", "This transport has been closed."), false);
      // Only on the graceful path — see `ClientChannel.onClose`.
      try {
        channel.onClose?.();
      } catch {
        // Cleanup must not throw out of `close()`.
      }
    },
  };

  return {
    transport,
    receive(frame: unknown): void {
      if (closed) return;
      if (frame === null || typeof frame !== "object" || Array.isArray(frame)) {
        teardown(
          new EngineError("ENGINE_ERROR", "The engine sent a frame that is not an object."),
          true,
        );
        return;
      }
      const record = frame as Record<string, unknown>;
      if (typeof record.event === "string") deliver(record);
      else handleResponse(record);
    },
    opened(): void {
      const queued = outbox;
      outbox = [];
      for (const frame of queued) writeEncoded(frame);
    },
    fail(reason: Error, notify: boolean): void {
      teardown(reason, notify);
    },
    get closed(): boolean {
      return closed;
    },
    get fatal(): Error | undefined {
      return fatal;
    },
  };
}
