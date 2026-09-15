/**
 * Spike 4 — in-process transport over the REAL registered IPC handlers.
 *
 * This is the piece that answers "does the existing handler layer survive being
 * reached through a generic envelope?". Nothing is reimplemented: the production
 * `register*Handlers()` functions run against a capturing `ipcMain` (the exact
 * pattern the integration suite already uses), and this transport dispatches
 * envelope requests into them.
 *
 * P2's CommandRegistry replaces the `ipcMain.handle` capture; P4's stdio/ws
 * transports replace the direct function call. The envelope semantics defined
 * here are what those transports must reproduce.
 *
 * SPIKE CODE — throwaway, do not productionise.
 */
import { commands, isKnownAction, LEGACY_CHANNEL, type CommandAction, type CommandDef } from "./commands";
import { rpcError, type RpcRequest, type RpcResponse } from "./envelope";
import type { Transport } from "./transport";

export type LegacyHandler = (event: unknown, ...args: unknown[]) => unknown | Promise<unknown>;

/**
 * Envelope semantics, stated once:
 *
 * 1. Unknown action → `{ok:false, error:{code:"UNKNOWN_COMMAND"}}`.
 * 2. Payload that fails the command's schema → `{ok:false, error:{code:"INVALID_PARAMS"}}`.
 *    The server re-validates — the wire is untrusted.
 * 3. A handler that **resolves** — with ANY shape, including `{ok:false, error}` —
 *    is a success at the envelope level; the value rides in `data` untouched.
 *    This is what keeps the renderer byte-compatible: the legacy handlers report
 *    many of their failures as resolved values, and the renderer already branches
 *    on those.
 * 4. A handler that **throws / rejects** → `{ok:false, error:{code:"INTERNAL"}}`
 *    — the same contract `ipcRenderer.invoke` has today.
 */
export function createInProcessTransport(
  getHandler: (channel: string) => LegacyHandler | undefined,
): Transport {
  return {
    async send(request: RpcRequest): Promise<RpcResponse> {
      const { id, action } = request;
      if (typeof action !== "string" || !isKnownAction(action)) {
        return { id, ok: false, error: rpcError("UNKNOWN_COMMAND", `unknown action "${String(action)}"`) };
      }
      const def = commands[action as CommandAction] as CommandDef<unknown, unknown>;
      const parsed = def.params.safeParse(request.payload);
      if (!parsed.success) {
        return { id, ok: false, error: rpcError("INVALID_PARAMS", `invalid payload for "${action}"`) };
      }
      const handler = getHandler(LEGACY_CHANNEL[action as CommandAction]);
      if (!handler) {
        return { id, ok: false, error: rpcError("INTERNAL", `no legacy handler for "${action}"`) };
      }
      try {
        const data = await handler({}, ...def.toArgs(parsed.data));
        return { id, ok: true, data };
      } catch (e) {
        return { id, ok: false, error: rpcError("INTERNAL", e instanceof Error ? e.message : String(e)) };
      }
    },
  };
}
