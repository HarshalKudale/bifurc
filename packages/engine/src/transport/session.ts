/**
 * **Moved 2026-09-22** — the shared client session now lives in `@bifurc/protocol`'s
 * `wire/session.ts`, header and all (the "why this file exists" rationale went with it).
 *
 * The reason is a browser: `@bifurc/web-client` implements a `ClientChannel` over the native
 * `WebSocket` API and needs `createClientSession` at **runtime** to get correlation, the
 * all-or-nothing subscription rule, close semantics and the "a request in flight when the peer dies
 * rejects rather than hangs" guarantee. Reimplementing that in the browser package is exactly the
 * third-copy drift this module was extracted to prevent. Two things had to change for it to be
 * loadable outside Node, and both are recorded in the moved file: `randomUUID` came from
 * `node:crypto`, and the `ClientChannel` contract was typed against `Buffer`.
 *
 * This file remains as a **re-export** so `stdio.ts`, `socket.ts`, `ws.ts` and their tests keep
 * working unchanged.
 */
export {
  createClientSession,
  describeError,
  type ClientChannel,
  type ClientSession,
  type ClientSessionOptions,
} from "@bifurc/protocol";
