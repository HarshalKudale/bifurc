/**
 * P3 work item 1 — the blob store (`File_Ops_Protocol.md` §3, `plan/04-phase-3-file-ops.md`).
 *
 * The engine is the authority for *generated artifacts*; the client is the authority for the
 * *user's machine*. Neither can name a path on the other's filesystem, so a transfer has two
 * halves: the client uploads bytes once and gets a `blobId`, and the engine stages a result it
 * produced somewhere the client can pull from. This module is the staging area.
 *
 * ## Layout
 *
 * ```
 * <dataDir>/blobs/
 *   <blobId>/
 *     content       # the raw bytes — a REAL FILE, see "why a real file" below
 *     meta.json     # filename, mimeType, size, sha256, createdAt
 *   .staging/
 *     stage_<hex>   # in-flight writes from `createStaging()`
 * ```
 *
 * **Why a real file and not a buffer or a BLOB column.** `unzipper.Open.file()` will not accept a
 * buffer (`importers/workspace-zip.ts:41`) and `archiver` pipes to a `WriteStream`
 * (`exporters/workspace-zip.ts:15`). A real file is the only shape both accept, so it is the shape
 * the store commits to rather than a convenience the callers work around.
 *
 * **Why under `dataDir()`.** In Docker the blob root must be on the mounted volume, or an engine
 * restart loses every in-flight transfer. `plan/04`'s risk table asks for a test asserting exactly
 * this — see `tests/blob/store.test.ts` ("blob root is under dataDir()").
 *
 * **Why `meta.json` is written last.** The write order is `content`, then `meta.json`. Every
 * read path requires `meta.json`, so a `put` that dies mid-write leaves a directory that is
 * *invisible* (not-found) rather than one that reads back truncated bytes. The orphan is reclaimed
 * by the sweep. This is a poor man's atomic write and it needs no rename dance.
 *
 * `readMeta()` also checks that the content file exists, so **"metadata exists ⇒ the blob is
 * usable"** is an explicit invariant rather than an emergent one. The ordering already guarantees
 * it for anything this module writes; the check means a directory whose content was removed by
 * something else reports *not found* instead of a size and a hash for bytes that are not there.
 *
 * ## The sliding lease
 *
 * `createdAt` is the lease anchor and `BLOB_TTL_MS` the term. `statBlob` reports the remainder but
 * does **not** extend it; `readBlob` **does**, on every read that returns bytes. Without that, a
 * 100 MB export pulled at the default 512 KB chunk size takes ~200 round-trips and a slow client
 * could have its blob swept out from under it mid-transfer. The sweep is the safety net for a
 * client that *stops* touching a blob, and a client that is reading is by definition not that.
 *
 * ## What this module deliberately does not do
 *
 * It does not decide inline-vs-blob. `BLOB_INLINE_THRESHOLD_BYTES` belongs to the *egress*
 * commands (`export.create` and friends, work item 3) — the store's `put` always returns a
 * `blobId`. It also never derives a name from a path: `filename` arrives from the client, because
 * engine-side paths are client-side information (`File_Ops_Protocol.md` §8).
 */
import * as crypto from "crypto";
import * as fs from "fs";
import * as path from "path";
import {
  BLOB_MAX_INGRESS_BYTES,
  BLOB_READ_CHUNK_BYTES,
  BLOB_TTL_MS,
  type BlobPutParams,
  type BlobPutResult,
  type BlobReadParams,
  type BlobReadResult,
  type BlobReleaseResult,
  type BlobStatResult,
} from "@bifurc/protocol";
import { dataDir } from "../store/paths";

// ─────────────────────────────────────────────────────────────────────────────
// Layout constants
// ─────────────────────────────────────────────────────────────────────────────

export const BLOB_DIR_NAME = "blobs";
export const BLOB_CONTENT_NAME = "content";
export const BLOB_META_NAME = "meta.json";
/** In-flight writes live here, one level down, so the sweep can treat the directory itself
 * separately from the blob directories it is counting. */
export const BLOB_STAGING_DIR_NAME = ".staging";

/** `blob_` + 16 random bytes as lowercase hex. */
const BLOB_ID_RE = /^blob_[0-9a-f]{32}$/;

// ─────────────────────────────────────────────────────────────────────────────
// Errors
// ─────────────────────────────────────────────────────────────────────────────

/**
 * `plan/04`'s acceptance criterion is "size cap enforced in `blob.put` with a **clear error**".
 * A typed code rather than a bare `Error` because P4's transport has to translate this into the
 * protocol error envelope, and it cannot pattern-match on a message string.
 *
 * Handlers throw rather than returning `{ok:false}`: the four `blob.*` result types carry no error
 * field, so the envelope is the only place an error can live (see `packages/protocol/src/errors.ts`).
 */
export type BlobErrorCode =
  | "blob-not-found"
  | "blob-invalid-id"
  | "blob-too-large"
  | "blob-size-mismatch"
  | "blob-invalid-offset";

export class BlobError extends Error {
  readonly code: BlobErrorCode;

  constructor(code: BlobErrorCode, message: string) {
    super(message);
    this.name = "BlobError";
    this.code = code;
  }
}

// ─────────────────────────────────────────────────────────────────────────────
// Paths
// ─────────────────────────────────────────────────────────────────────────────

/** The blob root. Resolved lazily from the data root on every call, so it follows
 * `setDataRoot()` — same discipline as the rest of the store modules, and the reason
 * `tsup`'s `bundle: false` matters. */
export function blobRoot(): string {
  return path.join(dataDir(), BLOB_DIR_NAME);
}

/** Where `createStaging()` puts in-flight writes. */
export function stagingRoot(): string {
  return path.join(blobRoot(), BLOB_STAGING_DIR_NAME);
}

/**
 * Reject anything that is not exactly a generated blob id.
 *
 * This is the primary traversal defence: the pattern admits no `.`, no separator and no
 * user-chosen text, so `../../etc/passwd` cannot even be expressed. `blobDir()` adds a
 * containment assertion on top, because "the regex is airtight" is the kind of claim that stops
 * being true after an unrelated edit.
 */
function assertBlobId(blobId: string): void {
  if (!BLOB_ID_RE.test(blobId)) {
    throw new BlobError(
      "blob-invalid-id",
      `Not a valid blob id: ${JSON.stringify(blobId)}. Expected "blob_" followed by 32 hex characters.`,
    );
  }
}

function blobDir(blobId: string): string {
  assertBlobId(blobId);
  const root = blobRoot();
  const dir = path.join(root, blobId);
  // Defence in depth — see `assertBlobId`. A resolved path that is not a direct child of the
  // root means the id escaped, whatever the reason.
  if (path.dirname(dir) !== root) {
    throw new BlobError("blob-invalid-id", `Blob id escapes the blob root: ${JSON.stringify(blobId)}.`);
  }
  return dir;
}

/**
 * Absolute path of a blob's bytes — the path `archiver` writes to and `unzipper.Open.file()`
 * reads from. Exposed because the zip exporter/importer genuinely needs it (work item 2's two
 * non-mechanical files); nothing else should use it instead of `readBlob`.
 */
export function blobContentPath(blobId: string): string {
  return path.join(blobDir(blobId), BLOB_CONTENT_NAME);
}

// ─────────────────────────────────────────────────────────────────────────────
// Metadata
// ─────────────────────────────────────────────────────────────────────────────

export interface BlobMeta {
  blobId: string;
  filename: string;
  mimeType: string;
  size: number;
  /** Lowercase hex SHA-256 of the content. Integrity checks, and the cert fingerprint (item 5). */
  sha256: string;
  /** Epoch ms — the lease anchor. Refreshed by `readBlob`; see the header. */
  createdAt: number;
}

function writeMeta(dir: string, meta: BlobMeta): void {
  fs.writeFileSync(path.join(dir, BLOB_META_NAME), JSON.stringify(meta), "utf-8");
}

/**
 * Read a blob's metadata, or throw `blob-not-found`.
 *
 * A metadata file that is absent, unparseable or inconsistent with its own directory is reported
 * as **not found** rather than as a distinct corruption error. From a client's point of view
 * those are the same situation — "there is nothing here you can use" — and the sweep is already
 * responsible for collecting the wreckage.
 *
 * The content file is checked too, which makes **"metadata exists ⇒ the blob is usable"** an
 * explicit invariant rather than an emergent one. Writing `meta.json` last already guarantees it
 * for anything this module produces, but stating it here means a directory whose content was
 * removed by something else reports *not found* instead of a size and a hash for bytes that are
 * not there. One `existsSync` per read is nothing next to a `readdir`-and-`stat` sweep.
 */
function readMeta(blobId: string): BlobMeta {
  const dir = blobDir(blobId);
  const metaPath = path.join(dir, BLOB_META_NAME);
  let raw: string;
  try {
    raw = fs.readFileSync(metaPath, "utf-8");
  } catch {
    throw new BlobError("blob-not-found", `No such blob: ${blobId}. It may have been released or swept.`);
  }
  if (!fs.existsSync(path.join(dir, BLOB_CONTENT_NAME))) {
    throw new BlobError("blob-not-found", `Blob ${blobId} has no content — it may be mid-write or damaged.`);
  }
  let meta: BlobMeta;
  try {
    meta = JSON.parse(raw) as BlobMeta;
  } catch {
    throw new BlobError("blob-not-found", `Blob ${blobId} has unreadable metadata.`);
  }
  if (meta?.blobId !== blobId || typeof meta.size !== "number" || typeof meta.sha256 !== "string") {
    throw new BlobError("blob-not-found", `Blob ${blobId} has inconsistent metadata.`);
  }
  return meta;
}

/**
 * Best-effort metadata read **from a directory path**, returning `null` instead of throwing.
 *
 * The sweep iterates directory *entries*, not ids: an orphan left by a crashed `put` has a
 * perfectly valid name but no metadata, and a stray file may not be a blob at all. Neither should
 * be an exception, so this is the non-throwing twin of `readMeta`.
 */
export function readMetaFromDir(dir: string): BlobMeta | null {
  try {
    const meta = JSON.parse(fs.readFileSync(path.join(dir, BLOB_META_NAME), "utf-8")) as BlobMeta;
    if (typeof meta?.size !== "number" || typeof meta.sha256 !== "string") return null;
    return meta;
  } catch {
    return null;
  }
}

function sha256Of(buf: Buffer): string {
  return crypto.createHash("sha256").update(buf).digest("hex");
}

/**
 * SHA-256 of a file on disk, read in bounded chunks.
 *
 * Streaming rather than `readFileSync` because the staging path exists precisely for payloads
 * that are too large to hold in memory (a workspace zip). Chunked `readSync` keeps the hash
 * synchronous, which the store's whole API is — `CommandRegistry.invoke()` returns whatever a
 * handler returns, and several existing handlers are sync.
 */
function sha256OfFile(filePath: string): string {
  const hash = crypto.createHash("sha256");
  const fd = fs.openSync(filePath, "r");
  try {
    const buf = Buffer.alloc(1024 * 1024);
    let read: number;
    while ((read = fs.readSync(fd, buf, 0, buf.length, null)) > 0) {
      hash.update(buf.subarray(0, read));
    }
  } finally {
    fs.closeSync(fd);
  }
  return hash.digest("hex");
}

function newBlobId(): string {
  return `blob_${crypto.randomBytes(16).toString("hex")}`;
}

// ─────────────────────────────────────────────────────────────────────────────
// blob.put
// ─────────────────────────────────────────────────────────────────────────────

/**
 * The pre-decode half of the ingress rules, as a pure function.
 *
 * Extracted (and exported) rather than inlined into `putBlob` for two reasons. It is the part of
 * `putBlob` that is genuinely subtle — the *ordering* is the defence, so it deserves to be
 * readable on its own — and it is otherwise only reachable in a test by allocating a 140 MB
 * base64 string, which is a test nobody would keep running.
 *
 * Three rules, in increasing cost:
 *
 *  1. `declared` must be a sane non-negative integer. The Zod schema guarantees this on the wire;
 *     the store does not assume it was validated, because it is also callable in-process.
 *  2. `declared` must fit the ingress cap — rejected **before anything is allocated**.
 *  3. the base64 string must be short enough that it *could* decode to something under the cap —
 *     rejected **before anything is decoded**. Decoding a 1 GB string allocates ~750 MB, which is
 *     the exact OOM this exists to prevent.
 *
 * What is deliberately *not* here: a check that the decoded length fits the cap. Given rules 1–3
 * and `putBlob`'s requirement that the decoded length **equal** the declared size, it is
 * unreachable — if the declared size fits and the decode matches it, the decode fits.
 */
export function assertIngressWithinLimit(
  declared: number,
  base64Length: number,
  maxBytes: number = BLOB_MAX_INGRESS_BYTES,
): void {
  if (!Number.isInteger(declared) || declared < 0) {
    throw new BlobError("blob-size-mismatch", `Declared size must be a non-negative integer, got ${declared}.`);
  }
  if (declared > maxBytes) {
    throw new BlobError(
      "blob-too-large",
      `Declared size ${declared} bytes exceeds the ${maxBytes}-byte ingress limit.`,
    );
  }
  // Base64 is 4 characters per 3 bytes rounded up to a 4-character group, plus at most two
  // padding characters. Computed from `maxBytes` rather than the module constant so the bound and
  // the cap cannot drift apart.
  const maxBase64Length = Math.ceil(maxBytes / 3) * 4 + 4;
  if (base64Length > maxBase64Length) {
    throw new BlobError(
      "blob-too-large",
      `Payload of ${base64Length} base64 characters cannot decode to under the ` +
        `${maxBytes}-byte ingress limit.`,
    );
  }
}

/**
 * Stage an upload and return its id.
 *
 * The declared size is checked before allocating, the payload is checked before decoding, and the
 * decoded length must **equal** the declared size.
 *
 * That last requirement is the only real integrity check available. Node's base64 decoder silently
 * ignores characters it does not recognise, so without the comparison a truncated or corrupted
 * payload would stage as a *smaller but perfectly valid* blob — the kind of failure that surfaces
 * much later as "my import produced half the entities". `BlobPutParams`'s doc comment asks for
 * exactly this re-check, because the schema cannot perform it (base64 length is only an upper
 * bound on the decoded size).
 *
 * @param now Override for the lease anchor. Exists so TTL behaviour is testable without fake
 *   timers; production callers omit it.
 */
export function putBlob(params: BlobPutParams, now: number = Date.now()): BlobPutResult {
  assertIngressWithinLimit(params.size, params.data.length);

  const bytes = Buffer.from(params.data, "base64");
  if (bytes.length !== params.size) {
    throw new BlobError(
      "blob-size-mismatch",
      `Declared size ${params.size} bytes but decoded ${bytes.length} bytes.`,
    );
  }

  return writeBlob(bytes, params.filename, params.mimeType, now);
}

/** The one place a blob directory is created. Shared by `putBlob` and `StagingHandle.commit`. */
function writeBlob(bytes: Buffer, filename: string, mimeType: string, now: number): BlobPutResult {
  const blobId = newBlobId();
  const dir = path.join(blobRoot(), blobId);
  fs.mkdirSync(dir, { recursive: true });

  const sha256 = sha256Of(bytes);
  fs.writeFileSync(path.join(dir, BLOB_CONTENT_NAME), bytes);
  // LAST — see the module header. Every read path gates on this file.
  writeMeta(dir, { blobId, filename, mimeType, size: bytes.length, sha256, createdAt: now });

  return { blobId, sha256 };
}

// ─────────────────────────────────────────────────────────────────────────────
// blob.stat
// ─────────────────────────────────────────────────────────────────────────────

/**
 * Metadata plus the remaining lease.
 *
 * Does **not** extend the lease — that is `readBlob`'s job. The purpose of `ttlRemainingMs` is to
 * let a client decide whether it has time to start a large download, which would be defeated if
 * the act of asking reset the clock.
 */
export function statBlob(blobId: string, now: number = Date.now()): BlobStatResult {
  const meta = readMeta(blobId);
  return {
    size: meta.size,
    mimeType: meta.mimeType,
    sha256: meta.sha256,
    filename: meta.filename,
    ttlRemainingMs: Math.max(0, meta.createdAt + BLOB_TTL_MS - now),
  };
}

// ─────────────────────────────────────────────────────────────────────────────
// blob.read
// ─────────────────────────────────────────────────────────────────────────────

/**
 * One slice of a blob, base64-encoded.
 *
 * `eof` is the client's loop terminator, and it is computed from the *bytes actually read*
 * rather than from `offset + length >= size`, so a short read at the end of a file still
 * terminates the loop correctly.
 *
 * A read that returns bytes refreshes the lease (see the module header). A zero-byte read — the
 * final `offset === size` probe — does not, so an idle client cannot hold a blob alive forever
 * with an empty read.
 */
export function readBlob(params: BlobReadParams, now: number = Date.now()): BlobReadResult {
  const meta = readMeta(params.blobId);
  const offset = params.offset ?? 0;
  const length = params.length ?? BLOB_READ_CHUNK_BYTES;

  if (offset > meta.size) {
    throw new BlobError(
      "blob-invalid-offset",
      `Offset ${offset} is past the end of blob ${params.blobId} (${meta.size} bytes).`,
    );
  }

  const want = Math.min(length, meta.size - offset);
  const fd = fs.openSync(path.join(blobDir(params.blobId), BLOB_CONTENT_NAME), "r");
  let read: number;
  let slice: Buffer;
  try {
    slice = Buffer.alloc(want);
    read = fs.readSync(fd, slice, 0, want, offset);
  } finally {
    fs.closeSync(fd);
  }

  if (read > 0) refreshLease(params.blobId, meta, now);

  return {
    data: slice.subarray(0, read).toString("base64"),
    eof: offset + read >= meta.size,
  };
}

/**
 * Extend the lease, by rewriting the metadata's `createdAt`.
 *
 * Deliberately *not* an `fs.utimesSync` on the directory: the sweep reads `createdAt` out of
 * `meta.json`, so touching mtime alone would extend nothing. The mtime fallback in the sweep is
 * only for directories that have no metadata at all (a crashed `put`).
 */
function refreshLease(blobId: string, meta: BlobMeta, now: number): void {
  try {
    writeMeta(blobDir(blobId), { ...meta, createdAt: now });
  } catch {
    // Best effort. A failed lease refresh must not fail a read that already succeeded — the
    // worst case is that the sweep collects a blob the client is still reading, which the
    // client sees as `blob-not-found` and can re-request.
  }
}

// ─────────────────────────────────────────────────────────────────────────────
// blob.release
// ─────────────────────────────────────────────────────────────────────────────

/**
 * Delete a staged blob. The fast path out of staging — imports are transform-and-discard, so the
 * source file must not linger.
 *
 * Returns `ok: false` when there was nothing to release. That is **not** an error condition: the
 * blob may have been released already, or collected by the sweep. It is reported rather than
 * swallowed so a client can tell "I cleaned up" from "there was nothing there", which is
 * genuinely different information during a retry.
 */
export function releaseBlob(blobId: string): BlobReleaseResult {
  const dir = blobDir(blobId);
  if (!fs.existsSync(dir)) return { ok: false };
  fs.rmSync(dir, { recursive: true, force: true });
  return { ok: true };
}

// ─────────────────────────────────────────────────────────────────────────────
// Streaming ingress — `createStaging()`
// ─────────────────────────────────────────────────────────────────────────────

/**
 * A file to write into, and the two ways to finish.
 *
 * This exists for the one producer that cannot hand over a `Buffer`: the workspace zip exporter
 * pipes `archiver` into a `WriteStream` and never holds the archive in memory (`plan/04` work
 * item 2, "the two non-mechanical files"). The alternative — buffering a 200 MB archive to
 * satisfy a buffer-shaped `put` — would defeat the size cap and the point of streaming.
 *
 * The caller writes to `path`, then calls `commit()` or `discard()`. Staging lives under
 * `<blobs>/.staging/`, so an abandoned handle is reclaimed by the sweep even if the process dies
 * before either call — which is exactly what a crashed client looks like from here.
 */
export interface StagingHandle {
  /** Absolute path to write the payload to. */
  readonly path: string;
  /** Hash, measure and move the staged file into the store. Single-use. */
  commit(meta: { filename: string; mimeType: string }, now?: number): BlobPutResult;
  /** Abandon the staged file. Safe to call after `commit()` (no-op) and twice (no-op). */
  discard(): void;
}

export interface StagingOptions {
  /**
   * Override the ingress cap. Defaults to `BLOB_MAX_INGRESS_BYTES`, which is the contract.
   *
   * Present for the same reason `putBlob` takes `now`: the staged payload is by definition too
   * large to hold in memory, so exercising the cap through this path would otherwise mean writing
   * a real 100 MB file to disk in a test. A deployment that wants a tighter cap than the protocol
   * default can also use it.
   */
  maxBytes?: number;
}

export function createStaging(opts: StagingOptions = {}): StagingHandle {
  const maxBytes = opts.maxBytes ?? BLOB_MAX_INGRESS_BYTES;
  const dir = stagingRoot();
  fs.mkdirSync(dir, { recursive: true });
  const stagingPath = path.join(dir, `stage_${crypto.randomBytes(16).toString("hex")}`);
  let settled = false;

  return {
    path: stagingPath,

    commit({ filename, mimeType }, now: number = Date.now()): BlobPutResult {
      if (settled) throw new Error("createStaging(): this handle has already been committed or discarded.");
      settled = true;

      const size = fs.statSync(stagingPath).size;
      if (size > maxBytes) {
        fs.rmSync(stagingPath, { force: true });
        throw new BlobError(
          "blob-too-large",
          `Staged payload of ${size} bytes exceeds the ${maxBytes}-byte ingress limit.`,
        );
      }

      const sha256 = sha256OfFile(stagingPath);
      const blobId = newBlobId();
      const target = path.join(blobRoot(), blobId);
      fs.mkdirSync(target, { recursive: true });
      // Same volume by construction (both under the data root), so this is a rename, not a copy:
      // a 200 MB archive is moved into place in constant time.
      fs.renameSync(stagingPath, path.join(target, BLOB_CONTENT_NAME));
      writeMeta(target, { blobId, filename, mimeType, size, sha256, createdAt: now });

      return { blobId, sha256 };
    },

    discard(): void {
      if (settled) return;
      settled = true;
      fs.rmSync(stagingPath, { force: true });
    },
  };
}
