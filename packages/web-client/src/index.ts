/**
 * `@bifurc/web-client` — the browser half of the Bifurc client.
 *
 * It exports **two** things, and both are halves of what a browser was missing: a `Transport` over the
 * platform's native `WebSocket` (the wire) and `createBrowserLocal` (the sixteen client-local
 * operations — `createClient` *requires* a `local`, and there is no Electron shell in a page to supply
 * one). `@bifurc/client` is already built for a browser bundle (`platform: "neutral"` in its
 * `tsup.config.ts`, and no Node built-in in its graph), so the composition is the app's, and it is one
 * line:
 *
 * ```ts
 * import { createClient } from "@bifurc/client";
 * import { createBrowserLocal, createBrowserTransport } from "@bifurc/web-client";
 *
 * const client = createClient(createBrowserTransport({ url }), { local: createBrowserLocal() });
 * ```
 *
 * This header said "exactly one thing" and showed a `createClient` call with **no `local`** for a day
 * after `local.ts` landed — an example that would not compile, in the file whose job is to say what the
 * package is. A docblock that restates a file's own exports drifts the moment they change; the
 * correction is recorded rather than quietly made.
 *
 * ## Why this package exists at all
 *
 * `@bifurc/engine` ships a `ws` client, but a browser cannot load it: `ws.ts` imports `node:crypto`,
 * `node:http`, `node:https`, `node:net` and the `ws` package. This package depends on
 * `@bifurc/protocol` and nothing else at runtime, and `@bifurc/protocol` is the one package in the
 * repo that already declares itself environment-agnostic and enforces it at build time
 * (`platform: "neutral"`). So the browser's dependency graph contains no Node built-in anywhere — and
 * if a future edit introduces one, the build fails here rather than in a page at runtime.
 */
export { createBrowserTransport, type BrowserTransportOptions } from "./channel";

/**
 * The browser's answer to `ClientLocal` — the sixteen client-local operations, without which
 * `createClient` cannot be called at all in a page. See `local.ts` for why seven of the sixteen report a
 * truthful failure rather than pretending (two throw, five return a structured refusal), and why
 * `openExternal` is a security boundary here rather than a convenience.
 */
export { createBrowserLocal, type BrowserLocalOptions } from "./local";
