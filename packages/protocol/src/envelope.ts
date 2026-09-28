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

/**
 * The bare action name of the handshake.
 *
 * Bare — no `.` — like `stdio`'s `subscribe`/`unsubscribe`, because every real command is namespaced.
 * It is **not** a registry command: `createAuthenticatedTransport` intercepts it, and a `hello`
 * command in the protocol would be shadowed rather than served.
 *
 * **Why it lives here, beside the schemas, and not in the engine that intercepts it.** The action name
 * is part of the wire contract — a client has to send it — and `plan/06` is explicit that the handshake
 * is *"a plain `request("hello", …)` on the raw transport before `createClient()`"*. A client that
 * reached for the engine's copy would pull the engine into a browser bundle to learn a string.
 * `transport/auth/authenticated.ts` imports it from here and keeps its own load-time guard that no
 * *command* has taken the name.
 */
export const HELLO_ACTION = "hello";

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

/**
 * ## The `token` field was removed on 2026-09-25 — from both schemas
 *
 * `HelloRequest.token` was P4 work item 4's per-start secret (read from `<dataDir>/engine.token`, or
 * from `BIFURC_ENGINE_TOKEN` in a container) and `HelloResponse.token` was an optional echo of it that
 * nothing ever read. Both are gone, along with the file, the environment variable and the
 * `--token-file` flag.
 *
 * The reason is a deployment decision rather than a simplification. The engine is **never** deployed
 * remotely: it runs inside the Electron app, or as a container on the machine whose browser uses it. Its
 * RPC surface binds loopback and refuses anything else, so the token was guarding a boundary that no
 * longer exists — and `plan/05`'s actual threat (an unauthenticated *remote* RPC surface) is answered by
 * the bind plus the origin guard on the WebSocket upgrade, which is the check that distinguishes a page
 * served from this machine from a page the user visited somewhere else.
 *
 * **The handshake stays.** `protocolVersion`, `clientName`, `clientVersion` and `lastSeq` all still do
 * their jobs — version compatibility, session identity, the audit trail and replay. Only the credential
 * half went. `createAuthenticatedTransport`, whose name is now historical, still refuses a first request
 * that is not `hello`, and still enforces per-command scopes.
 */
export const HelloResponseSchema = z.object({
  protocolVersion: z.string(),
  engineVersion: z.string(),
  capabilities: z.array(z.enum(Object.values(Capability) as [CapabilityValue, ...CapabilityValue[]])),
  sessionId: z.string(),
  /**
   * The hostname this engine answers as — `localhost`, since `plan/15` work item 9's `--domain` flag was
   * **removed on 2026-09-24** (the engine is no longer deployable in the cloud; see `runtime/hostArgs.ts`).
   *
   * The engine's **own** name: not the client's, not a request's. It is what a mapping's domain is derived
   * *from*: a mapping on an engine that answers as `engine.example.com` is `app.engine.example.com`. That
   * derivation is the whole reason this field exists — the mapping editor needs a suffix to offer, and today it
   * hardcodes `.localhost` (`plan/15` §4.3). **There is one suffix now**, but the field stays on the wire so
   * the client still *reads* it rather than guessing it, which is §4.1's rule.
   *
   * **Optional, and absence is meaningful.** The desktop shell's transport is not the authenticated one
   * (`plan/06`: the renderer reaches the engine over IPC, with no handshake), so no `hello` response is ever
   * produced there; the transports are also constructed directly by tests. Absent therefore means *this engine
   * did not say* — which a client must treat as "no domain-derived suggestions available", not as a licence to
   * substitute a guess. Guessing is the failure `plan/15` §4.1 names.
   *
   * **This is not `hostKind`.** §4.1's `local`/`remote` signal is a separate field, decided by the host, and
   * work item 7's. A client that derived that decision from this string would be making the very call the
   * host is supposed to make, so the two are deliberately not folded into one field.
   *
   * Adding it is legal: the protocol is not frozen against growth — only the four companion-extension
   * commands are (`plan/README.md:150`).
   */
  hostDomain: z.string().optional(),
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
