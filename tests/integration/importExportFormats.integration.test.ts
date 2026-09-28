/**
 * Round-trip tests for the REMAINING import/export formats — the ones that speak
 * someone else's schema (Postman, WireMock, HAR, Insomnia, OpenAPI) plus the
 * whole-workspace snapshot.
 *
 * These complement `importExport.integration.test.ts` (which covers the native
 * Bifurc-JSON formats and the collision strategies). The formats here mostly mint
 * fresh ids on import, so they are verified by CONTENT rather than by id.
 *
 * Runs in the integration project (shared process-global data-root override ⇒
 * `fileParallelism: false`).
 */

import { describe, it, expect, afterAll } from "vitest";
import * as fs from "fs";
import * as os from "os";
import * as path from "path";
import { createWorkspace, TEST_WS, type WorkspaceFixture } from "./proxyHarness";
import { getExporter, getImporter } from "@bifurc/engine/importExport/registry";
import { exportToFile, importFile, preflightFile } from "./importExportHarness";
import { loadConfig } from "@bifurc/engine/store/config";
import { readAllEntities } from "@bifurc/engine/store/workspaceFs";

// ── Fixtures ──────────────────────────────────────────────────────────────────

const MOCK = {
    id: "mock-fmt", name: "Fmt Mock", method: "POST", urlPattern: "/api/fmt", useRegex: false, enabled: true,
    capturedHeaders: {}, capturedBody: "",
    responseStatus: 202, responseHeaders: { "content-type": "application/json" },
    responseBody: '{"fmt":true}', responseDelay: 0, createdAt: 1, folderId: null, workspaceId: TEST_WS,
};

const MOCK_REGEX = { ...MOCK, id: "mock-regex", name: "Regex Mock", useRegex: true, urlPattern: "^/api/\\d+$", responseDelay: 250 };

const REQUEST = {
    id: "req-fmt", name: "Fmt Request", method: "POST", url: "http://api.fmt/items",
    headers: { "content-type": "application/json", "x-tag": "t1" },
    body: '{"n":1}', createdAt: 1704067200000, folderId: null, workspaceId: TEST_WS,
};

const MAPPING = {
    id: "map-fmt", domain: "fmt.localhost", target: "127.0.0.1:7001", enabled: true, label: "Fmt", createdAt: 1, workspaceId: TEST_WS,
};

const RULE = {
    id: "rule-fmt", name: "Fmt Rule", pattern: "http://api.fmt/*", useRegex: true,
    targetType: "external", targetMappingId: "", targetExternal: "127.0.0.1:7002",
    requestScript: "lp.request.headers['x-r'] = '1';",
    responseScript: "lp.response.body = 'rw';",
    enabled: true, createdAt: 1, folderId: null, workspaceId: TEST_WS,
};

const SOCKET = {
    id: "sock-fmt", name: "Fmt Socket", url: "ws://api.fmt/ws", headers: {}, createdAt: 1, folderId: null, workspaceId: TEST_WS,
};

const WEBHOOK = {
    id: "hook-fmt", name: "Fmt Hook", urlSuffix: "fmt-hook", createdAt: 1, folderId: null, workspaceId: TEST_WS,
};

const ENVIRONMENT = {
    id: "env-fmt", name: "Fmt Env", createdAt: 1, workspaceId: TEST_WS,
    variables: [{ id: "v1", key: "K", value: "V" }],
};

// ── Harness bookkeeping ───────────────────────────────────────────────────────

const fixtures: WorkspaceFixture[] = [];
const outDir = fs.mkdtempSync(path.join(os.tmpdir(), "bifurc-fmt-"));

function newWorkspace(): WorkspaceFixture {
    const ws = createWorkspace();
    fixtures.push(ws);
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

/** Source workspace containing one of every entity kind. */
function seedAll(ws: WorkspaceFixture): void {
    seedEntities(ws, "mocks", [MOCK]);
    seedEntities(ws, "mappings", [MAPPING]);
    seedEntities(ws, "rules", [RULE]);
    seedEntities(ws, "environments", [ENVIRONMENT]);
    seedEntities(ws, "requests", [REQUEST]);
    seedEntities(ws, "sockets", [SOCKET]);
    seedEntities(ws, "webhooks", [WEBHOOK]);
}

afterAll(() => {
    for (const f of fixtures) f.cleanup();
    try { fs.rmSync(outDir, { recursive: true, force: true }); } catch { /* best effort */ }
});

// ── Foreign-format round trips ────────────────────────────────────────────────

describe("WireMock round trip", () => {
    it("preserves method, url pattern, regex flag, status and body", async () => {
        const src = newWorkspace();
        seedEntities(src, "mocks", [MOCK]);

        const file = path.join(outDir, "mocks-wiremock.json");
        expect((await exportToFile(TEST_WS, getExporter("mocks", "mocks-wiremock")!, file)).ok).toBe(true);

        // WireMock keeps the original id, so it can be located directly.
        const pf = preflightFile(getImporter("mocks", "mocks-wiremock")!, TEST_WS, file);
        expect(pf.ok, pf.error).toBe(true);
        expect(pf.itemCount).toBe(1);
        expect(pf.collisionIds).toContain(MOCK.id);

        newWorkspace();
        const res = await importFile(getImporter("mocks", "mocks-wiremock")!, TEST_WS, file, "override");
        expect(res.ok, res.error).toBe(true);

        const m = readAllEntities<any>(TEST_WS, "mocks").find((x) => x.id === MOCK.id);
        expect(m).toBeDefined();
        expect(m.name).toBe("Fmt Mock");
        expect(m.method).toBe("POST");
        expect(m.urlPattern).toBe("/api/fmt");
        expect(m.useRegex).toBe(false);
        expect(m.responseStatus).toBe(202);
        expect(m.responseBody).toBe('{"fmt":true}');
        // WireMock import deliberately lands mocks disabled so a bulk import cannot
        // silently change what the proxy serves. The flag lives in enabled.json
        // (entity files strip `enabled`), so read it through loadConfig().
        expect(loadConfig().mocks.find((x) => x.id === MOCK.id)!.enabled).toBe(false);
    });

    it("maps a regex matcher to urlPattern and keeps the fixed delay", async () => {
        const src = newWorkspace();
        seedEntities(src, "mocks", [MOCK_REGEX]);

        const file = path.join(outDir, "mocks-wiremock-regex.json");
        expect((await exportToFile(TEST_WS, getExporter("mocks", "mocks-wiremock")!, file)).ok).toBe(true);
        newWorkspace();
        expect((await importFile(getImporter("mocks", "mocks-wiremock")!, TEST_WS, file, "override")).ok).toBe(true);

        const m = readAllEntities<any>(TEST_WS, "mocks").find((x) => x.id === MOCK_REGEX.id);
        expect(m.useRegex).toBe(true);
        expect(m.urlPattern).toBe("^/api/\\d+$");
        expect(m.responseDelay).toBe(250);
    });

    it("maps a wildcard method to ANY and back", async () => {
        const src = newWorkspace();
        seedEntities(src, "mocks", [{ ...MOCK, id: "mock-any", method: "*" }]);

        const file = path.join(outDir, "mocks-wiremock-any.json");
        expect((await exportToFile(TEST_WS, getExporter("mocks", "mocks-wiremock")!, file)).ok).toBe(true);
        const written = JSON.parse(fs.readFileSync(file, "utf-8"));
        expect(written.mappings[0].request.method).toBe("ANY");

        newWorkspace();
        expect((await importFile(getImporter("mocks", "mocks-wiremock")!, TEST_WS, file, "override")).ok).toBe(true);
        const m = readAllEntities<any>(TEST_WS, "mocks").find((x) => x.id === "mock-any");
        expect(m.method).toBe("*");
    });

    it("accepts a bare array of stubs as well as { mappings: [...] }", async () => {
        const file = path.join(outDir, "wiremock-array.json");
        fs.writeFileSync(file, JSON.stringify([
            { id: "bare-1", name: "Bare", request: { method: "GET", url: "/bare" }, response: { status: 200, body: "b" } },
        ]), "utf-8");

        newWorkspace();
        const pf = preflightFile(getImporter("mocks", "mocks-wiremock")!, TEST_WS, file);
        expect(pf.ok, pf.error).toBe(true);
        expect(pf.itemCount).toBe(1);

        const res = await importFile(getImporter("mocks", "mocks-wiremock")!, TEST_WS, file, "override");
        expect(res.ok, res.error).toBe(true);
        const m = readAllEntities<any>(TEST_WS, "mocks").find((x) => x.id === "bare-1");
        expect(m.name).toBe("Bare");
        expect(m.method).toBe("GET");
        expect(m.urlPattern).toBe("/bare");
    });
});

describe("Postman round trip", () => {
    it("preserves mocks", async () => {
        const src = newWorkspace();
        seedEntities(src, "mocks", [MOCK]);

        const file = path.join(outDir, "mocks-postman.json");
        expect((await exportToFile(TEST_WS, getExporter("mocks", "mocks-postman")!, file)).ok).toBe(true);
        const pf = preflightFile(getImporter("mocks", "mocks-postman")!, TEST_WS, file);
        expect(pf.ok, pf.error).toBe(true);
        expect(pf.itemCount).toBe(1);

        newWorkspace();
        const res = await importFile(getImporter("mocks", "mocks-postman")!, TEST_WS, file, "override");
        expect(res.ok, res.error).toBe(true);
        expect(res.imported).toBe(1);

        const mocks = readAllEntities<any>(TEST_WS, "mocks");
        expect(mocks).toHaveLength(1);
        expect(mocks[0].name).toBe("Fmt Mock");
        expect(mocks[0].method).toBe("POST");
        expect(mocks[0].urlPattern).toBe("/api/fmt");
        expect(mocks[0].responseStatus).toBe(202);
        expect(mocks[0].responseBody).toBe('{"fmt":true}');
    });

    it("preserves requests", async () => {
        const src = newWorkspace();
        seedEntities(src, "requests", [REQUEST]);

        const file = path.join(outDir, "requests-postman.json");
        expect((await exportToFile(TEST_WS, getExporter("requests", "requests-postman")!, file)).ok).toBe(true);
        const pf = preflightFile(getImporter("requests", "requests-postman")!, TEST_WS, file);
        expect(pf.ok, pf.error).toBe(true);
        expect(pf.itemCount).toBe(1);

        newWorkspace();
        const res = await importFile(getImporter("requests", "requests-postman")!, TEST_WS, file, "override");
        expect(res.ok, res.error).toBe(true);

        const reqs = readAllEntities<any>(TEST_WS, "requests");
        expect(reqs).toHaveLength(1);
        expect(reqs[0].name).toBe("Fmt Request");
        expect(reqs[0].method).toBe("POST");
        expect(reqs[0].url).toBe("http://api.fmt/items");
        expect(JSON.parse(reqs[0].body)).toEqual({ n: 1 });
        expect(reqs[0].headers["x-tag"]).toBe("t1");
    });
});

describe("HAR round trip", () => {
    it("preserves method, url, body and non-skipped headers", async () => {
        const src = newWorkspace();
        seedEntities(src, "requests", [REQUEST]);

        const file = path.join(outDir, "requests.har");
        expect((await exportToFile(TEST_WS, getExporter("requests", "requests-har")!, file)).ok).toBe(true);
        const pf = preflightFile(getImporter("requests", "requests-har")!, TEST_WS, file);
        expect(pf.ok, pf.error).toBe(true);
        expect(pf.itemCount).toBe(1);

        newWorkspace();
        const res = await importFile(getImporter("requests", "requests-har")!, TEST_WS, file, "override");
        expect(res.ok, res.error).toBe(true);

        const reqs = readAllEntities<any>(TEST_WS, "requests");
        expect(reqs).toHaveLength(1);
        expect(reqs[0].method).toBe("POST");
        expect(reqs[0].url).toBe("http://api.fmt/items");
        expect(JSON.parse(reqs[0].body)).toEqual({ n: 1 });
        expect(reqs[0].headers["x-tag"]).toBe("t1");
        expect(reqs[0].headers["content-type"]).toBe("application/json");
        // HAR has no concept of a saved name, so it is derived from the request.
        expect(reqs[0].name).toBe("POST http://api.fmt/items");
    });

    it("writes a valid HAR 1.2 log", async () => {
        const src = newWorkspace();
        seedEntities(src, "requests", [REQUEST]);

        const file = path.join(outDir, "requests-shape.har");
        expect((await exportToFile(TEST_WS, getExporter("requests", "requests-har")!, file)).ok).toBe(true);

        const har = JSON.parse(fs.readFileSync(file, "utf-8"));
        expect(har.log.version).toBe("1.2");
        expect(har.log.creator.name).toBe("Bifurc");
        expect(har.log.entries).toHaveLength(1);
        expect(har.log.entries[0].request.method).toBe("POST");
        expect(har.log.entries[0].request.postData.text).toBe('{"n":1}');
    });

    it("rejects a JSON file that is not a HAR", () => {
        const file = path.join(outDir, "not-a-har.json");
        fs.writeFileSync(file, JSON.stringify({ hello: "world" }), "utf-8");
        newWorkspace();
        const pf = preflightFile(getImporter("requests", "requests-har")!, TEST_WS, file);
        expect(pf.ok).toBe(false);
        expect(pf.error).toBeTruthy();
    });
});

describe("Insomnia round trip", () => {
    it("preserves name, method, url, headers and body", async () => {
        const src = newWorkspace();
        seedEntities(src, "requests", [REQUEST]);

        const file = path.join(outDir, "requests-insomnia.json");
        expect((await exportToFile(TEST_WS, getExporter("requests", "requests-insomnia")!, file)).ok).toBe(true);

        const written = JSON.parse(fs.readFileSync(file, "utf-8"));
        expect(written.__export_format).toBe(4);
        expect(written.resources.some((r: any) => r._type === "request")).toBe(true);

        const pf = preflightFile(getImporter("requests", "requests-insomnia")!, TEST_WS, file);
        expect(pf.ok, pf.error).toBe(true);
        expect(pf.itemCount).toBe(1);

        newWorkspace();
        const res = await importFile(getImporter("requests", "requests-insomnia")!, TEST_WS, file, "override");
        expect(res.ok, res.error).toBe(true);

        const reqs = readAllEntities<any>(TEST_WS, "requests");
        expect(reqs).toHaveLength(1);
        expect(reqs[0].name).toBe("Fmt Request");
        expect(reqs[0].method).toBe("POST");
        expect(reqs[0].url).toBe("http://api.fmt/items");
        expect(JSON.parse(reqs[0].body)).toEqual({ n: 1 });
        expect(reqs[0].headers["x-tag"]).toBe("t1");
    });

    it("rejects a file with no resources array", () => {
        const file = path.join(outDir, "not-insomnia.json");
        fs.writeFileSync(file, JSON.stringify({ __export_format: 4 }), "utf-8");
        newWorkspace();
        const pf = preflightFile(getImporter("requests", "requests-insomnia")!, TEST_WS, file);
        expect(pf.ok).toBe(false);
    });
});

describe("OpenAPI round trip", () => {
    it("produces a valid 3.0.3 spec and re-imports the operation", async () => {
        const src = newWorkspace();
        seedEntities(src, "requests", [REQUEST]);

        const file = path.join(outDir, "requests-openapi.json");
        expect((await exportToFile(TEST_WS, getExporter("requests", "requests-openapi")!, file)).ok).toBe(true);

        const spec = JSON.parse(fs.readFileSync(file, "utf-8"));
        expect(spec.openapi).toBe("3.0.3");
        expect(Object.keys(spec.paths)).toEqual(["/items"]);
        expect(spec.paths["/items"].post.summary).toBe("Fmt Request");

        const pf = preflightFile(getImporter("requests", "requests-openapi")!, TEST_WS, file);
        expect(pf.ok, pf.error).toBe(true);
        expect(pf.itemCount).toBe(1);

        newWorkspace();
        const res = await importFile(getImporter("requests", "requests-openapi")!, TEST_WS, file, "override");
        expect(res.ok, res.error).toBe(true);
        expect(res.imported).toBe(1);

        const reqs = readAllEntities<any>(TEST_WS, "requests");
        expect(reqs).toHaveLength(1);
        expect(reqs[0].method).toBe("POST");
        expect(reqs[0].url).toBe("/items"); // no `servers` entry ⇒ no base URL
        expect(reqs[0].name).toBe("Fmt Request");
        expect(JSON.parse(reqs[0].body)).toEqual({ n: 1 });
    });

    it("rejects a JSON file that is neither openapi nor swagger", () => {
        const file = path.join(outDir, "not-openapi.json");
        fs.writeFileSync(file, JSON.stringify({ paths: {} }), "utf-8");
        newWorkspace();
        const pf = preflightFile(getImporter("requests", "requests-openapi")!, TEST_WS, file);
        expect(pf.ok).toBe(false);
        expect(pf.error).toContain("OpenAPI");
    });
});

// ── Whole-workspace snapshot ──────────────────────────────────────────────────

describe("workspace snapshot round trip", () => {
    it("carries every entity kind into a freshly created workspace", async () => {
        const src = newWorkspace();
        seedAll(src);

        const file = path.join(outDir, "workspace.json");
        const exp = await exportToFile(TEST_WS, getExporter("workspace", "workspace-json")!, file);
        expect(exp.ok, exp.error).toBe(true);

        const snapshot = JSON.parse(fs.readFileSync(file, "utf-8"));
        expect(snapshot.schema).toBe("lp-workspace-v1");
        expect(snapshot.data.mocks).toHaveLength(1);
        expect(snapshot.data.mappings).toHaveLength(1);
        expect(snapshot.data.requests).toHaveLength(1);
        expect(snapshot.data.wsConnections).toHaveLength(1);
        expect(snapshot.data.webhooks).toHaveLength(1);
        // The snapshot must carry the FULL rule, not the UI stub.
        expect(snapshot.data.proxyRules).toHaveLength(1);
        expect(snapshot.data.proxyRules[0].targetExternal).toBe("127.0.0.1:7002");
        expect(snapshot.data.proxyRules[0].requestScript).toBe(RULE.requestScript);

        const importer = getImporter("workspace", "workspace-json")!;
        const pf = preflightFile(importer, TEST_WS, file);
        expect(pf.ok, pf.error).toBe(true);
        // 1 mock + 1 mapping + 1 rule + 1 request + 1 socket + 1 webhook + 2 envs
        // (the implicit `__global__` env is included in the snapshot).
        expect(pf.itemCount).toBe(8);

        newWorkspace();
        const res = await importFile(importer, TEST_WS, file, "override");
        expect(res.ok, res.error).toBe(true);
        expect(res.imported).toBe(8);

        // The import switches the active workspace to the newly created one.
        const newWsId = loadConfig().activeWorkspaceId;
        expect(newWsId).not.toBe(TEST_WS);

        const mocks = readAllEntities<any>(newWsId, "mocks");
        expect(mocks).toHaveLength(1);
        expect(mocks[0].name).toBe("Fmt Mock");
        expect(mocks[0].workspaceId).toBe(newWsId);

        const mappings = readAllEntities<any>(newWsId, "mappings");
        expect(mappings).toHaveLength(1);
        expect(mappings[0].domain).toBe("fmt.localhost");
        expect(mappings[0].target).toBe("127.0.0.1:7001");

        // Rules must survive on disk — this is the regression that the explicit
        // write loop in the importer guards against.
        const rules = readAllEntities<any>(newWsId, "rules");
        expect(rules).toHaveLength(1);
        expect(rules[0].name).toBe("Fmt Rule");
        expect(rules[0].pattern).toBe("http://api.fmt/*");
        expect(rules[0].useRegex).toBe(true);
        expect(rules[0].targetExternal).toBe("127.0.0.1:7002");
        expect(rules[0].requestScript).toBe(RULE.requestScript);
        expect(rules[0].responseScript).toBe(RULE.responseScript);

        // Enabled state is carried through enabled.json for all three kinds.
        const cfg = loadConfig();
        expect(cfg.proxyRules.find((r) => r.id === RULE.id)!.enabled).toBe(true);
        expect(cfg.mappings.find((m) => m.id === MAPPING.id)!.enabled).toBe(true);
        expect(cfg.mocks.find((m) => m.id === MOCK.id)!.enabled).toBe(true);

        const requests = readAllEntities<any>(newWsId, "requests");
        expect(requests).toHaveLength(1);
        expect(requests[0].url).toBe("http://api.fmt/items");

        const sockets = readAllEntities<any>(newWsId, "sockets");
        expect(sockets).toHaveLength(1);
        expect(sockets[0].url).toBe("ws://api.fmt/ws");

        const webhooks = readAllEntities<any>(newWsId, "webhooks");
        expect(webhooks).toHaveLength(1);
        expect(webhooks[0].urlSuffix).toBe("fmt-hook");

        const envs = readAllEntities<any>(newWsId, "environments").filter((e) => e.id !== "__global__");
        expect(envs).toHaveLength(1);
        expect(envs[0].variables[0]).toEqual({ id: "v1", key: "K", value: "V" });
    });

    it("keeps disabled mappings and mocks disabled after a snapshot import", async () => {
        // Regression guard: `bootstrapEnabledSet` treats a missing `enabled` flag as
        // ENABLED, and the entity files strip the flag — so without an explicit
        // enabled-set write a deliberately-disabled mapping would come back enabled
        // and start proxying traffic again.
        const src = newWorkspace();
        seedEntities(src, "mappings", [{ ...MAPPING, id: "map-off", enabled: false }]);
        seedEntities(src, "mocks", [{ ...MOCK, id: "mock-off", enabled: false }]);
        seedEntities(src, "rules", [{ ...RULE, id: "rule-off", enabled: false }]);

        const file = path.join(outDir, "workspace-disabled.json");
        expect((await exportToFile(TEST_WS, getExporter("workspace", "workspace-json")!, file)).ok).toBe(true);

        newWorkspace();
        const importer = getImporter("workspace", "workspace-json")!;
        expect((await importFile(importer, TEST_WS, file, "override")).ok).toBe(true);

        const cfg = loadConfig();
        expect(cfg.mappings.find((m) => m.id === "map-off")!.enabled).toBe(false);
        expect(cfg.mocks.find((m) => m.id === "mock-off")!.enabled).toBe(false);
        expect(cfg.proxyRules.find((r) => r.id === "rule-off")!.enabled).toBe(false);
    });

    it("does not collide with an existing workspace of the same name", async () => {
        const src = newWorkspace();
        seedAll(src);
        const file = path.join(outDir, "workspace-name.json");
        expect((await exportToFile(TEST_WS, getExporter("workspace", "workspace-json")!, file)).ok).toBe(true);

        newWorkspace();
        const importer = getImporter("workspace", "workspace-json")!;
        expect((await importFile(importer, TEST_WS, file, "override")).ok).toBe(true);

        // Import again into the same install — the second workspace must be renamed.
        const second = await importFile(importer, TEST_WS, file, "override");
        expect(second.ok, second.error).toBe(true);

        const names = loadConfig().workspaces.map((w) => w.name);
        expect(names).toContain("Integration Workspace");
        expect(names.some((n) => n.includes("(2)"))).toBe(true);
    });

    it("rejects a file that is not a workspace snapshot", () => {
        const file = path.join(outDir, "not-workspace.json");
        fs.writeFileSync(file, JSON.stringify({ schema: "lp-mocks-v1" }), "utf-8");
        newWorkspace();
        const pf = preflightFile(getImporter("workspace", "workspace-json")!, TEST_WS, file);
        expect(pf.ok).toBe(false);
        expect(pf.error).toContain("lp-workspace-v1");
    });
});
