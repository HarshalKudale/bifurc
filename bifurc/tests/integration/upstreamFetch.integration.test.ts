/**
 * Integration tests for `fetchUpstreamResponse` — the buffered upstream fetch used
 * by the GraphQL / SOAP / replay paths (as opposed to the streaming
 * `passthroughToUpstream` used by plain proxy rules).
 *
 * This goes over a REAL TCP socket to a REAL upstream server. The existing
 * `tests/proxy/server.test.ts` mocks `http`/`https` wholesale, so it can only prove
 * that a request was *attempted*; it can never prove that a rule's request script
 * actually reached the upstream, or that a gzipped body is actually decoded.
 */

import { describe, it, expect, afterEach } from "vitest";
import * as zlib from "zlib";
import { startUpstream, type UpstreamServer } from "./proxyHarness";
import { fetchUpstreamResponse } from "@/proxy/responseUtils";
import type { ProxyRule } from "@/store/config";

let upstream: UpstreamServer | null = null;

afterEach(async () => {
    await upstream?.close();
    upstream = null;
});

function rule(over: Partial<ProxyRule> = {}): ProxyRule {
    return {
        id: "rule-1",
        name: "fetch rule",
        pattern: "http://api.test/data",
        useRegex: false,
        targetType: "external",
        targetMappingId: "",
        targetExternal: "",
        requestScript: "",
        responseScript: "",
        enabled: true,
        createdAt: 1,
        workspaceId: "ws-test",
        ...over,
    } as ProxyRule;
}

describe("fetchUpstreamResponse", () => {
    it("returns the upstream status, headers and body", async () => {
        upstream = await startUpstream((_req, res) => {
            res.writeHead(201, { "content-type": "application/json", "x-served-by": "real-upstream" });
            res.end(JSON.stringify({ ok: true }));
        });

        const res = await fetchUpstreamResponse("GET", upstream.target, "/anything", { host: "api.test" }, Buffer.alloc(0));

        expect(res.status).toBe(201);
        expect(res.headers["x-served-by"]).toBe("real-upstream");
        expect(JSON.parse(res.body.toString("utf-8"))).toEqual({ ok: true });
        expect(res.durationMs).toBeGreaterThanOrEqual(0);
    });

    it("decompresses a gzip body and strips content-encoding", async () => {
        const payload = JSON.stringify({ compressed: "yes", padding: "x".repeat(500) });
        upstream = await startUpstream((_req, res) => {
            res.writeHead(200, { "content-type": "application/json", "content-encoding": "gzip" });
            res.end(zlib.gzipSync(Buffer.from(payload, "utf-8")));
        });

        const res = await fetchUpstreamResponse("GET", upstream.target, "/gzip", { host: "api.test" }, Buffer.alloc(0));

        expect(res.body.toString("utf-8")).toBe(payload);
        // The stored headers must not claim the body is still compressed.
        expect(res.headers["content-encoding"]).toBeUndefined();
    });

    it("uses the absolute target's path+query, ignoring the path argument", async () => {
        upstream = await startUpstream();
        const target = `http://127.0.0.1:${upstream.port}/from-target?a=1`;

        await fetchUpstreamResponse("GET", target, "/ignored", { host: "api.test" }, Buffer.alloc(0));

        expect(upstream.requests).toHaveLength(1);
        expect(upstream.requests[0].url).toBe("/from-target?a=1");
    });

    it("uses the path argument when the target has no scheme", async () => {
        upstream = await startUpstream();

        await fetchUpstreamResponse("GET", upstream.target, "/from-path?b=2", { host: "api.test" }, Buffer.alloc(0));

        expect(upstream.requests[0].url).toBe("/from-path?b=2");
    });

    it("defaults to / when neither target nor path provide one", async () => {
        upstream = await startUpstream();

        await fetchUpstreamResponse("GET", `http://127.0.0.1:${upstream.port}`, "", { host: "api.test" }, Buffer.alloc(0));

        expect(upstream.requests[0].url).toBe("/");
    });

    it("forwards the method and request body", async () => {
        upstream = await startUpstream();

        await fetchUpstreamResponse(
            "POST",
            upstream.target,
            "/submit",
            { host: "api.test", "content-type": "application/json" },
            Buffer.from(JSON.stringify({ hello: "world" }), "utf-8"),
        );

        expect(upstream.requests[0].method).toBe("POST");
        expect(JSON.parse(upstream.requests[0].body)).toEqual({ hello: "world" });
    });

    it("runs the request script before sending, so injected headers reach the upstream", async () => {
        upstream = await startUpstream();

        await fetchUpstreamResponse(
            "POST",
            upstream.target,
            "/scripted",
            { host: "api.test", "content-type": "application/json" },
            Buffer.from(JSON.stringify({ original: true }), "utf-8"),
            rule({ requestScript: `lp.request.headers["x-injected"] = "yes"; lp.request.body = JSON.stringify({ replaced: true });` }),
        );

        expect(upstream.requests[0].headers["x-injected"]).toBe("yes");
        expect(JSON.parse(upstream.requests[0].body)).toEqual({ replaced: true });
        // content-length must be recomputed for the new body, or the upstream hangs.
        expect(Number(upstream.requests[0].headers["content-length"])).toBe(
            Buffer.byteLength(JSON.stringify({ replaced: true })),
        );
    });

    it("leaves the request untouched when the request script throws", async () => {
        upstream = await startUpstream();

        await fetchUpstreamResponse(
            "POST",
            upstream.target,
            "/broken-script",
            { host: "api.test" },
            Buffer.from("original-body", "utf-8"),
            rule({ requestScript: `throw new Error("boom");` }),
        );

        expect(upstream.requests[0].body).toBe("original-body");
    });

    it("runs the response script and returns its rewritten body and headers", async () => {
        upstream = await startUpstream((_req, res) => {
            res.writeHead(200, { "content-type": "application/json" });
            res.end(JSON.stringify({ from: "upstream" }));
        });

        const res = await fetchUpstreamResponse(
            "GET",
            upstream.target,
            "/rewrite",
            { host: "api.test" },
            Buffer.alloc(0),
            rule({ responseScript: `lp.response.body = JSON.stringify({ from: "script" });` }),
        );

        expect(JSON.parse(res.body.toString("utf-8"))).toEqual({ from: "script" });
    });

    it("falls back to the real upstream body when the response script throws", async () => {
        upstream = await startUpstream((_req, res) => {
            res.writeHead(200, { "content-type": "application/json" });
            res.end(JSON.stringify({ from: "upstream" }));
        });

        const res = await fetchUpstreamResponse(
            "GET",
            upstream.target,
            "/broken-response-script",
            { host: "api.test" },
            Buffer.alloc(0),
            rule({ responseScript: `throw new Error("nope");` }),
        );

        expect(JSON.parse(res.body.toString("utf-8"))).toEqual({ from: "upstream" });
        expect(res.status).toBe(200);
    });

    it("strips hop-by-hop headers from the upstream response", async () => {
        upstream = await startUpstream((_req, res) => {
            res.writeHead(200, { "content-type": "text/plain", connection: "keep-alive" });
            res.end("body");
        });

        const res = await fetchUpstreamResponse("GET", upstream.target, "/hop", { host: "api.test" }, Buffer.alloc(0));

        expect(res.headers["connection"]).toBeUndefined();
        expect(res.headers["content-type"]).toBe("text/plain");
    });

    it("rejects when the upstream is unreachable", async () => {
        // Port 1 is reserved and never listening.
        await expect(
            fetchUpstreamResponse("GET", "127.0.0.1:1", "/dead", { host: "api.test" }, Buffer.alloc(0)),
        ).rejects.toBeTruthy();
    });
});
