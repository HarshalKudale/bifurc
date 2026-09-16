/**
 * `blob.*` — the blob layer (`File_Ops_Protocol.md` §3).
 *
 * Four primitives, orthogonal to the domain commands. They exist because the client and the
 * engine each own half of the problem: the **client** is the authority for the user's machine
 * (so it shows the dialog, reads the file, and gets the filename), and the **engine** is the
 * authority for generated artifacts (so it stages them somewhere the client can pull from).
 *
 * Why this is not just "pass a path":
 *
 *  - The engine cannot be handed a client path at all once it is a separate process (P4–P6), and
 *    in Docker (P9) that path does not exist on the engine's filesystem.
 *  - The current design reads the file **twice** — `preflight` reads it, then `importer.run()`
 *    reads it again from the same path. Staging once and referencing twice removes that.
 *  - `dialog:openFile` currently caps inline reads at ~1 MB (`systemHandlers.ts:238`); the blob
 *    layer generalises that rule instead of re-deriving it per caller.
 *
 * **P1 left this file out.** `plan/04-phase-3-file-ops.md` lists "`blob.put` / `blob.stat` /
 * `blob.read` / `blob.release` specified in `@bifurc/protocol`" as a P3 precondition, but P1 only
 * landed the *envelope* half (`Capability.BLOB` in `envelope.ts`) and made `export.ts`
 * `blobId`-shaped. The commands themselves did not exist, so P3 could not start. Added 2026-09-16.
 */
import { z } from "zod";

// ─────────────────────────────────────────────────────────────────────────────
// Protocol constants
//
// These are deliberately exported rather than inlined. `plan/04` is explicit that the inline
// threshold and the ingress cap must not become magic numbers repeated in two places — the engine
// enforces them, and the client needs to know them to decide whether to expect a `blobId` or an
// inline payload back.
// ─────────────────────────────────────────────────────────────────────────────

/**
 * Artifacts at or below this size are returned **inline** (base64 in the command result) rather
 * than as a `blobId`. One round-trip for the common case, which is a small JSON export.
 *
 * Matches the existing cap in `dialog:openFile` (`systemHandlers.ts:238`) so the switch does not
 * change which payloads are cheap.
 */
export const BLOB_INLINE_THRESHOLD_BYTES = 1024 * 1024;

/**
 * Hard cap on a single `blob.put`, enforced by the engine **before** it allocates.
 *
 * This is a denial-of-service guard, not a product limit: the client is untrusted in every
 * deployment P4 onwards, and an unbounded `put` lets a client OOM the engine. Generous for exports,
 * which is why it is far above the inline threshold.
 */
export const BLOB_MAX_INGRESS_BYTES = 100 * 1024 * 1024;

/**
 * How long an unreleased blob survives before the sweep collects it.
 *
 * `blob.release` is the fast path; this is the safety net. Without it a Docker volume accumulates
 * orphaned staging directories forever — a client that crashes mid-transfer never releases.
 */
export const BLOB_TTL_MS = 60 * 60 * 1000;

/** Chunk size `blob.read` uses when the caller does not specify a `length`. */
export const BLOB_READ_CHUNK_BYTES = 512 * 1024;

/**
 * A reference to staged content — either a `blobId` the client can pull, or the bytes inline.
 *
 * Domain commands that produce an artifact (`export.create`, `runner.exportReport`,
 * `audit.export`, `capture.shareJson`) return this shape. `export.ts`'s result interfaces still
 * carry the older `blobId?` / `filePath?` pair; they adopt `BlobRef` during P3's conversion pass
 * (`plan/04` work item 2), which is why this type lives here rather than being inlined per command.
 *
 * Exactly one of the two is set. `blobId` wins if both somehow arrive.
 */
export type BlobRef = { blobId: string; inline?: never } | { blobId?: never; inline: string };

// ─────────────────────────────────────────────────────────────────────────────
// blob.put — ingress, client → engine
// ─────────────────────────────────────────────────────────────────────────────

export const BlobPutParams = z
  .object({
    /**
     * Display name, supplied by the **client**. The engine must never derive one from a path —
     * a path is client-side information that does not exist engine-side, and `File_Ops_Protocol.md`
     * §8 treats path leakage as a boundary violation.
     */
    filename: z.string().min(1),
    mimeType: z.string().min(1),
    /**
     * Byte length of the *decoded* payload, declared by the client so the engine can reject an
     * oversized upload before decoding it.
     *
     * The schema bounds it; it cannot verify it against `data`, because base64 length is only an
     * upper bound on the decoded size. The engine must re-check the decoded length and reject a
     * mismatch rather than trusting this field.
     */
    size: z.number().int().nonnegative().max(BLOB_MAX_INGRESS_BYTES),
    /** base64. Chunked/streaming upload is a transport concern (P4/P5), not a wire-shape change. */
    data: z.string().min(1),
  })
  .strict();
export type BlobPutParams = z.infer<typeof BlobPutParams>;

export interface BlobPutResult {
  blobId: string;
  /** SHA-256 of the decoded bytes, lowercase hex. Enables integrity checks and the cert fingerprint. */
  sha256: string;
}

// ─────────────────────────────────────────────────────────────────────────────
// blob.stat — egress, engine → client
// ─────────────────────────────────────────────────────────────────────────────

export const BlobStatParams = z.object({ blobId: z.string().min(1) }).strict();
export type BlobStatParams = z.infer<typeof BlobStatParams>;

export interface BlobStatResult {
  size: number;
  mimeType: string;
  sha256: string;
  filename: string;
  /**
   * Time left before the sweep collects this blob. Lets a client warn before starting a large
   * download it may not finish in time.
   */
  ttlRemainingMs: number;
}

// ─────────────────────────────────────────────────────────────────────────────
// blob.read — egress, engine → client
// ─────────────────────────────────────────────────────────────────────────────

export const BlobReadParams = z
  .object({
    blobId: z.string().min(1),
    /** Byte offset to start from. Omitted means 0. */
    offset: z.number().int().nonnegative().optional(),
    /** Bytes to return. Omitted means `BLOB_READ_CHUNK_BYTES`. */
    length: z.number().int().positive().optional(),
  })
  .strict();
export type BlobReadParams = z.infer<typeof BlobReadParams>;

export interface BlobReadResult {
  /** base64 of the requested slice. */
  data: string;
  /** True when this slice reached the end of the blob — the client's loop terminator. */
  eof: boolean;
}

// ─────────────────────────────────────────────────────────────────────────────
// blob.release — the fast path out of staging
// ─────────────────────────────────────────────────────────────────────────────

export const BlobReleaseParams = z.object({ blobId: z.string().min(1) }).strict();
export type BlobReleaseParams = z.infer<typeof BlobReleaseParams>;

export interface BlobReleaseResult {
  ok: boolean;
}
