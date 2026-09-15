/**
 * `runner.*` — collection runner report storage and per-folder run configuration.
 * `runner.saveReport`/`runner.exportReport` currently take `report: any` — flagged in the P0
 * spike as needing a proper schema rather than carrying `z.unknown()` forward as a habit. Typed
 * loosely here (`RunReport` shape below) pending that follow-up; still an improvement over `any`
 * because the wire envelope itself is validated.
 */
import { z } from "zod";

const RunResult = z.object({
  requestName: z.string(),
  method: z.string(),
  status: z.number().nullable().optional(),
  responseTime: z.number().optional(),
  error: z.string().optional(),
  tests: z.array(z.object({ name: z.string(), passed: z.boolean(), error: z.string().optional() })).optional(),
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

/** SPLIT (work item 1): engine renders HTML/JSON content; the client owns the save dialog and
 * the actual file write, per `File_Ops_Protocol.md`. Returns rendered content, not a path. */
export const RunnerExportReportParams = z.object({
  report: RunReport,
  format: z.enum(["html", "json"]),
}).strict();
export type RunnerExportReportParams = z.infer<typeof RunnerExportReportParams>;
export interface RunnerExportReportResult {
  ok: boolean;
  content?: string;
  suggestedFilename?: string;
  error?: string;
}

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
