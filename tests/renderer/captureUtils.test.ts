/**
 * Happy-path workflow test for the **Capture** screen's pure logic —
 * `renderer/components/capture/captureUtils.ts`.
 *
 * This module is what turns a captured `RequestLogEntry` into something the user can keep:
 * `buildMockInitial()` builds the mock, `reqToHeadersBody()` builds the saved request, and
 * `deriveType()`/`fulfilledBy()`/`statusColor()` drive what the table actually renders. A
 * regression here silently produces wrong mocks (wrong status, body base64-vs-text, headers
 * that should have been stripped) — and nothing asserted it.
 *
 * NOTE ON COVERAGE SCOPE: `captureUtils.ts` is a `.ts` file under `renderer/components/`,
 * but the coverage `include` glob only matched `.tsx` files there — so it was invisible to
 * the report even once this suite existed. `vitest.config.ts` now includes
 * `renderer/components` `.ts` files as well.
 */

import { describe, it, expect } from "vitest";
import type { RequestLogEntry } from "@/types";
import {
    b64ToText,
    tryFormat,
    ctToLang,
    statusColor,
    fmtTime,
    fmtDur,
    getHeader,
    resBodySize,
    urlName,
    deriveType,
    fulfilledBy,
    fulfilledColor,
    reqToHeadersBody,
    buildMockInitial,
} from "@/components/capture/captureUtils";

// ── Fixtures ─────────────────────────────────────────────────────────────────

const b64 = (s: string): string => Buffer.from(s, "utf-8").toString("base64");
const b64bytes = (n: number): string => Buffer.alloc(n, 0x61).toString("base64");

function entry(overrides: Partial<RequestLogEntry> = {}): RequestLogEntry {
    return {
        id: "log-1",
        ts: 1_700_000_000_000,
        method: "GET",
        url: "http://api.localhost/users",
        host: "api.localhost",
        status: 200,
        via: "rfc6761",
        target: "127.0.0.1:3000",
        durationMs: 12,
        reqHeaders: { host: "api.localhost", accept: "application/json" },
        reqBody: "",
        resHeaders: { "content-type": "application/json" },
        resBody: b64('{"ok":true}'),
        resStatus: 200,
        ...overrides,
    };
}

// ── base64 / text plumbing ───────────────────────────────────────────────────

describe("captureUtils — base64 and text plumbing", () => {
    it("decodes a base64 body back to text", () => {
        expect(b64ToText(b64("hello"))).toBe("hello");
    });

    it("returns an empty string for an empty body", () => {
        expect(b64ToText("")).toBe("");
    });

    it("round-trips multi-byte UTF-8", () => {
        const text = "héllo — 日本語 ✓";
        expect(b64ToText(b64(text))).toBe(text);
    });

    it("falls back to the raw input when the base64 is malformed", () => {
        expect(b64ToText("!!!not-base64!!!")).toBe("!!!not-base64!!!");
    });

    it("pretty-prints JSON and passes other text through untouched", () => {
        expect(tryFormat('{"a":1}')).toBe('{\n  "a": 1\n}');
        expect(tryFormat("plain text")).toBe("plain text");
    });

    it("maps a content-type to an editor language", () => {
        expect(ctToLang("application/json")).toBe("json");
        expect(ctToLang("text/html; charset=utf-8")).toBe("html");
        expect(ctToLang("application/xml")).toBe("xml");
        expect(ctToLang("application/javascript")).toBe("javascript");
        expect(ctToLang("text/ecmascript")).toBe("javascript");
        expect(ctToLang("text/plain")).toBe("text");
    });

    it("looks up headers case-insensitively", () => {
        const headers = { "Content-Type": "application/json", "X-Trace": "abc" };

        expect(getHeader(headers, "content-type")).toBe("application/json");
        expect(getHeader(headers, "x-trace")).toBe("abc");
        expect(getHeader(headers, "missing")).toBe("");
    });

    it("derives the URL name from a full URL, falling back for odd input", () => {
        expect(urlName("http://api.localhost/users?page=2")).toBe("/users?page=2");
        expect(urlName("http://api.localhost")).toBe("/");
        expect(urlName("not a url")).toBe("not a url");
    });
});

// ── display formatting ───────────────────────────────────────────────────────

describe("captureUtils — what the table renders", () => {
    it("colours a status by class", () => {
        expect(statusColor(null)).toBe("text-muted-foreground");
        expect(statusColor(200)).toBe("text-signal");
        expect(statusColor(204)).toBe("text-signal");
        expect(statusColor(302)).toBe("text-amber");
        expect(statusColor(404)).toBe("text-destructive");
        expect(statusColor(500)).toBe("text-destructive");
    });

    it("formats a timestamp as HH:MM:SS.mmm", () => {
        expect(fmtTime(1_700_000_000_000)).toMatch(/^\d{2}:\d{2}:\d{2}\.\d{3}$/);
    });

    it("formats a duration in ms below a second and in seconds above", () => {
        expect(fmtDur(null)).toBe("—");
        expect(fmtDur(0)).toBe("0ms");
        expect(fmtDur(999)).toBe("999ms");
        expect(fmtDur(1000)).toBe("1.0s");
        expect(fmtDur(2500)).toBe("2.5s");
    });

    it("reports the decoded response size, not the base64 length", () => {
        expect(resBodySize(entry({ resBody: "" }))).toBe("—");
        expect(resBodySize(entry({ resBody: b64("hello") }))).toBe("5 B");
        expect(resBodySize(entry({ resBody: b64bytes(1024) }))).toBe("1.0 KB");
        expect(resBodySize(entry({ resBody: b64bytes(2 * 1024 * 1024) }))).toBe("2.0 MB");
    });

    it("labels how a request was fulfilled", () => {
        for (const via of ["proxy", "rule", "rfc6761", "mock", "error"] as const) {
            expect(fulfilledBy(via)).toBeTruthy();
            expect(fulfilledColor(via)).toBeTruthy();
        }
        // Rule and RFC 6761 mappings are both "served by the proxy" — one label, one colour.
        expect(fulfilledBy("rule")).toBe(fulfilledBy("rfc6761"));
        expect(fulfilledColor("rule")).toBe(fulfilledColor("rfc6761"));
        expect(fulfilledColor("error")).not.toBe(fulfilledColor("mock"));
    });
});

// ── resource-type derivation ─────────────────────────────────────────────────

describe("captureUtils — deriveType", () => {
    const withCt = (ct: string, url = "http://api.localhost/thing") =>
        deriveType(entry({ resHeaders: { "content-type": ct }, url }));

    it("classifies by content-type first", () => {
        expect(withCt("text/html")).toBe("doc");
        expect(withCt("text/css")).toBe("css");
        expect(withCt("application/javascript")).toBe("js");
        expect(withCt("font/woff2")).toBe("font");
        expect(withCt("image/png")).toBe("img");
        expect(withCt("audio/mpeg")).toBe("media");
        expect(withCt("video/mp4")).toBe("media");
        expect(withCt("application/json")).toBe("xhr");
        expect(withCt("application/xml")).toBe("xhr");
        expect(withCt("text/plain")).toBe("xhr");
        expect(withCt("application/x-www-form-urlencoded")).toBe("xhr");
        expect(withCt("application/grpc")).toBe("xhr");
    });

    it("ignores content-type parameters", () => {
        expect(withCt("application/json; charset=utf-8")).toBe("xhr");
    });

    it("falls back to the URL extension for an unknown content-type", () => {
        expect(withCt("application/octet-stream", "http://api.localhost/logo.png")).toBe("img");
        expect(withCt("application/octet-stream", "http://api.localhost/data.json")).toBe("xhr");
        expect(withCt("application/octet-stream", "http://api.localhost/style.css")).toBe("css");
        expect(withCt("application/octet-stream", "http://api.localhost/app.mjs")).toBe("js");
    });

    it("returns 'other' when neither content-type nor extension helps", () => {
        expect(withCt("application/octet-stream", "http://api.localhost/thing")).toBe("other");
    });
});

// ── capture → saved request ──────────────────────────────────────────────────

describe("captureUtils — turning a capture into a saved request", () => {
    it("decodes the body and keeps the caller's headers", () => {
        const req = reqToHeadersBody(entry({ reqBody: b64('{"name":"ada"}') }));

        expect(req.method).toBe("GET");
        expect(req.url).toBe("http://api.localhost/users");
        expect(req.body).toBe('{"name":"ada"}');
        expect(req.headers["accept"]).toBe("application/json");
    });

    it("strips headers that must be recomputed on replay", () => {
        const req = reqToHeadersBody(entry({
            reqHeaders: {
                host: "api.localhost",
                connection: "keep-alive",
                "content-length": "13",
                "transfer-encoding": "chunked",
                "proxy-connection": "keep-alive",
                "x-keep": "yes",
            },
        }));

        expect(req.headers).toEqual({ "x-keep": "yes" });
    });

    it("leaves the name blank so the caller can label it", () => {
        expect(reqToHeadersBody(entry()).name).toBe("");
    });
});

// ── capture → mock ───────────────────────────────────────────────────────────

describe("captureUtils — turning a capture into a mock", () => {
    it("carries the request and response across verbatim", () => {
        const mock = buildMockInitial(entry({
            reqHeaders: { host: "api.localhost", accept: "application/json" },
            reqBody: b64('{"q":1}'),
            resHeaders: { "content-type": "application/json" },
            resBody: b64('{"ok":true}'),
        }));

        expect(mock.method).toBe("GET");
        expect(mock.urlPattern).toBe("http://api.localhost/users");
        expect(mock.useRegex).toBe(false);
        // The mock stores the *raw* capture, so it can replay byte-for-byte.
        expect(mock.capturedHeaders).toEqual({ host: "api.localhost", accept: "application/json" });
        expect(mock.capturedBody).toBe(b64('{"q":1}'));
        expect(mock.responseStatus).toBe(200);
        expect(mock.responseBody).toBe('{"ok":true}');
        expect(mock.responseBodyEncoding).toBeUndefined();
    });

    it("marks the response status, body and delay as mocked so the mock stands alone", () => {
        const mock = buildMockInitial(entry());

        expect(mock.responseStatusMocked).toBe(true);
        expect(mock.responseBodyMocked).toBe(true);
        expect(mock.responseDelayMocked).toBe(true);
        expect(mock.mockedResponseHeaders).toEqual([]);
    });

    it("keeps a binary response as base64 and flags the encoding", () => {
        const mock = buildMockInitial(entry({
            resHeaders: { "content-type": "image/png" },
            resBody: b64bytes(32),
        }));

        expect(mock.responseBodyEncoding).toBe("base64");
        expect(mock.responseBody).toBe(b64bytes(32));
    });

    it("falls back to '{}' when a text response has no body", () => {
        const mock = buildMockInitial(entry({ resBody: "" }));

        expect(mock.responseBody).toBe("{}");
        expect(mock.responseBodyEncoding).toBeUndefined();
    });

    it("defaults the status to 200 when the capture never got a response", () => {
        const mock = buildMockInitial(entry({ resStatus: null, status: null, via: "error" }));

        expect(mock.responseStatus).toBe(200);
    });

    it("preserves a non-2xx captured status", () => {
        const mock = buildMockInitial(entry({ resStatus: 503, status: 503 }));

        expect(mock.responseStatus).toBe(503);
    });
});
