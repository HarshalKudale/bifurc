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

export const RunnerConfigSchema = z.object({
  requestOrder: z.array(z.string()),
  delayMs: z.number(),
});

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
