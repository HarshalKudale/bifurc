/**
 * `audit.*`, `history.*` — the audit log (all mutations) and per-file git history.
 * `audit.export` is SPLIT: engine renders the content, client owns the save dialog
 * (see `plan/handler-classification.md`).
 */
import { z } from "zod";
import type { ArtifactResult } from "./blob";

export const AuditEntrySchema = z.object({
  commitHash: z.string(),
  ts: z.number(),
  action: z.string(),
  entity: z.string(),
  entityId: z.string(),
  entityName: z.string(),
  workspaceId: z.string(),
  actor: z.string(),
});
export type AuditEntry = z.infer<typeof AuditEntrySchema>;

export const AuditListParams = z.object({
  workspaceId: z.string().optional(),
  entity: z.string().optional(),
  action: z.string().optional(),
  entityId: z.string().optional(),
  search: z.string().optional(),
  limit: z.number().int().optional(),
  offset: z.number().int().optional(),
}).strict();
export type AuditListParams = z.infer<typeof AuditListParams>;
export interface AuditListResult {
  entries: AuditEntry[];
  total: number;
}

export const AuditDiffParams = z.object({
  commitHash: z.string(),
  entity: z.string(),
  entityId: z.string(),
  workspaceId: z.string(),
}).strict();
export type AuditDiffParams = z.infer<typeof AuditDiffParams>;
export interface EntityDiffResult {
  before: unknown | null;
  after: unknown | null;
}

export const HistoryListParams = z.object({
  workspaceId: z.string().optional(),
  filePath: z.string(),
  limit: z.number().int().optional(),
  offset: z.number().int().optional(),
}).strict();
export type HistoryListParams = z.infer<typeof HistoryListParams>;
export interface HistoryListResult {
  entries: AuditEntry[];
  total: number;
}

export const HistoryDiffParams = z.object({
  commitHash: z.string(),
  filePath: z.string(),
  workspaceId: z.string(),
}).strict();
export type HistoryDiffParams = z.infer<typeof HistoryDiffParams>;

/** SPLIT — engine renders the content; client owns the save dialog. */
export const AuditExportParams = z.object({
  format: z.enum(["json", "csv"]),
}).strict();
export type AuditExportParams = z.infer<typeof AuditExportParams>;
/**
 * The one egress artifact with no natural size ceiling: the handler queries with `limit: 0`, i.e.
 * *every* entry the active workspace has ever recorded. That is precisely the payload the
 * inline/blob threshold exists for, so this is the channel where `BlobRef` earns its keep.
 */
export type AuditExportResult = ArtifactResult;
