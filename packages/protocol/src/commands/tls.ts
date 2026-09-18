/**
 * `tls.*` — the CA certificate lifecycle. See `File_Ops_Protocol.md` §6 for the full design.
 * `tls.generate` is one of the 10 P0 spike commands — confirmed working end-to-end with real
 * `mkcert` CA generation (`plan/spike-results.md`).
 *
 * ## The fingerprint model (P3 work item 5)
 *
 * There are **two halves** and they live on different machines: the engine generates and stores the
 * CA, the client installs it into the host's trust store. Nothing keeps them in step, so a user can
 * generate a CA, trust it, then regenerate — and the client still trusts the **old** one. The
 * symptom is an opaque TLS error with no explanation.
 *
 * The mitigation is to carry a **fingerprint** across the boundary so the client can compare what it
 * installed against what the engine currently has:
 *
 * ```
 * tls.generate  → { fingerprint: "sha256:ab12…" }
 * tls.status    → { generated, fingerprint }
 * tls.removeCert→ { engineRemoved }            // the client un-trusts; the engine cannot
 * ```
 *
 * **This is why `certPath` / `keyPath` are gone.** An engine filesystem path is meaningless on a
 * client that may be in another process (P4) or another container (P9), and
 * `File_Ops_Protocol.md` §8 lists it as a boundary violation. The engine's own data-dir layout is
 * the engine's business; the client gets an identity it can compare, not a location it cannot use.
 *
 * **There is deliberately no `certBlobId` here**, although `File_Ops_Protocol.md` §6.1 sketches one.
 * The CA is a **durable** entity with a ten-year validity, and the blob store is a **transient**
 * staging area whose sweeper deletes anything past `BLOB_TTL_MS` — the two are opposites. The
 * engine also needs a real file on disk, because the proxy's TLS server is handed a path
 * (`tlsCaCertPath`) and `mkcert`'s CA is written as PEM. Adding a durable blob would mean two
 * sources of truth for the same certificate, and the egress channel that actually needs to move the
 * bytes (`tls.exportCert` → `ArtifactResult`) already does. Recorded in `plan/04` item 5.
 */
import { z } from "zod";
import type { ArtifactResult } from "./blob";

export const TlsGenerateParams = z.object({}).strict();
export type TlsGenerateParams = z.infer<typeof TlsGenerateParams>;
export interface TlsGenerateResult {
  ok: boolean;
  /**
   * `sha256:<64 lowercase hex>` over the certificate's DER — the string browsers and the OS show.
   * Absent when generation failed; see `TlsCertStatusResult.fingerprint` for the two null states.
   */
  fingerprint?: string;
  error?: string;
}

export const TlsCertStatusParams = z.object({}).strict();
export type TlsCertStatusParams = z.infer<typeof TlsCertStatusParams>;
export interface TlsCertStatusResult {
  generated: boolean;
  /**
   * `null` in two situations that must not be conflated:
   *
   * - `generated: false` — there is no CA yet. An ordinary first-run state.
   * - `generated: true, fingerprint: null` — both files exist but the certificate cannot be
   *   parsed. The pre-P3 code had no way to say this, which is why a corrupt `ca-cert.pem`
   *   surfaced later as an opaque TLS failure.
   */
  fingerprint: string | null;
}

export const TlsRemoveCertParams = z.object({}).strict();
export type TlsRemoveCertParams = z.infer<typeof TlsRemoveCertParams>;
export interface TlsRemoveCertResult {
  ok: boolean;
  /**
   * Whether the **engine's** half removed anything — `false` when there was no CA to remove.
   *
   * The client's half (`clientUntrusted`) is composed by the shell and never crosses the wire:
   * only the client has a trust store, and on a remote engine the engine has no way to inspect it
   * (`File_Ops_Protocol.md` §6.1). Reporting a single fused `ok` is what made the pre-P3 handler
   * unable to express "the files are gone but your browser still trusts the certificate".
   */
  engineRemoved: boolean;
}

/**
 * SPLIT — the engine reads the CA and returns the artifact; the client owns the save dialog and
 * the file write (`File_Ops_Protocol.md` §4).
 *
 * Takes no params. There is exactly one CA per engine so there is nothing to select, and the
 * renderer calls this channel with no arguments — so the engine's `suggestedName` is what the
 * dialog opens with.
 */
export const TlsExportCertParams = z.object({}).strict();
export type TlsExportCertParams = z.infer<typeof TlsExportCertParams>;
export type TlsExportCertResult = ArtifactResult;

/**
 * SPLIT — the client picks the file and uploads it with `blob.put`; the engine imports the bytes
 * (`File_Ops_Protocol.md` §5).
 *
 * A `blobId` rather than inline `content`, even though a PEM is only a few kilobytes. The cert is a
 * *persisted* entity — it is copied into the engine's data dir and is not transform-and-discard —
 * and `blob.put` is what applies the size cap and the three integrity checks. An inline `content`
 * parameter would be a second way in that skips both.
 *
 * Note the bytes are **not** validated as a certificate here, deliberately: the import contract
 * preserves arbitrary bytes, and the "is this actually a certificate?" question is answered at
 * status time instead (`TlsCertStatusResult.fingerprint`).
 */
export const TlsImportCertParams = z.object({
  blobId: z.string(),
}).strict();
export type TlsImportCertParams = z.infer<typeof TlsImportCertParams>;
export interface TlsImportResult {
  ok: boolean;
  error?: string;
}

export const TlsImportKeyParams = z.object({
  blobId: z.string(),
}).strict();
export type TlsImportKeyParams = z.infer<typeof TlsImportKeyParams>;
