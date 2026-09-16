/**
 * Spike 4 — protocol envelope.
 *
 * Generalises the shape that `packages/engine/src/companion/companionServer.ts` already ships in
 * production (`{id, action, payload}` → `{id, ok, data, error}`), with the two
 * changes P4 will need: a structured error (code + retryable) instead of a bare
 * string, and an explicit protocol version for the future `hello` handshake.
 *
 * SPIKE CODE — throwaway, do not productionise.
 */
import { z } from "zod";

export const PROTOCOL_VERSION = "0.1.0-spike";

export const RpcRequestSchema = z.object({
  id: z.string(),
  action: z.string(),
  payload: z.unknown(),
});

export const RpcErrorSchema = z.object({
  code: z.enum(["UNKNOWN_COMMAND", "INVALID_PARAMS", "INTERNAL"]),
  message: z.string(),
  retryable: z.boolean(),
});

export type RpcRequest = z.infer<typeof RpcRequestSchema>;
export type RpcError = z.infer<typeof RpcErrorSchema>;

export type RpcResponse =
  | { id: string; ok: true; data: unknown }
  | { id: string; ok: false; error: RpcError };

export function rpcError(code: z.infer<typeof RpcErrorSchema>["code"], message: string): RpcError {
  return { code, message, retryable: code === "INTERNAL" };
}
