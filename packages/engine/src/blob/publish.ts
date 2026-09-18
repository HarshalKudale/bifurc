/**
 * The one place the inline-vs-blob decision is made, for every egress artifact command.
 *
 * `File_Ops_Protocol.md` §3.2 puts the threshold decision on the **engine**, not the client, and
 * §4 names five channels that produce an artifact: `export.create`, `tls.exportCert`,
 * `runner.exportReport`, `capture.shareJson`, `audit.export`. All five return the same
 * `ArtifactResult`, so all five must make the same decision the same way — which is why this is a
 * module rather than a private helper duplicated five times.
 *
 * Below `BLOB_INLINE_THRESHOLD_BYTES` the artifact never touches the store, so the common case — a
 * small JSON export, a CA certificate — is one round-trip. Above it, the artifact is written to a
 * blob and the client pulls it with `blob.read`.
 *
 * `sha256` is computed on **both** branches. It is not redundant on the inline branch: the client
 * uses it to verify what it wrote, and returning it unconditionally means the client never has to
 * branch on which shape it received to know whether a digest exists.
 */
import { BLOB_INLINE_THRESHOLD_BYTES, type ArtifactResult } from "@bifurc/protocol";
import { putBlobBytes, sha256Of } from "./store";

export function publishArtifact(
  content: string,
  suggestedName: string,
  mimeType: string,
): ArtifactResult {
  const bytes = Buffer.from(content, "utf-8");
  const sha256 = sha256Of(bytes);

  if (bytes.length <= BLOB_INLINE_THRESHOLD_BYTES) {
    return {
      ok: true,
      inline: bytes.toString("base64"),
      suggestedName,
      size: bytes.length,
      mimeType,
      sha256,
    };
  }

  const { blobId } = putBlobBytes(bytes, suggestedName, mimeType);
  return { ok: true, blobId, suggestedName, size: bytes.length, mimeType, sha256 };
}
