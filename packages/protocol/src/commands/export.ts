/**
 * `export.*`, `import.*` — the import/export layer. Renamed from `importExport:*` per the
 * naming convention (work item 3). Full blob-layer rewiring is P3's job
 * (`File_Ops_Protocol.md`); this is the P1 schema shape, already `blobId`-shaped rather than
 * `filePath`-shaped so P3 does not have to redesign the wire contract, only implement the blob
 * store underneath it.
 */
import { z } from "zod";

export const ExportFormatsParams = z.object({}).strict();
export type ExportFormatsParams = z.infer<typeof ExportFormatsParams>;
export type ExportFormatsResult = Record<string, { id: string; label: string; extensions: string[] }[]>;

export const ExportCreateParams = z.object({
  kind: z.string(),
  format: z.string(),
  workspaceId: z.string(),
}).strict();
export type ExportCreateParams = z.infer<typeof ExportCreateParams>;
export interface ExportCreateResult {
  ok: boolean;
  /** P3: a blob reference, not a filesystem path. Pre-P3 engine code still returns `filePath`. */
  blobId?: string;
  filePath?: string;
  error?: string;
  canceled?: boolean;
}

export const ImportPreflightParams = z.object({
  kind: z.string(),
  format: z.string(),
  workspaceId: z.string(),
  blobId: z.string().optional(),
  filePath: z.string().optional(), // pre-P3
}).strict();
export type ImportPreflightParams = z.infer<typeof ImportPreflightParams>;
export interface ImportPreflightResult {
  ok: boolean;
  blobId?: string;
  filePath?: string;
  itemCount?: number;
  collisionIds?: string[];
  error?: string;
  canceled?: boolean;
}

export const ImportCommitParams = z.object({
  kind: z.string(),
  format: z.string(),
  workspaceId: z.string(),
  blobId: z.string().optional(),
  filePath: z.string().optional(), // pre-P3
  collisionStrategy: z.enum(["skip", "overwrite", "rename"]).optional(),
}).strict();
export type ImportCommitParams = z.infer<typeof ImportCommitParams>;
export interface ImportCommitResult {
  ok: boolean;
  imported?: number;
  skipped?: number;
  error?: string;
}
