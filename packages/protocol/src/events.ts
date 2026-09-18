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

export const EVENT_NAMES = [
  "event.sync.status",
  "event.sync.entityStatus",
  "event.log.entry",
  "event.log.chunk",
  "event.server.error",
  "event.entity.changed",
  "event.webhook.payload",
  "event.process.output",
  "event.settings.changed",
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
