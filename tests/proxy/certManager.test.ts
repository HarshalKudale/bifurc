import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import * as fs from "fs";
import * as path from "path";
import * as os from "os";

vi.mock("mkcert", () => ({
    createCA: vi.fn(async () => ({
        cert: "-----BEGIN CERTIFICATE-----\nFAKE_CERT\n-----END CERTIFICATE-----",
        key: "-----BEGIN PRIVATE KEY-----\nFAKE_KEY\n-----END PRIVATE KEY-----",
    })),
}));

vi.mock("child_process", () => ({
    execSync: vi.fn(),
}));

import { generateCA, installCA, getCertStatus } from "@/proxy/certManager";
import * as cp from "child_process";

describe("generateCA()", () => {
    let tmpDir: string;

    beforeEach(() => {
        tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "certmgr-test-"));
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
});

describe("installCA()", () => {
    const originalPlatform = process.platform;

    afterEach(() => {
        Object.defineProperty(process, "platform", { value: originalPlatform });
        vi.mocked(cp.execSync).mockReset();
    });

    it("runs certutil on Windows", () => {
        Object.defineProperty(process, "platform", { value: "win32" });
        const result = installCA("/fake/cert.pem");
        expect(result.ok).toBe(true);
        expect(cp.execSync).toHaveBeenCalledWith(
            expect.stringContaining("certutil"),
            expect.any(Object),
        );
    });

    it("runs security add-trusted-cert on macOS", () => {
        Object.defineProperty(process, "platform", { value: "darwin" });
        const result = installCA("/fake/cert.pem");
        expect(result.ok).toBe(true);
        expect(cp.execSync).toHaveBeenCalledWith(
            expect.stringContaining("security add-trusted-cert"),
            expect.any(Object),
        );
    });

    it("returns manual instructions on Linux", () => {
        Object.defineProperty(process, "platform", { value: "linux" });
        const result = installCA("/fake/cert.pem");
        expect(result.ok).toBe(false);
        expect(result.needsManualInstall).toBe(true);
        expect(result.instructions).toContain("update-ca-certificates");
    });

    it("returns error when execSync throws", () => {
        Object.defineProperty(process, "platform", { value: "win32" });
        vi.mocked(cp.execSync).mockImplementation(() => { throw new Error("access denied"); });
        const result = installCA("/fake/cert.pem");
        expect(result.ok).toBe(false);
        expect(result.error).toContain("access denied");
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
    });

    it("returns generated: true when both cert and key exist", () => {
        fs.writeFileSync(path.join(tmpDir, "ca-cert.pem"), "cert");
        fs.writeFileSync(path.join(tmpDir, "ca-key.pem"), "key");
        const status = getCertStatus(tmpDir);
        expect(status.generated).toBe(true);
        expect(status.certPath).toContain("ca-cert.pem");
        expect(status.keyPath).toContain("ca-key.pem");
    });

    it("returns generated: false when only cert exists", () => {
        fs.writeFileSync(path.join(tmpDir, "ca-cert.pem"), "cert");
        const status = getCertStatus(tmpDir);
        expect(status.generated).toBe(false);
    });
});
