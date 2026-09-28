/**
 * `src/ipc/certTrust.ts` — the **client half** of the certificate lifecycle: the host's OS trust
 * store.
 *
 * ## Why the assertions here are argv arrays
 *
 * This module's whole job is to run the right command with the right arguments on three platforms,
 * and **two of those platforms cannot be reached from this development environment**. Mocking
 * `child_process` (as the pre-P3 `certManager.test.ts` did) would assert that *a* command ran; it
 * would not catch `-delstore` where `-addstore` belongs, a missing `-user`, or a keychain path
 * built from the wrong platform's separator. So the platform logic is a **pure function of an
 * injected context** — `installPlan()` / `uninstallPlan()` — and the tests assert the exact argv.
 *
 * The two facts that shape the un-trust path were **measured against a real `certutil`** rather
 * than assumed, and both are pinned below:
 *
 * 1. `certutil -delstore` exits **0** whether or not it deleted anything, so the delete command's
 *    exit status cannot be the basis of a "removed" claim.
 * 2. `certutil -store -user Root <thumbprint>` exits **0** when present and **17**
 *    (`NTE_NOT_FOUND`) when absent — that is the verification primitive.
 *
 * `probeTrust()` is therefore a separate concept from `run()`, and `TrustResult.verified` is
 * separate from `TrustResult.ok`. The test that proves it matters is
 * *"reports ok but not verified when the delete command exits 0 and the store still has it"*.
 *
 * ## What is deliberately not asserted
 *
 * Whether Linux's `trust anchor` really works without elevation, and whether macOS'
 * `delete-certificate -t` really clears the trust settings. Neither can be verified on Windows;
 * both are recorded as P9/P12 verification tasks in `plan/13-checklist.md`. What *is* asserted is
 * that the fallback chain tries them in the documented order and degrades to instructions rather
 * than failing — which is the part that can be reasoned about from here.
 */
import { describe, it, expect } from "vitest";
import * as fs from "fs";
import * as os from "os";
import * as path from "path";
import {
    detectFirefox,
    firefoxRoots,
    installCA,
    installPlan,
    linuxInstallInstructions,
    linuxUninstallInstructions,
    probeTrust,
    realTrustContext,
    uninstallCA,
    uninstallPlan,
    type TrustContext,
} from "@/ipc/certTrust";

const THUMB = "CD1FDAFE36008D23A3484F29AE84A6B3EABD9F63";
const HOME = path.join(path.sep === "\\" ? "C:\\Users" : "/home", "tester");
const CERT = path.join(os.tmpdir(), "bifurc-ca-abc123.pem");

interface FakeCtx extends TrustContext {
    /** Every command that was executed, as `[cmd, ...args]`. */
    ran: string[][];
    /** Every command that was queried, as `[cmd, ...args]`. */
    probes: string[][];
}

/**
 * A `TrustContext` that records instead of executing.
 *
 * `run` and `probe` are wrapped rather than replaced so that a test can supply behaviour *and*
 * still assert on the calls — the argv assertions and the behavioural assertions then read the same
 * recording.
 */
function makeCtx(opts: {
    platform?: NodeJS.Platform;
    run?: (cmd: string, args: string[]) => void;
    probe?: (cmd: string, args: string[]) => boolean;
    has?: (cmd: string) => boolean;
} = {}): FakeCtx {
    const ran: string[][] = [];
    const probes: string[][] = [];
    return {
        platform: opts.platform ?? "win32",
        home: HOME,
        appData: path.join(HOME, "AppData", "Roaming"),
        ran,
        probes,
        run(cmd, args) {
            ran.push([cmd, ...args]);
            opts.run?.(cmd, args);
        },
        probe(cmd, args) {
            probes.push([cmd, ...args]);
            return opts.probe?.(cmd, args) ?? false;
        },
        has(cmd) {
            return opts.has?.(cmd) ?? false;
        },
    };
}

// ─────────────────────────────────────────────────────────────────────────────
// installPlan — the platform command shapes
// ─────────────────────────────────────────────────────────────────────────────

describe("installPlan()", () => {
    it("Windows: certutil into the user's Root store, with -user so no elevation is needed", () => {
        // `-user` is the load-bearing argument. Without it certutil targets the machine store and
        // needs an administrator — which is the difference between a one-click install and a UAC
        // prompt (`File_Ops_Protocol.md` §6.2).
        const plan = installPlan(makeCtx({ platform: "win32" }), CERT, THUMB);
        expect(plan.map((s) => [s.cmd, ...s.args])).toEqual([
            ["certutil", "-addstore", "-user", "Root", CERT],
        ]);
    });

    it("macOS: security add-trusted-cert into the login keychain, as a trust root", () => {
        const plan = installPlan(makeCtx({ platform: "darwin" }), CERT, THUMB);
        expect(plan.map((s) => [s.cmd, ...s.args])).toEqual([[
            "security", "add-trusted-cert", "-d", "-r", "trustRoot",
            "-k", path.join(HOME, "Library", "Keychains", "login.keychain-db"),
            CERT,
        ]]);
    });

    it("Linux: trust anchor first, because p11-kit's user store needs no elevation", () => {
        const ctx = makeCtx({ platform: "linux", has: (c) => c === "trust" });
        const plan = installPlan(ctx, CERT, THUMB);
        expect(plan.map((s) => [s.cmd, ...s.args])).toEqual([["trust", "anchor", CERT]]);
    });

    it("Linux: adds the pkexec step as a fallback when polkit is available", () => {
        const ctx = makeCtx({ platform: "linux", has: () => true });
        const plan = installPlan(ctx, CERT, THUMB);
        expect(plan.map((s) => [s.cmd, ...s.args])).toEqual([
            ["trust", "anchor", CERT],
            ["pkexec", "trust", "anchor", CERT],
        ]);
    });

    it("Linux: does not offer pkexec without trust, which would fail as 'command not found'", () => {
        // `pkexec trust anchor` elevates the same binary; it does not substitute for it. Emitting it
        // when `trust` is missing produces a message that reads like a permissions problem.
        const ctx = makeCtx({ platform: "linux", has: (c) => c === "pkexec" });
        expect(installPlan(ctx, CERT, THUMB)).toEqual([]);
    });
});

// ─────────────────────────────────────────────────────────────────────────────
// uninstallPlan
// ─────────────────────────────────────────────────────────────────────────────

describe("uninstallPlan()", () => {
    it("Windows: certutil -delstore -user Root by certificate hash", () => {
        const plan = uninstallPlan(makeCtx({ platform: "win32" }), CERT, THUMB);
        expect(plan.map((s) => [s.cmd, ...s.args])).toEqual([
            ["certutil", "-delstore", "-user", "Root", THUMB],
        ]);
    });

    it("macOS: deletes by SHA-1 hash with -t, so the trust setting goes too", () => {
        // `-Z` rather than `-c <common name>`: matching by name would remove any certificate with
        // the same subject. `-t` is what removes it from the *trust settings* rather than only from
        // the keychain, which is the difference between "deleted" and "still trusted".
        const plan = uninstallPlan(makeCtx({ platform: "darwin" }), CERT, THUMB);
        expect(plan.map((s) => [s.cmd, ...s.args])).toEqual([[
            "security", "delete-certificate", "-Z", THUMB, "-t",
            path.join(HOME, "Library", "Keychains", "login.keychain-db"),
        ]]);
    });

    it("Linux: trust anchor --remove, with the pkexec fallback", () => {
        const ctx = makeCtx({ platform: "linux", has: () => true });
        const plan = uninstallPlan(ctx, CERT, THUMB);
        expect(plan.map((s) => [s.cmd, ...s.args])).toEqual([
            ["trust", "anchor", "--remove", CERT],
            ["pkexec", "trust", "anchor", "--remove", CERT],
        ]);
    });
});

// ─────────────────────────────────────────────────────────────────────────────
// probeTrust — the primitive certutil's exit code cannot provide
// ─────────────────────────────────────────────────────────────────────────────

describe("probeTrust()", () => {
    it("Windows: queries the user Root store by thumbprint", () => {
        const ctx = makeCtx({ platform: "win32" });
        probeTrust(ctx, THUMB);
        expect(ctx.probes).toEqual([["certutil", "-store", "-user", "Root", THUMB]]);
    });

    it("macOS: queries the keychain by SHA-1 hash", () => {
        const ctx = makeCtx({ platform: "darwin" });
        probeTrust(ctx, THUMB);
        expect(ctx.probes).toEqual([[
            "security", "find-certificate", "-Z", THUMB,
            path.join(HOME, "Library", "Keychains", "login.keychain-db"),
        ]]);
    });

    it("Linux: answers null — 'cannot say' — rather than false", () => {
        // `false` would mean "verified absent" and would let callers claim a check they did not
        // perform. `null` is the honest answer and is why `verified` is `undefined` on Linux.
        const ctx = makeCtx({ platform: "linux" });
        expect(probeTrust(ctx, THUMB)).toBeNull();
        expect(ctx.probes).toEqual([]);
    });
});

// ─────────────────────────────────────────────────────────────────────────────
// installCA
// ─────────────────────────────────────────────────────────────────────────────

describe("installCA()", () => {
    it("does nothing when the store already has this certificate", () => {
        // Not an optimisation: `certutil -addstore` on a certificate that is already present fails
        // rather than being idempotent, so without this a second click reports an error for a store
        // that is already in the desired state.
        const ctx = makeCtx({ platform: "win32", probe: () => true });
        const result = installCA(ctx, CERT, THUMB);

        expect(result).toEqual({ ok: true, alreadyTrusted: true, verified: true });
        expect(ctx.ran).toEqual([]);
        expect(ctx.probes).toHaveLength(1);
    });

    it("Windows: runs the install and verifies it landed", () => {
        let present = false;
        const ctx = makeCtx({ platform: "win32", probe: () => present, run: () => { present = true; } });
        const result = installCA(ctx, CERT, THUMB);

        expect(result).toEqual({ ok: true, verified: true });
        expect(ctx.ran).toEqual([["certutil", "-addstore", "-user", "Root", CERT]]);
    });

    it("Windows: reports ok but not verified when the store does not show it afterwards", () => {
        // The command exited 0 and nothing threw, so the install *happened*; the re-query could not
        // confirm it. Reporting `verified: false` keeps the two claims separate instead of fusing
        // them into a single optimistic `ok`.
        const ctx = makeCtx({ platform: "win32", probe: () => false });
        const result = installCA(ctx, CERT, THUMB);

        expect(result).toEqual({ ok: true, verified: false });
    });

    it("Windows: surfaces the command's own stderr line as the error", () => {
        // Pre-P3 used `stdio: "ignore"`, so the user saw only "Command failed: certutil …". The
        // reason lives on stderr and the injected `run` is expected to have carried it into the
        // message — asserted in `defaultTrustContext()`'s test below.
        const ctx = makeCtx({
            platform: "win32",
            run: () => { throw new Error("certutil: CertUtil: -addstore command FAILED: 0x80070005 (E_ACCESSDENIED)"); },
        });
        const result = installCA(ctx, CERT, THUMB);

        expect(result.ok).toBe(false);
        expect(result.needsManualInstall).toBeUndefined();
        expect(result.error).toContain("E_ACCESSDENIED");
    });

    it("Linux: falls through from trust to pkexec, which is the point of the chain", () => {
        const ctx = makeCtx({
            platform: "linux",
            has: () => true,
            run: (cmd) => { if (cmd === "trust") throw new Error("trust: no user anchor store"); },
        });
        const result = installCA(ctx, CERT, THUMB);

        expect(result.ok).toBe(true);
        // `verified` is undefined on Linux — there is no probe this environment could verify.
        expect(result.verified).toBeUndefined();
        expect(ctx.ran).toEqual([
            ["trust", "anchor", CERT],
            ["pkexec", "trust", "anchor", CERT],
        ]);
    });

    it("Linux: degrades to instructions when no command can do it, and never fails outright", () => {
        const ctx = makeCtx({ platform: "linux", has: () => false });
        const result = installCA(ctx, CERT, THUMB);

        expect(result.ok).toBe(false);
        expect(result.needsManualInstall).toBe(true);
        expect(result.instructions).toContain("update-ca-certificates");
        expect(result.instructions).toContain(CERT);
        expect(ctx.ran).toEqual([]);
    });

    it("Linux: degrades to instructions when every command in the chain fails", () => {
        const ctx = makeCtx({
            platform: "linux",
            has: () => true,
            run: () => { throw new Error("polkit: authentication cancelled"); },
        });
        const result = installCA(ctx, CERT, THUMB);

        expect(result.ok).toBe(false);
        expect(result.needsManualInstall).toBe(true);
        expect(result.error).toContain("authentication cancelled");
        expect(ctx.ran).toHaveLength(2);
    });

    it("macOS: a failed keychain install is an error, not a manual-install fallback", () => {
        // The chain exists on Linux because Linux is the platform where the unattended path may not
        // exist at all. On macOS and Windows the single command either works or the user has a real
        // problem, and the pre-P3 behaviour was to say so.
        const ctx = makeCtx({
            platform: "darwin",
            run: () => { throw new Error("security: User interaction is not allowed."); },
        });
        const result = installCA(ctx, CERT, THUMB);

        expect(result.ok).toBe(false);
        expect(result.needsManualInstall).toBeUndefined();
        expect(result.error).toContain("User interaction is not allowed");
    });
});

// ─────────────────────────────────────────────────────────────────────────────
// uninstallCA
// ─────────────────────────────────────────────────────────────────────────────

describe("uninstallCA()", () => {
    it("is a success with no commands when the store does not have it", () => {
        // The requested end state already holds. Treating this as a failure would make "remove the
        // certificate" report an error on a machine that never installed it.
        const ctx = makeCtx({ platform: "win32", probe: () => false });
        expect(uninstallCA(ctx, CERT, THUMB)).toEqual({ ok: true, verified: true });
        expect(ctx.ran).toEqual([]);
    });

    it("Windows: deletes by thumbprint and verifies it is gone", () => {
        let present = true;
        const ctx = makeCtx({ platform: "win32", probe: () => present, run: () => { present = false; } });
        const result = uninstallCA(ctx, CERT, THUMB);

        expect(result).toEqual({ ok: true, verified: true });
        expect(ctx.ran).toEqual([["certutil", "-delstore", "-user", "Root", THUMB]]);
    });

    it("reports ok but not verified when the delete command exits 0 and the store still has it", () => {
        // **This is the test the measured `certutil` behaviour exists for.** `certutil -delstore`
        // prints "command completed successfully" and exits 0 for a certificate it did not find —
        // and, as here, the store can still report the certificate afterwards. A result built on
        // the exit code alone would tell the user their CA had been un-trusted when it had not.
        const ctx = makeCtx({ platform: "win32", probe: () => true });
        const result = uninstallCA(ctx, CERT, THUMB);

        expect(result.ok).toBe(true);
        expect(result.verified).toBe(false);
    });

    it("Linux: degrades to instructions when there is no trust binary", () => {
        const ctx = makeCtx({ platform: "linux", has: () => false });
        const result = uninstallCA(ctx, CERT, THUMB);

        expect(result.ok).toBe(false);
        expect(result.needsManualInstall).toBe(true);
        expect(result.instructions).toContain("update-ca-certificates");
        expect(ctx.ran).toEqual([]);
    });

    it("Linux: runs trust anchor --remove and reports no verification either way", () => {
        const ctx = makeCtx({ platform: "linux", has: (c) => c === "trust" });
        const result = uninstallCA(ctx, CERT, THUMB);

        expect(result.ok).toBe(true);
        expect(result.verified).toBeUndefined();
        expect(ctx.ran).toEqual([["trust", "anchor", "--remove", CERT]]);
    });
});

// ─────────────────────────────────────────────────────────────────────────────
// Instructions
// ─────────────────────────────────────────────────────────────────────────────

describe("the Linux instruction fallbacks", () => {
    it("names the certificate path, because the caller keeps the file for exactly this case", () => {
        // `certLifecycle.installEngineCa()` deletes its temp copy unless the result says
        // `needsManualInstall`. An instruction naming a deleted path is not an instruction.
        expect(linuxInstallInstructions(CERT)).toContain(CERT);
        expect(linuxInstallInstructions(CERT)).toContain("update-ca-certificates");
    });

    it("gives both removal routes, since it cannot know which one installed it", () => {
        const text = linuxUninstallInstructions(CERT);
        expect(text).toContain("/usr/local/share/ca-certificates/bifurc-ca.crt");
        expect(text).toContain("trust anchor --remove");
    });
});

// ─────────────────────────────────────────────────────────────────────────────
// Firefox — it does not use the OS trust store
// ─────────────────────────────────────────────────────────────────────────────

describe("firefoxRoots()", () => {
    it("Windows: the roaming profile directory and both Program Files locations", () => {
        const roots = firefoxRoots("win32", HOME, "C:\\Users\\tester\\AppData\\Roaming");
        expect(roots[0].profilesDir).toBe(
            path.join("C:\\Users\\tester\\AppData\\Roaming", "Mozilla", "Firefox", "Profiles"),
        );
        expect(roots[0].binaries.some((b) => b.includes("Mozilla Firefox"))).toBe(true);
    });

    it("macOS: ~/Library/Application Support and the app bundle", () => {
        const roots = firefoxRoots("darwin", HOME);
        expect(roots[0].profilesDir).toBe(
            path.join(HOME, "Library", "Application Support", "Firefox", "Profiles"),
        );
        expect(roots[0].binaries).toContain("/Applications/Firefox.app");
    });

    it("Linux: ~/.mozilla/firefox, including the snap path", () => {
        const roots = firefoxRoots("linux", HOME);
        expect(roots[0].profilesDir).toBe(path.join(HOME, ".mozilla", "firefox"));
        expect(roots[0].binaries).toContain("/snap/bin/firefox");
    });
});

describe("detectFirefox()", () => {
    it("counts profiles and treats an unlaunched install as present", () => {
        // The user who has installed Firefox but never launched it is exactly the one about to hit
        // the NSS problem, so a missing profiles directory must not read as "no Firefox".
        const dir = fs.mkdtempSync(path.join(os.tmpdir(), "ff-profiles-"));
        try {
            fs.mkdirSync(path.join(dir, "abc123.default-release"));
            fs.mkdirSync(path.join(dir, "xyz789.dev-edition"));
            fs.writeFileSync(path.join(dir, "profiles.ini"), "[General]\n");

            const found = detectFirefox([{ profilesDir: dir, binaries: [] }]);
            expect(found).toEqual({ present: true, profiles: 2 });
        } finally {
            fs.rmSync(dir, { recursive: true, force: true });
        }
    });

    it("reports absence when neither profiles nor a binary exist", () => {
        const missing = path.join(os.tmpdir(), "definitely-not-here-" + Date.now());
        expect(detectFirefox([{ profilesDir: missing, binaries: [missing] }]))
            .toEqual({ present: false, profiles: 0 });
    });

    it("reports presence from a binary alone", () => {
        const bin = path.join(os.tmpdir(), "fake-firefox-" + Date.now());
        fs.writeFileSync(bin, "");
        try {
            expect(detectFirefox([{ profilesDir: bin + "-profiles", binaries: [bin] }]).present).toBe(true);
        } finally {
            fs.rmSync(bin, { force: true });
        }
    });
});

// ─────────────────────────────────────────────────────────────────────────────
// The real context — the only part that touches child_process
// ─────────────────────────────────────────────────────────────────────────────

describe("realTrustContext()", () => {
    /**
     * The exec plumbing is the one thing here that cannot be faked into confidence, and it is
     * exercised against the real `certutil` where the platform allows it. Both directions of the
     * probe are safe and read-only: querying a certificate that is not in the store changes
     * nothing, and it is exactly the query `uninstallCA()` relies on to report `verified`.
     */
    it.runIf(process.platform === "win32")("reports false when certutil cannot find the thumbprint", () => {
        const ctx = realTrustContext();
        expect(ctx.probe("certutil", ["-store", "-user", "Root", "00112233445566778899AABBCCDDEEFF00112233"]))
            .toBe(false);
    });

    it.runIf(process.platform === "win32")("resolves certutil, and does not resolve a command that does not exist", () => {
        const ctx = realTrustContext();
        expect(ctx.has("certutil")).toBe(true);
        expect(ctx.has("definitely-not-a-real-binary-9f3a")).toBe(false);
    });
});
