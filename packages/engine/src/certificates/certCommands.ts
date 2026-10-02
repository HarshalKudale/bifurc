/**
 * The six `tls.*` commands the **engine** owns — `tls.generate`, `tls.certStatus`, `tls.removeCert`,
 * and the three egress/ingress ones, `tls.exportCert`, `tls.importCert`, `tls.importKey`.
 *
 * ## Why these moved out of the shell (P3 work item 5)
 *
 * They were registered in `src/ipc/handlers/tlsHandlers.ts`, alongside the three *file* channels
 * that genuinely are the shell's job (`tls:exportCert`, `tls:importCert`, `tls:importKey`). That
 * file is the shell's, so the engine's own commands were being served from the client — which works
 * only while the two are the same process. P4's transport and P9's container both break that
 * assumption, and these three touch nothing but the engine's own data dir, so they belong here.
 *
 * **The three file channels did not move with them in P3, and that asymmetry was the point then.**
 * They were the shell's because the *dialog* is the client's; the engine half is only "render or copy
 * bytes". They landed in `fileops/commands.ts` when P6 deleted the channels, and **this cleanup moved
 * them here** — that file was an aggregator holding three domains' commands, and `tls.exportCert` /
 * `tls.importCert` / `tls.importKey` are the CA's, which is this module's subject. `fileops/` now
 * holds the two egress channels that are nobody else's (`audit.export`, `capture.shareJson`).
 *
 * This module still never shows a dialog and never writes outside the engine's data dir
 * (`File_Ops_Protocol.md` §1) — the client owns the dialog on both sides of the boundary.
 *
 * ## What is *not* here: installation
 *
 * `tls:installCA` is absent by design. Installing into the OS trust store is a **host-scoped,
 * client-local mutation** (`plan/handler-classification.md` — CLIENT-classified), and it used to be
 * `certManager.installCA()` shelling out to `certutil` from inside the engine. The engine now
 * reports an **identity** for its CA and the client decides what to do with it; see
 * `src/ipc/certTrust.ts` for the install, the un-trust, and the platform privilege matrix.
 *
 * ## The two-sided shape of `tls.removeCert`
 *
 * The engine reports `engineRemoved` — its half, and only its half. It cannot see the client's
 * trust store, and on a remote engine there is no way for it to. The shell composes the other half
 * and reports both to the renderer.
 */
import * as fs from "fs";
import type {
    TlsCertStatusResult,
    TlsExportCertParams,
    TlsExportCertResult,
    TlsGenerateResult,
    TlsImportCertParams,
    TlsImportKeyParams,
    TlsImportResult,
    TlsRemoveCertResult,
} from "@bifurc/protocol";
import type { CommandRegistry } from "../kernel/registry";
import { caDir, caCertPath, caKeyPath, generateCA, getCertStatus, removeCA } from "./certManager";
import { blobContentPath, describeBlobError, statBlob } from "../workspace/blob/store";
import { publishArtifact } from "../workspace/blob/publish";

export function registerCertCommands(registry: CommandRegistry): void {
    registry.register("tls.generate", async (): Promise<TlsGenerateResult> => {
        try {
            const { identity } = await generateCA(caDir());
            return { ok: true, fingerprint: identity?.fingerprint };
        } catch (e: unknown) {
            // `generateCA()` throws for a bad `mkcert` result rather than returning a fingerprint-less
            // success, so this catch is the only place a "generated but untrustable" CA could be
            // reported as `ok: true` — and it is not.
            return { ok: false, error: e instanceof Error ? e.message : String(e) };
        }
    });

    registry.register("tls.certStatus", (): TlsCertStatusResult => getCertStatus(caDir()));

    registry.register("tls.removeCert", (): TlsRemoveCertResult => ({
        ok: true,
        engineRemoved: removeCA(caDir()).engineRemoved,
    }));

    // The `tls.*` trio below takes no `workspaceId`, and that is a decision rather than an omission —
    // the CA is one per **machine**, not per workspace. See `tlsExportCert` / `tlsImport`'s own notes.
    registry.register("tls.exportCert", (_p: TlsExportCertParams) => tlsExportCert());
    registry.register("tls.importCert", (p: TlsImportCertParams) => tlsImport(p.blobId, caCertPath()));
    registry.register("tls.importKey", (p: TlsImportKeyParams) => tlsImport(p.blobId, caKeyPath()));
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

