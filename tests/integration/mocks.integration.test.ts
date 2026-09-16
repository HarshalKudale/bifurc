/**
 * END-TO-END INTEGRATION TESTS — mocks.
 *
 * These boot the real proxy server against a real on-disk workspace and assert on the
 * actual bytes returned to a real HTTP client. They cover the behaviours that the
 * existing mock unit tests could not: that a mock actually *replaces* the upstream
 * response, that partial mocks actually merge with the live upstream, and that
 * environment variables and streaming actually reach the wire.
 */

import { describe, it, expect, afterEach, beforeEach } from "vitest";
import { logEmitter } from "@bifurc/engine/proxy/server";
import {
  createWorkspace,
  startProxy,
  startUpstream,
  proxyRequest,
  mockFixture,
  mappingFixture,
  type UpstreamServer,
  type RunningProxy,
  type WorkspaceFixture,
} from "./proxyHarness";

let ws: WorkspaceFixture;
let proxy: RunningProxy;
let upstream: UpstreamServer;

beforeEach(async () => {
  upstream = await startUpstream((_req, res) => {
    res.writeHead(200, { "content-type": "application/json", "x-upstream": "live" });
    res.end(JSON.stringify({ live: true }));
  });
});

afterEach(() => {
  proxy?.stop();
  ws?.cleanup();
});

// ── Fully mocked responses ────────────────────────────────────────────────────

describe("mocks — fully mocked responses (end-to-end)", () => {
  it("replaces the upstream response with the mock's status, headers and body", async () => {
    ws = createWorkspace({
      mappings: [mappingFixture({ id: "map-app", domain: "app.localhost", target: upstream.target })],
      mocks: [
        mockFixture({
          id: "mock-users",
          method: "GET",
          urlPattern: "http://app.localhost/api/users",
          useRegex: false,
          responseStatus: 201,
          responseHeaders: { "content-type": "application/json", "x-mocked": "1" },
          mockedResponseHeaders: ["content-type", "x-mocked"],
          responseBody: JSON.stringify({ mocked: true }),
        }),
      ],
    });
    proxy = await startProxy();

    const res = await proxyRequest(proxy.port, { target: "/api/users", host: "app.localhost" });

    expect(res.status).toBe(201);
    expect(res.headers["x-mocked"]).toBe("1");
    expect(JSON.parse(res.body)).toEqual({ mocked: true });
    // The upstream must never have been contacted for a full mock.
    expect(upstream.requests).toHaveLength(0);
  });

  it("matches mocks by regex", async () => {
    ws = createWorkspace({
      mappings: [mappingFixture({ id: "map-app", domain: "app.localhost", target: upstream.target })],
      mocks: [
        mockFixture({
          id: "mock-regex",
          method: "GET",
          urlPattern: "^http://app\\.localhost/api/users/\\d+$",
          useRegex: true,
          responseHeaders: {},
          responseBody: JSON.stringify({ byId: true }),
        }),
      ],
    });
    proxy = await startProxy();

    const res = await proxyRequest(proxy.port, { target: "/api/users/42", host: "app.localhost" });

    expect(res.status).toBe(200);
    expect(JSON.parse(res.body)).toEqual({ byId: true });
  });

  it("matches a wildcard-method mock against any verb", async () => {
    ws = createWorkspace({
      mappings: [mappingFixture({ id: "map-app", domain: "app.localhost", target: upstream.target })],
      mocks: [
        mockFixture({
          id: "mock-any",
          method: "*",
          urlPattern: "http://app.localhost/anything",
          useRegex: false,
          responseHeaders: {},
          responseBody: JSON.stringify({ any: true }),
        }),
      ],
    });
    proxy = await startProxy();

    const res = await proxyRequest(proxy.port, { method: "DELETE", target: "/anything", host: "app.localhost" });

    expect(res.status).toBe(200);
    expect(JSON.parse(res.body)).toEqual({ any: true });
  });

  it("serves a base64-encoded body", async () => {
    const payload = "BINARY-PAYLOAD-01";
    const encoded = Buffer.from(payload, "utf-8").toString("base64");
    ws = createWorkspace({
      mappings: [mappingFixture({ id: "map-app", domain: "app.localhost", target: upstream.target })],
      mocks: [
        mockFixture({
          id: "mock-bin",
          method: "GET",
          urlPattern: "http://app.localhost/blob",
          useRegex: false,
          responseHeaders: { "content-type": "application/octet-stream" },
          mockedResponseHeaders: ["content-type"],
          responseBody: encoded,
          responseBodyEncoding: "base64",
        }),
      ],
    });
    proxy = await startProxy();

    const res = await proxyRequest(proxy.port, { target: "/blob", host: "app.localhost" });

    expect(res.status).toBe(200);
    expect(res.headers["content-type"]).toBe("application/octet-stream");
    expect(res.body).toBe(payload);
  });

  it("logs a mock-served response with via=mock", async () => {
    ws = createWorkspace({
      mappings: [mappingFixture({ id: "map-app", domain: "app.localhost", target: upstream.target })],
      mocks: [
        mockFixture({
          id: "mock-users",
          method: "GET",
          urlPattern: "http://app.localhost/api/users",
          useRegex: false,
          responseHeaders: {},
        }),
      ],
    });
    proxy = await startProxy();

    const entries: any[] = [];
    const listener = (e: any) => entries.push(e);
    logEmitter.on("request", listener);

    await proxyRequest(proxy.port, { target: "/api/users", host: "app.localhost" });
    await new Promise((r) => setTimeout(r, 50));

    logEmitter.removeListener("request", listener);
    expect(entries[0]?.via).toBe("mock");
  });
});

// ── Mock selection rules ──────────────────────────────────────────────────────

describe("mocks — selection rules (end-to-end)", () => {
  it("does not apply a mock when the request method differs", async () => {
    ws = createWorkspace({
      mappings: [mappingFixture({ id: "map-app", domain: "app.localhost", target: upstream.target })],
      mocks: [
        mockFixture({
          id: "mock-post-only",
          method: "POST",
          urlPattern: "http://app.localhost/api/users",
          useRegex: false,
          responseHeaders: {},
        }),
      ],
    });
    proxy = await startProxy();

    const res = await proxyRequest(proxy.port, { method: "GET", target: "/api/users", host: "app.localhost" });

    expect(JSON.parse(res.body)).toEqual({ live: true });
    expect(upstream.requests).toHaveLength(1);
  });

  it("does not apply a mock that is not in enabled.json", async () => {
    ws = createWorkspace({
      mappings: [mappingFixture({ id: "map-app", domain: "app.localhost", target: upstream.target })],
      mocks: [
        mockFixture({
          id: "mock-off",
          method: "GET",
          urlPattern: "http://app.localhost/api/users",
          useRegex: false,
          responseHeaders: {},
          enabled: true,
        }),
      ],
      disabledMocks: ["mock-off"],
    });
    proxy = await startProxy();

    const res = await proxyRequest(proxy.port, { target: "/api/users", host: "app.localhost" });

    expect(JSON.parse(res.body)).toEqual({ live: true });
    expect(upstream.requests).toHaveLength(1);
  });

  it("does not crash on an invalid regex mock and falls through to upstream", async () => {
    ws = createWorkspace({
      mappings: [mappingFixture({ id: "map-app", domain: "app.localhost", target: upstream.target })],
      mocks: [
        mockFixture({
          id: "mock-bad-regex",
          method: "GET",
          urlPattern: "[invalid(",
          useRegex: true,
          responseHeaders: {},
        }),
      ],
    });
    proxy = await startProxy();

    const res = await proxyRequest(proxy.port, { target: "/api/users", host: "app.localhost" });

    expect(res.status).toBe(200);
    expect(JSON.parse(res.body)).toEqual({ live: true });
  });

  it("takes priority over a proxy rule for the same URL", async () => {
    const requestTarget = `http://127.0.0.1:${upstream.port}/both`;
    ws = createWorkspace({
      rules: [
        {
          id: "rule-same",
          name: "Same URL rule",
          pattern: requestTarget,
          useRegex: false,
          targetType: "external",
          targetMappingId: "",
          targetExternal: upstream.target,
          requestScript: "",
          responseScript: "",
          enabled: true,
          createdAt: 1,
          folderId: null,
        },
      ],
      mocks: [
        mockFixture({
          id: "mock-same",
          method: "GET",
          urlPattern: requestTarget,
          useRegex: false,
          responseHeaders: {},
          responseBody: JSON.stringify({ winner: "mock" }),
        }),
      ],
    });
    proxy = await startProxy();

    const res = await proxyRequest(proxy.port, { target: requestTarget, host: `127.0.0.1:${upstream.port}` });

    expect(JSON.parse(res.body)).toEqual({ winner: "mock" });
    expect(upstream.requests).toHaveLength(0);
  });
});

// ── Partial mocks (merge with live upstream) ──────────────────────────────────

describe("mocks — partial mocks merge with the live upstream (end-to-end)", () => {
  it("keeps the upstream body while overriding the status when only status is mocked", async () => {
    const requestTarget = `http://127.0.0.1:${upstream.port}/partial`;
    ws = createWorkspace({
      mocks: [
        mockFixture({
          id: "mock-partial",
          method: "GET",
          urlPattern: requestTarget,
          useRegex: false,
          responseStatus: 203,
          responseStatusMocked: true,
          responseBody: JSON.stringify({ should: "be ignored" }),
          responseBodyMocked: false,
          responseHeaders: {},
        }),
      ],
    });
    proxy = await startProxy();

    const res = await proxyRequest(proxy.port, { target: requestTarget, host: `127.0.0.1:${upstream.port}` });

    expect(res.status).toBe(203);
    expect(JSON.parse(res.body)).toEqual({ live: true });
    expect(upstream.requests).toHaveLength(1);
  });

  it("keeps the upstream status while overriding the body when only the body is mocked", async () => {
    const requestTarget = `http://127.0.0.1:${upstream.port}/partial-body`;
    ws = createWorkspace({
      mocks: [
        mockFixture({
          id: "mock-partial-body",
          method: "GET",
          urlPattern: requestTarget,
          useRegex: false,
          responseStatus: 200,
          responseStatusMocked: false,
          responseBody: JSON.stringify({ overridden: true }),
          responseBodyMocked: true,
          responseHeaders: {},
        }),
      ],
    });
    proxy = await startProxy();

    const res = await proxyRequest(proxy.port, { target: requestTarget, host: `127.0.0.1:${upstream.port}` });

    expect(res.status).toBe(200);
    expect(JSON.parse(res.body)).toEqual({ overridden: true });
  });
});

// ── Environment variable resolution ───────────────────────────────────────────

describe("mocks — environment variable resolution (end-to-end)", () => {
  it("resolves {{vars}} in the mock body from the active environment", async () => {
    ws = createWorkspace({
      activeEnvironmentId: "env-dev",
      environments: [
        {
          id: "env-dev",
          name: "Development",
          variables: [{ id: "v1", key: "API_URL", value: "http://real-api.example.com" }],
          createdAt: 1,
        },
      ],
      mappings: [mappingFixture({ id: "map-app", domain: "app.localhost", target: upstream.target })],
      mocks: [
        mockFixture({
          id: "mock-env",
          method: "GET",
          urlPattern: "http://app.localhost/api/config",
          useRegex: false,
          responseHeaders: {},
          responseBody: JSON.stringify({ url: "{{API_URL}}" }),
        }),
      ],
    });
    proxy = await startProxy();

    const res = await proxyRequest(proxy.port, { target: "/api/config", host: "app.localhost" });

    expect(JSON.parse(res.body)).toEqual({ url: "http://real-api.example.com" });
  });

  it("resolves {{vars}} in mock response headers", async () => {
    ws = createWorkspace({
      activeEnvironmentId: "env-dev",
      environments: [
        {
          id: "env-dev",
          name: "Development",
          variables: [{ id: "v1", key: "ORIGIN", value: "https://myapp.example.com" }],
          createdAt: 1,
        },
      ],
      mappings: [mappingFixture({ id: "map-app", domain: "app.localhost", target: upstream.target })],
      mocks: [
        mockFixture({
          id: "mock-env-hdr",
          method: "GET",
          urlPattern: "http://app.localhost/api/cors",
          useRegex: false,
          responseHeaders: { "access-control-allow-origin": "{{ORIGIN}}" },
          mockedResponseHeaders: ["access-control-allow-origin"],
          responseBody: "{}",
        }),
      ],
    });
    proxy = await startProxy();

    const res = await proxyRequest(proxy.port, { target: "/api/cors", host: "app.localhost" });

    expect(res.headers["access-control-allow-origin"]).toBe("https://myapp.example.com");
  });
});

// ── Streaming mocks ───────────────────────────────────────────────────────────

describe("mocks — streaming responses (end-to-end)", () => {
  it("streams an SSE mock as chunked text/event-stream", async () => {
    ws = createWorkspace({
      mappings: [mappingFixture({ id: "map-app", domain: "app.localhost", target: upstream.target })],
      mocks: [
        mockFixture({
          id: "mock-sse",
          method: "GET",
          urlPattern: "http://app.localhost/events",
          useRegex: false,
          streamingMode: "sse",
          streamingChunkDelay: 5,
          responseHeaders: {},
          responseBody: "data: hello\n\ndata: world\n\n",
        }),
      ],
    });
    proxy = await startProxy();

    const res = await proxyRequest(proxy.port, { target: "/events", host: "app.localhost" });

    expect(res.status).toBe(200);
    expect(res.headers["content-type"]).toContain("text/event-stream");
    expect(res.body).toContain("data: hello");
    expect(res.body).toContain("data: world");
  });
});

// ── Response delay ────────────────────────────────────────────────────────────

describe("mocks — response delay (end-to-end)", () => {
  it("waits for the configured delay before responding", async () => {
    ws = createWorkspace({
      mappings: [mappingFixture({ id: "map-app", domain: "app.localhost", target: upstream.target })],
      mocks: [
        mockFixture({
          id: "mock-delay",
          method: "GET",
          urlPattern: "http://app.localhost/slow",
          useRegex: false,
          responseDelay: 200,
          responseHeaders: {},
        }),
      ],
    });
    proxy = await startProxy();

    const started = Date.now();
    const res = await proxyRequest(proxy.port, { target: "/slow", host: "app.localhost" });
    const elapsed = Date.now() - started;

    expect(res.status).toBe(200);
    expect(elapsed).toBeGreaterThanOrEqual(150);
  });
});
