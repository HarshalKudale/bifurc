/**
 * `tls.*` — CA cert lifecycle. See `File_Ops_Protocol.md` §6 for the full design; this is the
 * P1 schema layer only. `tls.generate` is one of the 10 P0 spike commands — confirmed working
 * end-to-end with real `mkcert` CA generation (`plan/spike-results.md`).
 *
 * P3 changes `tls.generate`'s result to a fingerprint instead of a filesystem path (checklist:
 * "Cert: `tls.generate` returns a fingerprint"). Both fields are kept here during the
 * transition; P3 removes `certPath`/`keyPath`.
 */
import { z } from "zod";

export const TlsGenerateParams = z.object({}).strict();
export type TlsGenerateParams = z.infer<typeof TlsGenerateParams>;
export interface TlsGenerateResult {
  ok: boolean;
  certPath?: string; // pre-P3; removed once the fingerprint model lands
  keyPath?: string; // pre-P3
  fingerprint?: string; // post-P3
  error?: string;
}

export const TlsCertStatusParams = z.object({}).strict();
export type TlsCertStatusParams = z.infer<typeof TlsCertStatusParams>;
export interface TlsCertStatusResult {
  generated: boolean;
  certPath: string | null;
  keyPath: string | null;
}

export const TlsRemoveCertParams = z.object({}).strict();
export type TlsRemoveCertParams = z.infer<typeof TlsRemoveCertParams>;
export interface TlsRemoveCertResult {
  ok: boolean;
}

/** SPLIT — engine reads/returns cert content; client owns the save dialog. */
export const TlsExportCertParams = z.object({}).strict();
export type TlsExportCertParams = z.infer<typeof TlsExportCertParams>;
export interface TlsExportCertResult {
  ok: boolean;
  content?: string;
  suggestedFilename?: string;
  error?: string;
}

/** SPLIT — client picks the file, engine imports the content. */
export const TlsImportCertParams = z.object({
  content: z.string(),
}).strict();
export type TlsImportCertParams = z.infer<typeof TlsImportCertParams>;
export interface TlsImportResult {
  ok: boolean;
  error?: string;
}

export const TlsImportKeyParams = z.object({
  content: z.string(),
}).strict();
export type TlsImportKeyParams = z.infer<typeof TlsImportKeyParams>;
