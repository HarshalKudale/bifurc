/**
 * `@bifurc/web-client` — the browser half of the Bifurc client.
 *
 * It exports exactly one thing: a `Transport` over the platform's native `WebSocket`. That is the
 * whole of what a browser was missing. `@bifurc/client` is already built for a browser bundle
 * (`platform: "neutral"` in its `tsup.config.ts`, and no Node built-in in its graph), and it is a pure
 * mapping over a `Transport` — so the composition is the app's, and it is one line:
 *
 * ```ts
 * import { createClient } from "@bifurc/client";
 * import { createBrowserTransport } from "@bifurc/web-client";
 *
 * const client = createClient(createBrowserTransport({ url }));
 * ```
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
