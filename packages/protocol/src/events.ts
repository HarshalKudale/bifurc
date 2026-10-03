/**
 * P1 work item 6 — the event model.
 *
 * Seven events existed at P1, all delivered for free by being in-process:
 * `sync:status`, `sync:entityStatus`, `log:entry`, `log:chunk`, `server:error`,
 * `companion:refresh`, `webhook:payload`. Renamed here to the `event.<namespace>.<verb>`
 * convention (work item 3); `companion:refresh` becomes `entity.changed` per the P2 checklist
 * item "Replace `companion:refresh` with `entity.changed`".
 *
 * P4 work item 3 added the last two, `process.output` and `settings.changed`. They had existed on
 * the engine bus since P2 but had **no wire name**, which meant a client could not subscribe to
 * them at all — the engine emitted them and the shell consumed them in-process, so nothing noticed.
 * `BUS_EVENTS_NOT_ON_THE_WIRE` in `packages/engine/src/transport/types.ts` is what made the gap
 * visible rather than silent, and it is now down to `process.statusChange` alone.
 */
import { z } from "zod";
import { RunReportResultSchema, RunReportSchema } from "./commands/runner";
import { WsConnectionStateSchema, WsMessageSchema } from "./commands/ws";


/** Every event carries a monotonic `seq`. See "Sequence numbers" below for why. */
export const EventEnvelopeSchema = z.object({
  event: z.string(),
  seq: z.number().int().nonnegative(),
  payload: z.unknown(),
});
export type EventEnvelope<T = unknown> = { event: string; seq: number; payload: T };

// ── Event payload schemas ────────────────────────────────────────────────────

export const SyncStatusEventSchema = z.object({
  wsId: z.string(),
  status: z.enum(["idle", "syncing", "error"]),
  error: z.string().nullable().optional(),
});
export type SyncStatusEvent = z.infer<typeof SyncStatusEventSchema>;

export const WorkspaceGitStateSchema = z.object({
  hasRemote: z.boolean(),
  branch: z.string().nullable(),
  ahead: z.boolean(),
  behind: z.boolean(),
});

/**
 * `git` is **optional**, and that is a compatibility decision rather than a convenience.
 *
 * The field was added 2026-09-28 alongside the publish/sync/revert unification, and the file map
 * alone was the whole contract before it. A subscriber written against the old shape must keep
 * working — and, more importantly, a *producer* that cannot read git state (a workspace with no
 * repo) must be able to say "no opinion" rather than assert `hasRemote: false`, which a consumer
 * would read as "there is definitely no remote". Absent means unknown; present means known.
 */
export const SyncEntityStatusEventSchema = z.object({
  wsId: z.string(),
  status: z.record(z.string(), z.enum(["clean", "modified", "new", "deleted"])),
  git: WorkspaceGitStateSchema.optional(),
});
export type SyncEntityStatusEvent = z.infer<typeof SyncEntityStatusEventSchema>;

export const LogEntryEventSchema = z.object({
  id: z.string(),
  timestamp: z.number(),
  method: z.string(),
  url: z.string(),
  status: z.number().nullable(),
  durationMs: z.number().nullable(),
  workspaceId: z.string().optional(),
  via: z.string().optional(),
  origin: z.enum(["forward", "reverse"]).optional(),
  target: z.string().nullable().optional(),
  contentType: z.string().optional(),
  size: z.number().optional(),
});
export type LogEntryEvent = z.infer<typeof LogEntryEventSchema>;

/** `log:entry` fires per proxied request; coalesce into batches (see "Backpressure" below). */
export const LogEntryBatchEventSchema = z.object({
  entries: z.array(LogEntryEventSchema),
});
export type LogEntryBatchEvent = z.infer<typeof LogEntryBatchEventSchema>;

export const LogChunkEventSchema = z.object({
  logId: z.string(),
  chunk: z.string(),
  done: z.boolean(),
});
export type LogChunkEvent = z.infer<typeof LogChunkEventSchema>;

export const ServerErrorEventSchema = z.object({
  error: z.string(),
});
export type ServerErrorEvent = z.infer<typeof ServerErrorEventSchema>;

/** Replaces `companion:refresh` — carries what changed instead of "something changed, refetch". */
export const EntityChangedEventSchema = z.object({
  wsId: z.string(),
  kind: z.string(),
  id: z.string().optional(),
  action: z.enum(["created", "updated", "deleted"]),
});
export type EntityChangedEvent = z.infer<typeof EntityChangedEventSchema>;

/**
 * A child process writing to its stdout/stderr, or the engine writing a system line about it.
 *
 * Mirrors the engine bus's `process.output` payload exactly, field for field — this is a *name* being
 * added to an event that already exists, not a new event, so the two must not be allowed to drift.
 * `ts` is engine-side epoch milliseconds: a remote client cannot compute it, and a client-side
 * timestamp would be wrong by the network delay, which matters when the whole point of the stream is
 * to show the user when their process said something.
 */
export const ProcessOutputEventSchema = z.object({
  appId: z.string(),
  stream: z.enum(["stdout", "stderr", "system"]),
  data: z.string(),
  ts: z.number(),
});
export type ProcessOutputEvent = z.infer<typeof ProcessOutputEventSchema>;

/**
 * Settings changed, so anything holding a cached view of them should re-read.
 *
 * The payload is deliberately **empty**, matching the bus's `Record<string, never>`. It replaced a
 * direct `updateTrayMenu()` call from the engine into the shell (`main.ts`), and the shell's handler
 * takes no arguments — so there is nothing to carry. `plan/05` item 3's "carry what changed instead
 * of 'something changed, refetch'" principle applies to `entity.changed`; applying it here would mean
 * changing the emit site and its consumer for no gain, since the only consumer re-reads everything
 * anyway.
 *
 * `z.object({})` rather than `z.unknown()`: it accepts `{}` and **strips** anything else, so a client
 * cannot smuggle a payload through an event documented as having none.
 */
export const SettingsChangedEventSchema = z.object({});
export type SettingsChangedEvent = z.infer<typeof SettingsChangedEventSchema>;

/**
 * `event.runner.progress` — one request of a collection run has finished (`plan/19` WI 4).
 *
 * `index` is 0-based and matches the position in the finished report's `results`, so a client can place an
 * arriving result without counting. **`total` rides on every event rather than being remembered from the start
 * call**, because a client that attaches mid-run (`runner.status`, D29) never saw one.
 *
 * The bus name is `runner.progress` — the bridge derives it from `EVENT_NAMES` by stripping `event.`, so
 * there is no second table to keep in step (`wire/types.ts`).
 */
export const RunnerProgressEventSchema = z.object({
  workspaceId: z.string(),
  folderId: z.string(),
  index: z.number(),
  total: z.number(),
  result: RunReportResultSchema,
});
export type RunnerProgressEvent = z.infer<typeof RunnerProgressEventSchema>;

/**
 * `event.runner.finished` — a collection run ended, cancelled or not.
 *
 * **The whole report is carried**, and it is the same shape `runner.saveReport` takes. A client that was
 * disconnected for the run's entire life still gets its results from this one event, and a client that watched
 * the whole thing gets the object it would otherwise have assembled itself — which is what makes the event the
 * *only* thing a runner UI needs to render from.
 */
export const RunnerFinishedEventSchema = z.object({
  workspaceId: z.string(),
  folderId: z.string(),
  report: RunReportSchema,
});
export type RunnerFinishedEvent = z.infer<typeof RunnerFinishedEventSchema>;

/**
 * `event.ws.status` — a socket changed state (`plan/19` WI 6).
 *
 * **The payload carries `socketId`, and that is not decoration.** Every socket in the process emits on one
 * bus, so a subscriber that did not know which socket an event belonged to could only guess — and the
 * renderer has one hook instance per tab, all subscribed to the same two events.
 *
 * **The full state rides the event rather than a delta**, because a client that attached mid-stream never
 * saw the connect call: `status`, `error` and the close code are what it needs to render the dot and the
 * message line, and none of them can be recovered from "the state changed".
 */
export const WsStatusEventSchema = z.object({
  workspaceId: z.string(),
  socketId: z.string(),
  status: WsConnectionStateSchema,
  error: z.string().nullable(),
  /** Present once the socket has closed. */
  code: z.number().optional(),
  reason: z.string().optional(),
});
export type WsStatusEvent = z.infer<typeof WsStatusEventSchema>;

/**
 * `event.ws.message` — one frame, in either direction.
 *
 * The whole `WsMessage` is carried, `id` and `ts` included, so a client that re-attaches can place an
 * arriving frame against the retained log `ws.status` returned instead of appending a duplicate. `direction`
 * is what separates the two panes the socket editor renders.
 */
export const WsMessageEventSchema = z.object({
  workspaceId: z.string(),
  socketId: z.string(),
  message: WsMessageSchema,
});
export type WsMessageEvent = z.infer<typeof WsMessageEventSchema>;

/**
 * `event.engine.stopping` — the engine is about to stop, on purpose (`engine.shutdown`, or Quit in its tray
 * menu). Sent before the endpoint closes.
 *
 * `restart` is what an interface acts on: `false` means the engine is going away and every interface closes
 * with it; `true` means a successor is starting for the same data directory, so a client waits for the new
 * `runtime.json` and reconnects. A connection that drops with no such event before it is a crash.
 */
export const EngineStoppingEventSchema = z.object({ restart: z.boolean() });
export type EngineStoppingEvent = z.infer<typeof EngineStoppingEventSchema>;

/**
 * `event.ui.activate` — the user asked for the interface (Open in the engine's tray menu) while one is
 * already connected. A graphical client brings its window to the front; nothing is carried.
 */
export const UiActivateEventSchema = z.object({});
export type UiActivateEvent = z.infer<typeof UiActivateEventSchema>;

/** `event.grpc.stream.message` — one message of a live gRPC call, in either direction. */
export const GrpcStreamMessageEventSchema = z.object({
  workspaceId: z.string(),
  streamId: z.string(),
  message: z.object({
    id: z.string(),
    ts: z.number(),
    direction: z.enum(["sent", "received"]),
    data: z.string(),
  }),
});
export type GrpcStreamMessageEvent = z.infer<typeof GrpcStreamMessageEventSchema>;

/**
 * `event.grpc.stream.status` — a live gRPC call started or finished. `open` carries the response headers once
 * they arrive; `closed` carries the gRPC status the call ended with.
 */
export const GrpcStreamStatusEventSchema = z.object({
  workspaceId: z.string(),
  streamId: z.string(),
  status: z.enum(["open", "closed"]),
  grpcStatus: z.number().optional(),
  statusMessage: z.string().optional(),
  error: z.string().nullable(),
  metadata: z.record(z.string(), z.string()).optional(),
});
export type GrpcStreamStatusEvent = z.infer<typeof GrpcStreamStatusEventSchema>;

export const EVENT_NAMES = [
  "event.sync.status",
  "event.sync.entityStatus",
  "event.log.entry",
  "event.log.chunk",
  "event.server.error",
  "event.entity.changed",
  "event.process.output",
  "event.settings.changed",
  "event.runner.progress",
  "event.runner.finished",
  // `plan/19` WI 6 — the socket's two events. The list is total, so adding these is a deliberate protocol
  // change rather than a side effect of the module landing: a socket whose state the UI cannot watch would be
  // a connection the engine owns and nobody can render.
  "event.ws.status",
  "event.ws.message",
  "event.grpc.stream.status",
  "event.grpc.stream.message",
  // The engine's own lifetime, and its tray asking a connected interface to come forward.
  "event.engine.stopping",
  "event.ui.activate",
] as const;
export type EventName = (typeof EVENT_NAMES)[number];

/*
 * Sequence numbers
 * ----------------
 * On reconnect the client sends its last-seen `seq`; the engine replays from there or responds
 * `resyncRequired` (below) if replay is impossible (buffer overrun). Without this, a brief
 * network blip silently loses log entries and leaves the UI showing stale state forever — the
 * single most likely source of "works locally, broken remotely" bugs (work item 6).
 *
 * Backpressure and coalescing
 * ---------------------------
 * - `event.log.entry`: coalesce into batches of up to 100 entries or 250ms, whichever is first —
 *   see `LogEntryBatchEventSchema`.
 * - `event.log.chunk`: keep streaming, but P4 must add a flow-control window so the engine can
 *   pause a chatty child process.
 * - Documented drop policy under sustained overload: drop `event.log.entry` before
 *   `event.server.error`.
 */

export const ResyncRequiredEventSchema = z.object({
  event: z.string(),
  reason: z.enum(["buffer_overrun", "unknown_seq"]),
});
export type ResyncRequiredEvent = z.infer<typeof ResyncRequiredEventSchema>;
