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
  /**
   * P4 work item 2. The counterpart of `WS`: a session over a unix domain socket or a Windows named
   * pipe advertises this, so a client can tell which transport it is actually on from the `hello`
   * response alone rather than from what it happened to construct.
   *
   * Added after `WS` rather than alongside it because the socket transport was the last of the four
   * to be built (D3 = b). Additive and legal — `HelloResponseSchema` derives its enum from
   * `Object.values(Capability)`, so this needed no schema change, and the protocol is not frozen
   * against growth (`plan/README.md`: only the four companion-extension commands are frozen).
   */
  SOCKET: "socket",
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
  /**
   * P4 work item 4. The per-start secret the client read from `<dataDir>/engine.token`.
   *
   * **Optional in the schema, mandatory in practice for any transport with a boundary.** It is
   * optional here because `in-process` and `stdio` have none — the pipe is inherited and
   * process-scoped, so a token there would be security theatre — and one schema has to serve both.
   * The enforcement is the transport's job, not the schema's: `createAuthenticatedTransport`
   * refuses a `hello` with no token, and `in-process`/`stdio` never wrap it.
   *
   * Adding this field is legal: the protocol is not frozen against growth — only the four
   * companion-extension commands are (`plan/README.md:150`).
   */
  token: z.string().optional(),
  /**
   * P4 work item 3. The highest event `seq` the client has already seen, so the engine can replay
   * what it missed.
   *
   * Optional because a **first** connection has nothing to replay — absence means "send me only live
   * events", not "send me everything". A client that reconnects always has a value, even if it is `0`.
   *
   * Non-negative integer: `seq` starts at 1, so `0` is the honest "I have seen nothing". A float or a
   * negative is a client bug and is refused by the schema rather than silently coerced.
   */
  lastSeq: z.number().int().nonnegative().optional(),
});
export type HelloRequest = z.infer<typeof HelloRequestSchema>;

export const HelloResponseSchema = z.object({
  protocolVersion: z.string(),
  engineVersion: z.string(),
  capabilities: z.array(z.enum(Object.values(Capability) as [CapabilityValue, ...CapabilityValue[]])),
  sessionId: z.string(),
  token: z.string().optional(),
  /**
   * P4 work item 3. Present **only** when the client sent `lastSeq` and the engine can serve it: the
   * engine has retained every event after that seq and will deliver them once the client subscribes.
   *
   * `plan/05`: "`resyncRequired` **must** be handled by every client. Silently ignoring it produces a
   * UI that is permanently wrong in a way nobody can reproduce." Hence two explicit fields rather than
   * a nullable one: the absence of both means "this was a first connection, nothing was missed", which
   * is a different state from either.
   */
  replayedFrom: z.number().int().nonnegative().optional(),
  /**
   * The client asked to be caught up from a `lastSeq` the engine cannot serve — the events in between
   * were dropped, or the seq is ahead of anything the engine ever sent. The client must re-fetch
   * authoritative state with ordinary commands instead of waiting for events that will never come.
   */
  resyncRequired: z.boolean().optional(),
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
