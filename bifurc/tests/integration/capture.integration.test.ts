/**
 * Happy-path workflow test for **Capture** — the capture half of capture-and-replay.
 *
 * The Capture screen is fed by `logEmitter` (`src/proxy/logEmitter.ts`): the proxy builds a
 * `RequestLogEntry` for every request it serves — method/url/host/status, `via` (how it was
 * fulfilled), the target, the duration, and **base64 request/response bodies so the entry can
 * be replayed or turned into a mock**. Nothing asserted any of that before: the *replay* half
 * was covered, but the capture half was not, so a regression that dropped `reqBody` or broke
 * the 512 KB cap would have silently produced un-replayable captures.
 *
 * These drive REAL traffic through the REAL proxy (mapping / mock / rule / passthrough) and
 * assert the entries that come out of the emitter. Only Electron is mocked.
 *
 * Runs in the integration project (shared process-global data-root override ⇒
 * `fileParallelism: false`).
 */

import { describe, it, expect, beforeAll, afterAll, beforeEach, vi } from "vitest";
import * as fs from "fs";
import * as path from "path";
import * as os from "os";
import type { RequestLogEntry } from "@/proxy/logEmitter";
import { logEmitter } from "@/proxy/logEmitter";
import {
    createWorkspace,
    startUpstream,
    startProxy,
    proxyRequest,
    mappingFixture,
    mockFixture,
    ruleFixture,
    type UpstreamServer,
    type RunningProxy,
    type WorkspaceFixture,
} from "./proxyHarness";

// ── Electron: capture the real handlers instead of registering them ───────────

const { handlers, dialogState } = vi.hoisted(() => ({
    handlers: new Map<string, (...args: any[]) => any>(),
    dialogState: { result: { canceled: true, filePath: undefined as string | undefined } },
}));

vi.mock("electron", () => ({
    ipcMain: {
        handle: (channel: string, fn: (...args: any[]) => any) => { handlers.set(channel, fn); },
        on: vi.fn(),
    },
    BrowserWindow: { getAllWindows: vi.fn(() => []) },
    dialog: { showSaveDialog: vi.fn(async () => dialogState.result) },
    app: {
        getPath: vi.fn(() => "/tmp/test-user-data"),
        on: vi.fn(),
        whenReady: vi.fn(() => Promise.resolve()),
        commandLine: { appendSwitch: vi.fn() },
        quit: vi.fn(),
    },
}));

// `systemHandlers` imports `getMainWindow` from `@/main`, which must not be loaded for real.
vi.mock("@/main", () => ({ getMainWindow: () => null, updateTrayMenu: () => { } }));

import { registerSystemHandlers } from "@/ipc/handlers/systemHandlers";

async function invoke<T = any>(channel: string, ...args: any[]): Promise<T> {
    const fn = handlers.get(channel);
    if (!fn) throw new Error(`No handler registered for "${channel}"`);
    return await fn({}, ...args);
}

// ── Fixture ──────────────────────────────────────────────────────────────────

let ws: WorkspaceFixture;
let upstream: UpstreamServer;
let proxy: RunningProxy;

const captured: RequestLogEntry[] = [];
const onRequest = (e: RequestLogEntry): void => { captured.push(e); };

const BIG_BYTES = 600 * 1024;
const CAP_BYTES = 512 * 1024;

const b64 = (s: string): string => Buffer.from(s, "utf-8").toString("base64");
const fromB64 = (s: string): Buffer => Buffer.from(s, "base64");

/** Wait for an entry matching `pred` — the log is emitted asynchronously after the response. */
async function waitFor(
    pred: (e: RequestLogEntry) => boolean,
    timeoutMs = 4000,
): Promise<RequestLogEntry> {
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
        const found = captured.find(pred);
        if (found) return found;
        await new Promise((r) => setTimeout(r, 10));
    }
    throw new Error(
        `No captured entry matched. Saw: ${JSON.stringify(captured.map((e) => `${e.method} ${e.url} via=${e.via}`))}`,
    );
}

const byPath = (p: string) => (e: RequestLogEntry) => new URL(e.url).pathname === p;

beforeAll(async () => {
    upstream = await startUpstream((req, res) => {
        if (req.url === "/big") {
            res.writeHead(200, { "content-type": "application/octet-stream" });
            res.end(Buffer.alloc(BIG_BYTES, 0x62));
            return;
        }
        res.writeHead(200, { "content-type": "application/json", "x-upstream": "real" });
        res.end(JSON.stringify({ upstream: true, path: req.url }));
    });

    ws = createWorkspace({
        mappings: [
            mappingFixture({ id: "map-cap", domain: "api.localhost", target: upstream.target }),
        ],
        mocks: [
            mockFixture({
                id: "mock-cap",
                name: "Captured Mock",
                method: "GET",
                urlPattern: "http://mocked.localhost/thing",
                responseStatus: 201,
                responseHeaders: { "content-type": "application/json" },
                mockedResponseHeaders: ["content-type"],
                responseBody: JSON.stringify({ mocked: true }),
            }),
        ],
        rules: [
            ruleFixture({
                id: "rule-cap",
                name: "Captured Rule",
                pattern: "http://rule-target.test/data",
                targetType: "external",
                targetExternal: upstream.target,
            }),
        ],
    });

    logEmitter.on("request", onRequest);
    proxy = await startProxy();
    registerSystemHandlers();
});

afterAll(() => {
    logEmitter.off("request", onRequest);
    proxy?.stop();
    void upstream?.close();
    ws?.cleanup();
});

beforeEach(() => {
    captured.length = 0;
});

// ─────────────────────────────────────────────────────────────────────────────

describe("capture — the entry the Capture screen renders", () => {
    it("captures a mapped request with method, url, host, status and target", async () => {
        await proxyRequest(proxy.port, { target: "/api/users", host: "api.localhost" });

        const e = await waitFor(byPath("/api/users"));

        expect(e.method).toBe("GET");
        expect(e.url).toBe("http://api.localhost/api/users");
        expect(e.host).toBe("api.localhost");
        expect(e.status).toBe(200);
        expect(e.resStatus).toBe(200);
        expect(e.target).toBe(upstream.target);
    });

    it("records how the request was fulfilled: rfc6761 for a domain mapping", async () => {
        await proxyRequest(proxy.port, { target: "/api/mapped", host: "api.localhost" });

        expect((await waitFor(byPath("/api/mapped"))).via).toBe("rfc6761");
    });

    it("records how the request was fulfilled: mock when a mock served it", async () => {
        await proxyRequest(proxy.port, { target: "http://mocked.localhost/thing", host: "mocked.localhost" });

        const e = await waitFor((x) => x.via === "mock");

        expect(e.status).toBe(201);
        expect(e.target).toBe("mock:mock-cap");
    });

    it("records how the request was fulfilled: rule when a proxy rule matched", async () => {
        await proxyRequest(proxy.port, { target: "http://rule-target.test/data", host: "rule-target.test" });

        expect((await waitFor((x) => x.via === "rule")).via).toBe("rule");
    });

    it("records how the request was fulfilled: proxy for a forward-proxy passthrough", async () => {
        await proxyRequest(proxy.port, {
            target: `http://127.0.0.1:${upstream.port}/direct`,
            host: `127.0.0.1:${upstream.port}`,
        });

        const e = await waitFor(byPath("/direct"));

        expect(e.via).toBe("proxy");
        expect(e.status).toBe(200);
    });

    it("records an error entry for a domain with no mapping", async () => {
        const res = await proxyRequest(proxy.port, { target: "/nope", host: "unmapped.localhost" });

        const e = await waitFor(byPath("/nope"));

        expect(res.status).toBe(404);
        expect(e.via).toBe("error");
        expect(e.status).toBe(404);
        expect(e.target).toBeNull();
        expect(e.resBody).toBe("");
    });

    it("assigns each capture a unique id and a millisecond timestamp", async () => {
        await proxyRequest(proxy.port, { target: "/api/one", host: "api.localhost" });
        await proxyRequest(proxy.port, { target: "/api/two", host: "api.localhost" });

        const a = await waitFor(byPath("/api/one"));
        const b = await waitFor(byPath("/api/two"));

        expect(a.id).toBeTruthy();
        expect(b.id).toBeTruthy();
        expect(a.id).not.toBe(b.id);
        expect(a.ts).toBeGreaterThan(1_600_000_000_000);
        expect(a.ts).toBeLessThanOrEqual(Date.now());
    });

    it("reports a non-null duration", async () => {
        await proxyRequest(proxy.port, { target: "/api/timed", host: "api.localhost" });

        const e = await waitFor(byPath("/api/timed"));

        expect(e.durationMs).toBeTypeOf("number");
        expect(e.durationMs).toBeGreaterThanOrEqual(0);
    });

    it("does not capture the localhost home page", async () => {
        const res = await proxyRequest(proxy.port, { target: "/", host: "localhost" });
        expect(res.status).toBe(200);

        // Give the emitter a moment; nothing should arrive.
        await new Promise((r) => setTimeout(r, 150));
        expect(captured).toHaveLength(0);
    });
});

// ─────────────────────────────────────────────────────────────────────────────

describe("capture — the payload a replay needs", () => {
    it("captures the request headers", async () => {
        await proxyRequest(proxy.port, {
            target: "/api/headers",
            host: "api.localhost",
            headers: { "x-trace-id": "trace-123", accept: "application/json" },
        });

        const e = await waitFor(byPath("/api/headers"));

        expect(e.reqHeaders["x-trace-id"]).toBe("trace-123");
        expect(e.reqHeaders["accept"]).toBe("application/json");
        expect(e.reqHeaders["host"]).toBe("api.localhost");
    });

    it("does not capture hop-by-hop headers", async () => {
        await proxyRequest(proxy.port, {
            target: "/api/hop",
            host: "api.localhost",
            headers: { connection: "keep-alive", "x-keep": "yes" },
        });

        const e = await waitFor(byPath("/api/hop"));

        expect(e.reqHeaders["connection"]).toBeUndefined();
        expect(e.reqHeaders["x-keep"]).toBe("yes");
    });

    it("base64-encodes the request body so it can be replayed byte-for-byte", async () => {
        const body = JSON.stringify({ name: "ada", nested: { n: 1 } });
        await proxyRequest(proxy.port, {
            method: "POST",
            target: "/api/create",
            host: "api.localhost",
            body,
        });

        const e = await waitFor(byPath("/api/create"));

        expect(e.method).toBe("POST");
        expect(fromB64(e.reqBody).toString("utf-8")).toBe(body);
    });

    it("round-trips a multi-byte request body", async () => {
        const body = JSON.stringify({ greeting: "héllo — 日本語 ✓" });
        await proxyRequest(proxy.port, {
            method: "POST",
            target: "/api/utf8",
            host: "api.localhost",
            body,
        });

        const e = await waitFor(byPath("/api/utf8"));

        expect(fromB64(e.reqBody).toString("utf-8")).toBe(body);
    });

    it("leaves the request body empty for a bodyless request", async () => {
        await proxyRequest(proxy.port, { target: "/api/empty", host: "api.localhost" });

        expect((await waitFor(byPath("/api/empty"))).reqBody).toBe("");
    });

    it("base64-encodes the response body", async () => {
        await proxyRequest(proxy.port, { target: "/api/body", host: "api.localhost" });

        const e = await waitFor(byPath("/api/body"));

        expect(JSON.parse(fromB64(e.resBody).toString("utf-8"))).toEqual({
            upstream: true,
            path: "/api/body",
        });
        expect(e.resHeaders["x-upstream"]).toBe("real");
    });

    it("captures the mocked response so it can be re-served", async () => {
        await proxyRequest(proxy.port, { target: "http://mocked.localhost/thing", host: "mocked.localhost" });

        const e = await waitFor((x) => x.via === "mock");

        expect(JSON.parse(fromB64(e.resBody).toString("utf-8"))).toEqual({ mocked: true });
        expect(e.resStatus).toBe(201);
    });
});

// ─────────────────────────────────────────────────────────────────────────────

describe("capture — the 512 KB response-body cap", () => {
    it("caps an oversized captured response body", async () => {
        await proxyRequest(proxy.port, { target: "/big", host: "api.localhost" });

        const e = await waitFor(byPath("/big"));

        // The client still received the whole body; only the *capture* is truncated.
        expect(fromB64(e.resBody).length).toBe(CAP_BYTES);
    });

    it("keeps a response below the cap in full", async () => {
        await proxyRequest(proxy.port, { target: "/api/small", host: "api.localhost" });

        const e = await waitFor(byPath("/api/small"));
        const decoded = fromB64(e.resBody).toString("utf-8");

        expect(decoded).toContain('"upstream":true');
        expect(decoded.length).toBeLessThan(CAP_BYTES);
    });
});

// ─────────────────────────────────────────────────────────────────────────────

describe("capture:shareJson — exporting selected captures", () => {
    it("writes the selected entries to the chosen file", async () => {
        const outDir = fs.mkdtempSync(path.join(os.tmpdir(), "bifurc-share-"));
        const outFile = path.join(outDir, "captured-requests.json");
        dialogState.result = { canceled: false, filePath: outFile };

        await proxyRequest(proxy.port, { target: "/api/share-me", host: "api.localhost" });
        const e = await waitFor(byPath("/api/share-me"));

        const res = await invoke("capture:shareJson", [e], "captured-requests.json");

        expect(res.ok).toBe(true);
        const written = JSON.parse(fs.readFileSync(outFile, "utf-8"));
        expect(written).toHaveLength(1);
        expect(written[0].url).toBe("http://api.localhost/api/share-me");
        expect(written[0].resBody).toBe(e.resBody);

        fs.rmSync(outDir, { recursive: true, force: true });
    });

    it("reports cancellation without writing anything", async () => {
        dialogState.result = { canceled: true, filePath: undefined };

        const res = await invoke("capture:shareJson", [{ id: "x" }], "captured-requests.json");

        expect(res).toEqual({ ok: false, canceled: true });
    });
});
