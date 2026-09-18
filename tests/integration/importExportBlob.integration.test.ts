/**
 * P3 work items 3–4 — the blob round trip, through the **real command layer**.
 *
 * WHY THIS IS A SEPARATE SUITE
 * ----------------------------
 * `importExport.integration.test.ts` and `importExportFormats.integration.test.ts` test the format
 * codecs through `tests/integration/importExportHarness.ts`, which writes and reads plain files.
 * That is the right shape for "does this exporter's schema match its importer's", but it never
 * touches the machinery P3 actually added:
 *
 *   `export.create`    → inline bytes, or an artifact staged as a blob
 *   `blob.put`         → a `blobId` the engine can read twice
 *   `import.preflight` / `import.commit` → the same `blobId`, without re-uploading
 *   `blob.release`     → the client's cleanup
 *
 * So this file drives that sequence through a real `CommandRegistry`, which means every payload
 * also passes the frozen Zod schemas in `@bifurc/protocol`. A regression in the inline/blob
 * decision, the read loop, the filename plumbing or the collision-strategy enum fails here and
 * nowhere else.
 *
 * ## Two things the fixture forces, both worth knowing
 *
 * **1. The blob root has to be set separately.** `createWorkspace()` (proxyHarness) points the
 * *workspace store* at a temp dir via `setDataRootOverride`. It does not call `setDataRoot()`,
 * because until P3 nothing outside the workspace tree needed a data root. The blob store resolves
 * through `dataDir()` (`blob/store.ts#blobRoot`), so the fixture supplies it too — exactly as
 * `src/main.ts` does with `app.getPath("userData")`. Passing the fixture's **root** (not its
 * `data/` subdirectory) reproduces production's layout byte for byte:
 *
 *   <userData>/blobs/<blobId>/          ← blobRoot()
 *   <userData>/data/<workspaceId>/      ← workspaceFs.dataRoot()
 *
 * Getting it wrong is not silent — `dataDir()` throws `DataRootNotInitialisedError`, the
 * "loud by design" behaviour `plan/03` work item 4 asked for.
 *
 * **2. A staged blob belongs to the fixture that staged it.** Because the blob root moves with
 * `setDataRoot`, creating a *second* fixture mid-test would orphan the first fixture's blob. So a
 * test that wants to import into a fresh, empty workspace exports first, keeps the **bytes**, and
 * re-uploads them after the swap — which is what a client does anyway: it holds a file on its own
 * disk, and the engine's staging area is transient. `stageExport()` below is that pattern.
 *
 * Note also that `loadConfig()` keys `mocks`/`mappings`/`environments` off the **active**
 * workspace, so the realistic journey these tests exercise is "export the workspace you have open,
 * import back into it" — the same assumption the format-level suites make.
 *
 * Runs in the integration project (shared process-global data-root override ⇒
 * `fileParallelism: false`).
 */

import { describe, it, expect, afterAll } from "vitest";
import * as crypto from "crypto";
import * as fs from "fs";
import * as path from "path";
import {
    BLOB_INLINE_THRESHOLD_BYTES,
    type BlobPutResult,
    type BlobReadResult,
    type BlobStatResult,
    type ExportCreateResult,
    type ImportCommitResult,
    type ImportPreflightResult,
} from "@bifurc/protocol";
import { CommandRegistry, type CommandContext } from "@bifurc/engine/commands/registry";
import { registerBlobCommands } from "@bifurc/engine/blob/commands";
import { registerImportExportCommands } from "@bifurc/engine/importExport/commands";
import { blobRoot, statBlob } from "@bifurc/engine/blob/store";
import { getAllFormats } from "@bifurc/engine/importExport/registry";
import { loadConfig } from "@bifurc/engine/store/config";
import { readAllEntities } from "@bifurc/engine/store/workspaceFs";
import { resetDataRootForTests, setDataRoot } from "@bifurc/engine/store/paths";
import type { EntityKind } from "@bifurc/engine/importExport/types";
import { createWorkspace, TEST_WS, type WorkspaceFixture } from "./proxyHarness";

// ── Fixtures ──────────────────────────────────────────────────────────────────

const MOCK = {
    id: "mock-blob", name: "Blob Mock", method: "POST", urlPattern: "/api/blob", useRegex: true, enabled: true,
    capturedHeaders: {}, capturedBody: "",
    responseStatus: 201, responseHeaders: { "content-type": "application/json" },
    responseBody: '{"blob":true}', responseDelay: 0, createdAt: 1, folderId: null, workspaceId: TEST_WS,
};

const MAPPING = {
    id: "map-blob", domain: "blob.localhost", target: "127.0.0.1:9001", enabled: true, label: "Blob",
    createdAt: 1, workspaceId: TEST_WS,
};

const RULE = {
    id: "rule-blob", name: "Blob Rule", pattern: "http://api.blob/*", useRegex: true,
    targetType: "external", targetMappingId: "", targetExternal: "127.0.0.1:9002",
    requestScript: "lp.request.headers['x-b'] = '1';",
    responseScript: "lp.response.body = 'rw';",
    enabled: true, createdAt: 1, folderId: null, workspaceId: TEST_WS,
};

const REQUEST = {
    id: "req-blob", name: "Blob Request", method: "POST", url: "http://api.blob/items",
    headers: { "content-type": "application/json" }, body: '{"n":1}', createdAt: 1, folderId: null, workspaceId: TEST_WS,
};

const SOCKET = {
    id: "sock-blob", name: "Blob Socket", url: "ws://api.blob/ws", headers: {}, createdAt: 1, folderId: null, workspaceId: TEST_WS,
};

const WEBHOOK = {
    id: "hook-blob", name: "Blob Hook", urlSuffix: "blob-hook", createdAt: 1, folderId: null, workspaceId: TEST_WS,
};

const ENVIRONMENT = {
    id: "env-blob", name: "Blob Env", createdAt: 1, workspaceId: TEST_WS,
    variables: [{ id: "v1", key: "TOKEN", value: "abc123" }],
};

const fixtures: WorkspaceFixture[] = [];

/** Write raw entity files for a kind, maintaining that kind's `enabled.json` like the real store. */
function seedEntities(ws: WorkspaceFixture, dir: string, entities: any[]): void {
    const target = path.join(ws.dataRoot, TEST_WS, dir);
    fs.mkdirSync(target, { recursive: true });

    const enabledFile = path.join(target, "enabled.json");
    let current: string[] = [];
    try { current = JSON.parse(fs.readFileSync(enabledFile, "utf-8")); } catch { /* none yet */ }
    const enabledSet = new Set(current);

    for (const e of entities) {
        const { enabled: _e, ...rest } = e;
        void _e;
        fs.writeFileSync(path.join(target, `${e.id}.json`), JSON.stringify(rest, null, 2), "utf-8");
        if (e.enabled === false) enabledSet.delete(e.id);
        else enabledSet.add(e.id);
    }
    fs.writeFileSync(enabledFile, JSON.stringify([...enabledSet], null, 2), "utf-8");
}

/** One entity of every kind, so no format exports an empty artifact. */
function seedAll(ws: WorkspaceFixture): void {
    seedEntities(ws, "mocks", [MOCK]);
    seedEntities(ws, "mappings", [MAPPING]);
    seedEntities(ws, "rules", [RULE]);
    seedEntities(ws, "environments", [ENVIRONMENT]);
    seedEntities(ws, "requests", [REQUEST]);
    seedEntities(ws, "sockets", [SOCKET]);
    seedEntities(ws, "webhooks", [WEBHOOK]);
}

function newWorkspace(seed?: (ws: WorkspaceFixture) => void): WorkspaceFixture {
    const ws = createWorkspace();
    fixtures.push(ws);
    // See the header, point 1: the blob store resolves through `dataDir()`, which the proxy
    // harness does not set. The fixture root is the analogue of Electron's `userData`.
    setDataRoot(ws.root);
    seed?.(ws);
    return ws;
}

afterAll(() => {
    for (const f of fixtures) f.cleanup();
    resetDataRootForTests();
});

// ── The command layer under test ──────────────────────────────────────────────

/**
 * A fresh registry per test rather than the process-wide `commandRegistry` singleton:
 * `register()` throws on a double registration, and the singleton is shared across the whole run.
 * Both registration functions are the real ones the shell calls (`src/ipc/handlers.ts`), so this
 * exercises the same surface production does.
 */
function newRegistry(): CommandRegistry {
    const registry = new CommandRegistry();
    registerBlobCommands(registry);
    registerImportExportCommands(registry);
    return registry;
}

const ctx: CommandContext = { bus: {} as CommandContext["bus"] };

// ── Client-side primitives, written the way `src/ipc/importExportHandlers.ts` writes them ──

/** `export.create`, typed. */
async function exportCreate(
    registry: CommandRegistry,
    kind: string,
    format: string,
    filename?: string,
): Promise<ExportCreateResult> {
    return (await registry.invoke(
        "export.create",
        { kind, format, workspaceId: TEST_WS, ...(filename ? { filename } : {}) },
        ctx,
    )) as ExportCreateResult;
}

/** `blob.put` — the client uploading bytes it read from its own disk. */
function pushBlob(registry: CommandRegistry, filename: string, bytes: Buffer, mimeType = "application/json"): string {
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
 * shortcut (`importExport/commands.ts#readBlobText`); this is the loop the client actually runs,
 * so a broken `eof` or a mis-computed offset fails here.
 */
function pullBlob(registry: CommandRegistry, blobId: string): Buffer {
    const chunks: Buffer[] = [];
    let offset = 0;
    // A hung `eof` would otherwise spin forever and take the whole suite with it.
    for (let guard = 0; guard < 10_000; guard++) {
        const res = registry.invoke("blob.read", { blobId, offset }, ctx) as BlobReadResult;
        const slice = Buffer.from(res.data, "base64");
        if (slice.length) chunks.push(slice);
        offset += slice.length;
        if (res.eof) return Buffer.concat(chunks);
    }
    throw new Error(`blob.read never reported eof for ${blobId}`);
}

/** The bytes an `export.create` result refers to, whichever shape it came back in. */
function bytesOf(registry: CommandRegistry, res: Extract<ExportCreateResult, { ok: true }>): Buffer {
    return res.inline !== undefined ? Buffer.from(res.inline, "base64") : pullBlob(registry, res.blobId);
}

function preflight(registry: CommandRegistry, kind: string, format: string, blobId: string): ImportPreflightResult {
    return registry.invoke("import.preflight", { kind, format, workspaceId: TEST_WS, blobId }, ctx) as ImportPreflightResult;
}

async function commit(
    registry: CommandRegistry,
    kind: string,
    format: string,
    blobId: string,
    strategy?: "keep" | "override" | "new",
): Promise<ImportCommitResult> {
    return (await registry.invoke(
        "import.commit",
        { kind, format, workspaceId: TEST_WS, blobId, ...(strategy ? { collisionStrategy: strategy } : {}) },
        ctx,
    )) as ImportCommitResult;
}

const sha256 = (bytes: Buffer): string => crypto.createHash("sha256").update(bytes).digest("hex");

/** `export.create` + `blob.put` in one step: the artifact is now staged and ready to import. */
async function stageMockExport(): Promise<{
    registry: CommandRegistry;
    blobId: string;
    ws: WorkspaceFixture;
}> {
    const registry = newRegistry();
    const ws = newWorkspace((w) => seedEntities(w, "mocks", [MOCK]));
    const exp = await exportCreate(registry, "mocks", "mocks-json");
    expect(exp.ok, exp.ok ? "" : exp.error).toBe(true);
    if (!exp.ok) throw new Error("export failed");
    return { registry, blobId: pushBlob(registry, exp.suggestedName, bytesOf(registry, exp)), ws };
}

// ── The full client journey ───────────────────────────────────────────────────

describe("export → blob → import", () => {
    it("round-trips mocks through export.create, blob.put, preflight and commit", async () => {
        const registry = newRegistry();
        newWorkspace((ws) => seedEntities(ws, "mocks", [MOCK]));

        // 1. egress: the engine renders the artifact and hands back a reference
        const exp = await exportCreate(registry, "mocks", "mocks-json");
        expect(exp.ok, exp.ok ? "" : exp.error).toBe(true);
        if (!exp.ok) return;

        const bytes = bytesOf(registry, exp);
        expect(bytes.length).toBe(exp.size);
        expect(sha256(bytes)).toBe(exp.sha256);
        expect(exp.mimeType).toBe("application/json");
        expect(exp.suggestedName).toBe("mocks-export.json");

        // 2. ingress: the client stages the bytes. Uploading the *exported* artifact means the
        //    journey really is engine → client → engine, with the bytes passing through the wire
        //    primitive in both directions.
        const blobId = pushBlob(registry, exp.suggestedName, bytes);
        expect(blobId).toMatch(/^blob_/);

        // 3. preflight sees the entity and reports it as a collision (same workspace)
        const pf = preflight(registry, "mocks", "mocks-json", blobId);
        expect(pf.ok, pf.ok ? "" : pf.error).toBe(true);
        if (!pf.ok) return;
        expect(pf.itemCount).toBe(1);
        expect(pf.collisionIds).toContain(MOCK.id);

        // 4. commit applies it
        const res = await commit(registry, "mocks", "mocks-json", blobId, "override");
        expect(res.ok, res.ok ? "" : res.error).toBe(true);

        const onDisk = readAllEntities<any>(TEST_WS, "mocks").find((m) => m.id === MOCK.id);
        expect(onDisk?.name).toBe("Blob Mock");
        expect(onDisk?.responseStatus).toBe(201);
    });

    it("reports no collisions when the target workspace is empty", async () => {
        // Export first, keep the bytes, then move to a fresh workspace and re-upload — see the
        // header, point 2. This is the "import into a workspace I just created" journey.
        const registry = newRegistry();
        newWorkspace((ws) => seedEntities(ws, "mocks", [MOCK]));

        const exp = await exportCreate(registry, "mocks", "mocks-json");
        expect(exp.ok).toBe(true);
        if (!exp.ok) return;
        const bytes = bytesOf(registry, exp);

        newWorkspace();
        const blobId = pushBlob(registry, exp.suggestedName, bytes);

        const pf = preflight(registry, "mocks", "mocks-json", blobId);
        expect(pf.ok, pf.ok ? "" : pf.error).toBe(true);
        if (!pf.ok) return;
        expect(pf.itemCount).toBe(1);
        expect(pf.collisionIds).toEqual([]);

        const res = await commit(registry, "mocks", "mocks-json", blobId, "keep");
        expect(res.ok, res.ok ? "" : res.error).toBe(true);
        if (!res.ok) return;
        expect(res.imported).toBe(1);
        expect(res.skipped).toBe(0);
    });

    it("never re-uploads: one blob serves both preflight and commit", async () => {
        const { registry, blobId } = await stageMockExport();
        const before = statBlob(blobId).size;

        expect(preflight(registry, "mocks", "mocks-json", blobId).ok).toBe(true);
        // `import.preflight` is an in-process `readFileSync` that does NOT slide the lease, so a
        // commit that follows must still find the blob. This is the "upload once, reference twice"
        // property (`File_Ops_Protocol.md` §2.3) — the whole reason the interface stopped taking a
        // path and started taking content.
        expect((await commit(registry, "mocks", "mocks-json", blobId, "override")).ok).toBe(true);
        expect(statBlob(blobId).size).toBe(before);
    });

    it("leaves the blob in place after commit, so a failed commit is retryable", async () => {
        const { registry, blobId } = await stageMockExport();

        expect((await commit(registry, "mocks", "mocks-json", blobId, "override")).ok).toBe(true);
        // The engine does NOT auto-release. `File_Ops_Protocol.md` §5 makes cleanup the *client's*
        // call, because the user may resolve collisions and commit again — and a commit that
        // released its own source would make that second attempt fail with `blob-not-found`.
        expect(statBlob(blobId).size).toBeGreaterThan(0);

        // The client's cleanup. `ok:false` on the second call means "nothing there", which is
        // different information from "I deleted it" during a retry — hence a result, not a throw.
        expect(registry.invoke("blob.release", { blobId }, ctx)).toEqual({ ok: true });
        expect(registry.invoke("blob.release", { blobId }, ctx)).toEqual({ ok: false });
        expect(fs.existsSync(path.join(blobRoot(), blobId))).toBe(false);
    });

    it("reports a released blob as a blob-not-found error, not as a thrown exception", async () => {
        const { registry, blobId } = await stageMockExport();
        registry.invoke("blob.release", { blobId }, ctx);

        // A user who leaves the collision dialog open past `BLOB_TTL_MS` gets the same shape of
        // failure as one who cancelled and came back. Both are an `ok:false` result carrying a
        // greppable code — never an exception out of the command layer.
        const pf = preflight(registry, "mocks", "mocks-json", blobId);
        expect(pf.ok).toBe(false);
        expect(pf.ok ? "" : pf.error).toContain("blob-not-found");

        const res = await commit(registry, "mocks", "mocks-json", blobId, "override");
        expect(res.ok).toBe(false);
        expect(res.ok ? "" : res.error).toContain("blob-not-found");
    });

    it("reports an id that was never staged as blob-not-found too", () => {
        // This is the engine-side replacement for the pre-P3 "missing file" case. The engine can no
        // longer be asked about a path that does not exist, but it can still be asked about a
        // `blobId` that does not exist — a stale client, a second window, or a swept blob. The
        // schema accepts the id (it is a string); the store is what rejects it, as a result rather
        // than a throw.
        const registry = newRegistry();
        newWorkspace();

        const neverStaged = `blob_${"0".repeat(32)}`;
        const pf = preflight(registry, "mocks", "mocks-json", neverStaged);
        expect(pf.ok).toBe(false);
        expect(pf.ok ? "" : pf.error).toContain("blob-not-found");

        // An id that is not even blob-shaped fails the store's format check, and is reported the
        // same way rather than escaping as an exception.
        const malformed = preflight(registry, "mocks", "mocks-json", "../../etc/passwd");
        expect(malformed.ok).toBe(false);
        expect(malformed.ok ? "" : malformed.error).toContain("blob-invalid-id");
    });
});

// ── The filename the client supplies ─────────────────────────────────────────

describe("the blob's filename reaches the importer", () => {
    it("names an imported dotenv environment after the uploaded file", async () => {
        const registry = newRegistry();
        newWorkspace();

        // The client's file is literally `.env`, which is the name a user is most likely to have.
        // Stripping `/\.env.*$/` from it yields the empty string, so `dotenvEnvName`'s
        // `|| "Imported"` fallback is the only thing between this import and an unnamed environment.
        const blobId = pushBlob(registry, ".env", Buffer.from("TOKEN=abc123\n", "utf-8"), "text/plain");

        const pf = preflight(registry, "environments", "environments-dotenv", blobId);
        expect(pf.ok, pf.ok ? "" : pf.error).toBe(true);
        expect((await commit(registry, "environments", "environments-dotenv", blobId, "override")).ok).toBe(true);

        const envs = readAllEntities<any>(TEST_WS, "environments").filter((e) => e.id !== "__global__");
        expect(envs).toHaveLength(1);
        expect(envs[0].name).toBe("Imported");
        expect(Object.fromEntries(envs[0].variables.map((v: any) => [v.key, v.value]))).toEqual({ TOKEN: "abc123" });
    });

    it("derives a usable name from a dotted .env filename", async () => {
        const registry = newRegistry();
        newWorkspace();

        const blobId = pushBlob(registry, "staging.env", Buffer.from("K=V\n", "utf-8"), "text/plain");
        expect((await commit(registry, "environments", "environments-dotenv", blobId, "override")).ok).toBe(true);

        const envs = readAllEntities<any>(TEST_WS, "environments").filter((e) => e.id !== "__global__");
        expect(envs).toHaveLength(1);
        expect(envs[0].name).toBe("staging");
    });

    it("picks the YAML branch of the OpenAPI importer from the filename", async () => {
        // Two regressions are pinned down here, and they are separate bugs.
        //
        // 1. `importers/requests-openapi.ts` chose JSON vs YAML with `path.extname(filePath)`. A
        //    staged blob's content file is named `content` and has NO extension, so under the blob
        //    layer that lookup had nothing to work with and every YAML spec would have silently
        //    taken the JSON branch. The extension now comes from the client-supplied filename,
        //    which is the only place it can come from.
        //
        // 2. `preflight` did not use the same parser as `run` at all — it tried `JSON.parse` and,
        //    on failure, returned `Math.floor(lines containing ":" / 3)` as the item count. So a
        //    YAML spec's `itemCount` was fiction, and the assertion below would have read 3.
        const registry = newRegistry();
        newWorkspace();

        const spec = [
            "openapi: 3.0.3",
            "info:",
            "  title: YAML Spec",
            "  version: 1.0.0",
            "paths:",
            "  /yaml-items:",
            "    get:",
            "      summary: Fetch YAML items",
            "      responses:",
            "        '200':",
            "          description: ok",
        ].join("\n");

        const blobId = pushBlob(registry, "spec.yaml", Buffer.from(spec, "utf-8"), "application/yaml");

        const pf = preflight(registry, "requests", "requests-openapi", blobId);
        expect(pf.ok, pf.ok ? "" : pf.error).toBe(true);
        if (!pf.ok) return;
        expect(pf.itemCount).toBe(1);

        expect((await commit(registry, "requests", "requests-openapi", blobId, "override")).ok).toBe(true);

        const reqs = readAllEntities<any>(TEST_WS, "requests");
        expect(reqs).toHaveLength(1);
        expect(reqs[0].name).toBe("Fetch YAML items");
        expect(reqs[0].method).toBe("GET");
        expect(reqs[0].url).toBe("/yaml-items");
    });

    it("reports a malformed spec as an error rather than as an importable file", async () => {
        // The other half of the preflight fix: previously *any* text that failed `JSON.parse` was
        // reported `ok: true` with a guessed count, so a truncated or unrelated file was offered to
        // the user as an import with no warning.
        const registry = newRegistry();
        newWorkspace();

        const blobId = pushBlob(registry, "broken.yaml", Buffer.from("this: is: not: openapi\n", "utf-8"), "application/yaml");
        const pf = preflight(registry, "requests", "requests-openapi", blobId);
        expect(pf.ok).toBe(false);
        expect(pf.ok ? "" : pf.error).toBeTruthy();
    });
});

// ── The inline / blob decision ────────────────────────────────────────────────

describe("the inline-vs-blob threshold", () => {
    it("returns a small artifact inline and never touches the store", async () => {
        const registry = newRegistry();
        newWorkspace((ws) => seedEntities(ws, "mocks", [MOCK]));

        const exp = await exportCreate(registry, "mocks", "mocks-json");
        expect(exp.ok).toBe(true);
        if (!exp.ok) return;

        expect(exp.inline).toBeDefined();
        expect(exp.blobId).toBeUndefined();
        expect(exp.size).toBeLessThanOrEqual(BLOB_INLINE_THRESHOLD_BYTES);
        // `sha256` is returned for the inline case too — the client verifies what it wrote without
        // having to branch on which shape it received. It is the digest of the *decoded* bytes, not
        // of the base64 text, which is what makes that verification meaningful.
        expect(exp.sha256).toBe(sha256(Buffer.from(exp.inline!, "base64")));
        // The common case is one round trip: nothing was staged, so the blob root was never created.
        expect(fs.existsSync(blobRoot())).toBe(false);
    });

    it("stages an artifact above the threshold and returns a blobId instead", async () => {
        const registry = newRegistry();
        // ~1.2 MB of response bodies, comfortably past the 1 MB inline threshold.
        const bigBody = "x".repeat(100_000);
        newWorkspace((ws) =>
            seedEntities(
                ws,
                "mocks",
                Array.from({ length: 12 }, (_, i) => ({ ...MOCK, id: `mock-big-${i}`, responseBody: bigBody })),
            ),
        );

        const exp = await exportCreate(registry, "mocks", "mocks-json");
        expect(exp.ok).toBe(true);
        if (!exp.ok) return;

        expect(exp.inline).toBeUndefined();
        expect(exp.blobId).toBeDefined();
        expect(exp.size).toBeGreaterThan(BLOB_INLINE_THRESHOLD_BYTES);

        // The bytes are really there, and they really are the export.
        const bytes = pullBlob(registry, exp.blobId!);
        expect(bytes.length).toBe(exp.size);
        expect(sha256(bytes)).toBe(exp.sha256);
        const parsed = JSON.parse(bytes.toString("utf-8"));
        expect(parsed.mocks).toHaveLength(12);
        expect(parsed.mocks[0].responseBody).toHaveLength(100_000);

        // And the store agrees with what the command reported.
        const stat = registry.invoke("blob.stat", { blobId: exp.blobId }, ctx) as BlobStatResult;
        expect(stat.size).toBe(exp.size);
        expect(stat.sha256).toBe(exp.sha256);
        expect(stat.filename).toBe(exp.suggestedName);
        expect(stat.ttlRemainingMs).toBeGreaterThan(0);
    });

    it("lets the client's filename win over the engine's suggestion", async () => {
        const registry = newRegistry();
        newWorkspace((ws) => seedEntities(ws, "mocks", [MOCK]));

        // This is what the user typed into the save dialog. The engine echoes it back and records
        // it in the blob metadata rather than overriding it with its own default.
        const exp = await exportCreate(registry, "mocks", "mocks-json", "my-mocks-backup.json");
        expect(exp.ok).toBe(true);
        if (!exp.ok) return;
        expect(exp.suggestedName).toBe("my-mocks-backup.json");

        const stat = registry.invoke(
            "blob.stat",
            { blobId: pushBlob(registry, exp.suggestedName, bytesOf(registry, exp)) },
            ctx,
        ) as BlobStatResult;
        expect(stat.filename).toBe("my-mocks-backup.json");
    });

    it("falls back to a format-aware default name when the client supplies none", async () => {
        const registry = newRegistry();
        newWorkspace((ws) => seedEntities(ws, "environments", [ENVIRONMENT]));

        const exp = await exportCreate(registry, "environments", "environments-dotenv");
        expect(exp.ok).toBe(true);
        if (!exp.ok) return;
        // `<kind-kebab>-export.<first extension>` — the same name the pre-P3 save dialog prefilled.
        expect(exp.suggestedName).toBe("environments-export.env");
        expect(exp.mimeType).toBe("text/plain");
    });

    it("rejects an unknown format without throwing", async () => {
        const registry = newRegistry();
        newWorkspace();

        const badFormat = await exportCreate(registry, "mocks", "mocks-nope");
        expect(badFormat.ok).toBe(false);
        expect(badFormat.ok ? "" : badFormat.error).toContain("mocks-nope");

        const badKind = await exportCreate(registry, "nope", "mocks-json");
        expect(badKind.ok).toBe(false);
    });
});

// ── workspace-zip: the path-shaped exception ──────────────────────────────────

describe("workspace-zip through the command layer", () => {
    it("stages the archive as a blob and re-imports it into a new workspace", async () => {
        const registry = newRegistry();
        newWorkspace(seedAll);

        const exp = await exportCreate(registry, "workspace", "workspace-zip");
        expect(exp.ok, exp.ok ? "" : exp.error).toBe(true);
        if (!exp.ok) return;

        // `workspace-zip` is the one format that never returns inline, even below the threshold:
        // `archiver` piped into a staging file, and reading that file back to satisfy the inline
        // branch would undo the reason it streamed in the first place.
        expect(exp.inline).toBeUndefined();
        expect(exp.blobId).toBeDefined();
        expect(exp.mimeType).toBe("application/zip");
        expect(exp.suggestedName).toBe("workspace-export.zip");

        const archive = pullBlob(registry, exp.blobId!);
        expect(archive.length).toBe(exp.size);
        expect(sha256(archive)).toBe(exp.sha256);
        // ZIP magic — proof the exporter actually streamed an archive into the staging handle
        // rather than leaving it empty or failing silently.
        expect(archive.subarray(0, 2).toString("ascii")).toBe("PK");

        // Re-upload it the way the client would (a second, independent copy of the bytes).
        const blobId = pushBlob(registry, exp.suggestedName, archive, "application/zip");

        const pf = preflight(registry, "workspace", "workspace-zip", blobId);
        expect(pf.ok, pf.ok ? "" : pf.error).toBe(true);

        const res = await commit(registry, "workspace", "workspace-zip", blobId, "override");
        expect(res.ok, res.ok ? "" : res.error).toBe(true);
        if (!res.ok) return;
        expect(res.imported).toBeGreaterThan(0);

        // A zip import creates a NEW workspace and makes it active.
        const newWsId = loadConfig().activeWorkspaceId;
        expect(newWsId).not.toBe(TEST_WS);
        expect(readAllEntities<any>(newWsId, "mocks")).toHaveLength(1);
        expect(readAllEntities<any>(newWsId, "mocks")[0].name).toBe("Blob Mock");
    });

    it("rejects a non-zip upload in preflight", async () => {
        const registry = newRegistry();
        newWorkspace(seedAll);

        const blobId = pushBlob(registry, "not-a-zip.zip", Buffer.from("this is not an archive", "utf-8"), "application/zip");
        const pf = preflight(registry, "workspace", "workspace-zip", blobId);
        expect(pf.ok).toBe(false);
        expect(pf.ok ? "" : pf.error).toContain("ZIP");
    });
});

// ── Every registered format is reachable through the command layer ───────────

/**
 * The format-level suites prove each codec round-trips its own content. This sweep proves the
 * *transport* is wired for every format the registry offers — including the two path-shaped ones,
 * which take a completely different branch inside `export.create` and `import.preflight`.
 *
 * It asserts reachability, not content fidelity: that is the other suites' job, and duplicating it
 * here would make a codec regression fail in two places and be diagnosable in neither.
 */
const REACHABLE: Array<{ kind: EntityKind; format: string }> = [];
for (const [kind, formats] of Object.entries(getAllFormats())) {
    for (const f of formats) {
        if (f.supportsExport && f.supportsImport) REACHABLE.push({ kind: kind as EntityKind, format: f.id });
    }
}

// The two whole-workspace formats run last: their import switches the active workspace and
// registers a new one, which would perturb the fixture the other 15 export from.
const ORDERED = [
    ...REACHABLE.filter((f) => f.kind !== "workspace"),
    ...REACHABLE.filter((f) => f.kind === "workspace"),
];

describe("format coverage through the blob layer", () => {
    it("sweeps all 17 registered formats", () => {
        // workspace 2 + requests 5 + mocks 3 + environments 3 + mappings 1 + proxyRules 1 +
        // websockets 1 + webhooks 1. Pinned so a format that silently loses its implementation, or
        // a new one that is never exercised below, is a visible failure rather than a quiet gap.
        expect(REACHABLE).toHaveLength(17);
        expect(ORDERED).toHaveLength(17);
    });

    for (const { kind, format } of ORDERED) {
        it(`${kind}/${format} survives export → blob → import`, async () => {
            const registry = newRegistry();
            newWorkspace(seedAll);

            const exp = await exportCreate(registry, kind, format);
            expect(exp.ok, `export.create failed: ${exp.ok ? "" : exp.error}`).toBe(true);
            if (!exp.ok) return;

            const bytes = bytesOf(registry, exp);
            expect(bytes.length, "artifact is empty").toBeGreaterThan(0);
            expect(bytes.length).toBe(exp.size);
            expect(sha256(bytes)).toBe(exp.sha256);

            const blobId = pushBlob(registry, exp.suggestedName, bytes, exp.mimeType);

            const pf = preflight(registry, kind, format, blobId);
            expect(pf.ok, `import.preflight failed: ${pf.ok ? "" : pf.error}`).toBe(true);
            if (!pf.ok) return;
            // `workspace-zip` reports 0 — it counts files while extracting, not while probing.
            if (format !== "workspace-zip") {
                expect(pf.itemCount, "preflight found nothing to import").toBeGreaterThan(0);
            }

            const res = await commit(registry, kind, format, blobId, "override");
            expect(res.ok, `import.commit failed: ${res.ok ? "" : res.error}`).toBe(true);

            // Imports are transform-and-discard, so the client releases — and the release must work
            // for every format, including the two that read through a path.
            expect(registry.invoke("blob.release", { blobId }, ctx)).toEqual({ ok: true });
        });
    }
});

// ── Collision strategies ──────────────────────────────────────────────────────

describe("collision strategy over the wire", () => {
    /**
     * The protocol defect this pins down: `ImportCommitParams.collisionStrategy` was frozen as
     * `["skip", "overwrite", "rename"]` while every real call site has always used
     * `["keep", "override", "new"]`. As written, the schema rejected every real import outright.
     * See `plan/protocol-changes.md`.
     */
    it("accepts the strategies the engine actually implements", async () => {
        for (const strategy of ["keep", "override", "new"] as const) {
            const { registry, blobId } = await stageMockExport();
            const res = await commit(registry, "mocks", "mocks-json", blobId, strategy);
            expect(res.ok, `"${strategy}" was rejected: ${res.ok ? "" : res.error}`).toBe(true);
        }
    });

    it("rejects the retired strategy names at the schema, before the handler runs", async () => {
        for (const retired of ["skip", "overwrite", "rename"]) {
            const { registry, blobId } = await stageMockExport();
            // `CommandRegistry.invoke()` validates against the frozen schema, so this throws rather
            // than reaching `importCommit`. Leaving the old names accepted would mean two
            // vocabularies for one concept — and the engine only ever understood one of them.
            expect(() =>
                registry.invoke(
                    "import.commit",
                    { kind: "mocks", format: "mocks-json", workspaceId: TEST_WS, blobId, collisionStrategy: retired },
                    ctx,
                ),
            ).toThrow(/Invalid payload/);
        }
    });

    it("defaults to keep, the non-destructive choice", async () => {
        const { registry, blobId, ws } = await stageMockExport();
        // Overwrite the source entity on disk with a same-id, different-content one. Same fixture —
        // see the header, point 2 — so the incoming entity now collides with a pre-existing one.
        seedEntities(ws, "mocks", [{ ...MOCK, name: "PRE-EXISTING", responseStatus: 500 }]);

        const res = await commit(registry, "mocks", "mocks-json", blobId);
        expect(res.ok, res.ok ? "" : res.error).toBe(true);
        if (!res.ok) return;
        expect(res.imported).toBe(0);
        expect(res.skipped).toBe(1);

        const mocks = readAllEntities<any>(TEST_WS, "mocks");
        expect(mocks).toHaveLength(1);
        expect(mocks[0].name).toBe("PRE-EXISTING");
        expect(mocks[0].responseStatus).toBe(500);
    });

    it("override replaces the colliding entity", async () => {
        const { registry, blobId, ws } = await stageMockExport();
        seedEntities(ws, "mocks", [{ ...MOCK, name: "PRE-EXISTING" }]);

        expect((await commit(registry, "mocks", "mocks-json", blobId, "override")).ok).toBe(true);

        const mocks = readAllEntities<any>(TEST_WS, "mocks");
        expect(mocks).toHaveLength(1);
        expect(mocks[0].name).toBe("Blob Mock");
        expect(mocks[0].responseStatus).toBe(201);
    });

    it("new keeps both, under a freshly generated id", async () => {
        const { registry, blobId, ws } = await stageMockExport();
        seedEntities(ws, "mocks", [{ ...MOCK, name: "PRE-EXISTING" }]);

        expect((await commit(registry, "mocks", "mocks-json", blobId, "new")).ok).toBe(true);

        const mocks = readAllEntities<any>(TEST_WS, "mocks");
        expect(mocks).toHaveLength(2);
        expect(mocks.map((m) => m.name).sort()).toEqual(["Blob Mock", "PRE-EXISTING"]);
        expect(new Set(mocks.map((m) => m.id)).size).toBe(2);
    });
});

// ── The protocol schema is the gate ───────────────────────────────────────────

describe("the protocol schema gates every payload", () => {
    it("refuses ingress without a blobId", () => {
        const registry = newRegistry();
        newWorkspace();

        // `blobId` is required on the ingress params since P3. It was optional only because the
        // pre-P3 handlers took a path instead, and the acceptance criterion for work item 6 is
        // literally "no `filePath` in any protocol command or result type".
        for (const command of ["import.preflight", "import.commit"] as const) {
            expect(() =>
                registry.invoke(command, { kind: "mocks", format: "mocks-json", workspaceId: TEST_WS }, ctx),
            ).toThrow(/Invalid payload/);
        }
    });

    it("refuses a filePath, which is no longer part of any ingress payload", () => {
        const registry = newRegistry();
        newWorkspace();
        // The schemas are `.strict()`, so a stale client that still sends `filePath` is rejected
        // rather than silently ignored.
        expect(() =>
            registry.invoke(
                "import.preflight",
                { kind: "mocks", format: "mocks-json", workspaceId: TEST_WS, filePath: "/tmp/x.json" },
                ctx,
            ),
        ).toThrow(/Invalid payload/);
    });

    it("registers exactly the four export/import commands, all four owned by the engine", () => {
        // The shell used to register `export.formats` itself; it moved into
        // `registerImportExportCommands` so all four have one owner and one call site.
        const registry = newRegistry();
        const owned = registry.list().filter((c) => c.startsWith("export.") || c.startsWith("import.")).sort();
        expect(owned).toEqual(["export.create", "export.formats", "import.commit", "import.preflight"]);
    });

    it("refuses a double registration, so two implementations cannot compete for one wire name", () => {
        const registry = newRegistry();
        expect(() => registerImportExportCommands(registry)).toThrow(/already registered/);
        expect(() => registerBlobCommands(registry)).toThrow(/already registered/);
    });

    it("serves export.formats as pure data", () => {
        const registry = newRegistry();
        const formats = registry.invoke("export.formats", {}, ctx) as Record<string, { id: string }[]>;
        expect(Object.keys(formats)).toHaveLength(8);
        expect(formats.mocks.map((f) => f.id)).toContain("mocks-json");
    });
});
