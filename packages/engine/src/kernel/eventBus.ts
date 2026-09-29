/**
 * P2 work item 1 — the EventBus.
 *
 * Copies `logEmitter.ts`'s pattern (a plain `EventEmitter`, no Electron) rather than
 * inventing a new one — see `plan/03-phase-2-engine-extraction.md` work item 1. This is the
 * single place every engine module emits domain events; nothing in this file imports
 * `electron`. The Electron shell subscribes to this bus and forwards to `webContents` — as of P6
 * step 3c that is `src/ipc/rpcBridge.ts`, which broadcasts every subscribed event over
 * `EVENT_CHANNEL`; the `eventBridge.ts` that used to do it was deleted. P4's transport subscribes to
 * it too, for remote clients.
 *
 * Once `packages/engine` exists as its own package (the rest of P2's work item 8), this file
 * moves there verbatim — it already has zero Electron coupling.
 *
 * **Since engine-layout phase 8 it also has zero *value* imports from the rest of the engine**, which
 * is §5.1's `kernel/` rule and the property §5.2 calls what makes the layering rule mechanically
 * checkable. See the note further down about where `emitEntityStatus()` went.
 */
import { EventEmitter } from "events";
import { logEmitter, type RequestLogEntry } from "./telemetry";
import type { WebhookPayload } from "../webhooks/webhookServer";

export interface EngineEvents {
  "sync.status": { wsId: string; status: string; error?: string | null; updatedIds?: string[] };
  /**
   * `status` is the per-file map (`relPath` → dirty marker); `git` is the workspace's position
   * relative to its remote. Both are carried because both are needed to decide what a given entity
   * should be offered — see `sync/gitState.ts` for why the file map alone cannot answer it.
   */
  "sync.entityStatus": {
    wsId: string;
    status: Record<string, string>;
    git?: { hasRemote: boolean; branch: string | null; ahead: boolean; behind: boolean };
  };
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
 * ⚠ `emitEntityStatus()` used to live here and **does not any more** — engine-layout phase 8's
 * follow-up moved it to `runtime/entityStatus.ts`. It read `getWorkspaceSyncStatus()` and
 * `getWorkspaceGitState()` from `gitops/`, which made this file import two **values** from a feature
 * module and so made §5.1's *"`kernel/` imports nothing from the engine"* false. **This file now has
 * exactly one engine-internal import, and it is a type** (`WebhookPayload`, erased at compile time).
 * Keep it that way: a value import here is a layering regression, and the ratchet will say so.
 */

/**
 * Which buses already have the log wiring, so a second call is a no-op. See `wireLogEventsToBus`.
 */
const wiredLogBuses = new WeakSet<EngineEventBus>();

/**
 * P6 finding 1 — put the three log events **on the bus**, where a transport can see them.
 *
 * ## The defect this fixes
 *
 * `EngineEvents` declares `log.entry`, `log.chunk` and `server.error`; `ENGINE_EVENT_NAMES` lists all
 * three; `BUS_NAME_BY_WIRE_NAME` maps them, `assertBridgeIsTotal()` asserts them at import time, and
 * the P4 conformance suite has cases for them. **Nothing ever emitted them.** They travelled only
 * through the separate `logEmitter` EventEmitter, and the shell's `eventBridge.ts` — the file step 3c
 * deleted — subscribed to `logEmitter` **directly**.
 *
 * So the moment the renderer's subscriptions move onto the transport, `onLogEntry`, `onLogChunk` and
 * `onServerError` have nothing to deliver: the capture panel, the request-log panel and the
 * server-error banner all go dead at once. **And every unit test stays green**, because they test the
 * bridge's *mapping* rather than its *traffic* — which is why this stayed invisible until the shell
 * was read end to end.
 *
 * It belongs in the engine rather than in each shell: P7's web UI, P8's CLI and P9's Docker image all
 * need the same three events, and three copies of this wiring is three places to forget it. It is the same
 * shape of fix `emitEntityStatus()` was — one shared implementation replacing several per-shell copies —
 * though that function has since moved to `runtime/entityStatus.ts`, so this file is now the bus and only
 * the bus.
 *
 * ## Idempotent per bus, deliberately
 *
 * The bus is a process-wide singleton and **two callers are expected to wire it**: `createEngine()`
 * and the shell's `registerIpcHandlers()`. (Today only one runs — the shell uses the module singletons
 * and never calls `createEngine()` — but a consumer that does both must not receive every log entry
 * twice, and the symptom of that would be duplicated rows in the capture panel rather than anything
 * that reads as a wiring bug.) A second call for an already-wired bus is a no-op returning a no-op
 * detacher; detaching clears the flag, so a later re-wire works.
 *
 * ## `source` is injectable for the same reason `createInProcessTransport`'s `bus` is
 *
 * `logEmitter` is also a process-wide singleton, so a test using the default would leave listeners
 * attached for every later test file in the Vitest worker — the leak `EngineEventBus`'s own comment
 * describes, one layer down.
 *
 * @returns a detacher. Safe to call more than once.
 */
export function wireLogEventsToBus(
  target: EngineEventBus = bus,
  source: typeof logEmitter = logEmitter,
): () => void {
  if (wiredLogBuses.has(target)) return () => {};
  wiredLogBuses.add(target);

  /**
   * The three names on the left are `logEmitter`'s, not the bus's: it emits `"request"` / `"chunk"` /
   * `"server-error"` while the bus calls them `"log.entry"` / `"log.chunk"` / `"server.error"`.
   *
   * Unlike the bus-name ↔ wire-name bridge in `transport/types.ts`, this mapping is written out by
   * hand and cannot be derived: those two sets differ by a uniform `event.` prefix, whereas
   * `"request"` and `"log.entry"` share nothing to strip. That is also why the bus names are the
   * ones the transport already knows — the odd name out is the emitter's, and it is not worth
   * renaming, since `logEmitter` is engine-internal and has been emitting these three names since
   * before the bus existed.
   *
   * The payloads need no translation. In particular `log.entry` carries **one** `RequestLogEntry`
   * here, not the `{entries: [...]}` batch: batching is a *wire* concern owned by `eventPump.ts`'s
   * `toClientEvent`, which every serialising transport applies. Putting the batch shape on the bus
   * would hand every in-process subscriber an envelope no engine code produces.
   */
  const onRequest = (entry: RequestLogEntry): void => target.emitTyped("log.entry", entry);
  const onChunk = (chunk: { logId: string; chunk: string; done: boolean }): void =>
    target.emitTyped("log.chunk", chunk);
  // `server.ts` emits this from the `listen()` error branch, where `lastError` has just been assigned
  // a message — never the `null` it is reset to on success. Hence `string`, matching `EngineEvents`.
  const onServerError = (message: string): void => target.emitTyped("server.error", message);

  source.on("request", onRequest);
  source.on("chunk", onChunk);
  source.on("server-error", onServerError);

  let detached = false;
  return () => {
    if (detached) return;
    detached = true;
    wiredLogBuses.delete(target);
    source.off("request", onRequest);
    source.off("chunk", onChunk);
    source.off("server-error", onServerError);
  };
}
