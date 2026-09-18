/**
 * P3 work items 3–4 — the `importExport:*` IPC channels, with the **shell as the client**.
 *
 * ## The split this file implements
 *
 * `File_Ops_Protocol.md` §1: the engine owns its data dir and generated artifacts, the client owns
 * the user's machine. Every crossing is bytes over RPC, never a path. So the four steps are:
 *
 * ```
 * egress   client: showSaveDialog → localPath
 *          client: export.create { kind, format, workspaceId, filename } → inline | blobId
 *          client: write the bytes → localPath
 *
 * ingress  client: showOpenDialog → localPath, read it, enforce the size cap
 *          client: blob.put { filename, mimeType, size, data } → blobId   ← uploaded ONCE
 *          client: import.preflight { …, blobId } → { itemCount, collisionIds }
 *          client: import.commit   { …, blobId, collisionStrategy } → { imported, skipped }
 *          client: blob.release    { blobId }
 * ```
 *
 * Before P3 all of that lived in this file but with the engine's hands on it: it called
 * `dialog.showSaveDialog` itself and passed the resulting **user path** straight into
 * `exporter.run(wsId, filePath)`, which wrote to it. The engine cannot do that once it is a separate
 * process (P4) and cannot do it at all in Docker (P9). Now the dialog and the file write are here,
 * on the client side of the boundary, and the engine only ever sees bytes and a filename.
 *
 * ## Why the renderer is untouched
 *
 * In the P6 architecture "the client" is the Electron shell — both this main-process half and the
 * renderer. Splitting the work between them is an internal detail of that client, so the IPC
 * contract with the renderer is left exactly as it was: `window.api` stays byte-identical, which is
 * `README.md`'s non-negotiable #3 and the regression signal for the whole critical path.
 *
 * `plan/04` work item 7 plans the opposite — moving the dialog into `ImportExportModal.tsx` and
 * carrying `blobId` through renderer state. That belongs with P5/P6, when the renderer stops being
 * a `window.api` consumer and becomes the RPC client. Doing it here would have changed
 * `window.api`'s return shape for no benefit, because the dialog has to live somewhere in the shell
 * either way. See the note in `plan/04`.
 */

import { ipcMain, dialog } from "electron";
import * as path from "path";
import {
  type ExportCreateResult,
  type ImportCommitResult,
  type ImportPreflightResult,
} from "@bifurc/protocol";
// Importing anything from the registry runs its module-scope `registerFormat()` calls, which is
// what makes `export.formats` non-empty. The explicit bare side-effect import that used to sit here
// was redundant next to this one.
import { getFormats } from "@bifurc/engine/importExport/registry";
// The blob-chunk loop, the upload path and the `call()` wrapper are shared with every other
// file-operation channel — see `fileOpsClient.ts` for why they are not private to this file.
import { call, releaseQuietly, uploadLocalFile, writeArtifact } from "@/ipc/fileOpsClient";

// ─────────────────────────────────────────────────────────────────────────────
// The renderer-facing request shapes
//
// These mirror `renderer/types/ipc.ts` by hand — `src/` does not import from `renderer/`. They are
// deliberately NOT the engine's command params: `filePath` is still here because the renderer still
// sends it, and it still means what it says (a path on the user's machine, which only the client
// can act on). The engine's `ImportCommitParams` has `blobId` instead.
// ─────────────────────────────────────────────────────────────────────────────

type EntityKind =
  | "workspace" | "requests" | "mocks" | "environments"
  | "mappings" | "proxyRules" | "websockets" | "webhooks";

interface ExportRequest { kind: EntityKind; format: string; wsId: string }
interface PreflightRequest { kind: EntityKind; format: string; wsId: string }
interface ImportRequest {
  kind: EntityKind;
  format: string;
  wsId: string;
  filePath: string;
  collisionStrategy: "keep" | "override" | "new";
}

export function registerImportExportHandlers(): void {
  ipcMain.handle("importExport:formats", () => call("export.formats", {}));

  ipcMain.handle("importExport:export", async (_e, req: ExportRequest) => {
    const { kind, format, wsId } = req;

    // ── 1. Dialog FIRST ──────────────────────────────────────────────────────
    // `File_Ops_Protocol.md` §3.2 and §9.6: fail fast on cancel, so the engine never renders a
    // 200 MB workspace archive the user then abandons.
    const formats = formatsFor(kind, format);
    const safeKind = kind.replace(/([A-Z])/g, "-$1").toLowerCase();
    const { filePath, canceled } = await dialog.showSaveDialog({
      title: `Export ${kindLabel(kind)}`,
      defaultPath: `${safeKind}-export.${formats.extensions[0] ?? "json"}`,
      filters: [
        { name: formats.label, extensions: formats.extensions },
        { name: "All Files", extensions: ["*"] },
      ],
    });
    if (canceled || !filePath) return { ok: false, canceled: true };

    // ── 2. The engine renders it ─────────────────────────────────────────────
    // Only the *name* crosses, never the path — the engine has no use for a path it cannot act on.
    const created = await call<ExportCreateResult>("export.create", {
      kind, format, workspaceId: wsId, filename: path.basename(filePath),
    });
    if (!created.ok) return { ok: false, error: created.error };

    // ── 3. The client writes the bytes ───────────────────────────────────────
    // `writeArtifact` releases the blob in a `finally`, so a failed write does not leave it for the
    // sweep — egress blobs are pull-once, and re-pulling would not fix a bad destination path.
    try {
      writeArtifact(created, filePath);
    } catch (err) {
      return { ok: false, error: `Could not write ${path.basename(filePath)}: ${String(err)}` };
    }

    return { ok: true, filePath };
  });

  ipcMain.handle("importExport:preflight", async (_e, req: PreflightRequest) => {
    const { kind, format, wsId } = req;

    const formats = formatsFor(kind, format);
    const { filePaths, canceled } = await dialog.showOpenDialog({
      title: `Import ${kindLabel(kind)}`,
      filters: [
        { name: formats.label, extensions: formats.extensions },
        { name: "All Files", extensions: ["*"] },
      ],
      properties: ["openFile"],
    });
    if (canceled || filePaths.length === 0) return { ok: false, canceled: true };

    const localPath = filePaths[0];
    const uploaded = await uploadLocalFile(localPath);
    if (!uploaded.ok) return uploaded;

    const pf = await call<ImportPreflightResult>("import.preflight", {
      kind, format, workspaceId: wsId, blobId: uploaded.blobId,
    });
    if (!pf.ok) {
      releaseQuietly(uploaded.blobId);
      return { ok: false, error: pf.error };
    }

    rememberUpload(localPath, uploaded.blobId);

    // `filePath` is echoed back because the renderer threads it from here into `importData`. That
    // round trip through React state is exactly what `File_Ops_Protocol.md` §2.3 flags, and the fix
    // — carrying `blobId` instead — is `plan/04` item 7, deferred with the renderer.
    return { ok: true, filePath: localPath, itemCount: pf.itemCount, collisionIds: pf.collisionIds };
  });

  ipcMain.handle("importExport:import", async (_e, req: ImportRequest) => {
    const { kind, format, wsId, filePath, collisionStrategy } = req;

    // The remembered upload, or a fresh one if this process never saw the preflight (an app
    // restart between the two steps). See `pendingUploads`.
    const uploaded = await uploadFor(filePath);
    if (!uploaded.ok) return { ok: false, error: uploaded.error };

    try {
      return await call<ImportCommitResult>("import.commit", {
        kind, format, workspaceId: wsId, blobId: uploaded.blobId, collisionStrategy,
      });
    } finally {
      // Imports are transform-and-discard (`File_Ops_Protocol.md` §5): the source file never
      // persists on the engine. Released on failure too — a failed commit is retried from the
      // start (the renderer returns to its idle state), which re-runs preflight and re-uploads.
      forgetUpload(filePath);
      releaseQuietly(uploaded.blobId);
    }
  });
}

// ─────────────────────────────────────────────────────────────────────────────
// Upload bookkeeping
//
// The renderer identifies an upload by the local path it came from, so the shell has to remember
// which blob that path produced. This map is that memory, and it is the only piece of state the
// shell adds in P3.
//
// It exists purely because the renderer is untouched. Once `plan/04` item 7 lands and the renderer
// carries `blobId` through its own state, this map is deleted and `importExport:import` takes the
// id directly. Bounded by construction: one entry per file the user has picked since the last
// preflight, cleared on commit.
// ─────────────────────────────────────────────────────────────────────────────

const pendingUploads = new Map<string, string>();

function rememberUpload(localPath: string, blobId: string): void {
  const previous = pendingUploads.get(localPath);
  // Re-running preflight on the same file (the user changed the format and tried again) replaces
  // the entry; release the one it displaces rather than leaving it for the sweep.
  if (previous && previous !== blobId) releaseQuietly(previous);
  pendingUploads.set(localPath, blobId);
}

function forgetUpload(localPath: string): void {
  pendingUploads.delete(localPath);
}

async function uploadFor(localPath: string): Promise<{ ok: true; blobId: string } | { ok: false; error: string }> {
  const remembered = pendingUploads.get(localPath);
  if (remembered) return { ok: true, blobId: remembered };
  return uploadLocalFile(localPath);
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

function formatsFor(kind: EntityKind, formatId: string): { label: string; extensions: string[] } {
  return getFormats(kind).find((f) => f.id === formatId) ?? { label: "Files", extensions: ["json"] };
}

function kindLabel(kind: EntityKind): string {
  const labels: Record<EntityKind, string> = {
    workspace: "Workspace",
    requests: "Requests",
    mocks: "Mocks",
    environments: "Environments",
    mappings: "Mappings",
    proxyRules: "Proxy Rules",
    websockets: "WebSockets",
    webhooks: "Webhooks",
  };
  return labels[kind] ?? kind;
}
