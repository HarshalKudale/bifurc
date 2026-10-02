/**
 * Happy-path workflow test for the **Capture** screen's presentation logic —
 * `packages/gui/src/components/capture/captureUtils.ts`.
 *
 * **This file's scope shrank on 2026-09-30** (`plan/19` WI 5, item 2). The constructing half —
 * `buildMockInitial()` and `reqToHeadersBody()`, which turned a captured `RequestLogEntry` into a mock or a
 * saved request — moved to the engine (`packages/engine/src/utils/captureEntities.ts`), and its cases moved
 * with it to `tests/engine/captureEntities.test.ts`. What is asserted here is what the module still owns:
 * base64/text plumbing, the table's formatting, resource-type derivation and the fulfilled-by labels. A
 * regression in these mislabels a row or mis-formats a duration, which is why they are still worth pinning.
 *
 * NOTE ON COVERAGE SCOPE: `captureUtils.ts` is a `.ts` file under `packages/gui/src/components/`,
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

// ── moved out, 2026-09-30 ────────────────────────────────────────────────────
//
// The "turning a capture into a saved request" and "turning a capture into a mock" blocks were here.
// `plan/19` WI 5 item 2 moved `reqToHeadersBody` / `buildMockInitial` into the engine, and the cases went
// with them — `tests/engine/captureEntities.test.ts`. They are not duplicated back here: two copies of one
// suite is the same defect as two copies of one implementation, one layer up.
