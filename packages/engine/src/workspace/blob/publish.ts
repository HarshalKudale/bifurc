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
 *
 * ## `workspaceId` (P14 D17, completed for the egress set by WI 8)
 *
 * The parameter must be passed by any caller that operates on a workspace, because an artifact
 * above `BLOB_INLINE_THRESHOLD_BYTES` is staged and the client then reads it with `blob.read`
 * **scoped to the same workspace**. Omitting it here does not merely lose a namespace — it makes the
 * engine's own export unreadable by the client that requested it, which is how this was caught:
 * `export.create` passed its `workspaceId` to the import half but not to `publishArtifact`, and
 * `tests/integration/importExportBlob.integration.test.ts` failed at the client's first `blob.read`.
 *
 * **All five egress commands are now consistent**, and the set is worth stating exactly because it
 * was inconsistent twice:
 *
 * | Command | `workspaceId` |
 * |---|---|
 * | `export.create` | yes — wired in D17 |
 * | `runner.exportReport`, `capture.shareJson`, `audit.export` | yes — WI 8 |
 * | `tls.exportCert` | **no, deliberately** — the CA is one per *machine* (`caCertPath()` under `appDataDir()`), so a workspace is not a participant, and a `ca-cert.pem` is **1,240 bytes** against a 1 MiB threshold, i.e. it cannot reach the blob branch at all. Recorded in `fileOps/commands.ts` beside the same decision for `tlsImport`. |
 *
 * **This is a contract-consistency fix, not a live bug fix, and the difference matters for how it is
 * verified.** Today's only client (`packages/client/src/index.ts`) sends **no** `workspaceId` on
 * `blob.put` / `blob.read` / `blob.release` — it relies on the host-wide default, which is what the
 * unscoped egress commands produced, so the two matched. The inconsistency bites the first client
 * that *does* scope its blob reads (P7's web UI, D15's CLI): it would find `export.create`'s output
 * and not the other three's. Which is why the fix is here before that client exists.
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
  workspaceId?: string,
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

  const { blobId } = putBlobBytes(bytes, suggestedName, mimeType, Date.now(), workspaceId);
  return { ok: true, blobId, suggestedName, size: bytes.length, mimeType, sha256 };
}
