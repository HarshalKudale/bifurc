import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";

vi.mock("@/lib/randomizer", () => ({
    resolveRandomizers: vi.fn((text: string) => text),
}));

import { resolveVars, matchMock, serveMock, serveStreamingMock, isFullyMocked, mergeMockWithUpstream } from "@/proxy/mockHandler";

const createSocket = () => ({ writable: true, write: vi.fn(), end: vi.fn() } as any);

function makeMock(overrides: Partial<any> = {}): any {
    return {
        id: "m1",
        enabled: true,
        method: "GET",
        urlPattern: "/api/test",
        useRegex: false,
        responseStatus: 200,
        responseStatusMocked: true,
        responseHeaders: { "x-custom": "value" },
        mockedResponseHeaders: [],
        responseBody: '{"ok":true}',
        responseBodyMocked: true,
        responseBodyEncoding: undefined,
        responseDelay: 0,
        responseDelayMocked: true,
        streamingMode: undefined,
        streamingChunkDelay: undefined,
        ...overrides,
    };
}

// ── resolveVars ──────────────────────────────────────────────────────────

describe("resolveVars()", () => {
    it("returns text as-is when env is null", () => {
        expect(resolveVars("hello {{name}}", null)).toBe("hello {{name}}");
    });

    describe("partial mock merging", () => {
        it("treats header mocking as explicit only", () => {
            const mock = makeMock();
            expect(isFullyMocked(mock)).toBe(false);
        });

        it("uses upstream headers when a header is not explicitly mocked", () => {
            const merged = mergeMockWithUpstream(
                makeMock({
                    responseHeaders: { "set-cookie": "old=1", "x-custom": "mocked" },
                    mockedResponseHeaders: ["x-custom"],
                }),
                {
                    status: 201,
                    headers: { "set-cookie": "fresh=1", "x-custom": "real" },
                    body: Buffer.from('{"real":true}'),
                    durationMs: 25,
                },
                null,
            );
            expect(merged.headers["set-cookie"]).toBe("fresh=1");
            expect(merged.headers["x-custom"]).toBe("mocked");
        });

        it("keeps upstream status and body when those fields are unmocked", () => {
            const merged = mergeMockWithUpstream(
                makeMock({
                    responseStatus: 418,
                    responseStatusMocked: false,
                    responseBody: '{"mocked":true}',
                    responseBodyMocked: false,
                }),
                {
                    status: 202,
                    headers: { "content-type": "application/json" },
                    body: Buffer.from('{"actual":true}'),
                    durationMs: 10,
                },
                null,
            );
            expect(merged.status).toBe(202);
            expect(merged.body.toString("utf8")).toBe('{"actual":true}');
        });
    });

    it("returns empty string for empty input", () => {
        expect(resolveVars("", null)).toBe("");
    });

    it("returns falsy value unchanged", () => {
        expect(resolveVars(undefined as any, null)).toBe(undefined);
    });

    it("replaces known env variables", () => {
        const env = { variables: [{ key: "host", value: "localhost" }] } as any;
        expect(resolveVars("http://{{host}}/api", env)).toBe("http://localhost/api");
    });

    it("leaves unknown variables as {{key}}", () => {
        const env = { variables: [{ key: "a", value: "1" }] } as any;
        expect(resolveVars("{{a}} and {{b}}", env)).toBe("1 and {{b}}");
    });

    it("does not resolve random.* tokens itself (delegates to resolveRandomizers)", () => {
        const env = { variables: [] } as any;
        const result = resolveVars("id={{random.uuid}}", env);
        expect(result).toBe("id={{random.uuid}}");
    });

    it("replaces multiple occurrences of the same variable", () => {
        const env = { variables: [{ key: "v", value: "X" }] } as any;
        expect(resolveVars("{{v}}-{{v}}", env)).toBe("X-X");
    });
});

// ── matchMock ────────────────────────────────────────────────────────────

describe("matchMock()", () => {
    it("returns null for empty mocks array", () => {
        expect(matchMock([], "GET", "/api", null)).toBeNull();
    });

    it("skips disabled mocks", () => {
        const m = makeMock({ enabled: false });
        expect(matchMock([m], "GET", "/api/test", null)).toBeNull();
    });

    it("skips mocks with non-matching method", () => {
        const m = makeMock({ method: "POST" });
        expect(matchMock([m], "GET", "/api/test", null)).toBeNull();
    });

    it("matches wildcard method '*'", () => {
        const m = makeMock({ method: "*" });
        expect(matchMock([m], "DELETE", "/api/test", null)).toBe(m);
    });

    it("method comparison is case-insensitive", () => {
        const m = makeMock({ method: "get" });
        expect(matchMock([m], "GET", "/api/test", null)).toBe(m);
    });

    it("matches exact URL pattern", () => {
        const m = makeMock({ urlPattern: "/api/users" });
        expect(matchMock([m], "GET", "/api/users", null)).toBe(m);
    });

    it("does not match partial URL when useRegex is false", () => {
        const m = makeMock({ urlPattern: "/api/users" });
        expect(matchMock([m], "GET", "/api/users/123", null)).toBeNull();
    });

    it("matches URL with regex pattern", () => {
        const m = makeMock({ urlPattern: "^/api/users/\\d+$", useRegex: true });
        expect(matchMock([m], "GET", "/api/users/42", null)).toBe(m);
    });

    it("regex non-match returns null", () => {
        const m = makeMock({ urlPattern: "^/api/users/\\d+$", useRegex: true });
        expect(matchMock([m], "GET", "/api/users/abc", null)).toBeNull();
    });

    it("invalid regex is skipped gracefully", () => {
        const m = makeMock({ urlPattern: "[invalid", useRegex: true });
        expect(matchMock([m], "GET", "/anything", null)).toBeNull();
    });

    it("resolves env variables in urlPattern", () => {
        const env = { variables: [{ key: "base", value: "/api" }] } as any;
        const m = makeMock({ urlPattern: "{{base}}/items" });
        expect(matchMock([m], "GET", "/api/items", env)).toBe(m);
    });

    it("returns first matching mock when multiple match", () => {
        const m1 = makeMock({ id: "first", urlPattern: "/x" });
        const m2 = makeMock({ id: "second", urlPattern: "/x" });
        expect(matchMock([m1, m2], "GET", "/x", null)).toBe(m1);
    });
});

// ── serveMock ────────────────────────────────────────────────────────────

describe("serveMock()", () => {
    beforeEach(() => { vi.useFakeTimers(); });
    afterEach(() => { vi.useRealTimers(); });

    it("writes HTTP status line to socket", () => {
        const sock = createSocket();
        serveMock(sock, makeMock(), null);
        const written = sock.write.mock.calls[0][0] as string;
        expect(written).toContain("HTTP/1.1 200 OK");
    });

    it("writes custom headers to socket", () => {
        const sock = createSocket();
        serveMock(sock, makeMock({ responseHeaders: { "x-test": "yes" } }), null);
        const head = sock.write.mock.calls[0][0] as string;
        expect(head).toContain("x-test: yes");
    });

    it("adds content-length header", () => {
        const sock = createSocket();
        const body = '{"hello":"world"}';
        serveMock(sock, makeMock({ responseBody: body, responseHeaders: {} }), null);
        const head = sock.write.mock.calls[0][0] as string;
        expect(head).toContain(`content-length: ${Buffer.byteLength(body)}`);
    });

    it("adds connection: close header", () => {
        const sock = createSocket();
        serveMock(sock, makeMock(), null);
        const head = sock.write.mock.calls[0][0] as string;
        expect(head).toContain("connection: close");
    });

    it("adds default content-type when not specified", () => {
        const sock = createSocket();
        serveMock(sock, makeMock({ responseHeaders: {} }), null);
        const head = sock.write.mock.calls[0][0] as string;
        expect(head).toContain("content-type:");
    });

    it("writes body as Buffer", () => {
        const sock = createSocket();
        serveMock(sock, makeMock({ responseBody: "test-body" }), null);
        const bodyCall = sock.write.mock.calls[1][0];
        expect(Buffer.isBuffer(bodyCall)).toBe(true);
        expect(bodyCall.toString()).toBe("test-body");
    });

    it("handles base64 encoded body", () => {
        const sock = createSocket();
        const b64 = Buffer.from("binary-data").toString("base64");
        serveMock(sock, makeMock({ responseBody: b64, responseBodyEncoding: "base64", responseHeaders: {} }), null);
        const bodyCall = sock.write.mock.calls[1][0];
        expect(bodyCall.toString()).toBe("binary-data");
    });

    it("calls socket.end() after writing", () => {
        const sock = createSocket();
        serveMock(sock, makeMock(), null);
        expect(sock.end).toHaveBeenCalled();
    });

    it("delays response when responseDelay > 0", () => {
        const sock = createSocket();
        serveMock(sock, makeMock({ responseDelay: 500 }), null);
        expect(sock.write).not.toHaveBeenCalled();
        vi.advanceTimersByTime(500);
        expect(sock.write).toHaveBeenCalled();
    });

    it("does not write when socket is not writable", () => {
        const sock = createSocket();
        sock.writable = false;
        serveMock(sock, makeMock({ responseDelay: 0, streamingMode: undefined }), null);
        expect(sock.end).not.toHaveBeenCalled();
    });

    it("routes to streaming handler when streamingMode is set", () => {
        const sock = createSocket();
        serveMock(sock, makeMock({ streamingMode: "sse", responseBody: "event1\n\nevent2" }), null);
        const head = sock.write.mock.calls[0][0] as string;
        expect(head).toContain("text/event-stream");
    });
});

// ── serveStreamingMock ───────────────────────────────────────────────────

describe("serveStreamingMock()", () => {
    beforeEach(() => { vi.useFakeTimers(); });
    afterEach(() => { vi.useRealTimers(); });

    it("does nothing when socket is not writable", () => {
        const sock = createSocket();
        sock.writable = false;
        serveStreamingMock(sock, makeMock({ streamingMode: "sse", responseBody: "a\n\nb" }), null);
        expect(sock.write).not.toHaveBeenCalled();
    });

    it("writes SSE headers with text/event-stream content-type", () => {
        const sock = createSocket();
        serveStreamingMock(sock, makeMock({ streamingMode: "sse", responseBody: "data: hi\n\n" }), null);
        const head = sock.write.mock.calls[0][0] as string;
        expect(head).toContain("content-type: text/event-stream");
        expect(head).toContain("transfer-encoding: chunked");
    });

    it("sends chunked events separated by delay", () => {
        const sock = createSocket();
        const mock = makeMock({ streamingMode: "sse", responseBody: "ev1\n\nev2", streamingChunkDelay: 50 });
        serveStreamingMock(sock, mock, null);
        const initialCalls = sock.write.mock.calls.length;
        vi.advanceTimersByTime(50);
        expect(sock.write.mock.calls.length).toBeGreaterThan(initialCalls);
    });

    it("ends socket after all events sent", () => {
        const sock = createSocket();
        const mock = makeMock({ streamingMode: "sse", responseBody: "only-one", streamingChunkDelay: 10 });
        serveStreamingMock(sock, mock, null);
        vi.advanceTimersByTime(10); // send first (and only) chunk
        vi.advanceTimersByTime(10); // terminal
        expect(sock.end).toHaveBeenCalled();
    });
});
