/**
 * `@bifurc/gui` — the shared Bifurc UI (D18).
 *
 * ## What this package is
 *
 * It exports **`<App />`** and nothing a host must construct. The UI is platform-independent: every call
 * that leaves it goes through one injected global, `window.api`, and the **host** decides what that
 * global is —
 *
 * | Host | Fills `window.api` with |
 * |---|---|
 * | Electron shell | `createClient(createIpcTransport(), { local })`, exposed by the preload via `contextBridge` |
 * | Browser | `createClient(createBrowserTransport({ url }), { local: createBrowserLocal() })` |
 *
 * ## Why there is no transport import here
 *
 * That absence is the point. A shared UI that knew how to connect would be a UI that could only connect
 * one way — so the transport is chosen by whoever mounts `<App />`, and this file stays ignorant of it.
 * The UI names the shape it needs (`BifurcApi`) and never builds it.
 *
 * ## The one thing a host must not forget
 *
 * `window.api` has to be assigned **before** the first render, because `App` reads it during mount. The
 * Electron shell gets that ordering for free (the preload runs before any page script); the browser host
 * does it explicitly in `web/bootstrap.ts`, and the ordering is why that file is imported first.
 */
export { default as App } from "./App";
export type { AppProps, AppSession } from "./App";
export type { BifurcApi } from "./types/window";
