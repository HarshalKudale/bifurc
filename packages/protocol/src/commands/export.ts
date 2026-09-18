/**
 * `export.*`, `import.*` — the import/export layer. Renamed from `importExport:*` per the
 * naming convention (work item 3).
 *
 * **P3 work item 2 reshaped these.** P1 landed them "already `blobId`-shaped rather than
 * `filePath`-shaped so P3 does not have to redesign the wire contract, only implement the blob
 * store underneath it" — but two things were still wrong, and both are corrected here:
 *
 *  1. `collisionStrategy` was `["skip", "overwrite", "rename"]` while every real call site uses
 *     `["keep", "override", "new"]`. As frozen, `import.commit` would have rejected every real
 *     import outright. See `plan/protocol-changes.md`.
 *  2. `filePath` was still present as an optional branch on both ingress params and both results.
 *     `plan/04` work item 6 and `File_Ops_Protocol.md` §8 require it gone, and its acceptance
 *     criterion is literally "no `filePath` in any protocol command or result type".
 *
 * `blobId` is now **required** on the ingress params. It was optional only because the pre-P3
 * handlers took a path instead; with the blob layer implemented there is no other way in.
 */
import { z } from "zod";
import type { ArtifactResult } from "./blob";

// ─────────────────────────────────────────────────────────────────────────────
// export.formats — pure data, unchanged since P1
// ─────────────────────────────────────────────────────────────────────────────

export const ExportFormatsParams = z.object({}).strict();
export type ExportFormatsParams = z.infer<typeof ExportFormatsParams>;
export type ExportFormatsResult = Record<string, { id: string; label: string; extensions: string[] }[]>;

// ─────────────────────────────────────────────────────────────────────────────
// export.create — egress: the engine renders, the client saves
// ─────────────────────────────────────────────────────────────────────────────

export const ExportCreateParams = z
  .object({
    kind: z.string(),
    format: z.string(),
    workspaceId: z.string(),
    /**
     * The name the client picked. Accepted so the engine can echo a sane default back, and so that
     * `suggestedName` can be format-aware — it is **not** a path, and the engine never writes to it.
     */
    filename: z.string().optional(),
  })
  .strict();
export type ExportCreateParams = z.infer<typeof ExportCreateParams>;

/**
 * The artifact plus the metadata the client needs to save it without a second round-trip.
 *
 * An alias of the shared `ArtifactResult` rather than a local declaration: all five egress channels
 * (`File_Ops_Protocol.md` §4) return the same shape, and four near-identical copies of it is how
 * they drift apart. The wire shape is unchanged — `inline` (base64) at or below
 * `BLOB_INLINE_THRESHOLD_BYTES`, `blobId` above it, with `size` / `mimeType` / `sha256` always
 * present.
 *
 * No `filePath`: the engine does not know where the client wants the bytes, and under a remote
 * engine it could not act on that knowledge anyway (`File_Ops_Protocol.md` §1).
 */
export type ExportCreateResult = ArtifactResult;

// ─────────────────────────────────────────────────────────────────────────────
// import.preflight — ingress, part 1: what is in this file?
// ─────────────────────────────────────────────────────────────────────────────

export const ImportPreflightParams = z
  .object({
    kind: z.string(),
    format: z.string(),
    workspaceId: z.string(),
    /** The blob the client uploaded with `blob.put`. Read twice, uploaded once. */
    blobId: z.string(),
  })
  .strict();
export type ImportPreflightParams = z.infer<typeof ImportPreflightParams>;

export type ImportPreflightResult =
  | { ok: true; itemCount: number; collisionIds: string[] }
  | { ok: false; error: string; canceled?: boolean };

// ─────────────────────────────────────────────────────────────────────────────
// import.commit — ingress, part 2: apply it
// ─────────────────────────────────────────────────────────────────────────────

export const ImportCommitParams = z
  .object({
    kind: z.string(),
    format: z.string(),
    workspaceId: z.string(),
    blobId: z.string(),
    /**
     * Corrected from `["skip", "overwrite", "rename"]` — see `plan/protocol-changes.md`.
     * Optional because `"keep"` is what a client that has not asked the user yet should send, and
     * the engine's own type treats it as the default.
     */
    collisionStrategy: z.enum(["keep", "override", "new"]).optional(),
  })
  .strict();
export type ImportCommitParams = z.infer<typeof ImportCommitParams>;

export type ImportCommitResult =
  | { ok: true; imported: number; skipped?: number }
  | { ok: false; error: string; canceled?: boolean };
