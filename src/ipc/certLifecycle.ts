/**
 * The client's certificate **lifecycle** — the composition of the three things that have to agree
 * for a local HTTPS proxy to work, and the reason P3 work item 5 exists.
 *
 * ```
 * engine      tls.certStatus / tls.exportCert / tls.removeCert      (over the command registry)
 * trust store certTrust.ts — install / un-trust / probe             (the host's OS store)
 * drift       AppSettings.tlsTrustedCa — what THIS client installed (shell-local, never on the wire)
 * ```
 *
 * ## The drift problem, and why a record is the only fix
 *
 * A CA can be regenerated on the engine at any time. The client's trust store still holds the *old*
 * one, and the symptom is an opaque TLS error with no explanation — the browser rejects a
 * certificate signed by an authority the user believes they installed.
 *
 * The fingerprint alone does not fix this: to *compare* you need the value you installed, and once
 * the engine has regenerated, its bytes are gone. So the record has to be written at install time —
 * including the certificate's **PEM**, not just its fingerprint, because the un-trust path needs
 * the actual certificate to hand to `certutil` / `security` / `trust`. That is why `TrustedCa`
 * carries the bytes as well as the identity: a fingerprint can tell you the CA changed, but only
 * the certificate can tell the OS to stop trusting it.
 *
 * ## Where the record lives, and why that does not put it on the wire
 *
 * In `AppSettings` — the same file that already holds `zoomLevel`, `themeId` and `hasSeenWelcome`,
 * which are likewise shell-only state the engine never reads. `loadConfig()` builds `AppConfig` from
 * an **explicit field list** (`store/config.ts`), so a new key cannot reach `config:get` by
 * accident, and `window.api` stays byte-identical through P6 (`README.md` non-negotiable #3).
 *
 * ## Install replaces, it does not accumulate
 *
 * If the record shows a *different* certificate was installed previously, that one is un-trusted
 * first, best-effort. Otherwise every regeneration leaves another stale root in the user's store
 * forever, and "remove the Bifurc CA" becomes an unbounded manual cleanup. A failure to remove the
 * stale entry does not fail the install — the user asked for the new one to be trusted, and it is.
 */
import * as crypto from "crypto";
import * as fs from "fs";
import * as os from "os";
import * as path from "path";
import type {
    TlsCertStatusResult,
    TlsExportCertResult,
    TlsRemoveCertResult,
} from "@bifurc/protocol";
import { identifyCert } from "@bifurc/engine/proxy/certManager";
import { loadSettings, saveSettings, type TrustedCa } from "@bifurc/engine/store/appSettings";
import { call, writeArtifact } from "@/ipc/fileOpsClient";
import {
    detectFirefox,
    installCA,
    realTrustContext,
    uninstallCA,
    type FirefoxPresence,
    type TrustContext,
    type TrustResult,
} from "@/ipc/certTrust";

/**
 * Whether the OS store agrees with the engine.
 *
 * - `none` — this client has never installed a CA. The ordinary first-run state.
 * - `trusted` — the record's fingerprint equals the engine's current one.
 * - `stale` — they differ, or the engine has no usable CA at all. **This is the state that
 *   produces the confusing TLS errors**, and it is the whole point of the fingerprint.
 */
export type TrustState = "none" | "trusted" | "stale";

export interface TrustSummary {
    /** The fingerprint this client installed, or `null` if it never installed one. */
    trustedFingerprint: string | null;
    trustState: TrustState;
    /**
     * Firefox is present on this machine, so it will **not** trust the CA even after a successful
     * install — it uses its own NSS store (`File_Ops_Protocol.md` §6.3). Detection only.
     */
    firefoxDetected: boolean;
    firefoxProfiles: number;
}

/**
 * The drift answer, given the engine's current fingerprint.
 *
 * Deliberately a pure function of `(record, engineFingerprint)`: the interesting logic is the
 * three-way comparison, and it is worth testing without a trust store, an engine or a filesystem.
 */
export function summariseTrust(
    trusted: TrustedCa | null | undefined,
    engineFingerprint: string | null,
    firefox: FirefoxPresence = detectFirefox(),
): TrustSummary {
    const trustedFingerprint = trusted?.fingerprint ?? null;
    const trustState: TrustState = trustedFingerprint === null
        ? "none"
        : trustedFingerprint === engineFingerprint
            ? "trusted"
            : "stale";

    return {
        trustedFingerprint,
        trustState,
        firefoxDetected: firefox.present,
        firefoxProfiles: firefox.profiles,
    };
}

export interface InstallOutcome extends TrustResult {
    /** The fingerprint of the certificate that was installed. Additive. */
    fingerprint?: string;
    /** A previously-installed, now-different CA was un-trusted as part of this call. Additive. */
    replacedStale?: boolean;
    firefoxDetected?: boolean;
    firefoxProfiles?: number;
}

export interface UninstallOutcome {
    ok: boolean;
    /** The engine's half — whether it had files to delete. */
    engineRemoved: boolean;
    /**
     * The client's half. `false` does **not** necessarily mean the CA is still trusted; read
     * `clientUntrustNote` when it is `false` and a record existed.
     */
    clientUntrusted: boolean;
    /** Set when this client had no record, so it could not know what to remove. Additive. */
    clientUntrustNote?: string;
    /** A manual-install instruction set, when the platform chain could not do it unattended. */
    instructions?: string;
    needsManualInstall?: boolean;
    error?: string;
}

/**
 * A one-shot certificate file in the OS temp dir.
 *
 * The client must hand the OS tools a **local path**, and on a remote engine the engine's own path
 * is not one. So the bytes are fetched over the artifact channel and materialised here. Randomised
 * so two concurrent installs cannot race on the same name, and it is deleted as soon as the tool
 * has read it — except when the platform chain degraded to manual instructions, where the file
 * *is* the deliverable.
 */
function tempCertPath(): string {
    return path.join(os.tmpdir(), `bifurc-ca-${crypto.randomBytes(6).toString("hex")}.pem`);
}

function removeQuietly(file: string): void {
    try {
        fs.unlinkSync(file);
    } catch {
        /* best effort — a leftover temp file is not worth failing the operation for */
    }
}

/**
 * Un-trust a certificate this client installed earlier, from the record alone.
 *
 * The engine may have regenerated since, so the record's PEM is the only copy of the certificate
 * that is actually in the user's store. Returns whether the un-trust succeeded.
 */
function uninstallRecorded(ctx: TrustContext, recorded: TrustedCa): boolean {
    const file = tempCertPath();
    try {
        fs.writeFileSync(file, recorded.pem, "utf-8");
        return uninstallCA(ctx, file, recorded.thumbprintSha1).ok;
    } catch {
        return false;
    } finally {
        removeQuietly(file);
    }
}

/**
 * Install the engine's **current** CA into the host trust store, and record what was installed.
 *
 * Order matters in three places, and each one is a bug the other order would produce:
 *
 * 1. `tls.certStatus` first, so "no CA yet" is a clear message rather than a failed export.
 * 2. The **bytes are fetched and identified** before anything is recorded, so the record describes
 *    the certificate that was actually installed rather than the one the engine had a moment ago.
 *    Reading the fingerprint from `tls.certStatus` instead would allow the two calls to straddle a
 *    regeneration and record a fingerprint that matches nothing on disk.
 * 3. The stale entry is removed **before** the new one is added, so the user is never in a state
 *    where both are trusted and the un-trust that follows is ambiguous.
 */
export async function installEngineCa(): Promise<InstallOutcome> {
    const status = await call<TlsCertStatusResult>("tls.certStatus", {});
    if (!status.generated) {
        // The pre-P3 message, kept verbatim: it is the one string the renderer's UI was written
        // against and it is still the right thing to say.
        return { ok: false, error: "No CA certificate found. Generate one first." };
    }

    const artifact = await call<TlsExportCertResult>("tls.exportCert", {});
    if (!artifact.ok) return { ok: false, error: artifact.error };

    const certFile = tempCertPath();
    let pem: string;
    try {
        // `writeArtifact` also releases the egress blob — see `fileOpsClient.ts`.
        writeArtifact(artifact, certFile);
        pem = fs.readFileSync(certFile, "utf-8");
    } catch (err) {
        removeQuietly(certFile);
        return { ok: false, error: `Could not stage the certificate: ${String(err)}` };
    }

    const identity = identifyCert(pem);
    if (!identity) {
        // Reachable: `tls.importCert` preserves arbitrary bytes by design, so a hand-edited
        // `ca-cert.pem` is a real state. Refusing here is the clear error that used to arrive
        // later as an opaque TLS failure.
        removeQuietly(certFile);
        return {
            ok: false,
            error: "The engine's CA certificate could not be read as a certificate. "
                + "Import or generate a valid one first.",
        };
    }

    const ctx = realTrustContext();
    const settings = loadSettings();
    const previous = settings.tlsTrustedCa ?? null;

    let replacedStale = false;
    if (previous && previous.fingerprint !== identity.fingerprint) {
        replacedStale = uninstallRecorded(ctx, previous);
    }

    const result = installCA(ctx, certFile, identity.thumbprintSha1);

    if (result.ok) {
        saveSettings({
            ...settings,
            tlsTrustedCa: {
                fingerprint: identity.fingerprint,
                thumbprintSha1: identity.thumbprintSha1,
                subject: identity.subject,
                pem,
                installedAt: Date.now(),
            },
        });
    }

    // Keep the file only when the user has to run the commands themselves — an instruction that
    // names a path we just deleted is not an instruction.
    if (result.ok || !result.needsManualInstall) removeQuietly(certFile);

    const firefox = detectFirefox();
    return {
        ...result,
        fingerprint: identity.fingerprint,
        replacedStale,
        firefoxDetected: firefox.present,
        firefoxProfiles: firefox.profiles,
    };
}

/**
 * Both halves of a removal: the engine deletes its keypair, the client un-trusts what it installed.
 *
 * Deliberately **not** all-or-nothing. Either half can fail on its own, and the two failures have
 * different remedies: "the files are gone but your browser still trusts the certificate" is the
 * state the pre-P3 fused `{ok: boolean}` could not express at all, and it is the one that produces
 * a security-relevant surprise (a certificate the user believes they removed).
 */
export async function uninstallEngineCa(): Promise<UninstallOutcome> {
    const engine = await call<TlsRemoveCertResult>("tls.removeCert", {});

    const settings = loadSettings();
    const previous = settings.tlsTrustedCa ?? null;

    if (!previous) {
        return {
            ok: true,
            engineRemoved: engine.engineRemoved,
            clientUntrusted: false,
            clientUntrustNote:
                "This machine has no record of installing the engine's CA, so nothing was removed "
                + "from the OS trust store. If it was installed manually from an exported "
                + "certificate, remove it from your system's certificate manager.",
        };
    }

    const ctx = realTrustContext();
    const file = tempCertPath();
    let result: TrustResult;
    try {
        fs.writeFileSync(file, previous.pem, "utf-8");
        result = uninstallCA(ctx, file, previous.thumbprintSha1);
    } catch (err) {
        result = { ok: false, error: err instanceof Error ? err.message : String(err) };
    } finally {
        removeQuietly(file);
    }

    // Clear the record only when the store no longer holds it. Keeping a record of a certificate
    // that IS still trusted is what makes a retry possible; clearing it would lose the PEM, and
    // with it any way to remove the certificate later.
    if (result.ok) saveSettings({ ...settings, tlsTrustedCa: null });

    return {
        ok: true,
        engineRemoved: engine.engineRemoved,
        clientUntrusted: result.ok,
        needsManualInstall: result.needsManualInstall,
        instructions: result.instructions,
        error: result.error,
    };
}

/**
 * The drift fields the status channel reports, additively.
 *
 * Reads the engine's fingerprint rather than trusting the record, because the record is exactly the
 * thing that may be out of date. The renderer ignores every field here; P7 is where they get shown.
 */
export async function readTrustSummary(): Promise<TrustSummary & { engineFingerprint: string | null }> {
    const status = await call<TlsCertStatusResult>("tls.certStatus", {});
    const summary = summariseTrust(loadSettings().tlsTrustedCa, status.fingerprint);
    return { ...summary, engineFingerprint: status.fingerprint };
}
