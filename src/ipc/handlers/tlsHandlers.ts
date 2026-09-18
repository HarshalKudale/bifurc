/**
 * `tls:*` — the shell as the **client** for the whole certificate surface.
 *
 * Six channels, and after P3 work item 5 every one of them is a client-side operation over an
 * engine command. Nothing here is registered into the engine's `CommandRegistry` any more: the
 * three cert-*lifecycle* commands moved to `packages/engine/src/proxy/certCommands.ts`, because
 * they touch nothing but the engine's own data dir.
 *
 * ```
 * tls:generate     → tls.generate     { fingerprint }              + a path the shell computes
 * tls:certStatus   → tls.certStatus   { generated, fingerprint }   + drift + Firefox
 * tls:removeCert   → tls.removeCert   { engineRemoved }            + the client's un-trust
 * tls:exportCert   → dialog → tls.exportCert → artifact → write the bytes here
 * tls:importCert   → dialog → blob.put → tls.importCert { blobId } → blob.release
 * tls:importKey    → dialog → blob.put → tls.importKey  { blobId } → blob.release
 * ```
 *
 * ## Why the renderer still gets `certPath` / `keyPath` back
 *
 * `TlsSettingsSection.tsx` does `if (result.ok && result.certPath && result.keyPath)
 * handleGlobalChange({ tlsCaCertPath: result.certPath, tlsCaKeyPath: result.keyPath })` — the
 * settings never learn the CA exists without them, and TLS stays off. The engine deliberately
 * reports **no** path any more (`File_Ops_Protocol.md` §8; `plan/protocol-changes.md`
 * 2026-09-17), so the shell computes both from `caCertPath()` / `caKeyPath()`, which resolve to
 * exactly the files `tls.generate` wrote.
 *
 * This is category 3 in `plan/04` item 6 — an engine FS layout detail the *shell* synthesises for
 * the renderer — and it is a genuine leak. It stays because `window.api` is byte-identical through
 * P6 (`README.md` non-negotiable #3), and P7 is where it stops being a path.
 *
 * ## Why `tls:exportCert` checks before it shows a dialog
 *
 * Every other egress channel shows the dialog first, so the engine never renders an artifact the
 * user then abandons. That rationale does not apply to a 2 KB certificate, and asking the engine
 * "is there a CA?" is cheaper than making the user dismiss a save dialog to find out. This
 * preserves the pre-P3 order, which checked `fs.existsSync` before opening the dialog.
 */
import { ipcMain, dialog } from "electron";
import * as path from "path";
import type { TlsCertStatusResult, TlsExportCertResult, TlsImportResult } from "@bifurc/protocol";
import { caCertPath, caKeyPath } from "@bifurc/engine/proxy/certManager";
import { call, releaseQuietly, uploadLocalFile, writeArtifact } from "@/ipc/fileOpsClient";
import { readTrustSummary, uninstallEngineCa } from "@/ipc/certLifecycle";

export function registerTlsHandlers() {
    /**
     * The renderer-facing shape is the pre-P3 one; `fingerprint` is additive and ignored by every
     * current caller. On failure there is deliberately no `certPath`, so the renderer's `&&` guard
     * keeps the settings unchanged — the same behaviour as before, when a failed `generateCA`
     * returned neither.
     */
    ipcMain.handle("tls:generate", async () => {
        const result = await call<{ ok: boolean; fingerprint?: string; error?: string }>("tls.generate", {});
        if (!result.ok) return { ok: false, error: result.error };
        return {
            ok: true,
            certPath: caCertPath(),
            keyPath: caKeyPath(),
            fingerprint: result.fingerprint,
        };
    });

    /**
     * `generated` / `certPath` / `keyPath` are the declared renderer shape and are preserved
     * verbatim — **even though no renderer code currently calls this** (`renderer/types/window.ts`
     * declares it; nothing reads it). Non-negotiable #3 is about the surface, not about whether
     * something happens to use it today, and P7 is where the drift fields become visible.
     *
     * `trustState` is the field P7 needs and is the reason the fingerprint exists: `stale` means
     * the engine's CA has changed since this machine trusted it, which is otherwise an
     * unexplained TLS failure.
     */
    ipcMain.handle("tls:certStatus", async () => {
        const summary = await readTrustSummary();
        return {
            generated: summary.engineFingerprint !== null,
            certPath: caCertPath(),
            keyPath: caKeyPath(),
            fingerprint: summary.engineFingerprint,
            trustedFingerprint: summary.trustedFingerprint,
            trustState: summary.trustState,
            firefoxDetected: summary.firefoxDetected,
            firefoxProfiles: summary.firefoxProfiles,
        };
    });

    /**
     * Two-sided by design (`File_Ops_Protocol.md` §6.1). The engine deletes its keypair; the client
     * un-trusts the certificate it installed. Either half can succeed alone, and the pre-P3 fused
     * `{ok: true}` could not express the difference — which is how a user ends up believing they
     * removed a certificate their browser still trusts.
     */
    ipcMain.handle("tls:removeCert", () => uninstallEngineCa());

    ipcMain.handle("tls:exportCert", async () => {
        // See the header note: status first, dialog second, for this channel only.
        const status = await call<TlsCertStatusResult>("tls.certStatus", {});
        if (!status.generated) return { ok: false, error: "No CA certificate found." };

        const { filePath, canceled } = await dialog.showSaveDialog({
            title: "Export CA Certificate",
            defaultPath: "bifurc-ca.pem",
            filters: [{ name: "Certificate", extensions: ["pem", "crt", "cer"] }],
        });
        if (canceled || !filePath) return { ok: false, canceled: true };

        const artifact = await call<TlsExportCertResult>("tls.exportCert", {});
        if (!artifact.ok) return { ok: false, error: artifact.error };

        try {
            writeArtifact(artifact, filePath);
        } catch (err) {
            return { ok: false, error: `Could not write ${path.basename(filePath)}: ${String(err)}` };
        }
        return { ok: true, filePath };
    });

    ipcMain.handle("tls:importCert", () =>
        importTlsFile("tls.importCert", "Select CA Certificate",
            [{ name: "Certificate", extensions: ["pem", "crt", "cer"] }], caCertPath()));

    ipcMain.handle("tls:importKey", () =>
        importTlsFile("tls.importKey", "Select CA Private Key",
            [{ name: "Private Key", extensions: ["pem", "key"] }], caKeyPath()));
}

/**
 * Ingress: pick the file, upload it **once**, let the engine copy it into place, release the blob.
 *
 * The `path` in the result is computed here rather than returned by the engine — see the header
 * note. `{ ok: false }` with no `error` is the cancel shape the pre-P3 handler returned and the
 * renderer already ignores; the new failure modes carry a message.
 */
async function importTlsFile(
    command: "tls.importCert" | "tls.importKey",
    title: string,
    filters: Electron.FileFilter[],
    destPath: string,
): Promise<{ ok: boolean; path?: string; error?: string }> {
    const { filePaths, canceled } = await dialog.showOpenDialog({
        title,
        filters,
        properties: ["openFile"],
    });
    if (canceled || !filePaths[0]) return { ok: false };

    const uploaded = await uploadLocalFile(filePaths[0]);
    if (!uploaded.ok) return { ok: false, error: uploaded.error };

    try {
        const res = await call<TlsImportResult>(command, { blobId: uploaded.blobId });
        if (!res.ok) return { ok: false, error: res.error };
        return { ok: true, path: destPath };
    } finally {
        // A cert is a *persisted* import (`File_Ops_Protocol.md` §5), not transform-and-discard: the
        // engine copied the bytes into its data dir, so the staged blob has done its job. Released on
        // failure too — a failed import is retried from the dialog, which re-uploads.
        releaseQuietly(uploaded.blobId);
    }
}
