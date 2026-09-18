/**
 * P6 work item 1, step 2 — the **preload half** of the shell seam: a `Transport` that speaks to the
 * main process over Electron IPC.
 *
 * It is the mirror of `./ipc/rpcBridge.ts`, and the pair is what lets `@bifurc/client` run in the
 * preload at all. `plan/07`'s stack:
 *
 * ```
 * renderer  →  (Electron IPC, unchanged)  →  main  →  (RPC)  →  engine
 * ```
 *
 * **Why keep the Electron IPC hop rather than connecting the renderer straight to the engine?**
 * Because `contextIsolation: true` and `nodeIntegration: false` are set (`main.ts:170–174`), so the
 * renderer cannot open a socket of its own. The hop stays — and keeping it is precisely what makes
 * P6 a *wiring* change instead of a renderer rewrite.
 *
 * ## `kind` is `"ipc"`, not `"in-process"`
 *
 * The two dispatch identically, and they are not the same thing. This one really does cross a
 * process boundary: the payload is structured-cloned, the failure mode is Electron's flattened
 * rejection, and — see below — it cannot deliver events. Anything keying off `kind` needs to know
 * that, so `"ipc"` is a member of `TransportKind` in its own right.
 *
 * ## `subscribe()` now carries real events (P6 step 3)
 *
 * It used to throw `UNSUPPORTED` outright, which the contract requires of a transport that *cannot*
 * deliver rather than one that has not been wired yet. It is wired now: the main half keeps one
 * `Transport` subscription per wire name and broadcasts each `EventEnvelope` on `EVENT_CHANNEL`,
 * which this file receives on a single `ipcRenderer.on` and dispatches by `envelope.event`.
 *
 * Both of the blockers the previous version of this comment listed are resolved:
 *
 *  1. `log.entry` / `log.chunk` / `server.error` now reach the engine's bus —
 *     `wireLogEventsToBus()` in `packages/engine/src/eventBus.ts`, called from both `createEngine()`
 *     and `registerIpcHandlers()`. Without it the capture panel, the request-log panel and the
 *     server-error banner would have gone dead *with every unit test green* (see `plan/07` finding 1).
 *  2. The push direction exists: `ipcRenderer.invoke` is request/response and cannot carry events, so
 *     `EVENT_CHANNEL` is a separate `webContents.send` → `ipcRenderer.on` path.
 *
 * **Nothing about the renderer changes.** The preload's seven `on*` methods still use the legacy
 * channels and `src/ipc/eventBridge.ts` still broadcasts them; this is an additional path, so the
 * phase stays revertable until step 3 deletes the legacy one.
 *
 * ## Why the control frames are fire-and-forget, and why that is not the silent no-op the contract bans
 *
 * `subscribe()` must throw **synchronously** — the contract is explicit, and `inProcess.ts` does. But
 * `ipcRenderer.invoke` is asynchronous, so a rejection from the main half cannot become a synchronous
 * throw here. Two things make that acceptable rather than a hole:
 *
 *  - **The realistic failure is synchronous anyway.** An unknown wire name is refused locally against
 *    `BUS_NAME_BY_WIRE_NAME` — the same table `inProcess.ts` uses — so the common mistake (`subscribe`
 *    to a name that is not on the wire) throws where the contract says it should, with the same code
 *    and the same wording.
 *  - **A dead bridge is not silent.** If the main half is unreachable, every `request()` already
 *    fails loudly with `ENGINE_ERROR`; a subscribe that quietly does nothing cannot be the *first*
 *    symptom, because the client cannot have got this far without a successful request. The
 *    alternative — letting the rejection escape a `void`ed promise — is an unhandled rejection in the
 *    preload, which is strictly worse than a documented no-op on an already-broken transport.
 *
 * ## A cost `plan/07` does not mention
 *
 * Importing `remoteErrorToEngineError` pulls `@bifurc/engine/transport/types` into the **preload**
 * bundle, and that module imports `@bifurc/protocol` at runtime — so zod and the protocol tables are
 * loaded into the renderer process as well as the main one. That is the price of not re-deriving the
 * error projection, and re-deriving it is the drift this codebase deliberately avoids (a second copy
 * is how `stdio` and `ws` would drift apart). It is a startup/memory cost, not a correctness one.
 * `BUS_NAME_BY_WIRE_NAME` comes from that same already-loaded module, so the local validation above
 * adds nothing to it.
 */
import { ipcRenderer } from "electron";

import { EngineError } from "@bifurc/protocol";
import {
  BUS_NAME_BY_WIRE_NAME,
  remoteErrorToEngineError,
  toRpcError,
  type EngineEvent,
  type Transport,
} from "@bifurc/engine/transport/types";

import {
  EVENT_CHANNEL,
  RPC_CHANNEL,
  SUBSCRIBE_ACTION,
  UNSUBSCRIBE_ACTION,
  type EventFrame,
  type RpcFrame,
  type RpcResult,
} from "./ipc/rpcContract";

export function createIpcTransport(): Transport {
  /**
   * Read per call rather than captured, so a `close()` during an in-flight request is observed by
   * the *next* one. Mirrors `inProcess.ts`, whose `assertOpen()` does the same — and it matters for
   * the same reason: a closed transport must reject rather than silently do nothing, because
   * `retry.ts` distinguishes "the transport is gone" from "the call failed" by exactly this.
   */
  let closed = false;

  /**
   * Callbacks per **wire** name, in registration order.
   *
   * A `Set` rather than an array because the same callback may be registered for two names, and
   * because a double-registration of the same callback for one name should be one subscription —
   * `inProcess.ts` makes the same choice for the same reason ("a duplicate name in one call is one
   * subscription, not two").
   */
  const listeners = new Map<string, Set<(e: EngineEvent) => void>>();

  /**
   * Whether the single `ipcRenderer.on` is attached.
   *
   * Lazily, and that is load-bearing rather than tidy: attaching it at construction would mean a
   * transport that never subscribes still holds a listener on a channel the main half is not sending
   * on, and — more importantly — it would make the client's lazy hub eager in effect, since the
   * preload is constructed on every renderer start. The hub is deliberately lazy (nothing calls
   * `subscribe()` until a listener registers), and this preserves that property.
   */
  let dispatching = false;

  /** Fan one envelope out to the callbacks registered for its name. */
  function dispatch(envelope: EventFrame | undefined): void {
    // A frame with no `event` is a malformed or foreign message on our channel. Ignored rather than
    // thrown, because this runs inside an Electron event handler where a throw is swallowed into
    // `ipcRenderer`'s internals and reads as a renderer crash.
    const name = envelope?.event;
    if (typeof name !== "string") return;
    const set = listeners.get(name);
    if (!set) return;
    // Snapshot before iterating: a callback may unsubscribe itself (or a sibling) while being called,
    // and mutating the `Set` mid-iteration would skip the next callback.
    for (const cb of [...set]) {
      try {
        cb(envelope as EngineEvent);
      } catch {
        // One listener's bug must not stop the others or tear down the channel — the same
        // containment `EventLog` and the engine bus apply to their own subscribers.
      }
    }
  }

  function attachDispatcher(): void {
    if (dispatching) return;
    dispatching = true;
    ipcRenderer.on(EVENT_CHANNEL, (_e: unknown, envelope: EventFrame) => dispatch(envelope));
  }

  /**
   * Send a control frame. Deliberately not awaited — see the header — but the promise is returned so
   * a test can assert on it rather than having to race the microtask queue.
   */
  function sendControl(action: string, events: string[]): Promise<RpcResult> {
    return ipcRenderer.invoke(RPC_CHANNEL, { cmd: action, payload: { events } } satisfies RpcFrame)
      .then((r) => r as RpcResult)
      .catch(
        (err): RpcResult => ({
          ok: false,
          // Reused rather than hand-built: `RpcError` carries a `retryable` flag, and a literal that
          // guessed it would disagree with what every other transport reports for the same failure.
          error: toRpcError(
            new EngineError(
              "ENGINE_ERROR",
              `The RPC bridge is not reachable: ${err instanceof Error ? err.message : String(err)}`,
            ),
          ),
        }),
      );
  }

  return {
    kind: "ipc",

    async request(cmd: string, payload: unknown): Promise<unknown> {
      if (closed) {
        throw new EngineError("ENGINE_ERROR", "Cannot request(): this transport has been closed.");
      }

      let result: RpcResult;
      try {
        result = (await ipcRenderer.invoke(RPC_CHANNEL, { cmd, payload } satisfies RpcFrame)) as RpcResult;
      } catch (err) {
        // The *hop* failed rather than the command: no handler is registered for the channel (the
        // bridge was never started, or this preload is talking to an older main), or the main
        // process is gone. Electron's own error is untyped, so it is reclassified rather than
        // passed through — otherwise it would reach the retry policy with no code at all, which is
        // the failure this whole result shape exists to prevent.
        throw new EngineError(
          "ENGINE_ERROR",
          `The RPC bridge is not reachable: ${err instanceof Error ? err.message : String(err)}`,
        );
      }

      // The two `ok` flags answer different questions and both are real. *This* one describes the
      // IPC hop; the engine's envelope rule says a handler that resolves `{ok:false, error}` is an
      // envelope-level **success**, so `result.value` may legitimately be `{ok:false}` and must ride
      // through untouched. See `./ipc/rpcContract.ts`.
      if (result && result.ok === true) return result.value;

      // Rebuilt, not rethrown: Electron flattened the original error, so the code is recovered from
      // the `RpcError` the main half resolved with. This is the line that keeps `isRetryable(code)`
      // working across the hop.
      throw remoteErrorToEngineError(result?.error);
    },

    subscribe(events: string[], cb: (e: EngineEvent) => void): () => void {
      if (closed) {
        throw new EngineError("ENGINE_ERROR", "Cannot subscribe(): this transport has been closed.");
      }
      if (events.length === 0) return () => {};

      // Resolved *before* any listener is attached, and against the same table the main half uses, so
      // the transport contract's "throws synchronously" is honoured for the failure that actually
      // happens. See the header for why an unreachable bridge cannot be made synchronous.
      for (const wire of events) {
        if (!BUS_NAME_BY_WIRE_NAME.has(wire)) {
          throw new EngineError(
            "UNSUPPORTED",
            `This transport cannot deliver "${wire}". It is either not a known event or not yet on ` +
              `the wire; see BUS_EVENTS_NOT_ON_THE_WIRE in packages/engine/src/transport/types.ts.`,
          );
        }
      }

      attachDispatcher();

      /** Names that go from zero listeners to one, i.e. the ones the main half must be told about. */
      const opened: string[] = [];
      for (const wire of events) {
        let set = listeners.get(wire);
        if (!set) {
          set = new Set();
          listeners.set(wire, set);
        }
        // Checked before the add: after it, the size is never 0 and a 0→1 edge could not be detected.
        if (set.size === 0) opened.push(wire);
        set.add(cb);
      }
      if (opened.length > 0) void sendControl(SUBSCRIBE_ACTION, opened);

      let detached = false;
      return () => {
        if (detached) return;
        detached = true;

        const closedNames: string[] = [];
        for (const wire of events) {
          const set = listeners.get(wire);
          if (!set) continue;
          set.delete(cb);
          if (set.size === 0) {
            listeners.delete(wire);
            closedNames.push(wire);
          }
        }
        if (closedNames.length > 0) void sendControl(UNSUBSCRIBE_ACTION, closedNames);
      };
    },

    async close(): Promise<void> {
      if (closed) return;
      closed = true;

      // Release every name still held, so the main half's per-name counts cannot outlive the renderer
      // that opened them. A window torn down with live subscriptions would otherwise pin engine
      // listeners for the life of the process — the leak class `socket.ts` records for closed
      // sessions.
      const held = [...listeners.keys()];
      listeners.clear();
      if (held.length > 0) void sendControl(UNSUBSCRIBE_ACTION, held);

      if (dispatching) {
        dispatching = false;
        ipcRenderer.removeAllListeners(EVENT_CHANNEL);
      }
    },
  };
}
