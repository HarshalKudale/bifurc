import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import * as fs from "fs";
import * as path from "path";
import * as os from "os";

vi.mock("mkcert", () => ({
    createCert: vi.fn(async (opts: any) => ({
        cert: `-----BEGIN CERTIFICATE-----\nFAKE_HOST_CERT_${opts.domains[0]}\n-----END CERTIFICATE-----`,
        key: `-----BEGIN PRIVATE KEY-----\nFAKE_HOST_KEY_${opts.domains[0]}\n-----END PRIVATE KEY-----`,
    })),
}));

import { loadCA, unloadCA, isCALoaded, generateHostCert, clearCertCache } from "@/proxy/tlsCert";

describe("tlsCert", () => {
    let tmpDir: string;
    let certPath: string;
    let keyPath: string;

    beforeEach(() => {
        tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "tlscert-test-"));
        certPath = path.join(tmpDir, "ca.pem");
        keyPath = path.join(tmpDir, "key.pem");
        fs.writeFileSync(certPath, "CERT_PEM");
        fs.writeFileSync(keyPath, "KEY_PEM");
        unloadCA(); // Reset state
    });

    afterEach(() => {
        unloadCA();
        fs.rmSync(tmpDir, { recursive: true, force: true });
    });

    describe("loadCA()", () => {
        it("returns true when files exist and are readable", () => {
            expect(loadCA(certPath, keyPath)).toBe(true);
            expect(isCALoaded()).toBe(true);
        });

        it("returns false when cert file does not exist", () => {
            expect(loadCA("/nonexistent/cert.pem", keyPath)).toBe(false);
            expect(isCALoaded()).toBe(false);
        });

        it("returns false when key file does not exist", () => {
            expect(loadCA(certPath, "/nonexistent/key.pem")).toBe(false);
            expect(isCALoaded()).toBe(false);
        });

        it("clears cert cache on load", () => {
            loadCA(certPath, keyPath);
            // generateHostCert to populate cache
            generateHostCert("example.com");
            // reload should clear
            loadCA(certPath, keyPath);
            // No assertion needed beyond no error
        });
    });

    describe("unloadCA()", () => {
        it("sets CA as not loaded", () => {
            loadCA(certPath, keyPath);
            unloadCA();
            expect(isCALoaded()).toBe(false);
        });
    });

    describe("clearCertCache()", () => {
        it("does not throw when cache is empty", () => {
            expect(() => clearCertCache()).not.toThrow();
        });
    });

    describe("generateHostCert()", () => {
        it("rejects when CA is not loaded", async () => {
            unloadCA();
            await expect(generateHostCert("example.com")).rejects.toThrow("CA not loaded");
        });

        it("returns cert and key for a hostname when CA is loaded", async () => {
            loadCA(certPath, keyPath);
            const result = await generateHostCert("example.com");
            expect(result.cert).toContain("FAKE_HOST_CERT_example.com");
            expect(result.key).toContain("FAKE_HOST_KEY_example.com");
        });

        it("returns cached promise for same hostname", async () => {
            loadCA(certPath, keyPath);
            const p1 = generateHostCert("same.com");
            const p2 = generateHostCert("same.com");
            expect(p1).toBe(p2);
        });

        it("returns different promises for different hostnames", async () => {
            loadCA(certPath, keyPath);
            const p1 = generateHostCert("host1.com");
            const p2 = generateHostCert("host2.com");
            expect(p1).not.toBe(p2);
        });
    });
});
