/**
 * P1 work item 6 — the event model.
 *
 * Seven events exist today, all delivered for free by being in-process:
 * `sync:status`, `sync:entityStatus`, `log:entry`, `log:chunk`, `server:error`,
 * `companion:refresh`, `webhook:payload`. Renamed here to the `event.<namespace>.<verb>`
 * convention (work item 3); `companion:refresh` becomes `entity.changed` per the P2 checklist
 * item "Replace `companion:refresh` with `entity.changed`".
 */
import { z } from "zod";

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

export const SyncEntityStatusEventSchema = z.object({
  wsId: z.string(),
  status: z.record(z.string(), z.enum(["clean", "modified", "new", "deleted"])),
});
export type SyncEntityStatusEvent = z.infer<typeof SyncEntityStatusEventSchema>;

export const LogEntryEventSchema = z.object({
  id: z.string(),
  timestamp: z.number(),
  method: z.string(),
  url: z.string(),
  status: z.number().nullable(),
  durationMs: z.number().nullable(),
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

export const WebhookPayloadEventSchema = z.object({
  webhookId: z.string(),
  method: z.string(),
  headers: z.record(z.string(), z.string()),
  body: z.string(),
  receivedAt: z.number(),
});
export type WebhookPayloadEvent = z.infer<typeof WebhookPayloadEventSchema>;

export const EVENT_NAMES = [
  "event.sync.status",
  "event.sync.entityStatus",
  "event.log.entry",
  "event.log.chunk",
  "event.server.error",
  "event.entity.changed",
  "event.webhook.payload",
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
