/**
 * Export → import round-trip tests for the Import/Export feature.
 *
 * WHY THIS EXISTS
 * ---------------
 * `tests/ipc/importExport/postman.test.ts` covers the pure Postman codec, but the
 * 18 importer/exporter *wrappers* (`run()` / `preflight()`) were completely
 * untested. Those wrappers are where the real risk lives: an exporter writes one
 * schema and the matching importer reads another, so a field rename on either side
 * silently drops user data. Nothing type-checks that pair — only an actual
 * write-to-disk → read-back-from-disk round trip does.
 *
 * Each case below therefore:
 *   1. builds a REAL workspace on disk with one entity,
 *   2. runs the REAL exporter to a real file,
 *   3. runs preflight and asserts it sees the entity as a collision,
 *   4. builds a SECOND, empty workspace,
 *   5. runs the REAL importer against the exported file,
 *   6. reads the entity back off disk and asserts the user-visible fields survived.
 *
 * Runs in the integration project (shared process-global data-root override ⇒
 * `fileParallelism: false`).
 */

import { describe, it, expect, afterAll } from "vitest";
import * as fs from "fs";
import * as os from "os";
import * as path from "path";
import { createWorkspace, TEST_WS, type WorkspaceFixture } from "./proxyHarness";
import { getExporter, getImporter } from "@/ipc/importExport/registry";
import { loadConfig } from "@bifurc/engine/store/config";
import { readAllEntities } from "@bifurc/engine/store/workspaceFs";
import type { EntityKind, CollisionStrategy } from "@/ipc/importExport/types";

// ── Fixtures ──────────────────────────────────────────────────────────────────
// `workspaceId` is set on every fixture because that is what the real CRUD
// handlers persist, and several exporters filter on it.

const MOCK = {
    id: "mock-rt", name: "RT Mock", method: "POST", urlPattern: "/api/rt", useRegex: true, enabled: true,
    capturedHeaders: { "x-a": "1" }, capturedBody: "cap",
    responseStatus: 201, responseHeaders: { "content-type": "application/json" },
    responseBody: '{"rt":true}', responseDelay: 0, createdAt: 1, folderId: null, workspaceId: TEST_WS,
};

const MAPPING = {
    id: "map-rt", domain: "rt.localhost", target: "127.0.0.1:9999", enabled: true, label: "RT",
    createdAt: 1, workspaceId: TEST_WS,
};

const RULE = {
    id: "rule-rt", name: "RT Rule", pattern: "http://api.rt/data", useRegex: false,
    targetType: "external", targetMappingId: "", targetExternal: "127.0.0.1:9998",
    requestScript: "lp.request.headers['x-script'] = '1';",
    responseScript: "lp.response.body = 'rewritten';",
    enabled: true, createdAt: 1, folderId: null, workspaceId: TEST_WS,
};

const ENVIRONMENT = {
    id: "env-rt", name: "RT Env", createdAt: 1, workspaceId: TEST_WS,
    variables: [{ id: "v1", key: "TOKEN", value: "abc123" }, { id: "v2", key: "BASE", value: "http://x" }],
};

const REQUEST = {
    id: "req-rt", name: "RT Request", method: "POST", url: "http://api.rt/submit",
    headers: { "content-type": "application/json", "x-keep": "yes" },
    body: '{"a":1}', createdAt: 1, folderId: null, workspaceId: TEST_WS,
};

const SOCKET = {
    id: "sock-rt", name: "RT Socket", url: "ws://api.rt/ws", headers: { "x-a": "1" },
    createdAt: 1, folderId: null, workspaceId: TEST_WS,
};

const WEBHOOK = {
    id: "hook-rt", name: "RT Hook", urlSuffix: "rt-hook", createdAt: 1, folderId: null, workspaceId: TEST_WS,
};

// ── Harness bookkeeping ───────────────────────────────────────────────────────

const fixtures: WorkspaceFixture[] = [];
const outDir = fs.mkdtempSync(path.join(os.tmpdir(), "bifurc-io-"));

function newWorkspace(seed?: (ws: WorkspaceFixture) => void): WorkspaceFixture {
    const ws = createWorkspace();
    fixtures.push(ws);
    seed?.(ws);
    return ws;
}

/**
 * Write raw entity files for kinds `createWorkspace` does not know about.
 * Mirrors the real store: `enabled` is stripped from the entity file and tracked in
 * that kind's `enabled.json` instead, so the helper maintains both.
 */
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

afterAll(() => {
    for (const f of fixtures) f.cleanup();
    try { fs.rmSync(outDir, { recursive: true, force: true }); } catch { /* best effort */ }
});

// ── Case table ────────────────────────────────────────────────────────────────

interface Case {
    label: string;
    kind: EntityKind;
    format: string;
    /** Directory under the workspace where the entity lives on disk. */
    dir: string;
    id: string;
    seed: (ws: WorkspaceFixture) => void;
    /** User-visible fields that MUST survive the round trip. */
    expected: Record<string, unknown>;
    pick: (e: any) => Record<string, unknown>;
    /** Defaults to 1. Environments export the implicit `__global__` env as well. */
    itemCount?: number;
}

const pick = (e: any, keys: string[]) => Object.fromEntries(keys.map((k) => [k, e[k]]));

const CASES: Case[] = [
    {
        label: "mocks (Bifurc JSON)",
        kind: "mocks", format: "mocks-json", dir: "mocks", id: MOCK.id,
        seed: (ws) => seedEntities(ws, "mocks", [MOCK]),
        pick: (e) => pick(e, ["name", "method", "urlPattern", "useRegex", "responseStatus", "responseBody", "responseHeaders"]),
        expected: {
            name: "RT Mock", method: "POST", urlPattern: "/api/rt", useRegex: true,
            responseStatus: 201, responseBody: '{"rt":true}', responseHeaders: { "content-type": "application/json" },
        },
    },
    {
        label: "mappings (Bifurc JSON)",
        kind: "mappings", format: "mappings-json", dir: "mappings", id: MAPPING.id,
        seed: (ws) => seedEntities(ws, "mappings", [MAPPING]),
        pick: (e) => pick(e, ["domain", "target", "label"]),
        expected: { domain: "rt.localhost", target: "127.0.0.1:9999", label: "RT" },
    },
    {
        label: "proxy rules (Bifurc JSON)",
        kind: "proxyRules", format: "proxyrules-json", dir: "rules", id: RULE.id,
        seed: (ws) => seedEntities(ws, "rules", [RULE]),
        pick: (e) => pick(e, ["name", "pattern", "targetType", "targetExternal", "requestScript", "responseScript"]),
        expected: {
            name: "RT Rule", pattern: "http://api.rt/data", targetType: "external",
            targetExternal: "127.0.0.1:9998",
            requestScript: "lp.request.headers['x-script'] = '1';",
            responseScript: "lp.response.body = 'rewritten';",
        },
    },
    {
        label: "environments (Bifurc JSON)",
        kind: "environments", format: "environments-json", dir: "environments", id: ENVIRONMENT.id,
        seed: (ws) => seedEntities(ws, "environments", [ENVIRONMENT]),
        pick: (e) => pick(e, ["name", "variables"]),
        expected: { name: "RT Env", variables: ENVIRONMENT.variables },
        // The export includes the implicit `__global__` environment that every
        // workspace gets, so the file holds 2 environments, not 1.
        itemCount: 2,
    },
    {
        label: "websockets (Bifurc JSON)",
        kind: "websockets", format: "websockets-json", dir: "sockets", id: SOCKET.id,
        seed: (ws) => seedEntities(ws, "sockets", [SOCKET]),
        pick: (e) => pick(e, ["name", "url", "headers"]),
        expected: { name: "RT Socket", url: "ws://api.rt/ws", headers: { "x-a": "1" } },
    },
    {
        label: "webhooks (Bifurc JSON)",
        kind: "webhooks", format: "webhooks-json", dir: "webhooks", id: WEBHOOK.id,
        seed: (ws) => seedEntities(ws, "webhooks", [WEBHOOK]),
        pick: (e) => pick(e, ["name", "urlSuffix"]),
        expected: { name: "RT Hook", urlSuffix: "rt-hook" },
    },
];

describe("import/export round trip", () => {
    for (const c of CASES) {
        it(`${c.label}: export then import preserves the entity`, async () => {
            // 1. source workspace with the entity
            newWorkspace(c.seed);

            const exporter = getExporter(c.kind, c.format);
            const importer = getImporter(c.kind, c.format);
            expect(exporter, `no exporter for ${c.kind}/${c.format}`).toBeDefined();
            expect(importer, `no importer for ${c.kind}/${c.format}`).toBeDefined();

            const file = path.join(outDir, `${c.kind}-${c.format}.out`);
            const exp = await exporter!.run(TEST_WS, file);
            expect(exp.ok, `export failed: ${exp.error}`).toBe(true);
            expect(fs.existsSync(file)).toBe(true);

            // 2. preflight must see the entity and report it as a collision
            const pf = importer!.preflight(TEST_WS, file);
            expect(pf.ok, `preflight failed: ${pf.error}`).toBe(true);
            expect(pf.itemCount).toBe(c.itemCount ?? 1);
            expect(pf.collisionIds).toContain(c.id);

            // 3. import into a brand-new, empty workspace
            newWorkspace();
            const res = await importer!.run(TEST_WS, file, "override");
            expect(res.ok, `import failed: ${res.error}`).toBe(true);
            // Nothing collides in a fresh workspace, so every item in the file lands.
            expect(res.imported).toBe(c.itemCount ?? 1);

            // 4. read it back OFF DISK — the file is what git sync and the proxy read
            const onDisk = readAllEntities<any>(TEST_WS, c.dir);
            const found = onDisk.find((e) => e.id === c.id);
            expect(found, `${c.kind} ${c.id} was not written to disk`).toBeDefined();
            expect(c.pick(found)).toEqual(c.expected);
        });
    }
});

// ── Enabled-state fidelity ────────────────────────────────────────────────────

describe("proxy rule enabled state", () => {
    it("survives a proxyrules-json round trip", async () => {
        // Rule files strip the `enabled` flag (it lives in enabled.json), so the
        // exporter has to re-inject it. Without that, every rule would land disabled
        // on import and the imported workspace would silently proxy nothing.
        newWorkspace((ws) => seedEntities(ws, "rules", [{ ...RULE, enabled: true }]));
        const file = path.join(outDir, "rule-enabled.json");
        expect((await getExporter("proxyRules", "proxyrules-json")!.run(TEST_WS, file)).ok).toBe(true);
        expect(JSON.parse(fs.readFileSync(file, "utf-8")).proxyRules[0].enabled).toBe(true);

        newWorkspace();
        expect((await getImporter("proxyRules", "proxyrules-json")!.run(TEST_WS, file, "override")).ok).toBe(true);
        expect(loadConfig().proxyRules.find((r) => r.id === RULE.id)!.enabled).toBe(true);
    });

    it("carries a disabled rule through as disabled", async () => {
        newWorkspace((ws) => seedEntities(ws, "rules", [{ ...RULE, enabled: false }]));
        const file = path.join(outDir, "rule-disabled.json");
        expect((await getExporter("proxyRules", "proxyrules-json")!.run(TEST_WS, file)).ok).toBe(true);
        expect(JSON.parse(fs.readFileSync(file, "utf-8")).proxyRules[0].enabled).toBe(false);

        newWorkspace();
        expect((await getImporter("proxyRules", "proxyrules-json")!.run(TEST_WS, file, "override")).ok).toBe(true);
        expect(loadConfig().proxyRules.find((r) => r.id === RULE.id)!.enabled).toBe(false);
    });
});

// ── Lossy / special formats ───────────────────────────────────────────────────

describe("cURL export/import", () => {    it("preserves method, url, body, headers and the request name", async () => {
        newWorkspace((ws) => seedEntities(ws, "requests", [REQUEST]));

        const exporter = getExporter("requests", "requests-curl")!;
        const importer = getImporter("requests", "requests-curl")!;
        const file = path.join(outDir, "requests.curl");

        expect((await exporter.run(TEST_WS, file)).ok).toBe(true);

        const pf = importer.preflight(TEST_WS, file);
        expect(pf.ok, pf.error).toBe(true);
        expect(pf.itemCount).toBe(1);

        newWorkspace();
        const res = await importer.run(TEST_WS, file, "override");
        expect(res.ok, res.error).toBe(true);
        expect(res.imported).toBe(1);

        const reqs = readAllEntities<any>(TEST_WS, "requests");
        expect(reqs).toHaveLength(1);
        const r = reqs[0];
        // cURL import always mints a fresh id — verify by content instead.
        expect(r.name).toBe("RT Request");
        expect(r.method).toBe("POST");
        expect(r.url).toBe("http://api.rt/submit");
        expect(JSON.parse(r.body)).toEqual({ a: 1 });
        expect(r.headers["content-type"]).toBe("application/json");
        expect(r.headers["x-keep"]).toBe("yes");
    });
});

describe("dotenv export/import", () => {
    it("preserves variable keys and values", async () => {
        newWorkspace((ws) => seedEntities(ws, "environments", [ENVIRONMENT]));

        const exporter = getExporter("environments", "environments-dotenv")!;
        const importer = getImporter("environments", "environments-dotenv")!;
        const file = path.join(outDir, "env.env");

        const exp = await exporter.run(TEST_WS, file);
        expect(exp.ok, exp.error).toBe(true);
        expect(fs.readFileSync(file, "utf-8")).toContain("TOKEN=abc123");

        const pf = importer.preflight(TEST_WS, file);
        expect(pf.ok, pf.error).toBe(true);
        expect(pf.itemCount).toBe(2);

        newWorkspace();
        const res = await importer.run(TEST_WS, file, "override");
        expect(res.ok, res.error).toBe(true);

        // dotenv flattens every environment into ONE file and one imported env —
        // lossy by design, so assert the variables rather than the env count.
        const envs = readAllEntities<any>(TEST_WS, "environments").filter((e) => e.id !== "__global__");
        expect(envs).toHaveLength(1);
        const vars = Object.fromEntries(envs[0].variables.map((v: any) => [v.key, v.value]));
        expect(vars).toEqual({ TOKEN: "abc123", BASE: "http://x" });
    });

    it("quotes values containing spaces and hash characters", async () => {
        newWorkspace((ws) =>
            seedEntities(ws, "environments", [
                { ...ENVIRONMENT, id: "env-quote", variables: [
                    { id: "q1", key: "SPACED", value: "a b" },
                    { id: "q2", key: "HASHED", value: "x#y" },
                    { id: "q3", key: "PLAIN", value: "plain" },
                ] },
            ]),
        );

        const exporter = getExporter("environments", "environments-dotenv")!;
        const importer = getImporter("environments", "environments-dotenv")!;
        const file = path.join(outDir, "env-quote.env");

        expect((await exporter.run(TEST_WS, file)).ok).toBe(true);
        const text = fs.readFileSync(file, "utf-8");
        expect(text).toContain('SPACED="a b"');
        expect(text).toContain('HASHED="x#y"');
        expect(text).toContain("PLAIN=plain");

        newWorkspace();
        expect((await importer.run(TEST_WS, file, "override")).ok).toBe(true);
        const env = readAllEntities<any>(TEST_WS, "environments").find((e) => e.id !== "__global__");
        const vars = Object.fromEntries(env.variables.map((v: any) => [v.key, v.value]));
        expect(vars).toEqual({ SPACED: "a b", HASHED: "x#y", PLAIN: "plain" });
    });
});

// ── Collision strategies ──────────────────────────────────────────────────────

describe("collision strategies", () => {
    async function exportMock(): Promise<string> {
        newWorkspace((ws) => seedEntities(ws, "mocks", [MOCK]));
        const exporter = getExporter("mocks", "mocks-json")!;
        const file = path.join(outDir, "collisions-mocks.json");
        expect((await exporter.run(TEST_WS, file)).ok).toBe(true);
        return file;
    }

    async function importInto(seedTarget: boolean, strategy: CollisionStrategy) {
        // Export FIRST: `exportMock` creates its own source workspace, and
        // `createWorkspace` swaps the process-global data-root override — so the
        // target workspace must be created last, immediately before the import.
        const file = await exportMock();
        newWorkspace((ws) => {
            if (seedTarget) {
                seedEntities(ws, "mocks", [{ ...MOCK, name: "PRE-EXISTING", responseStatus: 500 }]);
            }
        });
        const res = await getImporter("mocks", "mocks-json")!.run(TEST_WS, file, strategy);
        return { res, mocks: readAllEntities<any>(TEST_WS, "mocks") };
    }

    it("keep: skips a colliding entity and leaves the existing one untouched", async () => {
        const { res, mocks } = await importInto(true, "keep");
        expect(res.ok, res.error).toBe(true);
        expect(res.skipped).toBe(1);
        expect(res.imported).toBe(0);
        expect(mocks).toHaveLength(1);
        expect(mocks[0].name).toBe("PRE-EXISTING");
        expect(mocks[0].responseStatus).toBe(500);
    });

    it("override: replaces the colliding entity in place", async () => {
        const { res, mocks } = await importInto(true, "override");
        expect(res.ok, res.error).toBe(true);
        expect(res.imported).toBe(1);
        expect(mocks).toHaveLength(1);
        expect(mocks[0].name).toBe("RT Mock");
        expect(mocks[0].responseStatus).toBe(201);
    });

    it("new: keeps both, under a freshly generated id", async () => {
        const { res, mocks } = await importInto(true, "new");
        expect(res.ok, res.error).toBe(true);
        expect(res.imported).toBe(1);
        expect(mocks).toHaveLength(2);
        const names = mocks.map((m) => m.name).sort();
        expect(names).toEqual(["PRE-EXISTING", "RT Mock"]);
        expect(mocks.every((m) => typeof m.id === "string" && m.id.length > 0)).toBe(true);
        expect(new Set(mocks.map((m) => m.id)).size).toBe(2);
    });

    it("imports cleanly into an empty workspace with any strategy", async () => {
        const { res, mocks } = await importInto(false, "keep");
        expect(res.ok, res.error).toBe(true);
        expect(res.imported).toBe(1);
        expect(res.skipped).toBe(0);
        expect(mocks).toHaveLength(1);
    });
});

// ── Preflight failure paths ───────────────────────────────────────────────────

describe("preflight", () => {
    it("reports a schema mismatch instead of importing garbage", () => {
        const file = path.join(outDir, "not-bifurc.json");
        fs.writeFileSync(file, JSON.stringify({ schema: "something-else", mocks: [] }), "utf-8");
        newWorkspace();

        for (const [kind, format] of [
            ["mocks", "mocks-json"],
            ["mappings", "mappings-json"],
            ["proxyRules", "proxyrules-json"],
            ["environments", "environments-json"],
            ["websockets", "websockets-json"],
            ["webhooks", "webhooks-json"],
        ] as Array<[EntityKind, string]>) {
            const pf = getImporter(kind, format)!.preflight(TEST_WS, file);
            expect(pf.ok, `${kind}/${format} accepted a foreign schema`).toBe(false);
            expect(pf.error, `${kind}/${format} produced no error message`).toBeTruthy();
        }
    });

    it("reports an error for a missing file rather than throwing", () => {
        newWorkspace();
        const pf = getImporter("mocks", "mocks-json")!.preflight(TEST_WS, path.join(outDir, "does-not-exist.json"));
        expect(pf.ok).toBe(false);
        expect(pf.error).toBeTruthy();
    });

    it("reports zero collisions when the workspace is empty", async () => {
        newWorkspace((ws) => seedEntities(ws, "mocks", [MOCK]));
        const file = path.join(outDir, "empty-target.json");
        expect((await getExporter("mocks", "mocks-json")!.run(TEST_WS, file)).ok).toBe(true);

        newWorkspace();
        const pf = getImporter("mocks", "mocks-json")!.preflight(TEST_WS, file);
        expect(pf.ok).toBe(true);
        expect(pf.itemCount).toBe(1);
        expect(pf.collisionIds).toEqual([]);
    });
});
