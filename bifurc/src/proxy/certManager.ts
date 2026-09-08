import * as fs from "fs";
import * as path from "path";
import * as os from "os";
import * as cp from "child_process";
import { createCA } from "mkcert";

const CA_CERT_FILE = "ca-cert.pem";
const CA_KEY_FILE = "ca-key.pem";

export interface CertStatus {
    generated: boolean;
    certPath: string | null;
    keyPath: string | null;
}

export async function generateCA(dataDir: string): Promise<{ certPath: string; keyPath: string }> {
    const ca = await createCA({
        organization: "Bifurc CA",
        countryCode: "US",
        state: "Development",
        locality: "Local",
        validity: 3650, // 10 years in days
    });

    const certPath = path.join(dataDir, CA_CERT_FILE);
    const keyPath = path.join(dataDir, CA_KEY_FILE);

    fs.writeFileSync(certPath, ca.cert, { encoding: "utf-8", mode: 0o600 });
    fs.writeFileSync(keyPath, ca.key, { encoding: "utf-8", mode: 0o600 });

    return { certPath, keyPath };
}

export interface InstallResult {
    ok: boolean;
    needsManualInstall?: boolean;
    instructions?: string;
    error?: string;
}

export function installCA(certPath: string): InstallResult {
    try {
        if (process.platform === "win32") {
            // Windows: add to current user Trusted Root CA store — no admin required
            cp.execSync(`certutil -addstore -user Root "${certPath}"`, {
                stdio: "ignore",
                timeout: 15000,
            });
            return { ok: true };
        }

        if (process.platform === "darwin") {
            // macOS: add to login keychain — triggers a keychain password dialog
            const loginKeychain = path.join(os.homedir(), "Library", "Keychains", "login.keychain-db");
            cp.execSync(
                `security add-trusted-cert -d -r trustRoot -k "${loginKeychain}" "${certPath}"`,
                { stdio: "ignore", timeout: 15000 },
            );
            return { ok: true };
        }

        // Linux: needs root — return manual instructions
        return {
            ok: false,
            needsManualInstall: true,
            instructions:
                `sudo cp "${certPath}" /usr/local/share/ca-certificates/bifurc-ca.crt\n` +
                `sudo update-ca-certificates`,
        };
    } catch (e: unknown) {
        return { ok: false, error: e instanceof Error ? e.message : String(e) };
    }
}

export function getCertStatus(dataDir: string): CertStatus {
    const certPath = path.join(dataDir, CA_CERT_FILE);
    const keyPath = path.join(dataDir, CA_KEY_FILE);
    const generated = fs.existsSync(certPath) && fs.existsSync(keyPath);
    return {
        generated,
        certPath: generated ? certPath : null,
        keyPath: generated ? keyPath : null,
    };
}
