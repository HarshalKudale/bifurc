/**
 * The client-side plumbing every file-operation channel shares.
 *
 * `File_Ops_Protocol.md` §1 puts the user's filesystem on the client and the engine's data dir on
 * the engine, and every crossing is bytes over RPC. That leaves each channel doing the same four
 * things, and this module is those four things written once:
 *
 * ```
 * egress   writeArtifact()   — decode `inline`, or pull the blob, then write the user's file
 * ingress  uploadLocalFile() — read the user's file, enforce the size cap, `blob.put`
 * both     call()            — the typed `commandRegistry.invoke` wrapper
 * both     releaseQuietly()  — best-effort `blob.release`
 * ```
 *
 * **Why this exists.** The pre-P3 code had the engine doing all of it (dialogs and `writeFileSync`
 * inside the engine's handlers). Converting five channels by hand would have produced five copies
 * of the blob-chunk loop, and the chunk loop is exactly the kind of code that drifts — one copy
 * would keep the `chunk.length === 0` guard and four would not. Extracted in P3 when the second
 * channel needed it, which is the first moment the duplication was real rather than hypothetical.
 *
 * **What stays out.** No dialog, no filename policy, no format knowledge. Those differ per channel
 * (a save dialog's filters, whether a suggested name exists, whether the chosen extension decides
 * the format) and belong at the call site.
 */
import * as fs from "fs";
import * as path from "path";
import {
  BLOB_MAX_INGRESS_BYTES,
  type BlobPutResult,
  type BlobReadResult,
} from "@bifurc/protocol";
import type { CommandAction } from "@bifurc/protocol";
import { commandRegistry } from "@bifurc/engine/commands/registry";
import { bus } from "@bifurc/engine/eventBus";

const ctx = { bus };

/**
 * `CommandRegistry.invoke()` is synchronous by design, but several handlers are `async` and it
 * returns whatever they return — so this awaits either shape and hands back the typed result.
 *
 * (The registry validates every payload against the frozen protocol schema before the handler runs,
 * which is why the call sites below pass plain objects with no local re-validation.)
 */
export async function call<T>(action: CommandAction, payload: unknown): Promise<T> {
  return (await commandRegistry.invoke(action, payload, ctx)) as T;
}

/** The egress half of an artifact: either the bytes are already in hand, or they are in a blob. */
export interface ArtifactRef {
  inline?: string;
  blobId?: string;
}

/**
 * Write an egress artifact to the path the user chose, then release its blob.
 *
 * The release is in a `finally`, and that is deliberate: egress blobs are pull-once, and the client
 * has the bytes in hand by then or it does not — re-pulling would not fix a bad destination path, so
 * a failed write must not leak the blob until the sweep notices.
 *
 * Throws on a write failure; the caller owns the message because only it knows the filename's role
 * in the user's mental model.
 */
export function writeArtifact(artifact: ArtifactRef, destPath: string): void {
  try {
    const bytes = artifact.inline !== undefined
      ? Buffer.from(artifact.inline, "base64")
      : pullBlob(artifact.blobId!);
    fs.writeFileSync(destPath, bytes);
  } finally {
    if (artifact.blobId) releaseQuietly(artifact.blobId);
  }
}

/**
 * Read the client's file and stage it on the engine.
 *
 * The size cap is checked here as well as in `blob.put`, against the same protocol constant. The
 * engine's check is the denial-of-service guard — it exists because the client is untrusted from P4
 * on. This one exists so the user gets "that file is too large" instead of a rejected upload, and
 * using `BLOB_MAX_INGRESS_BYTES` rather than a local number is what stops the two drifting apart.
 */
export async function uploadLocalFile(
  localPath: string,
): Promise<{ ok: true; blobId: string } | { ok: false; error: string }> {
  let bytes: Buffer;
  try {
    bytes = fs.readFileSync(localPath);
  } catch (err) {
    return { ok: false, error: `Could not read ${path.basename(localPath)}: ${String(err)}` };
  }

  if (bytes.length > BLOB_MAX_INGRESS_BYTES) {
    return {
      ok: false,
      error: `"${path.basename(localPath)}" is ${bytes.length} bytes, over the ` +
        `${BLOB_MAX_INGRESS_BYTES}-byte import limit.`,
    };
  }

  try {
    const res = await call<BlobPutResult>("blob.put", {
      filename: path.basename(localPath),
      mimeType: mimeFor(localPath),
      size: bytes.length,
      data: bytes.toString("base64"),
    });
    return { ok: true, blobId: res.blobId };
  } catch (err) {
    return { ok: false, error: err instanceof Error ? err.message : String(err) };
  }
}

/**
 * Pull a blob in `BLOB_READ_CHUNK_BYTES` slices.
 *
 * Goes through `blob.read` rather than reading the engine's content file directly, because this
 * module *is* the client: it must work unchanged when the engine is in another process (P4) or a
 * container (P9), and `blob.read` is the only read primitive that will exist on the wire.
 *
 * `eof` is computed by the store from the bytes actually read, so a short final slice still
 * terminates. The `chunk.length === 0` guard is belt-and-braces against a zero-length read that
 * does not set `eof`, which would otherwise spin forever.
 */
export function pullBlob(blobId: string): Buffer {
  const chunks: Buffer[] = [];
  let offset = 0;
  for (;;) {
    const res = commandRegistry.invoke("blob.read", { blobId, offset }, ctx) as BlobReadResult;
    const chunk = Buffer.from(res.data, "base64");
    if (chunk.length > 0) chunks.push(chunk);
    offset += chunk.length;
    if (res.eof || chunk.length === 0) break;
  }
  return Buffer.concat(chunks);
}

/**
 * `blob.release` is best-effort: a blob that is already gone is a `{ ok: false }`, not an error,
 * and the sweep collects anything this misses. A release failure must never fail the operation the
 * user actually asked for.
 */
export function releaseQuietly(blobId: string): void {
  try {
    commandRegistry.invoke("blob.release", { blobId }, ctx);
  } catch {
    /* best effort — the TTL sweep is the safety net */
  }
}

/**
 * MIME type for `blob.put`'s metadata.
 *
 * Only the import direction needs this, and only so `blob.stat` can report something sensible — no
 * consumer branches on it. Deliberately separate from the `mimeMap` inside `dialog:openFile`
 * (`handlers/clientHandlers.ts`): that one exists for binary body uploads and covers images, media
 * and fonts, whereas this one covers files a user picks to hand to the engine. Merging two small
 * maps with almost no overlap would obscure both.
 */
const MIME_BY_EXTENSION: Record<string, string> = {
  json: "application/json",
  har: "application/json",
  yaml: "application/yaml",
  yml: "application/yaml",
  zip: "application/zip",
  env: "text/plain",
  txt: "text/plain",
  sh: "text/plain",
  curl: "text/plain",
  pem: "application/x-pem-file",
  crt: "application/x-pem-file",
  cer: "application/x-pem-file",
  key: "application/x-pem-file",
};

export function mimeFor(localPath: string): string {
  const ext = path.extname(localPath).toLowerCase().slice(1);
  return MIME_BY_EXTENSION[ext] ?? "application/octet-stream";
}
