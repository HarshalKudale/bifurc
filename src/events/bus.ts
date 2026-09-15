/**
 * P2 work item 1 — the EventBus.
 *
 * Copies `src/proxy/logEmitter.ts`'s pattern (a plain `EventEmitter`, no Electron) rather than
 * inventing a new one — see `plan/03-phase-2-engine-extraction.md` work item 1. This is the
 * single place every engine module emits domain events; nothing in this file imports
 * `electron`. The Electron shell (today: `src/ipc/eventBridge.ts`, pre-P6) subscribes to this
 * bus and forwards to `webContents`. P4's transport subscribes to it too, for remote clients.
 *
 * Once `packages/engine` exists as its own package (the rest of P2's work item 8), this file
 * moves there verbatim — it already has zero Electron coupling.
 */
import { EventEmitter } from "events";
import type { RequestLogEntry } from "@/proxy/logEmitter";
import type { WebhookPayload } from "@/proxy/webhookServer";
import { getWorkspaceSyncStatus } from "@/sync/statusTracker";

export interface EngineEvents {
  "sync.status": { wsId: string; status: string; error?: string | null; updatedIds?: string[] };
  "sync.entityStatus": { wsId: string; status: Record<string, string> };
  "log.entry": RequestLogEntry;
  "log.chunk": { logId: string; chunk: string; done: boolean };
  "server.error": string;
  /** Replaces `companion:refresh` — carries what changed instead of "something changed, re-fetch". */
  "entity.changed": { wsId: string; kind: string; id?: string; action: "created" | "updated" | "deleted" };
  "webhook.payload": WebhookPayload;
  "process.output": { appId: string; stream: "stdout" | "stderr" | "system"; data: string; ts: number };
  "process.statusChange": { appId: string; status: string; [key: string]: unknown };
  /** Was a direct `updateTrayMenu()` call from `coreHandlers.ts` (an engine->shell import cycle).
   * The shell subscribes and updates its own tray; the engine no longer imports `@/main`. */
  "settings.changed": Record<string, never>;
}

class EngineEventBus extends EventEmitter {
  emitTyped<K extends keyof EngineEvents>(event: K, payload: EngineEvents[K]): void {
    this.emit(event, payload);
  }

  onTyped<K extends keyof EngineEvents>(event: K, listener: (payload: EngineEvents[K]) => void): this {
    this.on(event, listener);
    return this;
  }

  offTyped<K extends keyof EngineEvents>(event: K, listener: (payload: EngineEvents[K]) => void): this {
    this.off(event, listener);
    return this;
  }
}

/** Singleton — one bus per process, matching `logEmitter`'s existing pattern. */
export const bus = new EngineEventBus();

/**
 * Shared replacement for the THREE near-identical local `broadcastEntityStatus()` functions
 * that used to live in `coreHandlers.ts`, `entityCrudFactory.ts`, and `syncHandlers.ts` (plus a
 * fourth, slightly different copy in `companionServer.ts`) — each reaching into
 * `BrowserWindow.getAllWindows()` directly. One implementation, emitted on the bus; the shell's
 * `eventBridge.ts` (pre-P6) is the only remaining place that touches `BrowserWindow` for this.
 */
export function emitEntityStatus(wsId: string): void {
  getWorkspaceSyncStatus(wsId)
    .then((status) => {
      bus.emitTyped("sync.entityStatus", { wsId, status: JSON.parse(JSON.stringify(status)) });
    })
    .catch(() => { /* best-effort, matches prior behaviour */ });
}
