/**
 * `runner.*` — collection runner report storage and per-folder run configuration.
 * `runner.saveReport`/`runner.exportReport` take the renderer's `CollectionRunReport` — flagged in
 * the P0 spike as needing a proper schema rather than carrying `z.unknown()` forward as a habit.
 * Typed here (`RunReport` below) and **widened in P3** to carry every field the renderer's real
 * `RunnerRequestResult` has, because a schema narrower than its only caller silently truncates it.
 */
import { z } from "zod";
import type { ArtifactResult } from "./blob";

const RunResult = z.object({
  /**
   * These four are carried by the renderer's real `RunnerRequestResult`
   * (`renderer/lib/collectionRunner.ts`) and were dropped by the P1 schema, which was written from
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

const RunReport = z.object({
  folderId: z.string(),
  folderName: z.string().optional(),
  startedAt: z.number(),
  completedAt: z.number().optional(),
  totalRequests: z.number().optional(),
  totalTests: z.number().optional(),
  passedTests: z.number().optional(),
  failedTests: z.number().optional(),
  results: z.array(RunResult).optional(),
});

export const RunnerSaveReportParams = z.object({
  workspaceId: z.string(),
  report: RunReport,
}).strict();
export type RunnerSaveReportParams = z.infer<typeof RunnerSaveReportParams>;
export interface RunnerSaveReportResult {
  ok: boolean;
  error?: string;
}

/** SPLIT — engine renders HTML/JSON; the client owns the save dialog and the file write, per
 * `File_Ops_Protocol.md` §4. Returns the artifact, never a path. */
export const RunnerExportReportParams = z.object({
  report: RunReport,
  format: z.enum(["html", "json"]),
}).strict();
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
