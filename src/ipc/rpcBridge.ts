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
import { ipcMain } from "electron";

import { bus } from "@bifurc/engine/eventBus";
import { commandRegistry } from "@bifurc/engine/commands/registry";
import { createInProcessTransport } from "@bifurc/engine/transport/inProcess";
import { toRpcError } from "@bifurc/engine/transport/types";

import { RPC_CHANNEL, type RpcFrame, type RpcResult } from "./rpcContract";

/**
 * Guard against a double registration.
 *
 * `ipcMain.handle()` **throws** if a handler already exists for the channel, and
 * `registerIpcHandlers()` is documented as call-once. This mirrors `wireEventBridge()`'s idempotence
 * note rather than relying on the caller, because a thrown `Attempted to register a second handler`
 * at startup presents as "the app did not open a window", which points nowhere near the cause.
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
 * owns. That is sound here **only because step 2 delivers no events at all**: the preload half's
 * `subscribe()` refuses. When step 3 turns events on, this must be revisited — either the shell
 * grows an engine handle, or the three `log.*` events must be routed onto the bus first (see
 * `plan/07` finding 1).
 */
export function registerRpcBridge(): void {
  if (registered) return;
  registered = true;

  const transport = createInProcessTransport(commandRegistry, { bus });

  ipcMain.handle(RPC_CHANNEL, async (_event, frame: RpcFrame): Promise<RpcResult> => {
    try {
      // Destructured inside the `try` deliberately: a malformed frame (`undefined`, or an object
      // with no `cmd`) then lands in the catch and comes back as a classified `RpcError`, instead of
      // escaping as a `TypeError` that Electron would flatten into an unclassifiable rejection —
      // the very thing this result shape exists to prevent.
      const { cmd, payload } = frame ?? ({} as RpcFrame);
      return { ok: true, value: await transport.request(cmd, payload) };
    } catch (err) {
      return { ok: false, error: toRpcError(err) };
    }
  });
}

/** Test-only: forget that a bridge was registered, so a suite can register it again. */
export function resetRpcBridgeForTests(): void {
  registered = false;
}
