/**
 * Temporary shell-side bridge — subscribes to the engine's `bus` (and to `logEmitter`, which
 * was already Electron-free) and forwards to every open `BrowserWindow`, preserving today's
 * exact wire channel names and payload shapes.
 *
 * This is NOT engine code. It exists so that P2 can invert the broadcast pattern at every
 * emission site (`plan/03-phase-2-engine-extraction.md` work item 2) without breaking the
 * currently-shipping renderer, which still expects `ipcRenderer.on("sync:status", ...)` etc.
 * When P6 wires up the real transport, this file is deleted wholesale and the shell instead
 * gets these events over the RPC connection.
 *
 * `entity.changed` intentionally degrades back to the bare `companion:refresh` signal here —
 * the richer payload is for future clients (P4/P7); today's renderer only knows how to
 * "refetch everything", so that is what it gets, byte-identical to before this file existed.
 */
import { BrowserWindow } from "electron";
import { bus } from "@/events/bus";
import { logEmitter } from "@/proxy/logEmitter";

function broadcast(channel: string, payload?: unknown): void {
  for (const w of BrowserWindow.getAllWindows()) {
    if (!w.isDestroyed()) w.webContents.send(channel, payload);
  }
}

let wired = false;

/** Idempotent — safe to call more than once (mirrors `registerIpcHandlers()` being call-once
 * in practice, but avoids double-subscribing listeners if that ever changes). */
export function wireEventBridge(): void {
  if (wired) return;
  wired = true;

  bus.onTyped("sync.status", (state) => broadcast("sync:status", state));
  bus.onTyped("sync.entityStatus", ({ wsId, status }) => broadcast("sync:entityStatus", { wsId, status }));
  bus.onTyped("entity.changed", () => broadcast("companion:refresh"));
  bus.onTyped("webhook.payload", (payload) => broadcast("webhook:payload", payload));
  bus.onTyped("process.output", (chunk) => broadcast("app:log", chunk));
  bus.onTyped("process.statusChange", (state) => broadcast("app:statusChange", state));

  // `settings.changed` (tray update) is deliberately NOT subscribed here. Wiring it here would
  // reintroduce the exact `coreHandlers -> @/main` import cycle P2 is removing, just relocated
  // to this file instead of broken (`@/main` -> `@/ipc/handlers` -> `@/ipc/eventBridge` ->
  // `@/main`). `src/main.ts` — which already owns `updateTrayMenu()` and already sits at the
  // top of the dependency graph — subscribes to it directly instead.

  logEmitter.on("request", (entry) => broadcast("log:entry", entry));
  logEmitter.on("chunk", (chunk) => broadcast("log:chunk", chunk));
  logEmitter.on("server-error", (error) => broadcast("server:error", error));
}
