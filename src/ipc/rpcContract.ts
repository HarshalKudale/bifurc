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
import type { RpcError } from "@bifurc/protocol";

/**
 * The single channel the bridge listens on.
 *
 * Namespaced like the legacy channels (`config:get`, `shell:openExternal`) rather than given a bare
 * name, so it is visibly one channel among many during the migration — P6 step 2 runs this
 * **alongside** `registerIpcHandlers()`, and step 3 deletes the other side.
 */
export const RPC_CHANNEL = "engine:rpc";

/** One request, exactly as the preload sends it. */
export interface RpcFrame {
  /** A `@bifurc/protocol` command name, e.g. `config.get`. */
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
