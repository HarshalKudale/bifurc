/**
 * The **two egress channels that are nobody else's** — `audit.export` and `capture.shareJson`.
 *
 * `File_Ops_Protocol.md` §4 (egress) and §5 (ingress) enumerate every channel that crosses the
 * engine/client file boundary. The contract is one thing:
 *
 * ```
 * client: dialog.showSaveDialog() → localPath
 * client: invoke(cmd, {…})        → { inline } | { blobId, size, mimeType, sha256 }
 * client: writes bytes → localPath
 * ```
 *
 * ## Why this file is now two commands and not six
 *
 * It held six: the four egress channels above plus `tls:exportCert` / `runner:exportReport`, and the
 * two ingress ones `tls:importCert` / `tls:importKey`. Its own docblock argued they belonged together
 * because *"the boundary they implement is one thing"* — and that argument was half right, which is
 * exactly why it was wrong as a placement rule:
 *
 *  - **`tls.exportCert` / `tls.importCert` / `tls.importKey` → `certificates/certCommands.ts`.** They
 *    resolve `caCertPath()` / `caKeyPath()`, which are that module's own constants, and they are the
 *    three `tls.*` commands that were missing from the other three. The CA is a *machine* artifact,
 *    not a per-workspace one, and it has one owner.
 *  - **`runner.exportReport` → `runner/runnerCommands.ts`.** It renders through `renderRunnerHtml()`,
 *    which `runner.saveReport` in that very file already calls — so the two halves of "render the
 *    report" now sit together instead of in two modules.
 *
 * What is left is what no other module can own. The *contract* they share is documented above and in
 * `File_Ops_Protocol.md`; it was never a reason for them to share a **directory**.
 *
 * ## What this module deliberately does not do
 *
 * **It never shows a dialog and never writes outside the engine's own data dir.** That is the whole
 * point (`File_Ops_Protocol.md` §1) and it is what makes P4's remote transport and P9's container
 * possible.
 */
import type {
  AuditEntry,
  AuditExportParams,
  AuditExportResult,
  CaptureShareJsonParams,
  CaptureShareJsonResult,
} from "@bifurc/protocol";
import type { CommandRegistry } from "../kernel/registry";
import { publishArtifact } from "../workspace/blob/publish";
import { loadConfig } from "../workspace/config";
import { queryLog } from "../workspace/gitStore";

export function registerFileOpsCommands(registry: CommandRegistry): void {
  // Both carry a `workspaceId` (P14 WI 8) — for their blob half. See `workspace/blob/publish.ts`.
  registry.register("audit.export", (p: AuditExportParams) => auditExport(p));
  registry.register("capture.shareJson", (p: CaptureShareJsonParams) => captureShareJson(p));
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
