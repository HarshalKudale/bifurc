/**
 * P6 work item 1, step 2 — the **main-process half** of the shell seam.
 *
 * It exposes the engine's `CommandRegistry` over one Electron IPC channel, by putting
 * `createInProcessTransport()` behind it. The chain is:
 *
 * ```
 * renderer  →  (Electron IPC)  →  main  →  (Transport)  →  registry  →  handler
 * ```
 *
 * The middle link is the point of the phase. The registry was built in P2 so a transport could
 * dispatch through it with no Electron involved; this file is the first *real* client to do that,
 * and the hop it crosses is the one the whole programme exists to remove. Once it is proven, the
 * transport behind it can become `socket`/`stdio` — a change of one argument — and the shell stops
 * being load-bearing.
 *
 * ## This runs **alongside** `registerIpcHandlers()`, on purpose
 *
 * `plan/07`'s step 2 is explicit: keep the old path running and route **one** method
 * (`config:get`) through the new one. Everything else stays on the legacy channels, so the blast
 * radius of a bridge bug is a single method, and `git revert` of the phase restores the previous
 * behaviour with nothing to unpick. Step 3 flips the rest over and deletes the old handlers.
 *
 * That is also what makes the rollback flag in `plan/07` work item 6 meaningful: it can only
 * restore the old path while the old path still exists.
 *
 * ## Why the handler resolves a discriminated result instead of rejecting
 *
 * Electron flattens a rejection crossing this hop and **drops `EngineError.code`**, which the
 * client's retry policy branches on. The full argument is in `./rpcContract.ts`'s header; the short
 * version is that `toRpcError` is applied here and `remoteErrorToEngineError` on the far side, so
 * the code survives the round trip.
 */
import { BrowserWindow, ipcMain } from "electron";

import { EngineError, SubscribeRequestSchema, UnsubscribeRequestSchema } from "@bifurc/protocol";
import { bus } from "@bifurc/engine/eventBus";
import { commandRegistry } from "@bifurc/engine/commands/registry";
import { createInProcessTransport } from "@bifurc/engine/transport/inProcess";
import {
  BUS_NAME_BY_WIRE_NAME,
  toRpcError,
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
} from "./rpcContract";

/**
 * Guard against a double registration.
 *
 * `ipcMain.handle()` **throws** if a handler already exists for the channel, and
 * `registerIpcHandlers()` is documented as call-once. This guards rather than relying on the caller,
 * because a thrown `Attempted to register a second handler` at startup presents as "the app did not
 * open a window", which points nowhere near the cause.
 */
let registered = false;

/**
 * One transport for the whole shell, not one per request.
 *
 * A per-request transport would build a fresh `EventLog` for every call. That is not merely wasteful:
 * the log owns the engine-scoped `seq` authority, so a log per request would give every session a
 * counter starting at 1, which is the exact defect `eventLog.ts` was written to remove.
 *
 * Note what this transport does **not** get: a shared log. `createEngine()` owns one log for the
 * process and hands it to every transport, but the shell never calls `createEngine()` — it uses the
 * module singletons — so there is no such log to pass, and the factory creates a private one it
 * owns. That is sound here because the transport's lifetime *is* the shell's: the log dies when the
 * process does, and there is no second session for `seq` to be ambiguous against. (This was the
 * reason step 2 delivered no events at all; the log's ownership question and the push channel are the
 * same question, so both are settled here.)
 */
let transport: Transport | null = null;

/**
 * Live subscriptions, keyed by **wire** event name.
 *
 * ## Why the count, and what it is counting
 *
 * The count is *renderer* listeners, not engine ones: the preload refcounts too, but the two counters
 * answer different questions. The preload's says "how many callbacks in this renderer want this
 * name"; this one says "how many renderers want it". Only the second can decide whether the engine
 * should still be delivering it, and a single window closing must not unsubscribe a second window
 * that is still listening — which is exactly what a bare `Set` keyed by name would have done.
 *
 * ## Why one subscription per name rather than one per renderer
 *
 * `broadcast()` sends to **every** open window, matching `eventBridge.ts`'s existing semantics, so the
 * engine side is genuinely process-wide: one subscription per name is the whole need. A renderer that
 * did not ask for a name simply has no listener registered and drops the frame — the preload
 * dispatches by `envelope.event` and ignores anything it has no callback for. Per-window
 * subscriptions would mean per-window `webContents.send`, which is a different feature (targeted
 * delivery) that nothing asks for and that the legacy path never had.
 */
const subscriptions = new Map<string, { detach: () => void; count: number }>();

/**
 * Send one envelope to every live window.
 *
 * Deliberately the same shape as `eventBridge.ts`'s `broadcast()`, including the `isDestroyed()` guard
 * — a window can be torn down between the subscription and the event, and `webContents.send()` on a
 * destroyed window throws inside a bus listener, where a throw unwinds into machinery that is not
 * expecting it.
 */
function broadcastEvent(envelope: EventFrame): void {
  for (const w of BrowserWindow.getAllWindows()) {
    if (!w.isDestroyed()) w.webContents.send(EVENT_CHANNEL, envelope);
  }
}

/**
 * Take a reference on a wire event name, subscribing to the engine the first time.
 *
 * `transport.subscribe()` **throws** `UNSUPPORTED` for a name that is not on the wire, and that throw
 * is allowed to escape: it becomes a classified `RpcError` in the caller's `RpcResult`, which is what
 * the preload needs to report. Silently accepting an unknown name would leave a renderer waiting for
 * events that can never arrive — the failure `plan/07` calls out for `resyncRequired`.
 */
function retain(name: string): void {
  const existing = subscriptions.get(name);
  if (existing) {
    existing.count += 1;
    return;
  }
  if (transport === null) {
    throw new EngineError("ENGINE_ERROR", "Cannot subscribe: the RPC bridge is not registered.");
  }
  const detach = transport.subscribe([name], (e) => broadcastEvent(e));
  subscriptions.set(name, { detach, count: 1 });
}

/** Drop a reference, detaching from the engine when the last renderer lets go. */
function release(name: string): void {
  const existing = subscriptions.get(name);
  if (!existing) return;
  existing.count -= 1;
  if (existing.count > 0) return;
  subscriptions.delete(name);
  existing.detach();
}

/**
 * Apply one control frame. Returns the acknowledged names, which is the `RpcResult`'s value.
 *
 * ## Why the payload is validated here rather than trusted
 *
 * The frame arrives from the renderer, and the renderer is the untrusted side of this boundary — the
 * same reason `socket.ts` parses `subscribe` payloads with these schemas instead of destructuring. A
 * malformed payload that reached `retain()` as `undefined` would throw a `TypeError` inside a bus
 * listener path and come back as an unclassifiable failure.
 *
 * ## Why it is all-or-nothing
 *
 * Names are retained in a loop, so a payload of `["event.sync.status", "nonsense"]` would otherwise
 * subscribe the first and fail on the second, leaving the caller with a rejected request and a live
 * subscription it does not know about — a leak with no handle to release it. So the names are
 * validated against the transport **first**, and only then applied.
 */
function applyControl(action: string, payload: unknown): { events: string[] } {
  const schema = action === SUBSCRIBE_ACTION ? SubscribeRequestSchema : UnsubscribeRequestSchema;
  const parsed = schema.safeParse(payload);
  if (!parsed.success) {
    throw new EngineError("BAD_REQUEST", `Invalid ${action} payload: ${parsed.error.message}`);
  }

  const names = parsed.data.events;

  // `unsubscribe` has nothing to validate against — releasing an unknown name is a no-op by design,
  // because a caller that unsubscribes twice must not fail the second time.
  if (action === UNSUBSCRIBE_ACTION) {
    for (const name of names) release(name);
    return { events: names };
  }

  /**
   * Pre-flight the whole batch, so the all-or-nothing claim above is true rather than aspirational.
   *
   * Checked against the **name table**, not by calling `subscribe()` and detaching. A dry run through
   * the transport would not actually be side-effect-free: `EventLog`'s retention listeners are
   * attached on the *first* subscribe for a name and deliberately outlive their subscribers
   * (`eventLog.ts`'s "retention outlives its subscribers"), so a dry run would permanently retain a
   * name that the batch then rejected. `inProcess.ts` validates the same way for the same reason, and
   * using the same table keeps the two from disagreeing about what "on the wire" means.
   */
  for (const name of names) {
    if (!BUS_NAME_BY_WIRE_NAME.has(name)) {
      throw new EngineError(
        "UNSUPPORTED",
        `This transport cannot deliver "${name}". It is either not a known event or not yet on the ` +
          `wire; see BUS_EVENTS_NOT_ON_THE_WIRE in packages/engine/src/transport/types.ts.`,
      );
    }
  }

  for (const name of names) retain(name);
  return { events: names };
}

export function registerRpcBridge(): void {
  if (registered) return;
  registered = true;

  // A local, so the handler closes over a value that cannot be nulled from under it by
  // `resetRpcBridgeForTests()` — the module-level `transport` exists for that reset and for
  // `retain()`, not to be dereferenced on the request path.
  const session = createInProcessTransport(commandRegistry, { bus });
  transport = session;

  ipcMain.handle(RPC_CHANNEL, async (_event, frame: RpcFrame): Promise<RpcResult> => {
    try {
      // Destructured inside the `try` deliberately: a malformed frame (`undefined`, or an object
      // with no `cmd`) then lands in the catch and comes back as a classified `RpcError`, instead of
      // escaping as a `TypeError` that Electron would flatten into an unclassifiable rejection —
      // the very thing this result shape exists to prevent.
      const { cmd, payload } = frame ?? ({} as RpcFrame);

      // Transport control before commands. The two are mutually exclusive by construction:
      // `RESERVED_ACTIONS` refuses to load if the protocol ever grows a real command by either name.
      if (cmd === SUBSCRIBE_ACTION || cmd === UNSUBSCRIBE_ACTION) {
        return { ok: true, value: applyControl(cmd, payload) };
      }

      return { ok: true, value: await session.request(cmd, payload) };
    } catch (err) {
      return { ok: false, error: toRpcError(err) };
    }
  });
}

/**
 * Test-only: forget that a bridge was registered, so a suite can register it again.
 *
 * Also drops every subscription, because `vi.resetModules()` gives each test a fresh module graph
 * while `bus` is still the process-wide singleton: a subscription leaked from one test would keep
 * forwarding into a `BrowserWindow` mock the next test had already replaced, and the symptom would be
 * an event arriving in a test that never emitted one.
 */
export function resetRpcBridgeForTests(): void {
  registered = false;
  for (const { detach } of subscriptions.values()) detach();
  subscriptions.clear();
  transport = null;
}
