/**
 * `ws.*` — the **WebSocket connection registry**, engine-side (`plan/19` WI 6).
 *
 * ## Why these commands exist
 *
 * `packages/gui/src/hooks/useWebSocket.ts` held a module-level `Map` of live sockets and did the whole
 * protocol in the renderer. Two consequences followed, and both are the reason this file exists:
 *
 * 1. **A socket died with the window.** The renderer *was* the connection's lifetime, so closing a window,
 *    reloading it or switching to a build without the panel dropped every connection. Nothing else could
 *    have been true: the object that owned the socket was a browser context.
 * 2. **A CLI could not open one.** `Protocol_Module_Contract.md` §1's rule is that every byte reaching the
 *    wire is the engine's; a socket the engine cannot open is a protocol the CLI cannot speak.
 *
 * ## The four decisions `Protocol_Module_Contract.md` §4 requires of a module with long-lived state
 *
 * | decision | here |
 * |---|---|
 * | **Scope** | `connect` / `disconnect` / `send` are `execute`; `status` / `list` are `read`; `clearMessages` is `write`. The split is what lets a reconnecting client ask *what happened* without being granted the right to open or close a connection. |
 * | **Lifetime** | **D29's.** A transport disconnect and a workspace switch do **not** close a socket; a host restart loses every socket. `disconnectSocket()` is the only lever, and `ws.disconnect` is its only caller. |
 * | **Concurrency** | one socket per `{workspaceId, socketId}`; a second `ws.connect` on the same key **replaces** it. See `WsConnectParams` for why this is deliberately *not* D30's refusal. |
 * | **Event completeness** | `ws.status` returns the whole snapshot **including the retained message log**, so a client that attached mid-stream can render the backlog rather than only what happens next. That is the `total`-rides-every-event requirement in its socket form. |
 *
 * ## Why `workspaceId` is required rather than `WorkspaceScope`'s optional
 *
 * `runner.start` / `runner.cancel` / `runner.status` made the same choice for the same reason: a socket is
 * attributed to a workspace, the registry is keyed by the pair, and "the active workspace" is a *desktop*
 * convenience with no meaning to a host serving several. A socket whose workspace is inferred at connect
 * time and then changed under it would be a connection nobody can address.
 */
import { z } from "zod";

/**
 * The four states a socket can be in.
 *
 * The same set the renderer used, kept as a **closed enum on the wire** rather than a free string: the UI
 * colours a dot from it, and a new value would silently render as "disconnected".
 *
 * `error` is a *terminal* state, not a flag — a socket that failed is `error`, and it stays `error` until
 * something connects again. The renderer's `isDisconnected` is `disconnected || error`, so the distinction
 * is purely "was there a reason to show".
 */
export const WsConnectionStateSchema = z.enum(["connecting", "connected", "disconnected", "error"]);
export type WsConnectionState = z.infer<typeof WsConnectionStateSchema>;

/**
 * One line of the socket's stream, in either direction.
 *
 * `ts` is **engine-side epoch milliseconds**, for the reason `ProcessOutputEventSchema` records: a remote
 * client cannot compute it, and a client-side timestamp would be wrong by the network delay — which matters
 * when the point of the stream is to show *when* the peer said something.
 *
 * `id` is minted engine-side and monotonic per socket. It exists so a client that re-attaches and receives
 * the retained log alongside live events can place and de-duplicate them; without it, "the same message
 * twice" and "two identical messages" are indistinguishable.
 */
export const WsMessageSchema = z.object({
  id: z.string(),
  ts: z.number(),
  direction: z.enum(["sent", "received"]),
  data: z.string(),
});
export type WsMessage = z.infer<typeof WsMessageSchema>;

/**
 * `ws.connect` — open (or re-open) a socket, named by the caller.
 *
 * ## `socketId` is the caller's name for the connection, and that is the whole identity model
 *
 * The engine cannot know what a "tab" is, so it does not try: the caller supplies the name, and for the UI
 * that name is the tab id it already had. A CLI supplies whatever it likes. The engine's job is to enforce
 * one live socket per `{workspaceId, socketId}`.
 *
 * ## Why a second `connect` on a live key REPLACES rather than being refused
 *
 * D30 refused a second run of the same folder, and the difference is the kind of thing each command makes:
 *
 * - A **run** is a unit of work. A second one duplicates effort the caller did not ask for twice, so
 *   refusing is the safe answer.
 * - A **socket** is a channel. A second `connect` on a live key is a *reconnect* — the user pressed Connect
 *   again after an error, or the URL changed — and it is exactly what the renderer did before this move
 *   ("close any existing connection for this tab"). Refusing would make a broken socket unrecoverable
 *   without a separate disconnect first, which is a UI step the engine has no reason to require.
 *
 * So the policy is stated rather than copied: **replace, and the replacement is atomic** — the old socket is
 * closed before the new one is constructed, so the live count never exceeds the limit during a reconnect.
 *
 * ## Resolution is engine-side (D23), and the headers now actually reach the wire
 *
 * `url` and every header **value** are resolved against the workspace's merged environment. The renderer
 * resolved these before, and the renderer's browser `WebSocket` API **cannot send headers at all** — its own
 * comment said so ("we resolve them for future reference but they are stored, not injected"). Engine-side the
 * socket is a `ws` client, which can. So this is not a move of the same behaviour: a `{{TOKEN}}` in an
 * Authorization header went from being resolved-and-discarded to being resolved-and-sent.
 */
export const WsConnectParams = z.object({
  workspaceId: z.string(),
  socketId: z.string(),
  url: z.string(),
  /** Header names are taken verbatim; only the values are resolved. */
  headers: z.record(z.string(), z.string()).optional(),
}).strict();
export type WsConnectParams = z.infer<typeof WsConnectParams>;

/**
 * `ws.disconnect` — close a socket, the **only** thing that closes one.
 *
 * D29 states it as a rule: a client disconnect and a workspace switch do not cancel engine-owned work, and
 * "cancel on disconnect" is the default most implementations reach for. So nothing in the engine infers a
 * close from a client going away — there is no disconnect hook to find, deliberately — and this command is
 * the explicit request that stands in for one.
 *
 * **`{ok:true}` for an already-closed socket**, matching `runner.cancel`'s answer for a finished run: the
 * caller's intent ("this should not be open") is satisfied either way, and the alternative is a race every
 * client would have to defend against.
 */
export const WsDisconnectParams = z.object({
  workspaceId: z.string(),
  socketId: z.string(),
  /** WebSocket close code. Defaults to `1000` (normal closure) — the renderer's "User disconnected". */
  code: z.number().int().optional(),
  reason: z.string().optional(),
}).strict();
export type WsDisconnectParams = z.infer<typeof WsDisconnectParams>;

/**
 * `ws.send` — resolve the payload against the merged environment, then write it to the socket.
 *
 * **The payload is text and is resolved here**, which is `plan/19` §8 trap 1's rule applied to a stream
 * rather than to a request: the renderer used to resolve the outgoing body, and a socket the engine owns
 * cannot have two resolvers. `{{random.*}}` therefore mints **per send**, which is the correct behaviour for
 * a stream — each frame is its own event, not a retry of an earlier one — and is the opposite of the
 * request pipeline's problem (§8 trap 3), where a retry must not re-mint.
 *
 * `{ok:false}` when nothing is open, rather than a throw: "is this socket open?" is a question with a
 * legitimate answer, and the renderer shows it inline.
 */
export const WsSendParams = z.object({
  workspaceId: z.string(),
  socketId: z.string(),
  data: z.string(),
}).strict();
export type WsSendParams = z.infer<typeof WsSendParams>;

/**
 * `ws.status` — the snapshot for one socket, live or finished, or `null` if it was never opened.
 *
 * **This is what makes D29 usable**, and it is `runner.status`'s shape one namespace over: a client that
 * disconnected asks this on reconnect and gets the state, the error and **the retained messages**. `null` is
 * distinct from "disconnected": the first means the engine has never been asked to open this id, the second
 * means it did and the answer is in `status`.
 */
export const WsStatusParams = z.object({
  workspaceId: z.string(),
  socketId: z.string(),
}).strict();
export type WsStatusParams = z.infer<typeof WsStatusParams>;

/**
 * The socket's state. Mirrors `SocketSnapshot` in `packages/engine/src/websocket/wsRegistry.ts`, **which is
 * the authority** — the same relationship `RunnerStatusResult` has with `RunSnapshot`.
 *
 * `messages` is the **retained tail**, not the whole stream: the registry keeps a bounded log (see
 * `MAX_WS_MESSAGES`) because an unbounded per-socket log is a leak in a process that now outlives every
 * client. The bound is a stated decision rather than an omission, and a client that needs everything should
 * be consuming `event.ws.message` as it arrives.
 */
export interface WsStatusResult {
  workspaceId: string;
  socketId: string;
  /** The url **as connected**, unresolved — what the caller asked for, so it can be shown back verbatim. */
  url: string;
  status: WsConnectionState;
  error: string | null;
  /** The close code, once the socket has closed. */
  closeCode?: number;
  closeReason?: string;
  /** Epoch ms of the successful open, or `null` if it never opened. */
  openedAt: number | null;
  messages: WsMessage[];
}

/**
 * `ws.list` — the sockets that are **live right now**, plus the registry's limit.
 *
 * **Live only, and the contrast with `ws.status` is the design.** `ws.list` answers "what is open?", which is
 * what a connection-limit indicator and a CLI's `ws list` both want; `ws.status` answers "what happened to
 * this one?", which is what a reconnecting client wants and why its snapshot outlives the socket. Returning
 * finished sockets here would make the count meaningless.
 *
 * **`max` is on the wire rather than duplicated in the renderer.** The limit is a property of the registry —
 * the registry is what refuses the sixth connection — so the registry reports it. The alternative was a
 * pinned copy of `5` in the UI, which is the D28 list-vs-logic split spent on a number the engine can simply
 * answer, and which would have been wrong the moment the limit became configurable.
 */
export const WsListParams = z.object({
  workspaceId: z.string(),
}).strict();
export type WsListParams = z.infer<typeof WsListParams>;

export interface WsListResult {
  connections: WsStatusResult[];
  max: number;
}

/**
 * `ws.clearMessages` — drop the retained log for one socket.
 *
 * It is a command because the log is **engine state**. The renderer's `clearMessages` was local React state;
 * now that `ws.status` returns the log, a renderer-only clear would be undone by the next re-attach, which is
 * the class of bug this whole work item is about. The socket itself is untouched.
 *
 * Scope `write`: it mutates engine state and reaches no upstream, which is neither `read` nor `execute`.
 */
export const WsClearMessagesParams = z.object({
  workspaceId: z.string(),
  socketId: z.string(),
}).strict();
export type WsClearMessagesParams = z.infer<typeof WsClearMessagesParams>;

/** The shared answer for the four commands that can only succeed or explain why not. */
export interface WsOkResult {
  ok: boolean;
  error?: string;
}
