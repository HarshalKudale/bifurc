/**
 * The browser's `window.api` (D18).
 *
 * ## What this file is for
 *
 * In the Electron shell, `window.api` is `{ ...client }` exposed by `src/preload.ts` through
 * `contextBridge`. **This is the same line, for a browser** — and that sameness *is* the architecture:
 * the shared UI in `packages/gui` reads one injected global, so the only thing that varies between the
 * two hosts is what fills it.
 *
 * | Host | Fills `window.api` from |
 * |---|---|
 * | Electron | `createClient(createIpcTransport(), { local })` — over Electron IPC, no boundary, no auth |
 * | Browser | `createClient(createBrowserTransport({ url }), { local: createBrowserLocal() })` — over a socket, **after `hello`** |
 *
 * ## Why this is now async, and what that changed
 *
 * The browser's half has to **authenticate first** (`web/connect.ts` explains why), so `window.api`
 * cannot be installed and then connected — the handshake has to complete before anything reads the
 * global, because `App` reads it during mount. That is the reason `web/` is a container: it connects,
 * *then* renders. A synchronous installer would have had to render first and connect after, which is
 * exactly the order that produced "the first request must be hello" and a UI that died on arrival.
 *
 * ## Why the host gets its own teardown handle
 *
 * `window.api` deliberately does **not** carry `close`: it is non-enumerable on the client precisely so
 * that `Object.keys(client)` matches `window.api` exactly, which is what the P5 surface test asserts. So
 * the one operation that is *not* part of the UI's surface is handed to the host separately.
 */
import type { BifurcApi } from "@bifurc/gui";

import { connectToEngine, type ConnectOptions, type Connection } from "./connect";

/**
 * The endpoint is **not** derived here.
 *
 * It used to be: this file read `?engine=ws://host:port` and defaulted to the loopback port `plan/14`
 * D16 assigns. Now that the container has a form, the endpoint is *chosen* rather than assumed, and the
 * default belongs with the other form state in `settings.ts` — one place decides what an endpoint looks
 * like, and `?engine=` survives only as a prefill for the form.
 *
 * **The token is deliberately never read from a query string.** It is a per-start secret, and a URL ends
 * up in browser history, in a referrer header and in any proxy's access log. The form keeps it out of
 * all three.
 */

export interface InstalledApi {
  /** The engine this page is pointed at. */
  url: string;
  /** A real round trip, so "connected" means the engine answered rather than the socket opened. */
  probe(): Promise<unknown>;
  /** Tear the session down — it closes the transport too. */
  close(): Promise<void>;
}

/**
 * Connect, then install `window.api` from the authenticated client.
 *
 * Assigns to `globalThis` rather than to `window` so that a Node-side suite can call this without a DOM
 * — the same reason `connect.ts` has no DOM in it. Returns the handles the host still needs; throws a
 * `ConnectError` (see `connect.ts`) if the handshake fails, so the caller can render *why* rather than a
 * generic failure.
 */
export async function connectAndInstall(opts: ConnectOptions): Promise<InstalledApi> {
  const connection: Connection = await connectToEngine(opts);

  (globalThis as { api?: BifurcApi }).api = { ...connection.client };

  return { url: opts.url, probe: connection.probe, close: connection.close };
}

/**
 * Remove the installed global.
 *
 * Exists for tests, and for a container that wants to drop a dead session and return to its form
 * without a page reload. Leaving a stale `window.api` in place is worse than having none: the UI would
 * keep calling a client whose transport is closed, and every call would reject.
 */
export function uninstallWindowApi(): void {
  delete (globalThis as { api?: unknown }).api;
}
