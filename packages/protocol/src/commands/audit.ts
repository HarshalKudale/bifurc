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

/**
 * `audit.list` — the audit log query.
 *
 * **Widened 2026-09-19.** `filePath` / `fromTs` / `toTs` were missing, and the reason they had to be
 * added rather than routed around is that `QueryLogOptions` — the type the handler actually takes —
 * has all three. `.strict()` turns that gap into a `BAD_REQUEST` rather than a silently ignored
 * filter, so the schema was rejecting payloads the real caller sends:
 * `renderer/panels/AuditLogPanel.tsx:61–62` sets `fromTs` and `toTs` from the panel's date filter.
 *
 * Worth recording *how* this was mis-classified, because the mistake is instructive: the ratchet in
 * `tests/ipc/handlers.test.ts` filed it `NARROWED` and called the narrowing "**latent, not live**",
 * on the grounds that no renderer code calls `listAudit`. That check was a grep for the *method
 * name*, and the panel reaches it through `window.api` under a different call shape — so the
 * "nothing currently passes those" conclusion was exactly backwards, and routing this command before
 * the widening would have broken the date filter on the audit panel.
 */
export const AuditListParams = z.object({
  workspaceId: z.string().optional(),
  entity: z.string().optional(),
  action: z.string().optional(),
  entityId: z.string().optional(),
  /** Filter by a specific entity file, workspace-relative (e.g. `mocks/root/mock_abc.json`). */
  filePath: z.string().optional(),
  /** Inclusive lower bound on the entry timestamp, epoch ms. */
  fromTs: z.number().optional(),
  /** Inclusive upper bound on the entry timestamp, epoch ms. */
  toTs: z.number().optional(),
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
