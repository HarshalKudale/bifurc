/**
 * **Moved 2026-09-22** — this module's contents now live in `@bifurc/protocol`'s `wire/types.ts`.
 *
 * It moved because a **browser** has to load it. `@bifurc/web-client` implements a `ClientChannel`
 * over the native `WebSocket` API, and the `Transport` abstraction plus the client session are what
 * it is built against; `@bifurc/engine` is a Node package (`ws`, `simple-git`, `mkcert`, `archiver`)
 * and a browser bundle that reached into it would be the opposite of the decoupling P7 is for.
 * `@bifurc/protocol` already declares itself environment-agnostic and builds with
 * `platform: "neutral"`, so the build itself now enforces the rule.
 *
 * This file remains as a **re-export** so that the ~10 existing importers of
 * `@bifurc/engine/transport/types` keep working unchanged — the shell's `ipc/rpcBridge.ts` and
 * `ipcTransport.ts` among them. Repointing those callers at `@bifurc/protocol` directly is a
 * follow-up, not a prerequisite: a second import path for the same declarations is a wart, not a
 * defect, and churning ten files to remove it buys nothing a later pass cannot.
 */
export {
  type TransportKind,
  type EngineEvent,
  type Transport,
  BUS_NAME_BY_WIRE_NAME,
  WIRE_NAME_BY_BUS_NAME,
  wireNameFor,
  BUS_EVENTS_NOT_ON_THE_WIRE,
  assertBridgeIsTotal,
  RESERVED_ACTIONS,
  toRpcError,
  remoteErrorToEngineError,
} from "@bifurc/protocol";
