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
 * | Electron | `createClient(createIpcTransport(), { local })` — over Electron IPC |
 * | Browser | `createClient(createBrowserTransport({ url }), { local: createBrowserLocal() })` — over a socket |
 *
 * ## Why the ordering matters
 *
 * `App` reads `window.api` **during mount**, so the assignment has to happen before the first render.
 * Electron gets that for free — a preload runs before any page script. A browser has no such phase, so
 * the ordering is explicit and lives in `main.tsx`: install, then render.
 *
 * ## Why there is no capability list here
 *
 * The browser supplies *more* than the UI asks for: the client's surface is a superset of the 136
 * methods `BifurcApi` declares. Hiding host-local affordances (native dialogs, tray, titlebar) is a
 * *UI* decision made by the shared `<App />`, not by withholding methods here — which is why this file
 * is twelve lines and not a table.
 */
import type { BifurcApi } from "@bifurc/gui";

import { connectToEngine } from "./connect";

/**
 * `?engine=ws://host:port`, defaulting to the loopback port `plan/14` D16 assigns.
 *
 * Read from the query rather than baked in at build time, so one bundle can point at a local engine, a
 * Docker-hosted one or a remote one without a rebuild — which is the "connects to a local engine and to
 * a remote engine" acceptance criterion, and why `base: "./"` is set in the Vite config.
 *
 * Takes `search` as a parameter so the default is testable without a DOM.
 */
export function engineUrlFromLocation(search: string = location.search): string {
  const fromQuery = new URLSearchParams(search).get("engine");
  return fromQuery ?? "ws://127.0.0.1:8080";
}

export interface InstalledApi {
  /** The engine this page is pointed at. */
  url: string;
  /** A real round trip, so "connected" means the engine answered rather than the socket opened. */
  probe(): Promise<unknown>;
  /**
   * Tear the session down — it closes the transport too.
   *
   * The host needs this handle because `window.api` deliberately does **not** carry `close`: it is
   * non-enumerable on the client precisely so that `Object.keys(client)` matches `window.api` exactly,
   * which is what the P5 surface test asserts. So the one operation that is *not* part of the UI's surface
   * has to be handed to the host separately — and work item 2's reconnect will need it.
   */
  close(): Promise<void>;
}

export interface InstallOptions {
  /** Defaults to `engineUrlFromLocation()`. */
  url?: string;
  /**
   * Notified when the session dies for a reason the caller did not ask for.
   *
   * Left to the caller rather than wired to UI copy here: showing it is **work item 2**'s connection
   * state, and a browser needs it more than the desktop does — there is no process boundary to notice
   * the engine is gone, so without it a dropped engine leaves the UI on a stale screen forever.
   */
  onFatal?: (error: Error) => void;
}

/**
 * Install `window.api` for a browser host and return the handles the host still needs.
 *
 * Assigns to `globalThis` rather than to `window` so that a Node-side suite can call this without a
 * DOM — the same reason `connect.ts` has no DOM in it.
 */
export function installWindowApi(opts: InstallOptions = {}): InstalledApi {
  const url = opts.url ?? engineUrlFromLocation();
  const connection = connectToEngine({ url, onFatal: opts.onFatal });

  (globalThis as { api?: BifurcApi }).api = { ...connection.client };

  return { url, probe: connection.probe, close: () => connection.client.close() };
}
