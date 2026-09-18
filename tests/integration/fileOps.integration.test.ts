/**
 * P3 work items 3–4 — the six file-operation channels that are **not** import/export, driven
 * through a real `CommandRegistry`.
 *
 * WHY THIS SUITE EXISTS
 * ---------------------
 * `File_Ops_Protocol.md` §4 (egress) and §5 (ingress) enumerate every channel that crosses the
 * engine/client file boundary. `importExportBlob.integration.test.ts` covers `export.create` /
 * `import.preflight` / `import.commit`; this file covers the rest:
 *
 *   egress    tls.exportCert   runner.exportReport   capture.shareJson   audit.export
 *   ingress   tls.importCert   tls.importKey
 *
 * Before P3 all six did their own dialogs and their own `fs.writeFileSync` inside the shell. The
 * conversion changed *who owns the filesystem*, and that is the property nothing else tests: an
 * engine handler that reached for the user's disk would still pass every format-level suite, and a
 * client that never released its blob would still pass every happy path. So the assertions here are
 * about the boundary, not about the content:
 *
 *   - every egress result is an `ArtifactResult` and carries **no path** — exactly the six keys of
 *     `{ok, inline|blobId, suggestedName, size, mimeType, sha256}`, nothing more;
 *   - the inline/blob decision is made by the **engine** at `BLOB_INLINE_THRESHOLD_BYTES`, and both
 *     branches report the same `sha256` of the decoded bytes;
 *   - ingress takes a `blobId`, never content and never a path, and the engine does **not** release
 *     the blob — cleanup is the client's call (`File_Ops_Protocol.md` §5);
 *   - a failure is an `ok: false` result with a typed, path-free message, never a thrown exception
 *     and never an `ENOENT` from the engine's own disk.
 *
 * ## Fixture notes, both of which are traps
 *
 * **1. The CA lives under `appDataDir()`, which is not `dataDir()`.** `tls.exportCert` and the two
 * imports resolve the certificate through `appDataDir()` because that is where `generateCA()` and
 * `tls.removeCert` put it — on Windows `appDataDir()` is `%LOCALAPPDATA%/Bifurc` and does *not*
 * follow `setDataRoot()`. `proxyHarness.createWorkspace()` already calls
 * `setSettingsPathOverride(<root>/app.json)`, so `appDataDir()` is the fixture root and no test can
 * touch the developer's real certificate. `setDataRoot(<root>)` is then supplied separately so
 * `blobRoot()` resolves to `<root>/blobs` — the same split `src/main.ts` produces from
 * `app.getPath("userData")`.
 *
 * **2. `gitStore` needs the workspace root, `dataDir()` needs the fixture root.** They are two
 * different overrides: `setDataDirOverride(<root>/data)` is what `queryLog` resolves through, and
 * `setDataRoot(<root>)` is what `blobRoot()` resolves through. Passing the same string to both
 * would put the blobs inside the workspace tree.
 *
 * Runs in the integration project (shared process-global overrides ⇒ `fileParallelism: false`).
 */

import { describe, it, expect, afterAll } from "vitest";
import * as crypto from "crypto";
import * as fs from "fs";
import * as path from "path";
import {
    BLOB_INLINE_THRESHOLD_BYTES,
    type ArtifactResult,
    type BlobPutResult,
    type BlobReadResult,
    type AuditExportResult,
    type CaptureShareJsonResult,
    type RunnerExportReportResult,
    type TlsCertStatusResult,
    type TlsExportCertResult,
    type TlsGenerateResult,
    type TlsImportResult,
    type TlsRemoveCertResult,
} from "@bifurc/protocol";
import { CommandRegistry, type CommandContext } from "@bifurc/engine/commands/registry";
import { registerBlobCommands } from "@bifurc/engine/blob/commands";
import { registerFileOpsCommands } from "@bifurc/engine/fileOps/commands";
import { registerCertCommands } from "@bifurc/engine/proxy/certCommands";
import { identifyCert } from "@bifurc/engine/proxy/certManager";
import { blobRoot } from "@bifurc/engine/blob/store";
import { appDataDir } from "@bifurc/engine/store/appSettings";
import { resetDataRootForTests, setDataRoot } from "@bifurc/engine/store/paths";
import { initWorkspaceDir } from "@bifurc/engine/store/workspaceFs";
import {
    commitMutation,
    initWorkspaceRepo,
    setDataDirOverride,
} from "@bifurc/engine/store/gitStore";
import { createWorkspace, TEST_WS, type WorkspaceFixture } from "./proxyHarness";

const ctx: CommandContext = { bus: {} as CommandContext["bus"] };

const fixtures: WorkspaceFixture[] = [];

function newWorkspace(seed?: (ws: WorkspaceFixture) => void): WorkspaceFixture {
    const ws = createWorkspace();
    fixtures.push(ws);
    setDataRoot(ws.root);            // → blobRoot() = <root>/blobs        (see header, point 1)
    setDataDirOverride(ws.dataRoot); // → queryLog() resolves <root>/data   (see header, point 2)
    seed?.(ws);
    return ws;
}

afterAll(() => {
    for (const f of fixtures) f.cleanup();
    resetDataRootForTests();
});

/**
 * A fresh registry per test, never the process-wide `commandRegistry` singleton: `register()`
 * throws on a double registration and the singleton is shared across the whole run. All three
 * registration functions are the real ones `src/ipc/handlers.ts` calls.
 */
function newRegistry(): CommandRegistry {
    const registry = new CommandRegistry();
    registerBlobCommands(registry);
    registerFileOpsCommands(registry);
    registerCertCommands(registry);
    return registry;
}

// ── Client-side primitives, written the way `src/ipc/fileOpsClient.ts` writes them ──

/** `blob.put` — the client uploading a file it read from its own disk. */
function pushBlob(
    registry: CommandRegistry,
    filename: string,
    bytes: Buffer,
    mimeType = "application/x-pem-file",
): string {
    const res = registry.invoke(
        "blob.put",
        { filename, mimeType, size: bytes.length, data: bytes.toString("base64") },
        ctx,
    ) as BlobPutResult;
    return res.blobId;
}

/**
 * Pull a whole blob with the **transport** primitive: repeated `blob.read` slices until `eof`.
 *
 * Deliberately not `fs.readFileSync(blobContentPath(...))`. That is the engine's in-process
 * shortcut; this is the loop `src/ipc/fileOpsClient.ts#pullBlob` actually runs, so a broken `eof`
 * or a mis-computed offset fails here rather than only in the app.
 */
function pullBlob(registry: CommandRegistry, blobId: string): Buffer {
    const chunks: Buffer[] = [];
    let offset = 0;
    for (let guard = 0; guard < 10_000; guard++) {
        const res = registry.invoke("blob.read", { blobId, offset }, ctx) as BlobReadResult;
        const slice = Buffer.from(res.data, "base64");
        if (slice.length) chunks.push(slice);
        offset += slice.length;
        if (res.eof) return Buffer.concat(chunks);
    }
    throw new Error(`blob.read never reported eof for ${blobId}`);
}

/** The bytes an artifact refers to, whichever shape it came back in. */
function bytesOf(registry: CommandRegistry, res: Extract<ArtifactResult, { ok: true }>): Buffer {
    return res.inline !== undefined ? Buffer.from(res.inline, "base64") : pullBlob(registry, res.blobId);
}

const sha256 = (bytes: Buffer): string => crypto.createHash("sha256").update(bytes).digest("hex");

/** `tls.exportCert`, typed. */
function exportCert(registry: CommandRegistry): TlsExportCertResult {
    return registry.invoke("tls.exportCert", {}, ctx) as TlsExportCertResult;
}

/**
 * Assert that a successful artifact has **exactly** the six `ArtifactResult` keys.
 *
 * This is the item-6 acceptance criterion ("no `filePath` in any protocol command or result type")
 * expressed as a runtime assertion rather than a grep. A `path` field added back for a
 * renderer convenience — the tempting regression, since `tls:importCert` still has to return one
 * (see `src/ipc/handlers/tlsHandlers.ts`) — fails here.
 */
function expectArtifactKeys(artifact: object, branch: "inline" | "blobId"): void {
    const shared = ["ok", "mimeType", "sha256", "size", "suggestedName"];
    expect(Object.keys(artifact).sort()).toEqual([...shared, branch].sort());
    expect(JSON.stringify(artifact)).not.toContain("filePath");
}

// ── Registration ──────────────────────────────────────────────────────────────

describe("registration", () => {
    it("registers the six file-operation channels plus the three cert-lifecycle ones", () => {
        const registry = newRegistry();
        const owned = registry
            .list()
            .filter((c) => /^(tls|runner|audit|capture)\./.test(c))
            .sort();

        // `tls.generate` / `tls.certStatus` / `tls.removeCert` used to be registered in the shell
        // (`src/ipc/handlers/tlsHandlers.ts`). P3 work item 5 moved them here, because they touch
        // nothing but the engine's own data dir and a remote engine (P4) cannot serve them from the
        // client half.
        expect(owned).toEqual([
            "audit.export",
            "capture.shareJson",
            "runner.exportReport",
            "tls.certStatus",
            "tls.exportCert",
            "tls.generate",
            "tls.importCert",
            "tls.importKey",
            "tls.removeCert",
        ]);
    });

    it("keeps the split: fileOps owns the file-crossing three, certCommands owns the rest", () => {
        // The boundary item 5 drew. `tls:installCA` is in neither — it is CLIENT-classified
        // (`src/ipc/handlers/clientHandlers.ts`) and is not a registry command at all.
        const fileOps = new CommandRegistry();
        registerFileOpsCommands(fileOps);
        expect(fileOps.list().sort()).toEqual([
            "audit.export",
            "capture.shareJson",
            "runner.exportReport",
            "tls.exportCert",
            "tls.importCert",
            "tls.importKey",
        ]);

        const cert = new CommandRegistry();
        registerCertCommands(cert);
        expect(cert.list().sort()).toEqual(["tls.certStatus", "tls.generate", "tls.removeCert"]);
    });

    it("refuses a double registration, so two implementations cannot compete for one wire name", () => {
        const registry = newRegistry();
        expect(() => registerFileOpsCommands(registry)).toThrow(/already registered/);
    });
});

// ── tls.exportCert — egress ───────────────────────────────────────────────────

describe("tls.exportCert", () => {
    const PEM = "-----BEGIN CERTIFICATE-----\nMIIBkTCB+wIJAK\n-----END CERTIFICATE-----\n";

    it("returns the certificate inline, as an artifact with no path", () => {
        const registry = newRegistry();
        newWorkspace();
        fs.writeFileSync(path.join(appDataDir(), "ca-cert.pem"), PEM, "utf-8");

        const res = exportCert(registry);
        expect(res.ok, res.ok ? "" : res.error).toBe(true);
        if (!res.ok) return;

        // A 2 KB PEM is nowhere near the 1 MB threshold, so the common case is one round trip and
        // the blob store is never even created.
        expectArtifactKeys(res, "inline");
        expect(res.mimeType).toBe("application/x-pem-file");
        expect(res.suggestedName).toBe("bifurc-ca.pem");
        expect(res.size).toBe(Buffer.byteLength(PEM));
        expect(res.sha256).toBe(sha256(Buffer.from(PEM, "utf-8")));
        expect(Buffer.from(res.inline, "base64").toString("utf-8")).toBe(PEM);
        expect(fs.existsSync(blobRoot())).toBe(false);
    });

    it("reads the CA from appDataDir(), which is where generateCA() and removeCert() put it", () => {
        // The trap this pins: on Windows `dataDir()` and `appDataDir()` are different directories,
        // and a handler that resolved through `dataDir()` would find nothing while `tls.certStatus`
        // reported a certificate. `proxyHarness` points `appDataDir()` at the fixture root and
        // `setDataRoot()` at that same root, so this asserts the file is found — the same file the
        // shell writes to.
        const registry = newRegistry();
        newWorkspace();
        expect(fs.existsSync(path.join(appDataDir(), "ca-cert.pem"))).toBe(false);

        const before = exportCert(registry);
        expect(before.ok).toBe(false);
        expect(before.ok ? "" : before.error).toBe("No CA certificate found.");

        fs.writeFileSync(path.join(appDataDir(), "ca-cert.pem"), PEM, "utf-8");
        expect(exportCert(registry).ok).toBe(true);
    });

    it("reports a missing CA as a result, not as an exception", () => {
        // "No CA generated yet" is an ordinary state the UI already renders, so it must not escape
        // the command layer as a throw — the shell's `tls:exportCert` branches on it directly.
        const registry = newRegistry();
        newWorkspace();

        const res = exportCert(registry);
        expect(res.ok).toBe(false);
        expect(res.ok ? "" : res.error).toContain("No CA certificate found");
    });
});

// ── tls.importCert / tls.importKey — ingress ──────────────────────────────────

describe("tls.importCert and tls.importKey", () => {
    const CERT = "-----BEGIN CERTIFICATE-----\nCERTBYTES\n-----END CERTIFICATE-----\n";
    const KEY = "-----BEGIN PRIVATE KEY-----\nKEYBYTES\n-----END PRIVATE KEY-----\n";

    it("copies the staged bytes into the engine's data dir, byte for byte", () => {
        const registry = newRegistry();
        newWorkspace();

        const blobId = pushBlob(registry, "my-ca.pem", Buffer.from(CERT, "utf-8"));
        const res = registry.invoke("tls.importCert", { blobId }, ctx) as TlsImportResult;

        expect(res.ok, res.ok ? "" : res.error).toBe(true);
        expect(fs.readFileSync(path.join(appDataDir(), "ca-cert.pem"), "utf-8")).toBe(CERT);
    });

    it("writes the key to its own file, not over the certificate", () => {
        const registry = newRegistry();
        newWorkspace();

        const certBlob = pushBlob(registry, "ca.pem", Buffer.from(CERT, "utf-8"));
        const keyBlob = pushBlob(registry, "ca.key", Buffer.from(KEY, "utf-8"));

        expect((registry.invoke("tls.importCert", { blobId: certBlob }, ctx) as TlsImportResult).ok).toBe(true);
        expect((registry.invoke("tls.importKey", { blobId: keyBlob }, ctx) as TlsImportResult).ok).toBe(true);

        // Two destinations derived from two different constants. Getting them crossed would leave
        // TLS unable to start with a perfectly plausible-looking pair of files on disk.
        expect(fs.readFileSync(path.join(appDataDir(), "ca-cert.pem"), "utf-8")).toBe(CERT);
        expect(fs.readFileSync(path.join(appDataDir(), "ca-key.pem"), "utf-8")).toBe(KEY);
    });

    it("preserves the bytes exactly, rather than round-tripping through a string", () => {
        // The pre-P3 handler was `copyFileSync`. Reading a PEM as UTF-8 and writing it straight back
        // would silently normalise anything non-ASCII; a PEM is ASCII so it would not show up
        // today, but the test pins the property rather than the coincidence.
        const registry = newRegistry();
        newWorkspace();

        const raw = Buffer.from([0x2d, 0x2d, 0x2d, 0xff, 0xfe, 0x00, 0x0a, 0x80]);
        const blobId = pushBlob(registry, "binary.pem", raw);
        expect((registry.invoke("tls.importCert", { blobId }, ctx) as TlsImportResult).ok).toBe(true);

        expect(fs.readFileSync(path.join(appDataDir(), "ca-cert.pem")).equals(raw)).toBe(true);
    });

    it("is a persisted import: the engine keeps the bytes and does not release the blob", () => {
        // `File_Ops_Protocol.md` §5 — a cert is copied into the engine's data dir and lives on,
        // unlike a transform-and-discard import. The *client* releases, and only because the staged
        // copy has served its purpose.
        const registry = newRegistry();
        newWorkspace();

        const blobId = pushBlob(registry, "ca.pem", Buffer.from(CERT, "utf-8"));
        expect((registry.invoke("tls.importCert", { blobId }, ctx) as TlsImportResult).ok).toBe(true);

        expect(fs.existsSync(path.join(appDataDir(), "ca-cert.pem"))).toBe(true);
        expect(fs.existsSync(path.join(blobRoot(), blobId))).toBe(true);

        expect(registry.invoke("blob.release", { blobId }, ctx)).toEqual({ ok: true });
        expect(fs.existsSync(path.join(blobRoot(), blobId))).toBe(false);
        // Releasing the staging copy must not disturb the imported one.
        expect(fs.readFileSync(path.join(appDataDir(), "ca-cert.pem"), "utf-8")).toBe(CERT);
    });

    it("reports a swept or released blob as blob-not-found, with no engine path in the message", () => {
        // The defect this pins down: `blobContentPath()` validates the id's shape but never checks
        // that the blob exists, so reading it directly produced a raw `ENOENT: no such file or
        // directory, open 'I:\...\blobs\blob_…\content'` — an absolute engine-side path in a
        // protocol result, which `File_Ops_Protocol.md` §8 names as a boundary violation. The
        // handler now goes through `statBlob()` first.
        const registry = newRegistry();
        newWorkspace();

        const blobId = pushBlob(registry, "ca.pem", Buffer.from(CERT, "utf-8"));
        registry.invoke("blob.release", { blobId }, ctx);

        const res = registry.invoke("tls.importCert", { blobId }, ctx) as TlsImportResult;
        expect(res.ok).toBe(false);
        if (res.ok) return;
        expect(res.error).toContain("blob-not-found");
        expect(res.error).not.toMatch(/ENOENT/);
        // No path of any kind — not the blob root, not the data dir, not the destination.
        expect(res.error).not.toContain(blobRoot());
        expect(res.error).not.toContain(appDataDir());
        expect(res.error).not.toContain(path.sep + "blobs");
    });

    it("reports an id that was never staged the same way", () => {
        const registry = newRegistry();
        newWorkspace();

        const neverStaged = `blob_${"0".repeat(32)}`;
        const res = registry.invoke("tls.importCert", { blobId: neverStaged }, ctx) as TlsImportResult;
        expect(res.ok).toBe(false);
        expect(res.ok ? "" : res.error).toContain("blob-not-found");
    });

    it("rejects a traversal attempt at the store's id check, without touching the filesystem", () => {
        const registry = newRegistry();
        newWorkspace();

        const res = registry.invoke("tls.importCert", { blobId: "../../etc/passwd" }, ctx) as TlsImportResult;
        expect(res.ok).toBe(false);
        expect(res.ok ? "" : res.error).toContain("blob-invalid-id");
        expect(fs.existsSync(path.join(appDataDir(), "ca-cert.pem"))).toBe(false);
    });

    it("leaves the previous certificate untouched when an import fails", () => {
        // The write happens after the blob read succeeds, so a bad id cannot truncate a working CA.
        // Order matters here: a handler that opened the destination first would destroy TLS on a
        // failed re-import.
        const registry = newRegistry();
        newWorkspace();
        fs.writeFileSync(path.join(appDataDir(), "ca-cert.pem"), CERT, "utf-8");

        const res = registry.invoke(
            "tls.importCert",
            { blobId: `blob_${"0".repeat(32)}` },
            ctx,
        ) as TlsImportResult;
        expect(res.ok).toBe(false);
        expect(fs.readFileSync(path.join(appDataDir(), "ca-cert.pem"), "utf-8")).toBe(CERT);
    });
});

// ── tls.generate / tls.certStatus / tls.removeCert — the engine half of item 5 ──

/**
 * These three have no blob and no dialog, so the assertions are about the **shape of the answer**
 * rather than about bytes: no paths, and an identity that is the identity of the exported
 * certificate.
 *
 * `generateCA()` is real here (no `mkcert` mock in this file), and `appDataDir()` is redirected to
 * the fixture root by `createWorkspace()`, so no test can touch the developer's own certificate.
 */
describe("the cert-lifecycle commands", () => {
    it("tls.generate reports a fingerprint and no paths at all", async () => {
        const registry = newRegistry();
        newWorkspace();

        const res = await registry.invoke("tls.generate", {}, ctx) as TlsGenerateResult;

        expect(res.ok, res.ok ? "" : res.error).toBe(true);
        expect(res.fingerprint).toMatch(/^sha256:[0-9a-f]{64}$/);
        // The item-6 criterion for this channel: `certPath` / `keyPath` used to travel here.
        expect(Object.keys(res).sort()).toEqual(["fingerprint", "ok"]);
    });

    it("the fingerprint identifies the certificate tls.exportCert hands out", async () => {
        // A round-trip across the two channels the client actually uses. The identity the engine
        // reports for its CA must be the identity of the bytes it exports — if `generateCA` hashed
        // one thing and `tls.exportCert` served another, drift detection would compare apples to
        // oranges and the client would "reinstall" a CA that had never changed.
        const registry = newRegistry();
        newWorkspace();

        const generated = await registry.invoke("tls.generate", {}, ctx) as TlsGenerateResult;
        const artifact = exportCert(registry);
        expect(artifact.ok, artifact.ok ? "" : artifact.error).toBe(true);
        if (!artifact.ok) return;

        const identity = identifyCert(bytesOf(registry, artifact).toString("utf-8"));
        expect(identity).not.toBeNull();
        expect(identity!.fingerprint).toBe(generated.fingerprint);
    });

    it("tls.certStatus reports the same fingerprint, and still no paths", async () => {
        const registry = newRegistry();
        newWorkspace();
        const generated = await registry.invoke("tls.generate", {}, ctx) as TlsGenerateResult;

        const status = registry.invoke("tls.certStatus", {}, ctx) as TlsCertStatusResult;

        expect(Object.keys(status).sort()).toEqual(["fingerprint", "generated"]);
        expect(status.generated).toBe(true);
        expect(status.fingerprint).toBe(generated.fingerprint);
    });

    it("tls.certStatus separates 'no CA' from 'a file that is not a certificate'", () => {
        // The state the pre-P3 code could not express, and the reason a corrupt `ca-cert.pem` used
        // to arrive as an opaque TLS failure much later. Note the import path deliberately does
        // *not* reject these bytes — it preserves arbitrary bytes by design — so this is the only
        // place the difference is visible.
        const registry = newRegistry();
        newWorkspace();

        expect((registry.invoke("tls.certStatus", {}, ctx) as TlsCertStatusResult).generated).toBe(false);

        fs.writeFileSync(path.join(appDataDir(), "ca-cert.pem"), "not a certificate", "utf-8");
        fs.writeFileSync(path.join(appDataDir(), "ca-key.pem"), "not a key", "utf-8");

        const corrupt = registry.invoke("tls.certStatus", {}, ctx) as TlsCertStatusResult;
        expect(corrupt.generated).toBe(true);
        expect(corrupt.fingerprint).toBeNull();
    });

    it("tls.removeCert reports its own half, and only its own half", async () => {
        const registry = newRegistry();
        newWorkspace();
        await registry.invoke("tls.generate", {}, ctx);

        const removed = registry.invoke("tls.removeCert", {}, ctx) as TlsRemoveCertResult;

        // `clientUntrusted` is composed by the shell and never crosses the wire: the engine has no
        // way to inspect the user's trust store, and on a remote engine it never will.
        expect(removed).toEqual({ ok: true, engineRemoved: true });
        expect((registry.invoke("tls.certStatus", {}, ctx) as TlsCertStatusResult).generated).toBe(false);
    });

    it("tls.removeCert on a clean machine reports engineRemoved: false, not an error", () => {
        const registry = newRegistry();
        newWorkspace();
        expect(registry.invoke("tls.removeCert", {}, ctx)).toEqual({ ok: true, engineRemoved: false });
    });
});

// ── runner.exportReport — egress ──────────────────────────────────────────────

/** A report with every field the renderer's real `RunnerRequestResult` carries. */
const FULL_REPORT = {
    folderId: "folder-1",
    folderName: "Smoke Suite",
    startedAt: 1704067200000, // 2024-01-01T00:00:00.000Z
    completedAt: 1704067202500,
    totalRequests: 1,
    totalTests: 2,
    passedTests: 1,
    failedTests: 1,
    results: [{
        requestId: "req-1",
        requestName: "List users",
        method: "GET",
        url: "http://api.localhost/users",
        status: 200,
        responseTime: 41,
        tests: [
            { name: "status is 200", passed: true, durationMs: 3 },
            { name: "body has items", passed: false, error: "expected 1, got 0", durationMs: 4 },
        ],
        testLogs: ["[log] fetched 0 items"],
        preScriptError: "pre: not defined",
        postScriptError: "post: not defined",
    }],
};

function exportReport(
    registry: CommandRegistry,
    report: unknown,
    format: "html" | "json",
): RunnerExportReportResult {
    return registry.invoke("runner.exportReport", { report, format }, ctx) as RunnerExportReportResult;
}

describe("runner.exportReport", () => {
    it("renders HTML inline, as an artifact with no path", () => {
        const registry = newRegistry();
        newWorkspace();

        const res = exportReport(registry, FULL_REPORT, "html");
        expect(res.ok, res.ok ? "" : res.error).toBe(true);
        if (!res.ok) return;

        expectArtifactKeys(res, "inline");
        expect(res.mimeType).toBe("text/html");
        // `<folderName>-report-<iso with : and . replaced by ->.<format>` — the same name the
        // pre-P3 save dialog prefilled.
        expect(res.suggestedName).toBe("Smoke Suite-report-2024-01-01T00-00-00-000Z.html");

        const html = Buffer.from(res.inline, "base64").toString("utf-8");
        expect(html).toContain("<!DOCTYPE html>");
        expect(html).toContain("Collection Run: Smoke Suite");
        expect(html).toContain("List users");
        expect(html).toContain("2.50s"); // (completedAt - startedAt) / 1000
    });

    it("renders JSON when the client asks for JSON", () => {
        // `format` is decided by the **client**, because only it knows which extension the user
        // chose in the save dialog — the pre-P3 shell derived it from the chosen path *after* the
        // fact, and that decision has to happen before the command is called.
        const registry = newRegistry();
        newWorkspace();

        const res = exportReport(registry, FULL_REPORT, "json");
        expect(res.ok, res.ok ? "" : res.error).toBe(true);
        if (!res.ok) return;

        expect(res.mimeType).toBe("application/json");
        expect(res.suggestedName).toBe("Smoke Suite-report-2024-01-01T00-00-00-000Z.json");
        expect(JSON.parse(Buffer.from(res.inline, "base64").toString("utf-8"))).toEqual(FULL_REPORT);
    });

    it("carries every renderer field through the schema, not just the ones the HTML uses", () => {
        // The defect this pins: P1 typed `RunReport` from the *HTML renderer's field usage* rather
        // than from `CollectionRunReport`, so `requestId`, `url`, `testLogs`, `preScriptError` and
        // `postScriptError` were missing. `z.object()` strips unknown keys, so routing the real
        // report through it silently truncated the JSON export — and the HTML renderer touches only
        // fields the schema happened to have, which is exactly why it would have shipped unnoticed.
        // The renderer's `TestResultEntry.durationMs` is required, and was optional here.
        const registry = newRegistry();
        newWorkspace();

        const res = exportReport(registry, FULL_REPORT, "json");
        expect(res.ok).toBe(true);
        if (!res.ok) return;
        const parsed = JSON.parse(Buffer.from(res.inline, "base64").toString("utf-8"));

        const result = parsed.results[0];
        expect(result.requestId).toBe("req-1");
        expect(result.url).toBe("http://api.localhost/users");
        expect(result.testLogs).toEqual(["[log] fetched 0 items"]);
        expect(result.preScriptError).toBe("pre: not defined");
        expect(result.postScriptError).toBe("post: not defined");
        expect(result.tests[0].durationMs).toBe(3);
        expect(result.tests[1].error).toBe("expected 1, got 0");
    });

    it("falls back to a generic name and a zero duration when the report is incomplete", () => {
        // `folderName` and `completedAt` are optional in the schema but required in the renderer's
        // own type. A run that is exported before it finishes must not render `NaN` — the fallback
        // is `completedAt ?? startedAt`, i.e. 0.00s.
        const registry = newRegistry();
        newWorkspace();

        const partial = { folderId: "f", startedAt: 1704067200000 };
        const res = exportReport(registry, partial, "html");
        expect(res.ok, res.ok ? "" : res.error).toBe(true);
        if (!res.ok) return;

        expect(res.suggestedName).toBe("collection-report-2024-01-01T00-00-00-000Z.html");
        const html = Buffer.from(res.inline, "base64").toString("utf-8");
        expect(html).toContain("Duration: 0.00s");
        expect(html).not.toContain("NaN");
    });

    it("escapes HTML in user-controlled fields", () => {
        // The report is rendered into a file the user then opens in a browser. `folderName` and
        // `requestName` are user-typed, so an unescaped `<` would be markup injection into a
        // locally-opened document.
        const registry = newRegistry();
        newWorkspace();

        const hostile = {
            ...FULL_REPORT,
            folderName: "<script>alert(1)</script>",
            results: [{ ...FULL_REPORT.results[0], requestName: "<img onerror=x>" }],
        };
        const res = exportReport(registry, hostile, "html");
        expect(res.ok).toBe(true);
        if (!res.ok) return;

        const html = Buffer.from(res.inline, "base64").toString("utf-8");
        expect(html).not.toContain("<script>");
        expect(html).not.toContain("<img onerror");
        expect(html).toContain("&lt;script&gt;");
    });

    it("stages a report above the threshold as a blob instead", () => {
        // Same `publishArtifact` decision as every other egress channel. A 1.5 MB report of
        // response bodies is realistic for a large collection run.
        const registry = newRegistry();
        newWorkspace();

        const big = {
            ...FULL_REPORT,
            results: Array.from({ length: 20 }, (_, i) => ({
                ...FULL_REPORT.results[0],
                requestId: `req-${i}`,
                testLogs: ["y".repeat(100_000)],
            })),
        };
        const res = exportReport(registry, big, "json");
        expect(res.ok, res.ok ? "" : res.error).toBe(true);
        if (!res.ok) return;

        expectArtifactKeys(res, "blobId");
        expect(res.size).toBeGreaterThan(BLOB_INLINE_THRESHOLD_BYTES);

        const bytes = pullBlob(registry, res.blobId);
        expect(bytes.length).toBe(res.size);
        expect(sha256(bytes)).toBe(res.sha256);
        expect(JSON.parse(bytes.toString("utf-8")).results).toHaveLength(20);
    });
});

// ── capture.shareJson — egress ────────────────────────────────────────────────

describe("capture.shareJson", () => {
    const ENTRIES = [
        { id: "cap-1", method: "POST", url: "http://api.localhost/login", status: 200, body: '{"t":1}' },
        { id: "cap-2", method: "GET", url: "http://api.localhost/me", status: 401, body: "" },
    ];

    it("serializes the entries the renderer holds, with a default filename", () => {
        const registry = newRegistry();
        newWorkspace();

        const res = registry.invoke("capture.shareJson", { entries: ENTRIES }, ctx) as CaptureShareJsonResult;
        expect(res.ok, res.ok ? "" : res.error).toBe(true);
        if (!res.ok) return;

        expectArtifactKeys(res, "inline");
        expect(res.mimeType).toBe("application/json");
        expect(res.suggestedName).toBe("captured-requests.json");
        expect(JSON.parse(Buffer.from(res.inline, "base64").toString("utf-8"))).toEqual(ENTRIES);
    });

    it("lets the client's suggested name win", () => {
        const registry = newRegistry();
        newWorkspace();

        const res = registry.invoke(
            "capture.shareJson",
            { entries: ENTRIES, suggestedName: "login-bug.json" },
            ctx,
        ) as CaptureShareJsonResult;
        expect(res.ok).toBe(true);
        expect(res.ok ? res.suggestedName : "").toBe("login-bug.json");
    });

    it("stages a large capture as a blob, which is the threshold earning its keep", () => {
        // Capture entries are whole request/response bodies, so this is the channel most likely to
        // exceed 1 MB in normal use.
        const registry = newRegistry();
        newWorkspace();

        const entries = Array.from({ length: 12 }, (_, i) => ({
            id: `cap-${i}`,
            body: "z".repeat(100_000),
        }));
        const res = registry.invoke("capture.shareJson", { entries }, ctx) as CaptureShareJsonResult;
        expect(res.ok, res.ok ? "" : res.error).toBe(true);
        if (!res.ok) return;

        expectArtifactKeys(res, "blobId");
        expect(res.size).toBeGreaterThan(BLOB_INLINE_THRESHOLD_BYTES);
        expect(JSON.parse(pullBlob(registry, res.blobId).toString("utf-8"))).toHaveLength(12);
    });
});

// ── audit.export — egress, and the one channel that needs a real git log ──────

/** Three commits whose subjects exercise the CSV writer's quoting. */
const AUDIT_ROWS: Array<{ rel: string; name: string; action: "create" | "update" | "delete" }> = [
    { rel: "mocks/mock-fo-1.json", name: 'Audit "One", v1', action: "create" },
    { rel: "mocks/mock-fo-2.json", name: "Audit Two", action: "create" },
    { rel: "mocks/mock-fo-3.json", name: "Audit Three", action: "update" },
];

async function seedAuditLog(ws: WorkspaceFixture): Promise<void> {
    initWorkspaceDir(TEST_WS, "File Ops Workspace");
    await initWorkspaceRepo(TEST_WS);

    for (const [i, row] of AUDIT_ROWS.entries()) {
        const file = path.join(ws.dataRoot, TEST_WS, row.rel);
        fs.mkdirSync(path.dirname(file), { recursive: true });
        fs.writeFileSync(file, JSON.stringify({ id: `mock-fo-${i + 1}`, name: row.name }), "utf-8");
        // `commitMutation` skips a commit when nothing is staged, so each row needs its own file.
        const hash = await commitMutation({
            action: row.action,
            entity: "mock",
            entityId: `mock-fo-${i + 1}`,
            entityName: row.name,
            workspaceId: TEST_WS,
            relPath: row.rel,
            actor: "tester@bifurc",
        });
        if (!hash) throw new Error(`commitMutation staged nothing for ${row.rel}`);
    }
}

/**
 * `audit.export` is the one egress command here whose handler is `async` — it awaits `queryLog`,
 * which shells out to git. `CommandRegistry.invoke()` is synchronous by design but returns whatever
 * the handler returns, so this is a promise and must be awaited. The other five are synchronous.
 */
async function exportAudit(registry: CommandRegistry, format: "json" | "csv"): Promise<AuditExportResult> {
    return (await registry.invoke("audit.export", { format }, ctx)) as AuditExportResult;
}

describe("audit.export", () => {
    it("exports the whole active workspace's log as JSON, newest first", async () => {
        const registry = newRegistry();
        const ws = newWorkspace();
        await seedAuditLog(ws);

        const res = await exportAudit(registry, "json");
        expect(res.ok, res.ok ? "" : res.error).toBe(true);
        if (!res.ok) return;

        expectArtifactKeys(res, "inline");
        expect(res.mimeType).toBe("application/json");
        expect(res.suggestedName).toBe("audit-log.json");

        const entries = JSON.parse(Buffer.from(res.inline, "base64").toString("utf-8"));
        expect(entries).toHaveLength(3);
        expect(entries.map((e: { entityName: string }) => e.entityName)).toEqual([
            "Audit Three",
            "Audit Two",
            'Audit "One", v1',
        ]);
        expect(entries[0].action).toBe("update");
        expect(entries[0].actor).toBe("tester@bifurc");
        expect(entries[0].workspaceId).toBe(TEST_WS);
        expect(entries[0].commitHash).toMatch(/^[0-9a-f]{7,40}$/);
    });

    it("exports CSV with the frozen header and quotes only the free-text column", async () => {
        const registry = newRegistry();
        const ws = newWorkspace();
        await seedAuditLog(ws);

        const res = await exportAudit(registry, "csv");
        expect(res.ok, res.ok ? "" : res.error).toBe(true);
        if (!res.ok) return;

        expect(res.mimeType).toBe("text/csv");
        expect(res.suggestedName).toBe("audit-log.csv");

        const lines = Buffer.from(res.inline, "base64").toString("utf-8").split("\n");
        expect(lines).toHaveLength(4);
        expect(lines[0]).toBe("commitHash,ts,action,entity,entityId,entityName,workspaceId,actor");

        // `entityName` is the one user-controlled field, so it is the one that is quoted — with
        // `""` for an embedded quote. Only the newest row is asserted in full; the shape is the
        // same for the rest and pinning all three would make the header assertion redundant.
        expect(lines[1]).toContain('"Audit Three"');
        expect(lines[3]).toContain('"Audit ""One"", v1"');

        // A raw epoch number, not an ISO string: that is what the pre-P3 export wrote, and changing
        // it would break anyone's spreadsheet formula for no stated reason.
        expect(lines[1].split(",")[1]).toMatch(/^\d+$/);
    });

    it("resolves the workspace itself rather than taking one on the wire", async () => {
        // `AuditExportParams` is `{format}` only. The renderer's `exportAudit(format)` passes nothing
        // else, and widening the wire contract to carry an id the engine can already look up would
        // be a change with no beneficiary. This asserts the engine really does look it up: the
        // active workspace is the fixture's, so its entries are what comes back.
        const registry = newRegistry();
        const ws = newWorkspace();
        await seedAuditLog(ws);

        const res = await exportAudit(registry, "json");
        expect(res.ok).toBe(true);
        if (!res.ok) return;
        const entries = JSON.parse(Buffer.from(res.inline, "base64").toString("utf-8"));
        expect(new Set(entries.map((e: { workspaceId: string }) => e.workspaceId))).toEqual(new Set([TEST_WS]));
    });

    it("returns an empty artifact for a workspace with no history, not an error", async () => {
        // A workspace whose repo has no commits is the state every newly created workspace is in.
        // `queryLog` returns `{entries: [], total: 0}` and the export is a valid empty file.
        const registry = newRegistry();
        const ws = newWorkspace();
        initWorkspaceDir(TEST_WS, "Empty Workspace");
        await initWorkspaceRepo(TEST_WS);

        const json = await exportAudit(registry, "json");
        expect(json.ok, json.ok ? "" : json.error).toBe(true);
        if (json.ok) expect(JSON.parse(Buffer.from(json.inline!, "base64").toString("utf-8"))).toEqual([]);

        const csv = await exportAudit(registry, "csv");
        expect(csv.ok).toBe(true);
        // The header is always written — a zero-row CSV is still a CSV.
        if (csv.ok) expect(Buffer.from(csv.inline!, "base64").toString("utf-8")).toBe(
            "commitHash,ts,action,entity,entityId,entityName,workspaceId,actor",
        );
    });
});

// ── The protocol schema is the gate ───────────────────────────────────────────

describe("the protocol schema gates every payload", () => {
    it("refuses ingress without a blobId, and refuses the retired inline `content` param", () => {
        // Both import params were `{content: string}` before P3. A PEM is only a few kilobytes, so
        // inline content was tempting — but the cert is a *persisted* import, and `blob.put` is what
        // applies the size cap and the three integrity checks. An inline parameter would be a second
        // way in that skips both. The schemas are `.strict()`, so a stale client is rejected rather
        // than silently ignored.
        const registry = newRegistry();
        newWorkspace();

        for (const command of ["tls.importCert", "tls.importKey"] as const) {
            expect(() => registry.invoke(command, {}, ctx)).toThrow(/Invalid payload/);
            expect(() =>
                registry.invoke(command, { content: "-----BEGIN CERTIFICATE-----" }, ctx),
            ).toThrow(/Invalid payload/);
            expect(() =>
                registry.invoke(command, { blobId: "blob_x", content: "y" }, ctx),
            ).toThrow(/Invalid payload/);
        }
    });

    it("refuses a filePath on the artifact-producing commands", () => {
        const registry = newRegistry();
        newWorkspace();

        // The pre-P3 shapes took a destination path; the engine no longer accepts one at all.
        expect(() => registry.invoke("tls.exportCert", { filePath: "/tmp/ca.pem" }, ctx)).toThrow(/Invalid payload/);
        expect(() =>
            registry.invoke("runner.exportReport", { report: FULL_REPORT, format: "html", filePath: "/tmp/r.html" }, ctx),
        ).toThrow(/Invalid payload/);
        expect(() =>
            registry.invoke("capture.shareJson", { entries: [], filePath: "/tmp/c.json" }, ctx),
        ).toThrow(/Invalid payload/);
        expect(() =>
            registry.invoke("audit.export", { format: "json", filePath: "/tmp/a.json" }, ctx),
        ).toThrow(/Invalid payload/);
    });

    it("rejects an unknown format and an unknown capture shape", () => {
        const registry = newRegistry();
        newWorkspace();

        expect(() => registry.invoke("audit.export", { format: "xml" }, ctx)).toThrow(/Invalid payload/);
        expect(() => registry.invoke("runner.exportReport", { report: FULL_REPORT, format: "pdf" }, ctx)).toThrow(
            /Invalid payload/,
        );
        // `entries` is `z.array(z.unknown())` — a string is not an array.
        expect(() => registry.invoke("capture.shareJson", { entries: "nope" }, ctx)).toThrow(/Invalid payload/);
    });
});
