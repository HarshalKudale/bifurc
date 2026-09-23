/**
 * Ref-counted subscription multiplexing — P5 work item 6.
 *
 * The renderer subscribes to the same event from several components (`onLogEntry` is used by
 * more than one panel), so one `window.api.onX()` call per component must **not** become one
 * transport subscription per component. This hub owns the mapping.
 *
 * The contract it must preserve, in order of how easy each is to get wrong:
 *
 * 1. **The unsubscribe function.** `onLogEntry(cb)` returns `() => void`, and renderer effects
 *    call it from a cleanup function. Returning `undefined` — which is what a naive
 *    `transport.subscribe(...)`-passthrough does once you wrap it — leaks listeners silently.
 * 2. **Idempotent unsubscribe.** React 18 StrictMode double-invokes effects in development, so
 *    the cleanup runs twice. Calling the transport's unsubscribe twice is at best wasteful and
 *    at worst throws; this hub makes the second call a no-op.
 * 3. **Ref-counted teardown.** Subscribe on the first listener, unsubscribe on the last. The
 *    middle case is the one that matters: removing one of two listeners must not tear down the
 *    shared subscription underneath the survivor.
 * 4. **Fan-out isolation.** One listener throwing must not stop the others from being called —
 *    otherwise a bug in one panel silently blanks every other panel.
 *
 * `transportSubscriptionCount()` exists so a test can assert property 3 directly rather than
 * inferring it from delivery counts. It is deliberately part of the returned object rather than
 * a test-only back door: the count is the observable that makes "one subscription" checkable,
 * and hiding it behind a mock would mean testing the mock.
 */

import type { Transport } from "@bifurc/engine/transport/types";

type Listener = (payload: unknown) => void;

interface Slot {
  readonly listeners: Set<Listener>;
  readonly unsubscribe: () => void;
}

export interface SubscriptionHub {
  /**
   * Register `listener` for a **wire** event name. Returns an idempotent unsubscribe.
   * The first listener for an event opens the transport subscription; the last one closes it.
   */
  subscribe(event: string, listener: Listener): () => void;
  /** How many transport subscriptions are currently open. One per event with ≥1 listener. */
  transportSubscriptionCount(): number;
  /**
   * The highest event `seq` this hub has delivered, or `0` if it has delivered none.
   *
   * This is what a **reconnect** sends as `lastSeq`, so the engine can replay what was missed
   * (`plan/05` work item 3). Two properties make the maximum the right answer rather than a count:
   *
   * - **`seq` is engine-scoped, not per-subscription.** `wire/session.ts` says it plainly — *"a session
   *   subscribed to a subset of events sees gaps; events it did not subscribe to still consume a
   *   number"* — so a client-side tally would number events by *arrival* rather than by *origin* and
   *   produce a `lastSeq` the engine does not recognise. The gaps are the point: the client never
   *   wanted those events, so resuming after the highest one it did see loses nothing.
   * - **It is a high-water mark, so a duplicate delivery cannot lower it.** Re-subscribing after a
   *   reconnect re-delivers replayed events, and each one only ever raises this value.
   */
  lastSeq(): number;
  /** Tear every subscription down. Idempotent. Used by `createClient().close()`. */
  close(): void;
}

export function createSubscriptionHub(transport: Transport): SubscriptionHub {
  const slots = new Map<string, Slot>();

  /**
   * The high-water mark behind `lastSeq()`. Deliberately **not** reset by `close()`: a caller that
   * closes and reconnects is exactly the case this exists for, and clearing it there would send
   * `lastSeq: 0` on the reconnect that most needs a real value.
   */
  let highestSeq = 0;

  const subscribe: SubscriptionHub["subscribe"] = (event, listener) => {
    let slot = slots.get(event);

    if (!slot) {
      const listeners = new Set<Listener>();
      /**
       * Copy the listener set before dispatching. A listener that unsubscribes itself (or a
       * sibling) during dispatch would otherwise mutate the set mid-iteration — `Set`
       * iteration is live, so removing the current element silently skips the next one.
       */
      const unsubscribe = transport.subscribe([event], (e) => {
        /**
         * Recorded **before** dispatch, not after. The event has arrived either way, and a listener
         * that throws is swallowed below — so recording afterwards would leave the mark behind an
         * event that was in fact delivered, and the reconnect would ask for it a second time.
         */
        if (typeof e.seq === "number" && e.seq > highestSeq) highestSeq = e.seq;

        for (const l of [...listeners]) {
          try {
            l(e.payload);
          } catch {
            /**
             * Swallowed on purpose, and this is a considered choice rather than laziness.
             * A transport `subscribe` callback has nowhere to report a throw — there is no
             * promise to reject and no caller to return to — so letting it propagate would
             * abandon the remaining listeners in this very loop *and* leave the transport's
             * own dispatch in an undefined state. The engine's `eventPump` makes the same
             * call for the same reason.
             */
          }
        }
      });
      slot = { listeners, unsubscribe };
      slots.set(event, slot);
    }

    slot.listeners.add(listener);

    let released = false;
    return () => {
      if (released) return; // StrictMode double-cleanup
      released = true;

      const current = slots.get(event);
      if (!current) return;

      current.listeners.delete(listener);
      if (current.listeners.size === 0) {
        // Delete before unsubscribing: `unsubscribe` may synchronously deliver a final
        // event, and a slot still in the map would then re-enter dispatch for zero listeners.
        slots.delete(event);
        current.unsubscribe();
      }
    };
  };

  const close: SubscriptionHub["close"] = () => {
    const all = [...slots.values()];
    slots.clear();
    for (const slot of all) slot.unsubscribe();
  };

  return {
    subscribe,
    close,
    transportSubscriptionCount: () => slots.size,
    lastSeq: () => highestSeq,
  };
}
