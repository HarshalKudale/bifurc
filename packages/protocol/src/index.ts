/**
 * `@bifurc/protocol` — the versioned, schema-first engine<->client wire contract.
 *
 * Environment-agnostic (no Node built-ins) so it is importable from the engine (Node/Bun), the
 * Electron shell, the CLI, and a browser bundle (P1 acceptance criterion). See
 * `plan/02-phase-1-protocol.md` for the design rationale and
 * `plan/handler-classification.md` for how every legacy IPC channel maps onto this surface.
 */
export * from "./version";
export * from "./errors";
export * from "./envelope";
export * from "./events";
export * from "./commands/index";
// The client-side wire machinery: the `Transport` abstraction, the `ClientChannel` contract and the
// shared client session. It lives here rather than in `@bifurc/engine` because a **browser** has to
// load it, and engine is a Node package — see `wire/session.ts`'s header. `platform: "neutral"` in
// `tsup.config.ts` is what keeps it honest.
export * from "./wire/types";
export * from "./wire/session";
