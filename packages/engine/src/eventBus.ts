/**
 * P2 work item 1 — the EventBus.
 *
 * Copies `proxy/logEmitter.ts`'s pattern (a plain `EventEmitter`, no Electron) rather than
 * inventing a new one — see `plan/03-phase-2-engine-extraction.md` work item 1. This is the
 * single place every engine module emits domain events; nothing in this file imports
 * `electron`. The Electron shell (today: `src/ipc/eventBridge.ts`, pre-P6) subscribes to this
 * bus and forwards to `webContents`. P4's transport subscribes to it too, for remote clients.
 *
 * Once `packages/engine` exists as its own package (the rest of P2's work item 8), this file
 * moves there verbatim — it already has zero Electron coupling.
 */
import { EventEmitter } from "events";
import type { RequestLogEntry } from "./proxy/logEmitter";
import type { WebhookPayload } from "./proxy/webhookServer";
import { getWorkspaceSyncStatus } from "./sync/statusTracker";

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

/**
 * The runtime counterpart of `EngineEvents`' keys.
 *
 * `EngineEvents` is a type, so it vanishes at runtime and nothing can enumerate it — which means the
 * P4 transport's bus-name ↔ wire-name bridge (`transport/types.ts`) has nothing to validate itself
 * against. Without this list, `assertBridgeIsTotal()` would be checking a protocol-derived map
 * against an empty set and would pass no matter how wrong the mapping was.
 *
 * Typed as `readonly (keyof EngineEvents)[]`, so adding a member to `EngineEvents` and forgetting to
 * list it here is a compile error, and adding it here but not to the interface is one too. Keep the
 * two in step; `transport/types.ts` fails at import time if a bus event here has no wire name and is
 * not in its explicit `BUS_EVENTS_NOT_ON_THE_WIRE` list.
 */
export const ENGINE_EVENT_NAMES: readonly (keyof EngineEvents)[] = [
  "sync.status",
  "sync.entityStatus",
  "log.entry",
  "log.chunk",
  "server.error",
  "entity.changed",
  "webhook.payload",
  "process.output",
  "process.statusChange",
  "settings.changed",
];

/**
 * Exported (rather than left module-private) so that a **test** can create an isolated bus.
 *
 * The `bus` singleton below is process-wide by design, and Vitest shares a module registry across
 * every test file in a worker. That makes the singleton the wrong thing for the P4 conformance suite
 * to assert listener accounting against: a subscription leaked by one test is still attached when the
 * next one runs, so `listenerCount()` drifts and the failure looks like a transport bug in whichever
 * test happens to run second. `createInProcessTransport()` takes an optional `bus` for exactly this
 * reason, and this export is what makes that option usable.
 */
export class EngineEventBus extends EventEmitter {
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
 * fourth, slightly different copy in `companionServer.ts`) — each reaching into the shell's open
 * windows directly. One implementation, emitted on the bus; the shell's `eventBridge.ts` (pre-P6)
 * is the only remaining place that fans an event out to windows.
 */
export function emitEntityStatus(wsId: string): void {
  getWorkspaceSyncStatus(wsId)
    .then((status) => {
      bus.emitTyped("sync.entityStatus", { wsId, status: JSON.parse(JSON.stringify(status)) });
    })
    .catch(() => { /* best-effort, matches prior behaviour */ });
}
