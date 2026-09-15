/**
 * P1 work items 5, 6, 7 — the wire envelope.
 *
 * Generalises the shape `src/companion/companionServer.ts` already ships in production
 * (`{id, action, payload}` -> `{id, ok, data, error}`) — validated end-to-end against the REAL
 * registered handlers in the P0 spike (`spike/protocol/`, `tests/spike/protocolPoc.test.ts`).
 *
 * LOAD-BEARING DESIGN DECISION (confirmed by the spike, do not "fix" without re-reading
 * `plan/spike-results.md"):
 *
 *   A legacy handler that *resolves* — with ANY shape, including `{ok:false, error:"..."}` —
 *   is an envelope-level SUCCESS (`ok:true`); the value rides through untouched in `data`. Only
 *   a THROWN/REJECTED handler becomes an envelope-level failure (`ok:false`, `error: RpcError`).
 *   This is what keeps `window.api` byte-identical through P5/P6: the existing handlers report
 *   most of their failures as resolved values, and the renderer already branches on those.
 */
import { z } from "zod";
import { RpcErrorSchema, type RpcError } from "./errors";

export const RpcRequestSchema = z.object({
  id: z.string(),
  action: z.string(),
  payload: z.unknown(),
});
export type RpcRequest = z.infer<typeof RpcRequestSchema>;

export type RpcResponse<T = unknown> =
  | { id: string; ok: true; data: T }
  | { id: string; ok: false; error: RpcError };

export const RpcResponseSchema = z.union([
  z.object({ id: z.string(), ok: z.literal(true), data: z.unknown() }),
  z.object({ id: z.string(), ok: z.literal(false), error: RpcErrorSchema }),
]);

// ── hello handshake (work item 7) ────────────────────────────────────────────

export const Capability = {
  BLOB: "blob",
  STDIO: "stdio",
  WS: "ws",
  EVENTS_REPLAY: "events.replay",
  TLS_INSTALL: "tls.install",
} as const;
export type CapabilityValue = (typeof Capability)[keyof typeof Capability];

export const ClientNameSchema = z.enum([
  "electron-shell",
  "web-ui",
  "cli",
  "bifurc-extension", // the externally-released companion extension — see work item 7 exception
]);

export const HelloRequestSchema = z.object({
  protocolVersion: z.string(),
  clientName: ClientNameSchema,
  /**
   * Recorded per session so the engine can log and diagnose which build is connected — required
   * for the companion extension, which the engine cannot update atomically (see
   * "Protocol compatibility" in `plan/README.md`).
   */
  clientVersion: z.string(),
});
export type HelloRequest = z.infer<typeof HelloRequestSchema>;

export const HelloResponseSchema = z.object({
  protocolVersion: z.string(),
  engineVersion: z.string(),
  capabilities: z.array(z.enum(Object.values(Capability) as [CapabilityValue, ...CapabilityValue[]])),
  sessionId: z.string(),
  token: z.string().optional(),
});
export type HelloResponse = z.infer<typeof HelloResponseSchema>;

// ── subscribe / unsubscribe (work item 6) ────────────────────────────────────

export const SubscribeRequestSchema = z.object({
  events: z.array(z.string()),
});
export type SubscribeRequest = z.infer<typeof SubscribeRequestSchema>;

export const UnsubscribeRequestSchema = z.object({
  events: z.array(z.string()),
});
export type UnsubscribeRequest = z.infer<typeof UnsubscribeRequestSchema>;
