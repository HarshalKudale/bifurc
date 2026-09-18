/**
 * The frame contract of the shell's RPC bridge (P6 work item 1) — **the one module both halves of
 * the bridge import**, so the channel name and the result shape cannot drift apart.
 *
 * ## Why the result is discriminated rather than a rejection
 *
 * This is the whole reason the file exists, and it is a real Electron behaviour rather than a
 * stylistic preference. A rejection crossing `ipcMain.handle` → `ipcRenderer.invoke` does **not**
 * arrive intact: it is flattened into a plain `Error` whose `message` is the stringified original.
 * `EngineError.code` does not survive the hop.
 *
 * That breaks `@bifurc/client`'s retry policy, which branches on `isRetryable(err.code)` — the
 * three-term `isRetryable(code) && isIdempotent(command) && transport-open` rule in `retry.ts`. With
 * the code flattened, every failure reads as unclassified and the policy either retries everything
 * or nothing. So the handler must **resolve** with a discriminated result and let the preload half
 * rebuild the error:
 *
 * ```
 * { ok: true, value } | { ok: false, error: RpcError }
 * ```
 *
 * This is the same problem `stdio` and `ws` already solved, one hop earlier — which is why the
 * conversions reuse `toRpcError` / `remoteErrorToEngineError` from
 * `packages/engine/src/transport/types.ts` rather than re-deriving them. A third copy of that logic
 * is exactly the "written per-transport instead of shared" drift `plan/05` lists as a risk.
 *
 * Note the deliberate asymmetry with the *engine's* envelope rule: a handler that **resolves**
 * `{ok:false, error}` is an envelope-level success and its value rides through untouched (see
 * `Transport.request()`'s header). This `ok` flag is a different `ok` — it describes the **IPC hop**,
 * not the command. Both flags are real and they answer different questions, so `value` may itself
 * contain `{ok:false}`.
 *
 * ## Why `RpcError` is a type-only import
 *
 * The preload half imports this module for `RPC_CHANNEL`, so this module is loaded at runtime in the
 * **renderer** process. An `import type` is erased by the compiler and leaves no `require` behind,
 * so the contract costs the preload nothing and pulls no engine code into the renderer bundle.
 */
import type { EventEnvelope, RpcError } from "@bifurc/protocol";

/**
 * The single channel the bridge listens on.
 *
 * Namespaced like the legacy channels (`config:get`, `shell:openExternal`) rather than given a bare
 * name, so it is visibly one channel among many during the migration — P6 step 2 runs this
 * **alongside** `registerIpcHandlers()`, and step 3 deletes the other side.
 */
export const RPC_CHANNEL = "engine:rpc";

/**
 * The channel the **engine pushes events on** — main → renderer, the direction `ipcMain.handle`
 * cannot serve.
 *
 * ## Why this needs a channel of its own rather than riding `RPC_CHANNEL`
 *
 * `ipcMain.handle` / `ipcRenderer.invoke` is a **request/response** pair: one invoke, one resolve.
 * Events have no request to answer, so there is nothing to resolve and no `invoke` to hang them on.
 * The push direction is `webContents.send` → `ipcRenderer.on`, which is a different Electron API and
 * therefore a different channel.
 *
 * ## Why the renderer cannot just keep the legacy channels
 *
 * When this channel was added (step 3a) it could, and it did: `src/ipc/eventBridge.ts` was still
 * broadcasting `sync:status`, `log:entry` and the other seven, and that step did not touch them. The
 * point of adding this channel *then* was that the legacy event path was going to be deleted, and an
 * event path that has never carried a single frame is not something to discover a problem in on the
 * day the fallback disappears.
 *
 * **That has now happened.** Step 3c (`eb4c207`) deleted `eventBridge.ts` — but **not** together with
 * `registerIpcHandlers()`, which is what this comment originally predicted. Only the event half of
 * step 3 was replaceable: the command half still has four keys with no registry implementation and
 * four artifact-egress methods, so `registerIpcHandlers()` shrinks rather than disappears. The
 * lesson is worth keeping — the two deletions looked like one because they were described as one, and
 * a shared sentence in a plan is not a shared precondition.
 *
 * ## Why one channel rather than one per event
 *
 * The legacy side uses one channel per event (`sync:status`, `log:chunk`, …), which is why
 * `eventBridge.ts` needs nine subscriptions and a per-event `broadcast()` call. Here the event name
 * travels **inside** the envelope, exactly as it does on every other transport, so the preload needs
 * exactly one `ipcRenderer.on` and dispatches by name. That is also what makes the frame shape
 * identical to `socket`/`ws`/`stdio`: an `EventEnvelope` is what `EventLog` produces and what
 * `eventPump` writes, so nothing new is invented at this boundary.
 */
export const EVENT_CHANNEL = "engine:event";

/**
 * The control frames that ride `RPC_CHANNEL` but are **not** commands.
 *
 * These are the protocol's `RESERVED_ACTIONS` (`packages/engine/src/transport/types.ts`), and the
 * bridge has to recognise them because it is a transport boundary: `subscribe`/`unsubscribe` are
 * transport control, not engine commands, and a registry that received one would answer
 * `UNKNOWN_COMMAND`. That module refuses to load if the protocol ever grows a real command by either
 * name, so the branch below cannot shadow anything.
 *
 * They are **not** re-exported from the engine module here: the preload imports this file, and a value
 * import of `@bifurc/engine/transport/types` would be a second copy of the name table in the renderer
 * bundle. The two strings are duplicated with the guard as the thing that keeps them honest — a
 * mismatch fails `tests/ipc/eventChannel.test.ts`, which asserts them against `RESERVED_ACTIONS`.
 */
export const SUBSCRIBE_ACTION = "subscribe";
export const UNSUBSCRIBE_ACTION = "unsubscribe";

/** One request, exactly as the preload sends it. */
export interface RpcFrame {
  /** A `@bifurc/protocol` command name, e.g. `config.get` — or a reserved control action. */
  cmd: string;
  payload: unknown;
}

/**
 * The result of one request. Never a rejection — see the header.
 *
 * `value` is `unknown` rather than a generic parameter because the command set is open: the frame
 * crosses a structured-clone boundary where the type argument could not be checked anyway, and the
 * client's per-method annotations are what actually narrow it on the far side.
 */
export type RpcResult = { ok: true; value: unknown } | { ok: false; error: RpcError };

/**
 * What crosses `EVENT_CHANNEL`. Aliased rather than re-declared so the two cannot drift: this is the
 * protocol's own envelope, the same type `EventLog` emits and `eventPump` writes.
 */
export type EventFrame = EventEnvelope;
