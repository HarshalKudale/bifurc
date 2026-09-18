/**
 * The three cert-lifecycle commands the **engine** owns — `tls.generate`, `tls.certStatus`,
 * `tls.removeCert`.
 *
 * ## Why these moved out of the shell (P3 work item 5)
 *
 * They were registered in `src/ipc/handlers/tlsHandlers.ts`, alongside the three *file* channels
 * that genuinely are the shell's job (`tls:exportCert`, `tls:importCert`, `tls:importKey`). That
 * file is the shell's, so the engine's own commands were being served from the client — which works
 * only while the two are the same process. P4's transport and P9's container both break that
 * assumption, and these three touch nothing but the engine's own data dir, so they belong here.
 *
 * The file channels did not move with them, and that asymmetry is the point: this module never
 * shows a dialog and never writes outside the engine's data dir (`File_Ops_Protocol.md` §1).
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
import type {
    TlsCertStatusResult,
    TlsGenerateResult,
    TlsRemoveCertResult,
} from "@bifurc/protocol";
import type { CommandRegistry } from "../commands/registry";
import { caDir, generateCA, getCertStatus, removeCA } from "./certManager";

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
}
