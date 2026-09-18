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
 * ## `subscribe()` refuses, and that is safe *today*
 *
 * The contract is explicit that a transport must throw `UNSUPPORTED` rather than accept a
 * subscription it will never fire: a silent no-op is indistinguishable from "nothing has happened
 * yet" and would be debugged as a broken engine.
 *
 * Refusing is safe **only because the client's subscription hub is lazy** — `createSubscriptionHub`
 * does not call `transport.subscribe()` until the first listener registers, and in step 2 nothing
 * does: the preload's seven `on*` methods still use `ipcRenderer.on` over the legacy channels,
 * untouched. So the client can be constructed here without a single subscription being attempted.
 *
 * **This is the seam where step 3 breaks.** Two things must land before events can move onto this
 * transport, and neither is a change to this file:
 *
 *  1. `log.entry` / `log.chunk` / `server.error` are never emitted on the engine's bus at all — they
 *     travel only through `logEmitter`, which `src/ipc/eventBridge.ts` subscribes to directly. So
 *     the capture panel, the request-log panel and the server-error banner would all go dead, while
 *     every unit test stayed green. See `plan/07` finding 1.
 *  2. Events would then have to be *pushed* from main to preload, which is a different mechanism
 *     from `request()`: `ipcRenderer.invoke` is request/response, so a `seq`-carrying event stream
 *     needs its own channel and its own replay/reconnect handling.
 *
 * ## A cost `plan/07` does not mention
 *
 * Importing `remoteErrorToEngineError` pulls `@bifurc/engine/transport/types` into the **preload**
 * bundle, and that module imports `@bifurc/protocol` at runtime — so zod and the protocol tables are
 * loaded into the renderer process as well as the main one. That is the price of not re-deriving the
 * error projection, and re-deriving it is the drift this codebase deliberately avoids (a second copy
 * is how `stdio` and `ws` would drift apart). It is a startup/memory cost, not a correctness one.
 */
import { ipcRenderer } from "electron";

import { EngineError } from "@bifurc/protocol";
import { remoteErrorToEngineError, type Transport } from "@bifurc/engine/transport/types";

import { RPC_CHANNEL, type RpcFrame, type RpcResult } from "./ipc/rpcContract";

export function createIpcTransport(): Transport {
  /**
   * Read per call rather than captured, so a `close()` during an in-flight request is observed by
   * the *next* one. Mirrors `inProcess.ts`, whose `assertOpen()` does the same — and it matters for
   * the same reason: a closed transport must reject rather than silently do nothing, because
   * `retry.ts` distinguishes "the transport is gone" from "the call failed" by exactly this.
   */
  let closed = false;

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

    subscribe(): () => void {
      throw new EngineError(
        "UNSUPPORTED",
        "The IPC transport cannot deliver events yet: P6 step 2 routes commands only. The shell's " +
          "subscriptions still use the legacy ipcRenderer channels. See src/ipcTransport.ts.",
      );
    },

    async close(): Promise<void> {
      closed = true;
    },
  };
}
