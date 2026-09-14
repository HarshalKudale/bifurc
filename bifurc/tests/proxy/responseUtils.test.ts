import { describe, it, expect } from "vitest";
import * as http from "http";
import { buildResponseHeaders, parseRawHeaders } from "@/proxy/responseUtils";

/**
 * These two helpers are the only place where upstream headers are turned into
 * bytes the client receives. Two things must never regress:
 *   1. hop-by-hop headers must be stripped (otherwise `connection: close` and
 *      `transfer-encoding` leak into a response we already re-framed), and
 *   2. `set-cookie` must NOT be comma-joined (cookies legitimately contain
 *      commas in `Expires`, so joining them corrupts the response).
 */

function fakeRes(over: Partial<http.IncomingMessage> = {}): http.IncomingMessage {
    return {
        statusCode: 200,
        statusMessage: "OK",
        headers: {},
        ...over,
    } as unknown as http.IncomingMessage;
}

describe("buildResponseHeaders", () => {
    it("writes an HTTP/1.1 status line", () => {
        const { head } = buildResponseHeaders(fakeRes());
        expect(head.startsWith("HTTP/1.1 200 OK\r\n")).toBe(true);
    });

    it("tolerates a missing status message", () => {
        const { head } = buildResponseHeaders(fakeRes({ statusMessage: undefined }));
        expect(head.startsWith("HTTP/1.1 200 \r\n")).toBe(true);
    });

    it("strips hop-by-hop headers", () => {
        const { head, resHeaders } = buildResponseHeaders(
            fakeRes({
                headers: {
                    connection: "keep-alive",
                    "keep-alive": "timeout=5",
                    "transfer-encoding": "chunked",
                    "proxy-connection": "keep-alive",
                    upgrade: "websocket",
                    "content-type": "application/json",
                },
            }),
        );
        expect(resHeaders).toEqual({ "content-type": "application/json" });
        expect(head).not.toMatch(/transfer-encoding/i);
        expect(head).not.toMatch(/^connection:/im);
    });

    it("emits one set-cookie line per cookie and joins them with newlines", () => {
        const { head, resHeaders } = buildResponseHeaders(
            fakeRes({
                headers: {
                    "set-cookie": ["a=1; Path=/", "b=2; Expires=Wed, 21 Oct 2026 07:28:00 GMT"],
                },
            }),
        );
        expect(resHeaders["set-cookie"]).toBe(
            "a=1; Path=/\nb=2; Expires=Wed, 21 Oct 2026 07:28:00 GMT",
        );
        // Two distinct header lines — a comma-joined single line would break the cookie.
        expect(head.match(/^set-cookie:/gim)?.length).toBe(2);
        expect(head).not.toContain("a=1; Path=/, b=2");
    });

    it("handles a single set-cookie given as a string", () => {
        const { head, resHeaders } = buildResponseHeaders(
            fakeRes({ headers: { "set-cookie": "sid=abc; HttpOnly" } }),
        );
        expect(resHeaders["set-cookie"]).toBe("sid=abc; HttpOnly");
        expect(head.match(/^set-cookie:/gim)?.length).toBe(1);
    });

    it("joins multi-valued non-cookie headers with a comma", () => {
        const { resHeaders, head } = buildResponseHeaders(
            fakeRes({ headers: { vary: ["Accept", "Origin"] } }),
        );
        expect(resHeaders["vary"]).toBe("Accept, Origin");
        expect(head).toContain("vary: Accept, Origin\r\n");
    });

    it("drops null entries from array-valued headers", () => {
        const { resHeaders } = buildResponseHeaders(
            fakeRes({ headers: { "x-list": ["a", null as unknown as string, "b"] } }),
        );
        expect(resHeaders["x-list"]).toBe("a, b");
    });

    it("drops null entries from set-cookie without producing an empty line", () => {
        const { head, resHeaders } = buildResponseHeaders(
            fakeRes({ headers: { "set-cookie": ["a=1", null as unknown as string] } }),
        );
        expect(resHeaders["set-cookie"]).toBe("a=1");
        expect(head.match(/^set-cookie:/gim)?.length).toBe(1);
    });

    it("terminates the header block with a blank line", () => {
        const { head } = buildResponseHeaders(fakeRes({ headers: { "x-a": "1" } }));
        expect(head).toContain("\r\nx-a: 1\r\n");
        expect(head.endsWith("\r\n")).toBe(true);
    });
});

describe("parseRawHeaders", () => {
    it("strips hop-by-hop headers", () => {
        expect(
            parseRawHeaders({
                connection: "close",
                "transfer-encoding": "chunked",
                "content-type": "text/plain",
            } as http.IncomingHttpHeaders),
        ).toEqual({ "content-type": "text/plain" });
    });

    it("joins set-cookie with newlines, not commas", () => {
        expect(
            parseRawHeaders({ "set-cookie": ["a=1", "b=2"] } as http.IncomingHttpHeaders),
        ).toEqual({ "set-cookie": "a=1\nb=2" });
    });

    it("joins other multi-valued headers with a comma", () => {
        expect(
            parseRawHeaders({ "accept-encoding": ["gzip", "br"] } as http.IncomingHttpHeaders),
        ).toEqual({ "accept-encoding": "gzip, br" });
    });

    it("collapses an undefined array entry to an empty value but keeps the key", () => {
        // Observed behaviour: `[undefined].filter(Boolean)` yields `[]`, so the key
        // survives with an empty string rather than being removed. That is valid
        // HTTP (empty header values are allowed) but is documented here because it
        // is easy to mistake for a dropped header. See TESTING.md §6 for the nit.
        expect(
            parseRawHeaders({
                "x-empty": undefined as unknown as string,
                "x-ok": "yes",
            } as http.IncomingHttpHeaders),
        ).toEqual({ "x-empty": "", "x-ok": "yes" });
    });

    it("omits a header that is genuinely absent", () => {
        expect(parseRawHeaders({ "x-ok": "yes" } as http.IncomingHttpHeaders)).toEqual({ "x-ok": "yes" });
    });

    it("returns an empty object for empty headers", () => {
        expect(parseRawHeaders({})).toEqual({});
    });

    it("produces no header block / status line (unlike buildResponseHeaders)", () => {
        const out = parseRawHeaders({ "content-type": "application/json" } as http.IncomingHttpHeaders);
        expect(Object.keys(out)).toEqual(["content-type"]);
    });
});
