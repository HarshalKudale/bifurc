/**
 * `packages/engine/src/proxy/certManager.ts` — the **engine half** of the certificate lifecycle.
 *
 * ## What changed in P3 work item 5, and why this file looks different
 *
 * `installCA()` used to be tested here by mocking `child_process.execSync` and redefining
 * `process.platform`. It is **gone from this module** — installing into the OS trust store is a
 * client-classified operation (`File_Ops_Protocol.md` §6.2) and it now lives in
 * `src/ipc/certTrust.ts`, where the platform command shapes are asserted as exact argv arrays with
 * no mocking at all. The four tests that used to be here are superseded by that suite; the
 * `child_process` mock is gone with them.
 *
 * What replaced it here is an **identity** — `identifyCert()` — and the tests for it are the
 * interesting part, because a fingerprint is only useful if it is the *right* fingerprint.
 *
 * ## Why the fixture is a real certificate
 *
 * `generateCA()` now refuses to report success on output it cannot parse, so the old
 * `"-----BEGIN CERTIFICATE-----\nFAKE_CERT\n-----END CERTIFICATE-----"` fixture makes it **throw**.
 * That is the point of the change, and it is asserted explicitly below. Everything else uses
 * `REAL_CA_PEM`, a genuine `mkcert` CA generated once and pasted in.
 *
 * The literal fingerprint in `EXPECTED_FINGERPRINT` is what makes this a regression test rather
 * than a restatement of the implementation: it was computed independently (by `openssl`-equivalent
 * tooling and by hand, see the cross-check test) and is pinned here.
 */
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import * as crypto from "crypto";
import * as fs from "fs";
import * as path from "path";
import * as os from "os";

/**
 * A real CA certificate, plus its independently-computed fingerprints. See
 * `tests/fixtures/certFixtures.ts` for why the fixture is genuine rather than a placeholder, and
 * for the `mkcert` invocation it came from.
 */
import {
    FAKE_CA_PEM,
    REAL_CA_FINGERPRINT,
    REAL_CA_PEM,
    REAL_CA_SUBJECT_MARKER,
    REAL_CA_THUMBPRINT_SHA1,
    REAL_KEY_PEM,
} from "../fixtures/certFixtures";

const EXPECTED_FINGERPRINT = REAL_CA_FINGERPRINT;
const EXPECTED_THUMBPRINT_SHA1 = REAL_CA_THUMBPRINT_SHA1;

const fixtures = vi.hoisted(() => ({ cert: "", key: "" }));

vi.mock("mkcert", () => ({
    createCA: vi.fn(async () => ({ cert: fixtures.cert, key: fixtures.key })),
}));

import { generateCA, getCertStatus, identifyCert, removeCA } from "@bifurc/engine/proxy/certManager";

describe("identifyCert()", () => {
    it("reports the fingerprint a browser would show, for a real certificate", () => {
        const identity = identifyCert(REAL_CA_PEM);
        expect(identity).not.toBeNull();
        expect(identity!.fingerprint).toBe(EXPECTED_FINGERPRINT);
    });

    it("reports the SHA-1 thumbprint the platform tools need, upper-case and unseparated", () => {
        // `certutil -delstore -user Root <CertId>` and `security delete-certificate -Z <hash>` both
        // take this form. Node hands it back colon-separated in whatever case it likes, so the
        // normalisation is real work and not a formality.
        const identity = identifyCert(REAL_CA_PEM);
        expect(identity!.thumbprintSha1).toBe(EXPECTED_THUMBPRINT_SHA1);
        expect(identity!.thumbprintSha1).not.toContain(":");
        expect(identity!.thumbprintSha1).toBe(identity!.thumbprintSha1.toUpperCase());
    });

    it("carries the subject, so a UI can say which CA it is talking about", () => {
        expect(identifyCert(REAL_CA_PEM)!.subject).toContain(REAL_CA_SUBJECT_MARKER);
    });

    it("cross-checks the fingerprint against a hash of the DER, computed a different way", () => {
        // The independent derivation: decode the PEM body ourselves and SHA-256 the DER. This is a
        // different code path from `X509Certificate#fingerprint256`, so agreeing means both are
        // hashing the same bytes — a test that only re-read the same API would prove nothing.
        const body = REAL_CA_PEM.replace(/-----[^-]+-----/g, "").replace(/\s+/g, "");
        const der = Buffer.from(body, "base64");
        const expected = "sha256:" + crypto.createHash("sha256").update(der).digest("hex");
        expect(identifyCert(REAL_CA_PEM)!.fingerprint).toBe(expected);
    });

    it("returns null for a PEM whose body is not DER", () => {
        expect(identifyCert(FAKE_CA_PEM)).toBeNull();
    });

    it("returns null for arbitrary text, rather than fingerprinting it", () => {
        // The trap this avoids: hashing the base64 body by hand would happily "fingerprint" any
        // text file, and the caller would then trust a certificate that does not exist.
        expect(identifyCert("hello, this is not a certificate")).toBeNull();
        expect(identifyCert("")).toBeNull();
        expect(identifyCert("-----BEGIN CERTIFICATE-----\nAAAA\n-----END CERTIFICATE-----")).toBeNull();
    });
});

describe("generateCA()", () => {
    let tmpDir: string;

    beforeEach(() => {
        tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "certmgr-test-"));
        fixtures.cert = REAL_CA_PEM;
        fixtures.key = REAL_KEY_PEM;
    });

    afterEach(() => {
        fs.rmSync(tmpDir, { recursive: true, force: true });
    });

    it("creates cert and key files in the data directory", async () => {
        const result = await generateCA(tmpDir);
        expect(result.certPath).toContain("ca-cert.pem");
        expect(result.keyPath).toContain("ca-key.pem");
        expect(fs.existsSync(result.certPath)).toBe(true);
        expect(fs.existsSync(result.keyPath)).toBe(true);
    });

    it("writes PEM content to the files", async () => {
        const result = await generateCA(tmpDir);
        const cert = fs.readFileSync(result.certPath, "utf-8");
        expect(cert).toContain("CERTIFICATE");
    });

    it("returns the identity of the certificate it just wrote", async () => {
        const result = await generateCA(tmpDir);
        expect(result.identity!.fingerprint).toBe(EXPECTED_FINGERPRINT);
        expect(result.identity!.thumbprintSha1).toBe(EXPECTED_THUMBPRINT_SHA1);
    });

    it("creates the data directory when it does not exist yet", async () => {
        // `writeFileSync` does not create parents, and an `ENOENT` from it would be returned to the
        // client as an error message containing the engine's absolute path — a boundary violation
        // under `File_Ops_Protocol.md` §8. Creating the directory removes the failure instead.
        const nested = path.join(tmpDir, "does", "not", "exist");
        const result = await generateCA(nested);
        expect(fs.existsSync(result.certPath)).toBe(true);
    });

    it("refuses to report success on output it cannot parse", async () => {
        // A `mkcert` that returns something unparseable is a broken dependency, not a broken
        // certificate. Returning `{ ok: true }` without a fingerprint would leave the UI claiming a
        // CA exists that can never be trusted.
        fixtures.cert = FAKE_CA_PEM;
        await expect(generateCA(tmpDir)).rejects.toThrow(/could not be parsed/);
    });
});

describe("getCertStatus()", () => {
    let tmpDir: string;

    beforeEach(() => {
        tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "certstatus-test-"));
    });

    afterEach(() => {
        fs.rmSync(tmpDir, { recursive: true, force: true });
    });

    it("returns generated: false when files do not exist", () => {
        const status = getCertStatus(tmpDir);
        expect(status.generated).toBe(false);
        expect(status.fingerprint).toBeNull();
    });

    it("returns the fingerprint when a real cert and key exist", () => {
        fs.writeFileSync(path.join(tmpDir, "ca-cert.pem"), REAL_CA_PEM);
        fs.writeFileSync(path.join(tmpDir, "ca-key.pem"), REAL_KEY_PEM);
        const status = getCertStatus(tmpDir);
        expect(status.generated).toBe(true);
        expect(status.fingerprint).toBe(EXPECTED_FINGERPRINT);
    });

    it("reports no paths at all, whatever the state", () => {
        // The engine's own directory layout is the engine's business. `certPath` / `keyPath` used to
        // be on this result and travelled over the wire; `plan/protocol-changes.md` (2026-09-17)
        // removed them, and this asserts they did not come back.
        fs.writeFileSync(path.join(tmpDir, "ca-cert.pem"), REAL_CA_PEM);
        fs.writeFileSync(path.join(tmpDir, "ca-key.pem"), REAL_KEY_PEM);
        expect(Object.keys(getCertStatus(tmpDir)).sort()).toEqual(["fingerprint", "generated"]);
    });

    it("returns generated: false when only cert exists", () => {
        fs.writeFileSync(path.join(tmpDir, "ca-cert.pem"), REAL_CA_PEM);
        const status = getCertStatus(tmpDir);
        expect(status.generated).toBe(false);
    });

    it("distinguishes 'no CA' from 'a file that is not a certificate'", () => {
        // `generated: true, fingerprint: null` is the state the pre-P3 code could not express — it
        // is why a corrupt `ca-cert.pem` used to surface later as an opaque TLS failure.
        fs.writeFileSync(path.join(tmpDir, "ca-cert.pem"), "this is not a certificate");
        fs.writeFileSync(path.join(tmpDir, "ca-key.pem"), "nor is this a key");
        const status = getCertStatus(tmpDir);
        expect(status.generated).toBe(true);
        expect(status.fingerprint).toBeNull();
    });
});

describe("removeCA()", () => {
    let tmpDir: string;

    beforeEach(() => {
        tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "certremove-test-"));
    });

    afterEach(() => {
        fs.rmSync(tmpDir, { recursive: true, force: true });
    });

    it("deletes both files and reports that it removed something", () => {
        fs.writeFileSync(path.join(tmpDir, "ca-cert.pem"), REAL_CA_PEM);
        fs.writeFileSync(path.join(tmpDir, "ca-key.pem"), REAL_KEY_PEM);

        expect(removeCA(tmpDir)).toEqual({ engineRemoved: true });
        expect(fs.existsSync(path.join(tmpDir, "ca-cert.pem"))).toBe(false);
        expect(fs.existsSync(path.join(tmpDir, "ca-key.pem"))).toBe(false);
    });

    it("reports engineRemoved: false when there was nothing to remove", () => {
        // The engine's half of the two-sided result. `false` tells the client there was no
        // certificate here to have trusted in the first place.
        expect(removeCA(tmpDir)).toEqual({ engineRemoved: false });
    });

    it("removes the certificate even when the key is already gone", () => {
        fs.writeFileSync(path.join(tmpDir, "ca-cert.pem"), REAL_CA_PEM);
        expect(removeCA(tmpDir).engineRemoved).toBe(true);
        expect(fs.existsSync(path.join(tmpDir, "ca-cert.pem"))).toBe(false);
    });
});
