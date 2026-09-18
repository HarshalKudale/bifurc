/**
 * The **client half** of the certificate lifecycle (`File_Ops_Protocol.md` §6.1) — the host's OS
 * trust store.
 *
 * ```
 * ENGINE half (packages/engine/src/proxy/certManager.ts)   CLIENT half (this file)
 * ──────────────────────────────────────────────────────   ──────────────────────────────
 * generate the keypair                                     fetch the cert bytes
 * store them in the data dir                               install into the OS trust store
 * report an identity for them                              compare against what it installed
 * delete them on removeCert                                un-trust on removeCert
 * ```
 *
 * ## Why this is client-side at all
 *
 * `plan/handler-classification.md` classifies the trust store as **CLIENT**: it is a host-scoped,
 * client-local mutation, meaningless on a machine that is not the one running the browser. It used
 * to be `certManager.installCA()` — the engine shelling out to `certutil` — which works only while
 * the engine and the client are the same process. P4's transport and P9's container both break
 * that, and a remote engine has no way to reach the user's keychain.
 *
 * ## The privilege matrix (`File_Ops_Protocol.md` §6.2)
 *
 * | Platform | Install | Privilege |
 * |---|---|---|
 * | Windows | `certutil -addstore -user Root <cert>` | **none** — the user's own Root store |
 * | macOS | `security add-trusted-cert -d -r trustRoot -k <loginKeychain> <cert>` | **none** — may prompt for the keychain password |
 * | Linux | `trust anchor <cert>` → `pkexec trust anchor <cert>` → instructions | **root**, hence the chain |
 *
 * Linux is the outlier and the only platform with a **fallback chain** rather than a single
 * command: p11-kit's `trust anchor` writes to the *user* trust store and often needs no elevation
 * at all, `pkexec` raises a polkit graphical prompt when it does, and if neither binary is present
 * the last resort is a written instruction that cannot fail. `File_Ops_Protocol.md` §6.2 asks for
 * exactly this order and flags "verify `trust anchor` works without elevation" — it cannot be
 * verified in a Windows development environment, so it is recorded as a P9/P12 task in
 * `plan/13-checklist.md` rather than asserted here.
 *
 * ## Two verified facts that shape the un-trust path
 *
 * Both were measured against a real `certutil` (Windows 11) rather than assumed:
 *
 * 1. **`certutil -delstore` exits 0 whether or not it deleted anything.** Deleting a certificate
 *    that is not in the store prints *"CertUtil: -delstore command completed successfully."* and
 *    exits **0**. So its exit code cannot distinguish "removed" from "was never there", and a
 *    result built on it would report a successful un-trust that never happened.
 * 2. **`certutil -store -user Root <thumbprint>` does discriminate**: exit **0** when present,
 *    exit **17** (`NTE_NOT_FOUND`) when absent. That is the verification primitive, and it is why
 *    `TrustResult.verified` exists as a separate field from `ok`.
 *
 * `uninstallCA()` therefore **re-queries the store** instead of trusting the delete command's exit
 * status. On macOS the same role is played by `security find-certificate -Z <sha1>`. On Linux there
 * is no equivalent that this environment can verify, so `probeTrust()` returns `null` — "cannot
 * answer" — and `verified` is left undefined rather than being reported as `false`.
 *
 * ## The Firefox gotcha (`File_Ops_Protocol.md` §6.3)
 *
 * **Firefox does not use the OS trust store.** It ships its own NSS store, so a `certutil` install
 * makes Chrome, Edge and Safari work and leaves Firefox broken — with the same opaque TLS error the
 * user would blame the engine for. `detectFirefox()` exists so the UI can say so *before* the issue
 * report. Detection only; installing into NSS is not attempted.
 *
 * ## Testability
 *
 * Every side effect is behind `TrustContext`, so the platform command shapes are asserted as exact
 * argv arrays with no mocking at all (`tests/ipc/certTrust.test.ts`). The three primitives —
 * `run`, `probe`, `has` — are the only things a test has to fake, and `run` vs `probe` is the
 * distinction between "a non-zero exit is an error" and "a non-zero exit is an answer".
 */
import * as cp from "child_process";
import * as fs from "fs";
import * as os from "os";
import * as path from "path";

/** Matches the pre-P3 `installCA` timeout: long enough for a keychain prompt, short enough to fail. */
const CMD_TIMEOUT_MS = 15000;

export interface TrustStep {
    cmd: string;
    args: string[];
    /** Human-readable, for logs and for explaining a failure. Never shown to the user verbatim. */
    label: string;
}

/**
 * Everything platform-specific about this module, injectable.
 *
 * `run` and `probe` are deliberately separate rather than one function with a `throws` flag: they
 * express different *meanings*, not different options. `run` answers "did the command succeed?";
 * `probe` answers "what does the store say?", where a non-zero exit is the expected answer half the
 * time.
 */
export interface TrustContext {
    platform: NodeJS.Platform;
    home: string;
    /** `%APPDATA%` on Windows; only used to locate Firefox profiles. */
    appData?: string;
    /** Run a command; **throws** on a non-zero exit. */
    run(cmd: string, args: string[]): void;
    /** Run a command and report whether it exited zero. Never throws. */
    probe(cmd: string, args: string[]): boolean;
    /** True when `cmd` is resolvable on `PATH`. */
    has(cmd: string): boolean;
}

export interface TrustResult {
    ok: boolean;
    needsManualInstall?: boolean;
    instructions?: string;
    error?: string;
    /**
     * The CA was already in the trust store before this call, so nothing was installed. Additive;
     * the renderer ignores it. Without this, a second click on "Install" would report a failure on
     * a store that rejects duplicate adds.
     */
    alreadyTrusted?: boolean;
    /**
     * The store was **re-queried** afterwards and confirmed the intended end state — present after
     * an install, absent after an un-trust. `undefined` means this platform cannot answer
     * (`probeTrust()` returned `null`), which is not the same as `false`. Additive.
     */
    verified?: boolean;
}

// ─────────────────────────────────────────────────────────────────────────────
// The real context
// ─────────────────────────────────────────────────────────────────────────────

/**
 * Turn a failed `execFileSync` into a message worth showing.
 *
 * `stdio` captures **stderr and discards stdout**, because the interesting line is always on stderr
 * and certutil's last stderr line is the actual cause (`CertUtil: -addstore command FAILED:
 * 0x80070005 (E_ACCESSDENIED)`) — the thrown error's own message is only
 * `Command failed: certutil -addstore -user Root C:\…\ca.pem`, which names the command but not the
 * reason. Pre-P3 used `stdio: "ignore"`, so this is strictly more information than the user had.
 */
function describeExecFailure(cmd: string, err: unknown): string {
    const stderr = (err as { stderr?: Buffer | string } | null)?.stderr;
    const text = stderr ? String(stderr).trim() : "";
    if (text) {
        const lines = text.split("\n").map((l) => l.trim()).filter(Boolean);
        return `${cmd}: ${lines[lines.length - 1] ?? text}`;
    }
    return err instanceof Error ? err.message : String(err);
}

/**
 * `execFileSync`, not `execSync` with an interpolated string.
 *
 * The pre-P3 code built a shell string — `certutil -addstore -user Root "${certPath}"` — so a path
 * containing a quote or an `&` would have been misparsed. The path here is one we chose in the temp
 * directory so the risk was theoretical, but the argv form removes it rather than relying on that,
 * and it is the form the tests assert.
 */
function defaultTrustContext(): TrustContext {
    return {
        platform: process.platform,
        home: os.homedir(),
        appData: process.env.APPDATA,
        run(cmd, args) {
            try {
                cp.execFileSync(cmd, args, {
                    stdio: ["ignore", "ignore", "pipe"],
                    timeout: CMD_TIMEOUT_MS,
                    windowsHide: true,
                });
            } catch (err) {
                throw new Error(describeExecFailure(cmd, err));
            }
        },
        probe(cmd, args) {
            try {
                cp.execFileSync(cmd, args, {
                    stdio: "ignore",
                    timeout: CMD_TIMEOUT_MS,
                    windowsHide: true,
                });
                return true;
            } catch {
                return false;
            }
        },
        has(cmd) {
            const finder = process.platform === "win32" ? "where" : "which";
            try {
                cp.execFileSync(finder, [cmd], { stdio: "ignore", timeout: 5000, windowsHide: true });
                return true;
            } catch {
                return false;
            }
        },
    };
}

export function realTrustContext(): TrustContext {
    return defaultTrustContext();
}

/** The macOS login keychain, which is where a no-elevation `add-trusted-cert` writes. */
function loginKeychain(home: string): string {
    return path.join(home, "Library", "Keychains", "login.keychain-db");
}

// ─────────────────────────────────────────────────────────────────────────────
// The platform plans — pure, and the actual unit under test
// ─────────────────────────────────────────────────────────────────────────────

/**
 * The install commands to try, **in preference order**, for this platform.
 *
 * Windows and macOS return exactly one step, so a failure there is final and is reported as an
 * error — the pre-P3 behaviour. Linux returns a chain and `installCA()` falls through it, which is
 * the difference between "tell the user to open a terminal" and "just work".
 *
 * An empty array means "no command on this platform can do it" — only reachable on Linux with
 * neither `trust` nor `pkexec` installed — and `installCA()` turns it into written instructions.
 *
 * `thumbprintSha1` is unused on the install path but kept in the signature so install and uninstall
 * read identically at the call site; a platform that needs it later does not have to change every
 * caller.
 */
export function installPlan(ctx: TrustContext, certPath: string, thumbprintSha1: string): TrustStep[] {
    void thumbprintSha1;
    if (ctx.platform === "win32") {
        return [{
            cmd: "certutil",
            args: ["-addstore", "-user", "Root", certPath],
            label: "certutil -addstore -user Root (no elevation)",
        }];
    }
    if (ctx.platform === "darwin") {
        return [{
            cmd: "security",
            args: ["add-trusted-cert", "-d", "-r", "trustRoot", "-k", loginKeychain(ctx.home), certPath],
            label: "security add-trusted-cert into the login keychain (no elevation)",
        }];
    }

    const steps: TrustStep[] = [];
    // Both Linux steps need `trust`: `pkexec trust anchor` elevates the same binary, it does not
    // substitute for it. Guarding on `trust` alone for the second step would produce a `pkexec:
    // trust: command not found` that reads like a permissions problem.
    if (ctx.has("trust")) {
        steps.push({
            cmd: "trust",
            args: ["anchor", certPath],
            label: "trust anchor — p11-kit user trust store (no elevation)",
        });
        if (ctx.has("pkexec")) {
            steps.push({
                cmd: "pkexec",
                args: ["trust", "anchor", certPath],
                label: "pkexec trust anchor — polkit graphical auth prompt",
            });
        }
    }
    return steps;
}

/**
 * The un-trust commands, in preference order.
 *
 * Each platform identifies the certificate the way its own tool wants:
 *
 * - Windows: `-delstore -user Root <sha1>` — the "certificate hash" form, verified to be *accepted*
 *   (it exits 0 for a hash that is not present, which is why the result is verified separately).
 * - macOS: `-Z <sha1>` to match precisely rather than by common name, plus `-t` so the entry is
 *   removed from the trust settings and not only from the keychain.
 * - Linux: `--remove` on the same anchor path that added it.
 */
export function uninstallPlan(ctx: TrustContext, certPath: string, thumbprintSha1: string): TrustStep[] {
    if (ctx.platform === "win32") {
        return [{
            cmd: "certutil",
            args: ["-delstore", "-user", "Root", thumbprintSha1],
            label: "certutil -delstore -user Root (no elevation)",
        }];
    }
    if (ctx.platform === "darwin") {
        return [{
            cmd: "security",
            args: ["delete-certificate", "-Z", thumbprintSha1, "-t", loginKeychain(ctx.home)],
            label: "security delete-certificate -t (keychain and trust settings)",
        }];
    }

    const steps: TrustStep[] = [];
    if (ctx.has("trust")) {
        steps.push({
            cmd: "trust",
            args: ["anchor", "--remove", certPath],
            label: "trust anchor --remove — p11-kit user trust store",
        });
        if (ctx.has("pkexec")) {
            steps.push({
                cmd: "pkexec",
                args: ["trust", "anchor", "--remove", certPath],
                label: "pkexec trust anchor --remove — polkit graphical auth prompt",
            });
        }
    }
    return steps;
}

// ─────────────────────────────────────────────────────────────────────────────
// Verification — the part `certutil`'s exit code cannot give us
// ─────────────────────────────────────────────────────────────────────────────

/**
 * Ask the platform's trust store whether it holds this certificate.
 *
 * `true` / `false` are answers; **`null` means this platform cannot answer** and is deliberately
 * distinct from `false`. Linux has no probe here that this environment could verify, so it returns
 * `null` and callers leave `verified` undefined rather than claiming a check they did not perform.
 */
export function probeTrust(ctx: TrustContext, thumbprintSha1: string): boolean | null {
    if (ctx.platform === "win32") {
        return ctx.probe("certutil", ["-store", "-user", "Root", thumbprintSha1]);
    }
    if (ctx.platform === "darwin") {
        return ctx.probe("security", ["find-certificate", "-Z", thumbprintSha1, loginKeychain(ctx.home)]);
    }
    return null;
}

// ─────────────────────────────────────────────────────────────────────────────
// Install / uninstall
// ─────────────────────────────────────────────────────────────────────────────

/**
 * The last resort that cannot fail: tell the user the exact commands.
 *
 * The certificate path is included rather than a placeholder because the caller keeps the file in
 * place when it returns `needsManualInstall` — an instruction the user cannot copy is not an
 * instruction.
 */
export function linuxInstallInstructions(certPath: string): string {
    return [
        `# Install the Bifurc CA into the system trust store (needs root):`,
        `sudo cp "${certPath}" /usr/local/share/ca-certificates/bifurc-ca.crt`,
        `sudo update-ca-certificates`,
    ].join("\n");
}

export function linuxUninstallInstructions(certPath: string): string {
    return [
        `# Remove the Bifurc CA from the system trust store (needs root):`,
        `sudo rm -f /usr/local/share/ca-certificates/bifurc-ca.crt`,
        `sudo update-ca-certificates --fresh`,
        `# or, if it was installed with p11-kit:`,
        `trust anchor --remove "${certPath}"`,
    ].join("\n");
}

/**
 * Install the CA, verifying first that it is not already there.
 *
 * The pre-check is not an optimisation. On Windows, `certutil -addstore` on a certificate that is
 * already in the store fails rather than being idempotent, so a second click on "Install" would
 * show an error for a store that is already in exactly the desired state. Checking first turns that
 * into `{ok: true, alreadyTrusted: true}`.
 */
export function installCA(ctx: TrustContext, certPath: string, thumbprintSha1: string): TrustResult {
    if (probeTrust(ctx, thumbprintSha1) === true) {
        return { ok: true, alreadyTrusted: true, verified: true };
    }

    const steps = installPlan(ctx, certPath, thumbprintSha1);
    if (steps.length === 0) {
        return {
            ok: false,
            needsManualInstall: true,
            instructions: linuxInstallInstructions(certPath),
        };
    }

    let lastError: string | undefined;
    for (const step of steps) {
        try {
            ctx.run(step.cmd, step.args);
        } catch (err) {
            // A failed step falls through to the next preference, not to failure. On Windows and
            // macOS the list has one entry, so this loop still reports the error as before.
            lastError = err instanceof Error ? err.message : String(err);
            continue;
        }
        const verdict = probeTrust(ctx, thumbprintSha1);
        return { ok: true, verified: verdict === null ? undefined : verdict };
    }

    // Every command failed. On Linux the chain exists to degrade rather than to fail, so the last
    // resort is the written instruction; elsewhere a failure is a failure, as it was pre-P3.
    if (ctx.platform === "linux") {
        return {
            ok: false,
            needsManualInstall: true,
            instructions: linuxInstallInstructions(certPath),
            error: lastError,
        };
    }
    return { ok: false, error: lastError ?? "The install command failed." };
}

/**
 * Remove the CA from the trust store.
 *
 * `ok: true` with no commands run is the ordinary case for a client that never installed anything,
 * and it is a **success**, not an error: the requested end state ("this machine does not trust the
 * engine's CA") already holds.
 */
export function uninstallCA(ctx: TrustContext, certPath: string, thumbprintSha1: string): TrustResult {
    const before = probeTrust(ctx, thumbprintSha1);
    if (before === false) {
        return { ok: true, verified: true };
    }

    const steps = uninstallPlan(ctx, certPath, thumbprintSha1);
    if (steps.length === 0) {
        return {
            ok: false,
            needsManualInstall: true,
            instructions: linuxUninstallInstructions(certPath),
        };
    }

    let lastError: string | undefined;
    for (const step of steps) {
        try {
            ctx.run(step.cmd, step.args);
        } catch (err) {
            lastError = err instanceof Error ? err.message : String(err);
            continue;
        }
        // Re-query rather than trusting the exit status — see the header note. `certutil
        // -delstore` exits 0 even when it removed nothing, so `ok` alone would be a claim we
        // cannot support.
        const after = probeTrust(ctx, thumbprintSha1);
        return { ok: true, verified: after === null ? undefined : after === false };
    }

    if (ctx.platform === "linux") {
        return {
            ok: false,
            needsManualInstall: true,
            instructions: linuxUninstallInstructions(certPath),
            error: lastError,
        };
    }
    return { ok: false, error: lastError ?? "The un-trust command failed." };
}

// ─────────────────────────────────────────────────────────────────────────────
// Firefox — it does not use the OS trust store
// ─────────────────────────────────────────────────────────────────────────────

export interface FirefoxRoot {
    /** Where profiles live. A subdirectory count is the cheapest "has it been used?" signal. */
    profilesDir: string;
    /** Candidate executables, for the case where Firefox is installed but never launched. */
    binaries: string[];
}

export interface FirefoxPresence {
    present: boolean;
    /** How many profile directories exist. 0 with `present: true` means installed, never launched. */
    profiles: number;
}

/**
 * Where Firefox keeps its profiles and its binary, per platform. Pure — the unit under test.
 *
 * Separated from `detectFirefox()` so the paths can be asserted exactly per platform without
 * touching a filesystem, which is the same reason `installPlan()` is separate from `installCA()`.
 */
export function firefoxRoots(
    platform: NodeJS.Platform,
    home: string,
    appData?: string,
): FirefoxRoot[] {
    if (platform === "win32") {
        return [
            {
                profilesDir: path.join(appData ?? path.join(home, "AppData", "Roaming"), "Mozilla", "Firefox", "Profiles"),
                binaries: [
                    path.join(process.env["PROGRAMFILES"] ?? "C:\\Program Files", "Mozilla Firefox", "firefox.exe"),
                    path.join(process.env["PROGRAMFILES(X86)"] ?? "C:\\Program Files (x86)", "Mozilla Firefox", "firefox.exe"),
                ],
            },
        ];
    }
    if (platform === "darwin") {
        return [
            {
                profilesDir: path.join(home, "Library", "Application Support", "Firefox", "Profiles"),
                binaries: ["/Applications/Firefox.app"],
            },
        ];
    }
    return [
        {
            profilesDir: path.join(home, ".mozilla", "firefox"),
            binaries: ["/usr/bin/firefox", "/usr/local/bin/firefox", "/snap/bin/firefox"],
        },
    ];
}

/**
 * Detect Firefox. Detection only — the UI note is the mitigation, not an NSS install
 * (`File_Ops_Protocol.md` §6.3 keeps that as an option, not a requirement).
 *
 * A missing profiles directory is not evidence of absence on its own: a freshly installed Firefox
 * that has never been launched has no profiles, and that is precisely the user who is about to hit
 * the problem.
 */
export function detectFirefox(roots: FirefoxRoot[] = firefoxRoots(process.platform, os.homedir(), process.env.APPDATA)): FirefoxPresence {
    let profiles = 0;
    let present = false;

    for (const root of roots) {
        if (fs.existsSync(root.profilesDir)) {
            present = true;
            try {
                profiles += fs.readdirSync(root.profilesDir, { withFileTypes: true })
                    .filter((e) => e.isDirectory()).length;
            } catch {
                /* unreadable directory — still counts as present */
            }
        }
        if (root.binaries.some((b) => fs.existsSync(b))) present = true;
    }

    return { present, profiles };
}
