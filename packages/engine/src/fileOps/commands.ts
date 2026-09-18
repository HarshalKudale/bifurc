/**
 * P3 work items 3–4 — the engine half of the six file-operation channels that are not
 * import/export.
 *
 * `File_Ops_Protocol.md` §4 (egress) and §5 (ingress) enumerate every channel that crosses the
 * engine/client file boundary. `importExport:*` was converted first, in work item 2; these are the
 * rest:
 *
 * ```
 * egress   tls:exportCert   runner:exportReport   capture:shareJson   audit:export
 * ingress  tls:importCert   tls:importKey
 * ```
 *
 * ## The contract
 *
 * Egress — the engine renders an artifact and returns `ArtifactResult` (inline bytes, or a
 * `blobId` above the threshold); the client shows the dialog and writes the file:
 *
 * ```
 * client: dialog.showSaveDialog() → localPath
 * client: invoke(cmd, {…})        → { inline } | { blobId, size, mimeType, sha256 }
 * client: writes bytes → localPath
 * ```
 *
 * Ingress — the client reads its own file, uploads it once, and the engine copies the staged bytes
 * into its data dir:
 *
 * ```
 * client: dialog.showOpenDialog() → localPath
 * client: invoke("blob.put", {…}) → blobId
 * client: invoke("tls.importCert", { blobId })
 * client: invoke("blob.release", { blobId })
 * ```
 *
 * ## Why these six live together
 *
 * They are the commands that share this exact contract, and they share the artifact publisher
 * (`blob/publish.ts`) with `export.create`. Their *domains* differ — TLS, the collection runner,
 * the audit log, capture — but the boundary they implement is one thing, and scattering them across
 * four modules would mean four copies of "the dialog is the client's, only bytes cross the wire".
 *
 * ## Two things this module deliberately does not do
 *
 * **It never shows a dialog and never writes outside the engine's own data dir.** That is the whole
 * point (`File_Ops_Protocol.md` §1) and it is what makes P4's remote transport and P9's container
 * possible.
 *
 * **It does not validate an imported certificate.** The pre-P3 shell handler used `copyFileSync`
 * and accepted anything; adding a PEM sanity check here would be a behaviour change rather than a
 * refactor, and the import contract deliberately preserves arbitrary bytes — asserted, including a
 * deliberately non-ASCII payload, in `tests/integration/fileOps.integration.test.ts`. Item 5 closed
 * the gap it was originally deferred to **at status time** instead of at import time: `tls.certStatus`
 * now reports `generated: true, fingerprint: null` for a file that is not a certificate, which is
 * the clear error that used to arrive later as an opaque TLS failure. See
 * `proxy/certManager.ts#identifyCert`.
 */
import * as fs from "fs";
import type {
  AuditEntry,
  AuditExportParams,
  AuditExportResult,
  CaptureShareJsonParams,
  CaptureShareJsonResult,
  RunnerExportReportParams,
  RunnerExportReportResult,
  TlsExportCertParams,
  TlsExportCertResult,
  TlsImportCertParams,
  TlsImportKeyParams,
  TlsImportResult,
} from "@bifurc/protocol";
import type { CommandRegistry } from "../commands/registry";
import { blobContentPath, describeBlobError, statBlob } from "../blob/store";
import { publishArtifact } from "../blob/publish";
import { caCertPath, caKeyPath } from "../proxy/certManager";
import { loadConfig } from "../store/config";
import { queryLog } from "../store/gitStore";
import { renderRunnerHtml } from "../runner/reportHtml";

/**
 * The CA's location is resolved by `caCertPath()` / `caKeyPath()` in `../proxy/certManager`, which
 * owns the constants and the `appDataDir()`-vs-`dataDir()` decision. This module used to carry its
 * own copy of both; `certCommands.ts` became the second consumer in P3 work item 5, which is the
 * moment two copies became one answer. The note that used to live here — that a mismatch makes the
 * certificate invisible to whichever half looked in the other directory — is preserved there.
 */

export function registerFileOpsCommands(registry: CommandRegistry): void {
  registry.register("tls.exportCert", (_p: TlsExportCertParams) => tlsExportCert());
  registry.register("tls.importCert", (p: TlsImportCertParams) => tlsImport(p.blobId, caCertPath()));
  registry.register("tls.importKey", (p: TlsImportKeyParams) => tlsImport(p.blobId, caKeyPath()));
  registry.register("runner.exportReport", (p: RunnerExportReportParams) => runnerExportReport(p));
  registry.register("audit.export", (p: AuditExportParams) => auditExport(p));
  registry.register("capture.shareJson", (p: CaptureShareJsonParams) => captureShareJson(p));
}

// ─────────────────────────────────────────────────────────────────────────────
// tls.exportCert / tls.importCert / tls.importKey
// ─────────────────────────────────────────────────────────────────────────────

/**
 * Read the engine's CA certificate and hand it back as an artifact.
 *
 * A missing file is reported as an error rather than as an empty artifact, and with the same
 * message the pre-P3 handler used — "no CA generated yet" is an ordinary state the UI already knows
 * how to present.
 */
function tlsExportCert(): TlsExportCertResult {
  let content: string;
  try {
    content = fs.readFileSync(caCertPath(), "utf-8");
  } catch {
    return { ok: false, error: "No CA certificate found." };
  }
  return publishArtifact(content, "bifurc-ca.pem", "application/x-pem-file");
}

/**
 * Copy a staged blob into the engine's data dir.
 *
 * **Bytes, not text.** The pre-P3 handler was `fs.copyFileSync`, and reading a PEM as UTF-8 to write
 * it straight back would silently normalise anything non-ASCII. A PEM is ASCII so it would not
 * matter today, but there is no reason to introduce the difference.
 *
 * Note this is a *persisted* import, not transform-and-discard (`File_Ops_Protocol.md` §5): the
 * cert and key live on in `dataDir()` and the client still releases the blob, because the staged
 * copy has served its purpose.
 *
 * **`statBlob()` first, and that is not ceremony.** `blobContentPath()` validates the id's *shape*
 * but never checks that the blob exists, so reading it directly turns "the user left the dialog
 * open past `BLOB_TTL_MS`" into a raw `ENOENT` from `fs.readFileSync` — and an `ENOENT` message
 * contains the absolute engine-side path, which `File_Ops_Protocol.md` §8 names as a boundary
 * violation. Going through the store's metadata read first converts that into a typed
 * `blob-not-found` and keeps the message path-free. It does **not** slide the lease, which is what
 * we want: the bytes are consumed immediately after, so there is nothing to hold the blob for.
 */
function tlsImport(blobId: string, destPath: string): TlsImportResult {
  try {
    statBlob(blobId);
    const bytes = fs.readFileSync(blobContentPath(blobId));
    fs.writeFileSync(destPath, bytes);
    return { ok: true };
  } catch (err) {
    return { ok: false, error: describeBlobError(err) };
  }
}

// ─────────────────────────────────────────────────────────────────────────────
// runner.exportReport
// ─────────────────────────────────────────────────────────────────────────────

/**
 * Render a collection-run report as HTML or JSON.
 *
 * `format` is supplied by the **client**, because only the client knows what the user chose — the
 * pre-P3 shell decided it by testing whether the chosen path ended in `.json`, and that test has to
 * happen before the command is called, not after.
 *
 * The report itself arrives over the wire rather than being read back from the workspace: the
 * renderer can export a run it has not saved yet. That is why `RunReport` is widened in
 * `@bifurc/protocol` to carry every field the renderer's real `RunnerRequestResult` has — a schema
 * narrower than its only caller truncates the JSON export without saying so.
 */
function runnerExportReport(params: RunnerExportReportParams): RunnerExportReportResult {
  const { report, format } = params;
  const folderName = report.folderName ?? "collection";
  const ts = new Date(report.startedAt).toISOString().replace(/[:.]/g, "-");

  const content = format === "json" ? JSON.stringify(report, null, 2) : renderRunnerHtml(report);
  const mimeType = format === "json" ? "application/json" : "text/html";

  return publishArtifact(content, `${folderName}-report-${ts}.${format}`, mimeType);
}

// ─────────────────────────────────────────────────────────────────────────────
// audit.export
// ─────────────────────────────────────────────────────────────────────────────

/**
 * Render the active workspace's audit log as JSON or CSV.
 *
 * `limit: 0` means "no limit" to `queryLog`, i.e. the **whole** log. That is why this is the one
 * egress artifact with no natural size ceiling, and therefore the channel where the inline/blob
 * threshold earns its keep rather than being ceremony.
 *
 * The workspace is resolved here rather than passed in, because `AuditExportParams` is `{format}`
 * only — the renderer's `exportAudit(format)` passes nothing else, and widening the wire contract to
 * carry an id the engine can already look up would be a change with no beneficiary.
 */
async function auditExport(params: AuditExportParams): Promise<AuditExportResult> {
  const cfg = loadConfig();
  const { entries } = await queryLog({ workspaceId: cfg.activeWorkspaceId, limit: 0 });

  const content = params.format === "json"
    ? JSON.stringify(entries, null, 2)
    : auditEntriesToCsv(entries);

  return publishArtifact(
    content,
    `audit-log.${params.format}`,
    params.format === "json" ? "application/json" : "text/csv",
  );
}

/**
 * The CSV writer, moved verbatim from the shell.
 *
 * Only `entityName` is quoted, because it is the one free-text field a user controls; the rest are
 * a git hash, a timestamp, an enum, two ids and an actor name, none of which can contain a comma.
 * `ts` is left as a raw epoch number — that is what the pre-P3 export wrote, and changing it now
 * would break anyone's spreadsheet formula for no stated reason.
 */
function auditEntriesToCsv(entries: AuditEntry[]): string {
  const header = "commitHash,ts,action,entity,entityId,entityName,workspaceId,actor";
  const rows = entries.map((e) =>
    [
      e.commitHash, e.ts, e.action, e.entity, e.entityId,
      `"${e.entityName.replace(/"/g, '""')}"`,
      e.workspaceId, e.actor,
    ].join(",")
  );
  return [header, ...rows].join("\n");
}

// ─────────────────────────────────────────────────────────────────────────────
// capture.shareJson
// ─────────────────────────────────────────────────────────────────────────────

/**
 * Serialize captured requests for sharing.
 *
 * The entries come from the renderer — capture lives in the renderer's store, not the engine's — so
 * this command's engine half is only the serialization. It is still an engine command rather than a
 * client-side `JSON.stringify` because `File_Ops_Protocol.md` §4 lists it as an egress channel, and
 * because a client that is not Electron (P7's web UI) must not have to reimplement the format to
 * share a capture.
 */
function captureShareJson(params: CaptureShareJsonParams): CaptureShareJsonResult {
  return publishArtifact(
    JSON.stringify(params.entries, null, 2),
    params.suggestedName || "captured-requests.json",
    "application/json",
  );
}
