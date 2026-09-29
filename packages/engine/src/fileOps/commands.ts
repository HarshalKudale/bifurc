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
import type { CommandRegistry } from "../kernel/registry";
import { blobContentPath, describeBlobError, statBlob } from "../blob/store";
import { publishArtifact } from "../blob/publish";
import { caCertPath, caKeyPath } from "../certificates/certManager";
import { loadConfig } from "../workspace/config";
import { queryLog } from "../store/gitStore";
import { renderRunnerHtml } from "../runner/reportHtml";

/**
 * The CA's location is resolved by `caCertPath()` / `caKeyPath()` in `../certificates/certManager`, which
 * owns the constants and the `appDataDir()`-vs-`dataDir()` decision. This module used to carry its
 * own copy of both; `certCommands.ts` became the second consumer in P3 work item 5, which is the
 * moment two copies became one answer. The note that used to live here — that a mismatch makes the
 * certificate invisible to whichever half looked in the other directory — is preserved there.
 */

export function registerFileOpsCommands(registry: CommandRegistry): void {
  // The `tls.*` trio takes no `workspaceId`, and that is a decision rather than an omission — the CA
  // is one per **machine**, not per workspace. See `tlsExportCert` / `tlsImport`'s own notes.
  registry.register("tls.exportCert", (_p: TlsExportCertParams) => tlsExportCert());
  registry.register("tls.importCert", (p: TlsImportCertParams) => tlsImport(p.blobId, caCertPath()));
  registry.register("tls.importKey", (p: TlsImportKeyParams) => tlsImport(p.blobId, caKeyPath()));
  // These three DO carry one (P14 WI 8) — for their blob half. See `blob/publish.ts`.
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
 *
 * ## This is the one egress command with NO `workspaceId`, and that is a decision
 *
 * P14 WI 8 gave the other three egress commands the field so their blob half lands in the caller's
 * namespace. This one deliberately did not, for the same reason `tlsImport` below is unscoped:
 * **the CA is one per *machine*, not per workspace** (`caCertPath()` resolves under `appDataDir()`,
 * and `proxy/server.ts`'s `ownsCA` exists precisely because N workspaces share that one mkcert root).
 * A workspace is not a participant in a CA export, so a `workspaceId` here would be a field that
 * means nothing — worse than an absent one, because it would read as though the export were
 * workspace-scoped.
 *
 * **And it cannot reach the blob branch anyway, measured rather than assumed:** a `ca-cert.pem` is
 * **1,240 bytes** on this machine against a `BLOB_INLINE_THRESHOLD_BYTES` of **1,048,576** — about
 * 845× short, and a PEM is not going to grow three orders of magnitude. So there is no namespace for
 * it to land in. If a per-workspace CA ever arrives (it has not been proposed), the change is to the
 * CA's *location* in `certManager.ts` — one place — and this follows without knowing.
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
 *
 * ## The two store calls here are deliberately UNSCOPED, and the reason is not the same as the egress
 * ## set's
 *
 * P14 D17 namespaced the blob root per workspace and gave the four `blob.*` params a `workspaceId`.
 * This function reads through the same store with **no** workspace, so it resolves the host-wide root
 * — and, unlike the four egress commands, that is not an omission awaiting WI 8. It is the correct
 * answer, because **the CA is not a per-workspace entity**:
 *
 * - `caCertPath()` / `caKeyPath()` resolve under `appDataDir()` (`certificates/certManager.ts`), which is
 *   `%LOCALAPPDATA%/Bifurc` on Windows — a directory the workspace data root does not reach, by that
 *   module's own recorded and deliberate decision.
 * - `tls.exportCert` above reads the same two paths for the same reason: there is exactly one CA per
 *   engine, and `proxy/server.ts`'s `ownsCA` flag exists precisely because that one mkcert root is
 *   shared by every workspace on the machine.
 * - `tls.importCert` **uploads** into `blob.put` under whatever namespace the client chose and then
 *   reads it back here. So the two halves must agree, and the honest way to make them agree is for
 *   this side to read where the CA actually lives — which is what it does, and what
 *   `tests/integration/fileOps.integration.test.ts` exercises with an unscoped `blob.put`.
 *
 * **The asymmetry with `blob.*` is the point, and it is worth stating so a later reader does not
 * "fix" it.** A workspace-scoped namespace is for transfers *between a client and one workspace's
 * staging area*. A CA import is a transfer between a client and **the machine's** one CA — the
 * workspace on whose behalf the dialog was opened is not a participant. Adding a `workspaceId` here
 * would let a client stage under A and import into the shared CA, which is the same half-threading
 * D17's integration failure exposed in `export.create` — arriving this time as a *deliberate* bug
 * rather than an accident.
 *
 * If a per-workspace CA ever arrives (it has not been proposed), the change is to the CA's *location*
 * in `certManager.ts` — one place — and this function follows it without knowing. Scoping these two
 * calls would instead have to be undone.
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

  // `params.workspaceId` (P14 WI 8) is for the blob half only: above `BLOB_INLINE_THRESHOLD_BYTES` a
  // report stages a blob, and the client reads it scoped to its own workspace. A *large* report is
  // exactly the case the threshold exists for, so omitting it made the engine's own export unreadable
  // by the client that requested it — see `blob/publish.ts`.
  return publishArtifact(content, `${folderName}-report-${ts}.${format}`, mimeType, params.workspaceId);
}

// ─────────────────────────────────────────────────────────────────────────────
// audit.export
// ─────────────────────────────────────────────────────────────────────────────

/**
 * Render the requested workspace's audit log as JSON or CSV.
 *
 * `limit: 0` means "no limit" to `queryLog`, i.e. the **whole** log. That is why this is the one
 * egress artifact with no natural size ceiling, and therefore the channel where the inline/blob
 * threshold earns its keep rather than being ceremony.
 *
 * ## The workspace is now a parameter, and the old reasoning for not having one was desktop-only
 *
 * This used to read: *"The workspace is resolved here rather than passed in, because
 * `AuditExportParams` is `{format}` only — the renderer's `exportAudit(format)` passes nothing else,
 * and widening the wire contract to carry an id the engine can already look up would be a change with
 * no beneficiary."* Every clause of that is true **of the desktop** and false of a host:
 *
 * - "the engine can already look up" the workspace — via `loadConfig().activeWorkspaceId`, which
 *   `plan/14` §2.3 demotes to **the desktop's UI pointer**. On a host serving N workspaces there is no
 *   single "active" one, so an `audit.export` from workspace B returned the **active** workspace's log.
 *   The caller could not ask for the log it meant — the same class as WI 1's `server.start
 *   { workspaceId }` being accepted and ignored.
 * - "a change with no beneficiary" — the beneficiary is a multiplexed client, which is P7's web UI and
 *   D15's CLI. The cost of adding it later is discovering the wrong-workspace export in P7.
 *
 * **The fallback keeps the desktop identical**: `params.workspaceId ?? cfg.activeWorkspaceId`, so an
 * id-less caller gets exactly what it got before. And `workspaceId` also reaches `publishArtifact`,
 * which is the half that matters above `BLOB_INLINE_THRESHOLD_BYTES` — see the note there.
 */
async function auditExport(params: AuditExportParams): Promise<AuditExportResult> {
  const cfg = loadConfig();
  const workspaceId = params.workspaceId ?? cfg.activeWorkspaceId;
  const { entries } = await queryLog({ workspaceId, limit: 0 });

  const content = params.format === "json"
    ? JSON.stringify(entries, null, 2)
    : auditEntriesToCsv(entries);

  return publishArtifact(
    content,
    `audit-log.${params.format}`,
    params.format === "json" ? "application/json" : "text/csv",
    workspaceId,
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
    // P14 WI 8 — the blob half only. Captures are the second-most likely artifact to exceed the
    // threshold, since the entries come from the renderer's own store and are not bounded here.
    params.workspaceId,
  );
}
