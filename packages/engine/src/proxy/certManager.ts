/**
 * The CA keypair — the **engine half** of the certificate lifecycle (`File_Ops_Protocol.md` §6.1).
 *
 * ```
 * ENGINE half (this file)                  CLIENT half (src/ipc/certTrust.ts)
 * ───────────────────────                  ──────────────────────────────────
 * generate the keypair                     fetch the cert bytes
 * store them in the data dir               install into the OS trust store
 * report an identity for them              compare against what it installed
 * delete them on removeCert                un-trust on removeCert
 * ```
 *
 * ## What moved out, and why (P3 work item 5)
 *
 * `installCA()` used to live here and shell out to `certutil` / `security` /
 * `update-ca-certificates`. That was the boundary violation: the *host's* trust store belongs to the
 * client (`plan/handler-classification.md` — CLIENT-classified), and an engine running in another
 * process (P4) or another container (P9) cannot reach it at all. It now lives in
 * `src/ipc/certTrust.ts`, together with the un-trust it never had.
 *
 * What replaced it here is an **identity**. `certPath` / `keyPath` used to be the result of
 * `generateCA()`, and a path is exactly what a client on another machine cannot use. A fingerprint
 * is the thing that makes drift detectable — see `identifyCert()`.
 *
 * ## Why a fingerprint and not just "the bytes changed"
 *
 * `sha256:<hex>` over the DER is what a browser's certificate viewer and the OS certificate dialogs
 * show. A user comparing our value against theirs can see they match; comparing two opaque PEM
 * blobs is not something a person can do. It is also the only value that survives the client's
 * un-trust path being run later, when the engine may already have regenerated and the old bytes are
 * gone.
 */
import * as fs from "fs";
import * as path from "path";
import { X509Certificate } from "crypto";
import { createCA } from "mkcert";
import { appDataDir } from "../store/appSettings";

const CA_CERT_FILE = "ca-cert.pem";
const CA_KEY_FILE = "ca-key.pem";

/**
 * Where the CA lives. **The single answer to that question**, because there used to be two.
 *
 * `fileOps/commands.ts` (which reads and writes the same two files) had its own copy of these
 * constants and its own `path.join(appDataDir(), …)` calls, and that comment already warned that
 * "these three commands must resolve it the same way or the certificate becomes invisible to
 * whichever half looked in the other directory". Two copies of a path expression is one refactor
 * away from two different answers, so the second consumer is the moment to collapse it — which is
 * now, since `certCommands.ts` is the second consumer.
 *
 * **`appDataDir()`, not `dataDir()`.** They are the same directory on Linux/macOS —
 * `settingsPath()` falls through to `path.join(dataDir(), "app.json")` — but on Windows
 * `appDataDir()` resolves to `%LOCALAPPDATA%/Bifurc` regardless of `setDataRoot()`. That
 * divergence is **pre-existing and recorded** (`plan/03`, and the blob-root note in
 * `plan/README.md`'s P3 row): on Windows a `--data-dir` run splits workspaces from blobs, and the
 * CA has always followed `%LOCALAPPDATA%`. Fixing it is a product decision for P8/P9; introducing
 * a *third* answer here would not be.
 */
export function caDir(): string {
    return appDataDir();
}

export function caCertPath(): string {
    return path.join(appDataDir(), CA_CERT_FILE);
}

export function caKeyPath(): string {
    return path.join(appDataDir(), CA_KEY_FILE);
}

/**
 * What a certificate *is*, independent of where it is stored.
 *
 * Three views of one certificate, because the three platforms identify certificates differently:
 * `fingerprint` is what a human compares, `thumbprintSha1` is what `certutil -delstore` and
 * `security delete-certificate -Z` want, and `subject` is what the client shows when it reports
 * that the engine's CA changed.
 */
export interface CertIdentity {
    /** `sha256:<64 lowercase hex>` over the DER — the string a browser's cert viewer shows. */
    fingerprint: string;
    /** SHA-1, uppercase, no separators — Windows' "certificate hash" / `certutil`'s CertId. */
    thumbprintSha1: string;
    /** The subject distinguished name, as Node formats it. Display only; never parsed. */
    subject: string;
}

function hexNoSeparators(value: string): string {
    return value.replace(/:/g, "").toUpperCase();
}

/**
 * Parse a PEM certificate into its identity, or `null` if it is not a certificate.
 *
 * `null` rather than a throw is the load-bearing part of the signature: the two callers both need
 * "this file exists but is not usable" to be an *ordinary answer* rather than an exception —
 * `getCertStatus()` reports it as `fingerprint: null`, and `generateCA()` treats it as a bug in
 * `mkcert` rather than as a user error.
 *
 * `crypto.X509Certificate` rather than hashing the PEM body by hand: it validates the DER as it
 * parses, so "is this a certificate?" and "what is its fingerprint?" are one operation and cannot
 * disagree. Hashing the base64 body directly would happily fingerprint a text file.
 */
export function identifyCert(pem: string): CertIdentity | null {
    try {
        const x509 = new X509Certificate(pem);
        return {
            fingerprint: `sha256:${x509.fingerprint256.replace(/:/g, "").toLowerCase()}`,
            thumbprintSha1: hexNoSeparators(x509.fingerprint),
            subject: x509.subject,
        };
    } catch {
        return null;
    }
}

/** The fingerprint alone, for the two results that carry only that. */
export function fingerprintOf(pem: string): string | null {
    return identifyCert(pem)?.fingerprint ?? null;
}

export interface GeneratedCA {
    certPath: string;
    keyPath: string;
    /**
     * Always present in practice — `mkcert` just produced the certificate, so it parses. It is
     * typed as possibly-null so the caller has to decide what a `null` means instead of assuming
     * it cannot happen; see the throw below.
     */
    identity: CertIdentity | null;
}

export async function generateCA(dataDir: string): Promise<GeneratedCA> {
    const ca = await createCA({
        organization: "Bifurc CA",
        countryCode: "US",
        state: "Development",
        locality: "Local",
        validity: 3650, // 10 years in days
    });

    const certPath = path.join(dataDir, CA_CERT_FILE);
    const keyPath = path.join(dataDir, CA_KEY_FILE);

    // `writeFileSync` does not create parents, and an `ENOENT` from it would be returned to the
    // client as `{ok: false, error: "ENOENT: … open 'C:\\Users\\…\\Bifurc\\ca-cert.pem'"}` — an
    // absolute engine path in a protocol result, which `File_Ops_Protocol.md` §8 names as a
    // boundary violation. Creating the directory is cheaper than sanitising that message, and it
    // removes the failure rather than the evidence of it.
    fs.mkdirSync(dataDir, { recursive: true });

    fs.writeFileSync(certPath, ca.cert, { encoding: "utf-8", mode: 0o600 });
    fs.writeFileSync(keyPath, ca.key, { encoding: "utf-8", mode: 0o600 });

    // Not a user-facing failure path: `mkcert` returning something `X509Certificate` cannot parse
    // is a broken dependency, not a broken certificate. Surfacing it as an `ok: true` result with
    // no fingerprint would leave the UI claiming success over a CA that can never be trusted.
    const identity = identifyCert(ca.cert);
    if (!identity) {
        throw new Error(
            "mkcert produced a CA certificate that could not be parsed; refusing to report success.",
        );
    }

    return { certPath, keyPath, identity };
}

/**
 * The engine's own view of its CA — and deliberately **path-free**, even though this is the engine
 * looking at the engine's own directory.
 *
 * `certPath` / `keyPath` used to be here and were returned over the wire. They are not returned any
 * more, so they are not stored any more either: keeping them on the type would leave a field that
 * one careless line could put back on the wire. The paths are computed where they are needed
 * (`generateCA`, `removeCA`) and nowhere else.
 */
export interface CertStatus {
    generated: boolean;
    /** `null` when there is no CA, **or** when the file exists and is not a certificate. */
    fingerprint: string | null;
}

export function getCertStatus(dataDir: string): CertStatus {
    const certPath = path.join(dataDir, CA_CERT_FILE);
    const keyPath = path.join(dataDir, CA_KEY_FILE);
    if (!fs.existsSync(certPath) || !fs.existsSync(keyPath)) {
        return { generated: false, fingerprint: null };
    }

    // A read failure and an unparseable file are the same answer: the CA is present but unusable.
    let pem: string;
    try {
        pem = fs.readFileSync(certPath, "utf-8");
    } catch {
        return { generated: true, fingerprint: null };
    }
    return { generated: true, fingerprint: fingerprintOf(pem) };
}

/**
 * Delete both halves of the keypair. Returns whether anything was actually there.
 *
 * `engineRemoved` is the engine's half of `tls.removeCert`'s two-sided result. It is `true` if
 * either file was present, because "there was something to remove and it is gone now" is what the
 * caller is asking; a `false` tells the client there was nothing to un-trust in the first place.
 */
export function removeCA(dataDir: string): { engineRemoved: boolean } {
    const certPath = path.join(dataDir, CA_CERT_FILE);
    const keyPath = path.join(dataDir, CA_KEY_FILE);

    let engineRemoved = false;
    for (const p of [certPath, keyPath]) {
        try {
            if (fs.existsSync(p)) {
                fs.unlinkSync(p);
                engineRemoved = true;
            }
        } catch {
            /* best effort — a locked file must not stop the other one from being removed */
        }
    }
    return { engineRemoved };
}
