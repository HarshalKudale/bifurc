/**
 * `src/ipc/certLifecycle.ts` — where the engine's certificate, the client's trust store and the
 * client's drift record have to agree.
 *
 * ## What is actually being tested
 *
 * Not the happy path — that is three calls in a row. The interesting properties are the **ordering
 * and the record-keeping rules**, each of which is a bug in the other direction:
 *
 * - the record is written **only** on a successful install, because a record of a certificate that
 *   is not in the store makes the drift comparison lie;
 * - the record is **kept** when an un-trust fails, because the PEM is the only copy of the
 *   certificate the OS still trusts — clearing it would make the certificate unremovable;
 * - the stale entry is un-trusted **before** the new one is added, so there is never a moment with
 *   two Bifurc roots trusted;
 * - the temporary certificate file is kept when the platform degraded to manual instructions and
 *   deleted otherwise, because an instruction naming a deleted path is not an instruction;
 * - the identity recorded is derived from the **bytes that were installed**, not from a separate
 *   `tls.certStatus` call, so a regeneration between the two calls cannot be recorded wrongly.
 *
 * The engine and the trust store are both mocked; `identifyCert()` is not, because it is a pure
 * function over the fixture and mocking it would remove the thing being ordered around.
 */
import { describe, it, expect, vi, beforeEach } from "vitest";
import * as fs from "fs";
import {
    REAL_CA_FINGERPRINT,
    REAL_CA_PEM,
    REAL_CA_SUBJECT_MARKER,
    REAL_CA_THUMBPRINT_SHA1,
} from "../fixtures/certFixtures";

const mocks = vi.hoisted(() => ({
    call: vi.fn(),
    writeArtifact: vi.fn(),
    installCA: vi.fn(),
    uninstallCA: vi.fn(),
    realTrustContext: vi.fn(),
    detectFirefox: vi.fn(),
    loadSettings: vi.fn(),
    saveSettings: vi.fn(),
}));

vi.mock("@/ipc/fileOpsClient", () => ({
    call: mocks.call,
    writeArtifact: mocks.writeArtifact,
}));

vi.mock("@/ipc/certTrust", () => ({
    installCA: mocks.installCA,
    uninstallCA: mocks.uninstallCA,
    realTrustContext: mocks.realTrustContext,
    detectFirefox: mocks.detectFirefox,
}));

vi.mock("@bifurc/engine/store/appSettings", () => ({
    loadSettings: mocks.loadSettings,
    saveSettings: mocks.saveSettings,
}));

import { installEngineCa, summariseTrust, uninstallEngineCa } from "@/ipc/certLifecycle";

const OLD_RECORD = {
    fingerprint: "sha256:" + "0".repeat(64),
    thumbprintSha1: "OLDTHUMBPRINT000000000000000000000000000",
    subject: "CN=An Older Bifurc CA",
    pem: "-----BEGIN CERTIFICATE-----\nOLD\n-----END CERTIFICATE-----\n",
    installedAt: 1_700_000_000_000,
};

const CURRENT_RECORD = {
    fingerprint: REAL_CA_FINGERPRINT,
    thumbprintSha1: REAL_CA_THUMBPRINT_SHA1,
    subject: `CN=${REAL_CA_SUBJECT_MARKER}`,
    pem: REAL_CA_PEM,
    installedAt: 1_700_000_000_000,
};

/** The temp path `installEngineCa()` chose, captured from the `writeArtifact` call. */
function stagedPath(): string {
    return mocks.writeArtifact.mock.calls[0][1] as string;
}

function primeEngine(opts: {
    generated?: boolean;
    exportOk?: boolean;
    exportError?: string;
    engineRemoved?: boolean;
} = {}): void {
    const { generated = true, exportOk = true, exportError = "export failed", engineRemoved = true } = opts;
    mocks.call.mockImplementation(async (action: string) => {
        if (action === "tls.certStatus") {
            return { generated, fingerprint: generated ? REAL_CA_FINGERPRINT : null };
        }
        if (action === "tls.exportCert") {
            return exportOk
                ? { ok: true, inline: "", suggestedName: "bifurc-ca.pem", size: 1, mimeType: "application/x-pem-file", sha256: "x" }
                : { ok: false, error: exportError };
        }
        if (action === "tls.removeCert") return { ok: true, engineRemoved };
        throw new Error(`unexpected command in test: ${action}`);
    });
}

beforeEach(() => {
    vi.clearAllMocks();
    mocks.realTrustContext.mockReturnValue({ platform: "win32" });
    mocks.detectFirefox.mockReturnValue({ present: false, profiles: 0 });
    mocks.installCA.mockReturnValue({ ok: true, verified: true });
    mocks.uninstallCA.mockReturnValue({ ok: true, verified: true });
    mocks.loadSettings.mockReturnValue({ tlsTrustedCa: null });
    // The real `writeArtifact` decodes `inline`/pulls the blob and writes the file; the mock does
    // the only part that matters downstream, which is putting the certificate on disk where
    // `installEngineCa()` reads it back to identify it.
    mocks.writeArtifact.mockImplementation((_artifact: unknown, destPath: string) => {
        fs.writeFileSync(destPath, REAL_CA_PEM, "utf-8");
    });
    primeEngine();
});

// ─────────────────────────────────────────────────────────────────────────────
// summariseTrust — the drift comparison
// ─────────────────────────────────────────────────────────────────────────────

describe("summariseTrust()", () => {
    const noFirefox = { present: false, profiles: 0 };

    it("reports 'none' when this client has never installed a CA", () => {
        expect(summariseTrust(null, REAL_CA_FINGERPRINT, noFirefox)).toEqual({
            trustedFingerprint: null,
            trustState: "none",
            firefoxDetected: false,
            firefoxProfiles: 0,
        });
    });

    it("reports 'trusted' when the record matches what the engine has", () => {
        expect(summariseTrust(CURRENT_RECORD, REAL_CA_FINGERPRINT, noFirefox).trustState).toBe("trusted");
    });

    it("reports 'stale' when the engine's CA has changed since this machine trusted it", () => {
        // The whole reason the fingerprint is carried across the boundary. Without it this is an
        // opaque TLS error; with it, it is "the engine's CA has changed — reinstall".
        expect(summariseTrust(CURRENT_RECORD, OLD_RECORD.fingerprint, noFirefox).trustState).toBe("stale");
    });

    it("reports 'stale' when the engine has no usable CA at all", () => {
        // A trusted certificate for an authority the engine no longer holds is stale, not trusted.
        expect(summariseTrust(CURRENT_RECORD, null, noFirefox).trustState).toBe("stale");
    });

    it("passes the Firefox finding through", () => {
        const summary = summariseTrust(null, null, { present: true, profiles: 3 });
        expect(summary.firefoxDetected).toBe(true);
        expect(summary.firefoxProfiles).toBe(3);
    });
});

// ─────────────────────────────────────────────────────────────────────────────
// installEngineCa
// ─────────────────────────────────────────────────────────────────────────────

describe("installEngineCa()", () => {
    it("refuses early, with the pre-P3 message, when there is no CA to install", () => {
        primeEngine({ generated: false });
        return installEngineCa().then((result) => {
            expect(result).toEqual({ ok: false, error: "No CA certificate found. Generate one first." });
            // No point rendering an artifact for a certificate that does not exist.
            expect(mocks.call).not.toHaveBeenCalledWith("tls.exportCert", expect.anything());
            expect(mocks.installCA).not.toHaveBeenCalled();
        });
    });

    it("propagates an export failure instead of installing nothing", async () => {
        primeEngine({ exportOk: false, exportError: "the CA is unreadable" });
        const result = await installEngineCa();
        expect(result).toEqual({ ok: false, error: "the CA is unreadable" });
        expect(mocks.installCA).not.toHaveBeenCalled();
    });

    it("installs the exported bytes and records what it installed", async () => {
        const result = await installEngineCa();

        expect(result.ok).toBe(true);
        expect(result.fingerprint).toBe(REAL_CA_FINGERPRINT);
        expect(mocks.installCA).toHaveBeenCalledTimes(1);

        // The thumbprint handed to the platform tool is derived from the bytes that were written,
        // not from a second engine call — see the ordering note in the module header.
        const [ctx, certFile, thumbprint] = mocks.installCA.mock.calls[0];
        expect(ctx).toEqual({ platform: "win32" });
        expect(thumbprint).toBe(REAL_CA_THUMBPRINT_SHA1);
        expect(certFile).toBe(stagedPath());

        const saved = mocks.saveSettings.mock.calls[0][0];
        expect(saved.tlsTrustedCa).toMatchObject({
            fingerprint: REAL_CA_FINGERPRINT,
            thumbprintSha1: REAL_CA_THUMBPRINT_SHA1,
            pem: REAL_CA_PEM,
        });
        expect(saved.tlsTrustedCa.subject).toContain(REAL_CA_SUBJECT_MARKER);
    });

    it("deletes the staged certificate once the install has happened", async () => {
        await installEngineCa();
        expect(fs.existsSync(stagedPath())).toBe(false);
    });

    it("refuses a certificate it cannot parse, rather than trusting a file that is not one", async () => {
        // Reachable because `tls.importCert` preserves arbitrary bytes by design, so a hand-edited
        // `ca-cert.pem` is a real state. This is the clear error that used to arrive later as an
        // opaque TLS failure.
        mocks.writeArtifact.mockImplementation((_a: unknown, destPath: string) => {
            fs.writeFileSync(destPath, "not a certificate at all", "utf-8");
        });

        const result = await installEngineCa();

        expect(result.ok).toBe(false);
        expect(result.error).toContain("could not be read as a certificate");
        expect(mocks.installCA).not.toHaveBeenCalled();
        expect(mocks.saveSettings).not.toHaveBeenCalled();
    });

    it("un-trusts a previously-installed, different CA before installing the new one", async () => {
        mocks.loadSettings.mockReturnValue({ tlsTrustedCa: OLD_RECORD });

        const result = await installEngineCa();

        expect(result.replacedStale).toBe(true);
        // The old certificate's own thumbprint, from the record — the engine no longer has those
        // bytes, which is exactly why the record carries the PEM.
        expect(mocks.uninstallCA).toHaveBeenCalledWith(
            expect.anything(),
            expect.stringContaining("bifurc-ca-"),
            OLD_RECORD.thumbprintSha1,
        );
        // Order matters: never two Bifurc roots trusted at once.
        expect(mocks.uninstallCA.mock.invocationCallOrder[0])
            .toBeLessThan(mocks.installCA.mock.invocationCallOrder[0]);
    });

    it("does not un-trust anything when the record is already the current CA", async () => {
        mocks.loadSettings.mockReturnValue({ tlsTrustedCa: CURRENT_RECORD });
        const result = await installEngineCa();

        expect(result.replacedStale).toBe(false);
        expect(mocks.uninstallCA).not.toHaveBeenCalled();
    });

    it("does not record a failed install", async () => {
        // A record of a certificate that is not in the store would make the next drift comparison
        // report 'trusted' for a machine that trusts nothing.
        mocks.installCA.mockReturnValue({ ok: false, error: "access denied" });

        const result = await installEngineCa();

        expect(result.ok).toBe(false);
        expect(mocks.saveSettings).not.toHaveBeenCalled();
        expect(fs.existsSync(stagedPath())).toBe(false);
    });

    it("keeps the staged certificate when the platform fell back to manual instructions", async () => {
        // An instruction that names a path we deleted is not an instruction.
        mocks.installCA.mockReturnValue({
            ok: false,
            needsManualInstall: true,
            instructions: "sudo cp … && sudo update-ca-certificates",
        });

        const result = await installEngineCa();

        expect(result.needsManualInstall).toBe(true);
        expect(result.instructions).toContain("update-ca-certificates");
        expect(fs.existsSync(stagedPath())).toBe(true);
        fs.rmSync(stagedPath(), { force: true });
    });

    it("surfaces the Firefox finding, which is the difference between working and 'only Chrome works'", async () => {
        mocks.detectFirefox.mockReturnValue({ present: true, profiles: 2 });
        const result = await installEngineCa();

        expect(result.firefoxDetected).toBe(true);
        expect(result.firefoxProfiles).toBe(2);
    });
});

// ─────────────────────────────────────────────────────────────────────────────
// uninstallEngineCa — two-sided, and allowed to half-succeed
// ─────────────────────────────────────────────────────────────────────────────

describe("uninstallEngineCa()", () => {
    it("reports both halves when there is a record to un-trust", async () => {
        mocks.loadSettings.mockReturnValue({ tlsTrustedCa: CURRENT_RECORD });

        const result = await uninstallEngineCa();

        expect(result).toMatchObject({ ok: true, engineRemoved: true, clientUntrusted: true });
        expect(mocks.uninstallCA).toHaveBeenCalledWith(
            expect.anything(),
            expect.stringContaining("bifurc-ca-"),
            REAL_CA_THUMBPRINT_SHA1,
        );
        expect(mocks.saveSettings.mock.calls[0][0].tlsTrustedCa).toBeNull();
    });

    it("says so plainly when this client has no record of installing anything", async () => {
        // The honest answer: this client cannot remove what it never installed, and the note tells
        // the user where to look if they installed it by hand from an exported file.
        const result = await uninstallEngineCa();

        expect(result.engineRemoved).toBe(true);
        expect(result.clientUntrusted).toBe(false);
        expect(result.clientUntrustNote).toContain("no record");
        expect(mocks.uninstallCA).not.toHaveBeenCalled();
        expect(mocks.saveSettings).not.toHaveBeenCalled();
    });

    it("keeps the record when the un-trust fails, so the certificate can still be removed later", async () => {
        // Clearing the record here would discard the only remaining copy of the certificate the OS
        // still trusts — turning a retryable failure into an unremovable one.
        mocks.loadSettings.mockReturnValue({ tlsTrustedCa: CURRENT_RECORD });
        mocks.uninstallCA.mockReturnValue({ ok: false, error: "keychain locked" });

        const result = await uninstallEngineCa();

        expect(result.clientUntrusted).toBe(false);
        expect(result.error).toContain("keychain locked");
        expect(mocks.saveSettings).not.toHaveBeenCalled();
    });

    it("passes the manual instructions through when the platform chain cannot un-trust unattended", async () => {
        mocks.loadSettings.mockReturnValue({ tlsTrustedCa: CURRENT_RECORD });
        mocks.uninstallCA.mockReturnValue({
            ok: false,
            needsManualInstall: true,
            instructions: "trust anchor --remove …",
        });

        const result = await uninstallEngineCa();

        expect(result.needsManualInstall).toBe(true);
        expect(result.instructions).toContain("trust anchor --remove");
    });

    it("still reports the engine's half when there was nothing there to delete", async () => {
        primeEngine({ engineRemoved: false });
        const result = await uninstallEngineCa();
        expect(result.engineRemoved).toBe(false);
    });

    it("survives a write failure while staging the recorded certificate", async () => {
        // The temp file is written from the record, and the record comes from a JSON file a user can
        // edit. A failure here must be a result, not a rejected promise in an ipcMain handler.
        mocks.loadSettings.mockReturnValue({ tlsTrustedCa: { ...CURRENT_RECORD, pem: "" } });
        mocks.uninstallCA.mockImplementation(() => { throw new Error("staging exploded"); });

        const result = await uninstallEngineCa();

        expect(result.ok).toBe(true);
        expect(result.clientUntrusted).toBe(false);
    });
});
