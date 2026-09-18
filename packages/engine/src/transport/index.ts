/**
 * P4 — the transport layer.
 *
 * `plan/05` orders the work: define `Transport` and implement `in-process` first (this directory),
 * then `stdio`, then `ws` **plus auth**. `ws` must not be merged before work item 4 is complete —
 * that is a hard gate, because the engine generates CAs, executes scripts and spawns child processes,
 * so an unauthenticated remote RPC surface is a remote code execution hole rather than a hardening
 * backlog item.
 *
 * Current state:
 *
 * | Transport | Status |
 * |---|---|
 * | `in-process` | **done** — `inProcess.ts` |
 * | `stdio` | **done** — `stdio.ts` (`createStdioTransport` / `createStdioServer` / `spawnStdioEngine`) |
 * | `ws` | **done** — `ws.ts` (`createWsTransport` / `createWsServer`), auth-wrapped, loopback-only |
 * | `socket` / named pipe | **done** — `socket.ts` (`createSocketTransport` / `createSocketServer`), `0600` on POSIX |
 *
 * That is all four of `plan/05` work item 2's transports, which is the item closed. The client halves of
 * the three that cross a boundary share `session.ts` rather than each carrying their own copy of the
 * request state machine — see that file's header for why a third copy was the moment to extract one.
 *
 * Work item 4 (`auth/`) is **done as a mechanism**: `createAuthenticatedTransport` wraps any transport
 * with the `hello` handshake, a constant-time token check and per-session scopes. It is a decorator
 * precisely so it could be built and tested before the transport that needs it exists — `in-process`
 * and `stdio` are never wrapped, because `plan/05` says `stdio` needs no auth ("the pipe is inherited
 * and process-scoped") and `in-process` has no boundary at all. `ws` **is** wrapped, and the wrapper
 * is the whole of its authentication: `ws.ts` contains no auth logic.
 *
 * Work item 3's **sequencing and replay** half is done: `eventLog.ts` is the single engine-scoped `seq`
 * authority and the bounded ring buffer, `in-process` and the `ws` server subscribe through it, and
 * the decorator answers `hello`'s `lastSeq` with `replayedFrom` or `resyncRequired` and drains the
 * backlog on the session's first subscriptions.
 *
 * Item 3's **coalescing half** is now done too: `eventPump.ts` batches `event.log.entry` into the
 * protocol's `LogEntryBatchEvent` shape (100 entries or 250 ms, whichever is first) and sends every
 * other event immediately, flushing the batch in hand first so wire order stays emission order. The
 * three transports that cross a boundary each own one pump per session; `in-process` has no wire and
 * so does not coalesce, but it still produces the batch *shape*, because the shape is the protocol's
 * rather than the batching's. `VERIFIABLE.backpressure` is `true` as of the same change, which is what
 * lets a runner claim the capability.
 *
 * Still open in item 3: **flow control** for `log.chunk` / `process.output`. `plan/05` asks for a
 * window that lets the engine pause a chatty child process, and that is a client→engine credit — a
 * protocol addition, not a change to this layer. `eventPump.ts`'s header records why those two names
 * are deliberately *not* coalesced in the meantime.
 *
 * The conformance suite (`packages/engine/tests/conformance/`) is written to run against any
 * transport, so each new adapter is added as a runner rather than as a new suite — `plan/05` lists
 * "conformance suite written per-transport instead of shared" as a risk for a reason. Five runners
 * now: `in-process`, `stdio`, `in-process + authenticated`, `ws`, `socket`.
 *
 * There was a fifth for one day — `run-deferred-delivery.test.ts`, a double that deferred event
 * callbacks by a macrotask so the suite's `deliver()` barrier could be proven load-bearing before any
 * real asynchronous transport existed. It was retired on 2026-09-18, the day `run-ws.test.ts` landed:
 * its own header and `plan/05` both said "delete once `run-ws.test.ts` exists — at which point the
 * real thing is the control", and a permanent fifth runner re-running the whole suite for a property
 * a real socket now covers is exactly the per-transport duplication the risks table warns about. The
 * evidence it produced (neutering `deliver()` failed exactly six cases, all on that runner) is kept in
 * `plan/05`, so the knowledge survived the file.
 */
export * from "./types";
export { createInProcessTransport, type InProcessTransportOptions } from "./inProcess";
export {
  DEFAULT_MAX_AGE_MS,
  DEFAULT_MAX_EVENTS,
  EventLog,
  type EventLogOptions,
  type LoggedEvent,
  type ReplayOutcome,
} from "./eventLog";
export * from "./auth";
export {
  COALESCED_EVENT,
  createEventPump,
  DEFAULT_BATCH_WINDOW_MS,
  DEFAULT_MAX_BATCH,
  toClientEvent,
  toLogEntryBatch,
  toLogEntryEvent,
  type EventPump,
  type EventPumpOptions,
} from "./eventPump";
export {
  createFrameDecoder,
  encodeFrame,
  FrameError,
  frameFailure,
  FRAME_HEADER_BYTES,
  MAX_FRAME_BYTES,
  type FrameDecoder,
  type FrameDecoderOptions,
  type FrameErrorCode,
} from "./framing";
export {
  createStdioServer,
  createStdioTransport,
  spawnStdioEngine,
  type StdioServer,
  type StdioServerOptions,
  type StdioSpawnOptions,
  type StdioTransportOptions,
} from "./stdio";
export {
  createSocketServer,
  createSocketTransport,
  defaultSocketPath,
  type SocketServer,
  type SocketServerAuthOptions,
  type SocketServerOptions,
  type SocketTransportOptions,
} from "./socket";
export {
  CLOSE_CODE_POLICY,
  createWsServer,
  createWsTransport,
  DEFAULT_CONNECT_TIMEOUT_MS,
  DEFAULT_MAX_PAYLOAD_BYTES,
  DEFAULT_WS_HOST,
  LOOPBACK_HOSTS,
  type WsServer,
  type WsServerAuthOptions,
  type WsServerOptions,
  type WsTransportOptions,
} from "./ws";
