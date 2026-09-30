/**
 * `runner.*` — collection runner report storage and per-folder run configuration.
 * `runner.saveReport`/`runner.exportReport` take the renderer's `CollectionRunReport` — flagged in
 * the P0 spike as needing a proper schema rather than carrying `z.unknown()` forward as a habit.
 * Typed here (`RunReport` below) and **widened in P3** to carry every field the renderer's real
 * `RunnerRequestResult` has, because a schema narrower than its only caller silently truncates it.
 */
import { z } from "zod";
import type { ArtifactResult } from "./blob";
import { WorkspaceScope } from "./scope";

/**
 * **Exported since 2026-09-30** (`plan/19` WI 4) so that `events.ts` can carry the same shape on
 * `event.runner.progress` and `event.runner.finished`. A run's progress and its report describe the *same*
 * result object, and two declarations of it would be the "a shape that exists in two places" hazard this
 * file's own `RunnerConfigSchema` note warns about — the one that already cost this feature a truncation bug.
 */
export const RunReportResultSchema = z.object({
  /**
   * These four are carried by the renderer's real `RunnerRequestResult`
   * (`packages/gui/src/lib/collectionRunner.ts`) and were dropped by the P1 schema, which was written from
   * the HTML renderer's field *usage* rather than from the type. `z.object()` strips unknown keys,
   * so their absence silently truncated the **JSON** report export — the HTML renderer touches only
   * fields the schema happened to have, which is exactly why this would have shipped unnoticed.
   */
  requestId: z.string().optional(),
  url: z.string().optional(),
  testLogs: z.array(z.string()).optional(),
  preScriptError: z.string().optional(),
  postScriptError: z.string().optional(),
  requestName: z.string(),
  method: z.string(),
  status: z.number().nullable().optional(),
  responseTime: z.number().optional(),
  error: z.string().optional(),
  tests: z.array(z.object({
    name: z.string(),
    passed: z.boolean(),
    error: z.string().optional(),
    durationMs: z.number().optional(),
  })).optional(),
});

/** Exported with `RunReportResultSchema`, and for the same reason — `event.runner.finished` carries one. */
export const RunReportSchema = z.object({
  folderId: z.string(),
  folderName: z.string().optional(),
  startedAt: z.number(),
  completedAt: z.number().optional(),
  totalRequests: z.number().optional(),
  totalTests: z.number().optional(),
  passedTests: z.number().optional(),
  failedTests: z.number().optional(),
  results: z.array(RunReportResultSchema).optional(),
});

export const RunnerSaveReportParams = z.object({
  workspaceId: z.string(),
  report: RunReportSchema,
}).strict();
export type RunnerSaveReportParams = z.infer<typeof RunnerSaveReportParams>;
export interface RunnerSaveReportResult {
  ok: boolean;
  error?: string;
}

/** SPLIT — engine renders HTML/JSON; the client owns the save dialog and the file write, per
 * `File_Ops_Protocol.md` §4. Returns the artifact, never a path.
 *
 * `workspaceId` (P14 WI 8) is for the blob half: above `BLOB_INLINE_THRESHOLD_BYTES` a run report
 * stages a blob, and the client reads it scoped to its own workspace. A large report — which is the
 * case the threshold exists for — was therefore unreadable by the client that asked for it. */
export const RunnerExportReportParams = z.object({
  report: RunReportSchema,
  format: z.enum(["html", "json"]),
}).merge(WorkspaceScope).strict();
export type RunnerExportReportParams = z.infer<typeof RunnerExportReportParams>;
export type RunnerExportReportResult = ArtifactResult;

export const RunnerGetHistoryParams = z.object({
  workspaceId: z.string(),
  folderId: z.string(),
}).strict();
export type RunnerGetHistoryParams = z.infer<typeof RunnerGetHistoryParams>;
export interface RunnerHistoryEntry {
  timestamp: number;
  summary: { total: number; passed: number; failed: number };
}
export type RunnerGetHistoryResult = RunnerHistoryEntry[];

/**
 * A folder's saved runner configuration.
 *
 * **Widened 2026-09-19 from `{requestOrder: string[]; delayMs: number}`** — that frozen shape
 * described a payload no caller sends, and it was wrong in two independent ways:
 *
 * 1. It **required** `requestOrder`. `tests/integration/runnerStorage.integration.test.ts:207`
 *    round-trips `{delayMs: 250, stopOnFailure: true, iterations: 3}` and line 213 saves
 *    `{delayMs: 0}`, so the schema rejected both — with a *passing test* as the evidence, which is
 *    why the step-3b ratchet filed these two commands `BLOCKED` rather than unimplemented.
 * 2. It **stripped** everything else. `z.object()` drops unknown keys, so even a payload that passed
 *    would have lost `stopOnFailure` and `iterations` on the way to disk. That half is the more
 *    dangerous one: a dropped field is a data-loss bug in saved run configuration, not a validation
 *    error, and nothing would have reported it.
 *
 * The real shape is a free-form blob, because that is what the store actually persists — the shell's
 * `runner:saveConfig` body is `JSON.stringify(config)` with `config: any` and no validation, and
 * different callers send different subsets (`useCollectionRunner.ts:67` sends `{requestOrder,
 * delayMs}`; the integration test sends neither `requestOrder` nor the same field set twice).
 *
 * So this follows the precedent `entity.ts` set for exactly this situation: a large, evolving,
 * caller-owned shape stays `z.record` at the wire boundary and the domain layer owns validation.
 * `AppConfig` (`config.ts`) and `ApplicationConfig` (`application.ts`) are the same call.
 */
export const RunnerConfigSchema = z.record(z.string(), z.unknown());

export const RunnerSaveConfigParams = z.object({
  workspaceId: z.string(),
  folderId: z.string(),
  config: RunnerConfigSchema,
}).strict();
export type RunnerSaveConfigParams = z.infer<typeof RunnerSaveConfigParams>;

export const RunnerLoadConfigParams = z.object({
  workspaceId: z.string(),
  folderId: z.string(),
}).strict();
export type RunnerLoadConfigParams = z.infer<typeof RunnerLoadConfigParams>;
export type RunnerLoadConfigResult = z.infer<typeof RunnerConfigSchema> | null;

export const RunnerListFolderIdsParams = z.object({
  workspaceId: z.string(),
}).strict();
export type RunnerListFolderIdsParams = z.infer<typeof RunnerListFolderIdsParams>;
export type RunnerListFolderIdsResult = string[];

// ─────────────────────────────────────────────────────────────────────────────
// The run's own commands — plan/19 WI 4 (D29/D30)
// ─────────────────────────────────────────────────────────────────────────────

/**
 * `runner.start` — run a folder's ordered requests, in the engine.
 *
 * **It returns as soon as the run is registered, not when it finishes.** That is D29 in the small: a caller
 * that had to await the whole run would be a caller that owns its lifetime, and the point is that the engine
 * does. Progress arrives as `event.runner.progress` and the end as `event.runner.finished`, so a client that
 * disconnects and reconnects loses nothing it cannot recover from `runner.status`.
 *
 * `total` is in the result rather than left to the first progress event, so a UI can render "0 / n" before
 * anything has happened.
 *
 * **`{ok:false}` rather than a throw for "already running"** (D30): the caller asked a question — *may I start
 * this?* — and "no, because X" is an answer. A throw would make it an error envelope, which the transport
 * shapes differently and every client would have to unwrap separately.
 */
export const RunnerStartParams = z.object({
  workspaceId: z.string(),
  folderId: z.string(),
}).strict();
export type RunnerStartParams = z.infer<typeof RunnerStartParams>;
export interface RunnerStartResult {
  ok: boolean;
  /** How many requests the run intends to send. Present when `ok`. */
  total?: number;
  error?: string;
}

/**
 * `runner.cancel` — stop a run at its next iteration boundary.
 *
 * `{ok:false}` when there is no run for the folder. Cancelling a *finished* run answers `{ok:true}`: the
 * caller's intent is satisfied either way, and the alternative is a race every client would have to defend
 * against.
 */
export const RunnerCancelParams = z.object({
  workspaceId: z.string(),
  folderId: z.string(),
}).strict();
export type RunnerCancelParams = z.infer<typeof RunnerCancelParams>;
export interface RunnerCancelResult {
  ok: boolean;
  error?: string;
}

/**
 * `runner.status` — the run's state, running or finished, or `null` if it never started.
 *
 * **This is what makes D29 usable.** A client that disconnected mid-run asks this on reconnect and gets the
 * results so far, whether the run is still going, and — once it has ended — the report. `null` is distinct
 * from "finished with nothing": the first means the engine has never been asked to run this folder, the second
 * means it ran and the answer is in `report`.
 */
export const RunnerStatusParams = z.object({
  workspaceId: z.string(),
  folderId: z.string(),
}).strict();
export type RunnerStatusParams = z.infer<typeof RunnerStatusParams>;

/**
 * The run's state. Mirrors `RunSnapshot` in `packages/engine/src/runner/runRegistry.ts`, **which is the
 * authority** — the same relationship the storage commands above have with the files they write.
 */
export interface RunnerStatusResult {
  workspaceId: string;
  folderId: string;
  startedAt: number;
  total: number;
  completed: number;
  running: boolean;
  cancelled: boolean;
  completedAt?: number;
  error?: string;
  results: z.infer<typeof RunReportResultSchema>[];
  report?: z.infer<typeof RunReportSchema>;
}
