/**
 * END-TO-END INTEGRATION TESTS — mappings and proxy rules.
 *
 * These tests boot the real proxy server against a real on-disk workspace and send
 * real HTTP requests through it, asserting on the bytes that come back. They exist
 * because the previous proxy tests mocked `net`/`http`/the store, which meant they
 * could never catch a regression where a mapping pointed at the wrong port or a rule
 * routed to the wrong target.
 *
 * If any of these fail, traffic routing is genuinely broken for users.
 */

import { describe, it, expect, afterEach, beforeEach } from "vitest";
import { logEmitter } from "@bifurc/engine/proxy/server";
import {
  createWorkspace,
  startProxy,
  startUpstream,
  proxyRequest,
  mappingFixture,
  ruleFixture,
  type UpstreamServer,
  type RunningProxy,
  type WorkspaceFixture,
} from "./proxyHarness";

let ws: WorkspaceFixture;
let proxy: RunningProxy;
let upstreamA: UpstreamServer;
let upstreamB: UpstreamServer;

beforeEach(async () => {
  // Two distinct upstreams let us prove *which* target the proxy actually chose.
  upstreamA = await startUpstream((_req, res) => {
    res.writeHead(200, { "content-type": "application/json" });
    res.end(JSON.stringify({ from: "A" }));
  });
  upstreamB = await startUpstream((_req, res) => {
    res.writeHead(200, { "content-type": "application/json" });
    res.end(JSON.stringify({ from: "B" }));
  });
});

afterEach(() => {
  proxy?.stop();
  ws?.cleanup();
});

// ── Mappings (*.localhost) ────────────────────────────────────────────────────

describe("mappings — *.localhost routing (end-to-end)", () => {
  it("forwards a mapped *.localhost request to the mapped target", async () => {
    ws = createWorkspace({
      mappings: [mappingFixture({ id: "map-app", domain: "app.localhost", target: upstreamA.target })],
    });
    proxy = await startProxy();

    const res = await proxyRequest(proxy.port, {
      target: "/api/users",
      host: "app.localhost",
    });

    expect(res.status).toBe(200);
    expect(JSON.parse(res.body)).toEqual({ from: "A" });
    expect(upstreamA.requests).toHaveLength(1);
  });

  it("preserves method, path, query string and body when forwarding", async () => {
    ws = createWorkspace({
      mappings: [mappingFixture({ id: "map-app", domain: "app.localhost", target: upstreamA.target })],
    });
    proxy = await startProxy();

    await proxyRequest(proxy.port, {
      method: "POST",
      target: "/api/items?page=2&sort=asc",
      host: "app.localhost",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ name: "widget" }),
    });

    expect(upstreamA.requests).toHaveLength(1);
    const forwarded = upstreamA.requests[0];
    expect(forwarded.method).toBe("POST");
    expect(forwarded.url).toBe("/api/items?page=2&sort=asc");
    expect(forwarded.body).toBe('{"name":"widget"}');
  });

  it("returns 404 for a *.localhost domain with no mapping", async () => {
    ws = createWorkspace({ mappings: [] });
    proxy = await startProxy();

    const res = await proxyRequest(proxy.port, { target: "/", host: "nope.localhost" });

    expect(res.status).toBe(404);
    expect(res.body).toContain("Not Mapped");
  });

  it("ignores a mapping that is not in enabled.json", async () => {
    // The mapping file exists and says enabled: true, but it was disabled in the UI,
    // so it must not appear in the enabled set the proxy reads.
    ws = createWorkspace({
      mappings: [mappingFixture({ id: "map-app", domain: "app.localhost", target: upstreamA.target, enabled: true })],
      disabledMappings: ["map-app"],
    });
    proxy = await startProxy();

    const res = await proxyRequest(proxy.port, { target: "/", host: "app.localhost" });

    expect(res.status).toBe(404);
    expect(upstreamA.requests).toHaveLength(0);
  });

  it("serves the home page (200) for the bare localhost host", async () => {
    ws = createWorkspace({
      mappings: [mappingFixture({ id: "map-app", domain: "app.localhost", target: upstreamA.target })],
    });
    proxy = await startProxy();

    const res = await proxyRequest(proxy.port, { target: "/", host: "localhost" });

    expect(res.status).toBe(200);
    expect(res.body).toContain("app.localhost");
  });

  it("returns 502 when the mapped target is not reachable", async () => {
    const dead = await startUpstream();
    const deadTarget = dead.target;
    await dead.close(); // now nothing is listening on that port

    ws = createWorkspace({
      mappings: [mappingFixture({ id: "map-dead", domain: "dead.localhost", target: deadTarget })],
    });
    proxy = await startProxy();

    const res = await proxyRequest(proxy.port, { target: "/", host: "dead.localhost" });

    expect(res.status).toBe(502);
  });

  it("logs a mapped request with via=rfc6761", async () => {
    ws = createWorkspace({
      mappings: [mappingFixture({ id: "map-app", domain: "app.localhost", target: upstreamA.target })],
    });
    proxy = await startProxy();

    const entries: any[] = [];
    const listener = (e: any) => entries.push(e);
    logEmitter.on("request", listener);

    await proxyRequest(proxy.port, { target: "/api", host: "app.localhost" });
    // Give the async log emit a tick to land.
    await new Promise((r) => setTimeout(r, 50));

    logEmitter.removeListener("request", listener);
    expect(entries[0]?.via).toBe("rfc6761");
    expect(entries[0]?.status).toBe(200);
  });
});

// ── Proxy rules (forward-proxy / absolute-form) ───────────────────────────────

describe("proxy rules — routing to a target (end-to-end)", () => {
  it("routes a matching request to an external target", async () => {
    const requestTarget = `http://127.0.0.1:${upstreamB.port}/data`;
    ws = createWorkspace({
      rules: [
        ruleFixture({
          id: "rule-ext",
          pattern: requestTarget,
          useRegex: false,
          targetType: "external",
          targetExternal: upstreamA.target,
        }),
      ],
    });
    proxy = await startProxy();

    const res = await proxyRequest(proxy.port, { target: requestTarget, host: `127.0.0.1:${upstreamB.port}` });

    expect(res.status).toBe(200);
    expect(JSON.parse(res.body)).toEqual({ from: "A" });
    expect(upstreamA.requests).toHaveLength(1);
    expect(upstreamB.requests).toHaveLength(0);
  });

  it("routes a matching request to the target of a referenced mapping", async () => {
    const requestTarget = `http://api.test/v2/items`;
    ws = createWorkspace({
      mappings: [mappingFixture({ id: "map-api", domain: "api.localhost", target: upstreamA.target })],
      rules: [
        ruleFixture({
          id: "rule-map",
          pattern: requestTarget,
          useRegex: false,
          targetType: "mapping",
          targetMappingId: "map-api",
        }),
      ],
    });
    proxy = await startProxy();

    const res = await proxyRequest(proxy.port, { target: requestTarget, host: "api.test" });

    expect(res.status).toBe(200);
    expect(JSON.parse(res.body)).toEqual({ from: "A" });
    expect(upstreamA.requests[0]?.url).toBe("/v2/items");
  });

  it("matches rules by regex", async () => {
    const requestTarget = `http://api.test/v2/items`;
    ws = createWorkspace({
      rules: [
        ruleFixture({
          id: "rule-regex",
          pattern: "^http://api\\.test/v\\d+/.*$",
          useRegex: true,
          targetType: "external",
          targetExternal: upstreamA.target,
        }),
      ],
    });
    proxy = await startProxy();

    const res = await proxyRequest(proxy.port, { target: requestTarget, host: "api.test" });

    expect(res.status).toBe(200);
    expect(JSON.parse(res.body)).toEqual({ from: "A" });
  });

  it("returns 502 when a rule matches but its target is not configured", async () => {
    // The rule matches, but its target is empty. If the proxy ignored the rule and
    // fell through to passthrough, upstreamB would answer 200 — so a 502 here proves
    // the rule was matched AND that the missing-target guard fired.
    const requestTarget = `http://127.0.0.1:${upstreamB.port}/broken`;
    ws = createWorkspace({
      rules: [
        ruleFixture({
          id: "rule-broken",
          pattern: requestTarget,
          useRegex: false,
          targetType: "external",
          targetExternal: "",
        }),
      ],
    });
    proxy = await startProxy();

    const res = await proxyRequest(proxy.port, { target: requestTarget, host: `127.0.0.1:${upstreamB.port}` });

    expect(res.status).toBe(502);
    expect(upstreamB.requests).toHaveLength(0);
  });

  it("returns 502 when a rule references a mapping that does not exist", async () => {
    const requestTarget = `http://127.0.0.1:${upstreamB.port}/missing-mapping`;
    ws = createWorkspace({
      rules: [
        ruleFixture({
          id: "rule-missing-map",
          pattern: requestTarget,
          useRegex: false,
          targetType: "mapping",
          targetMappingId: "does-not-exist",
        }),
      ],
    });
    proxy = await startProxy();

    const res = await proxyRequest(proxy.port, { target: requestTarget, host: `127.0.0.1:${upstreamB.port}` });

    expect(res.status).toBe(502);
    expect(upstreamB.requests).toHaveLength(0);
  });

  it("ignores a rule that is not in enabled.json and passes through to the real target", async () => {
    const requestTarget = `http://127.0.0.1:${upstreamB.port}/data`;
    ws = createWorkspace({
      rules: [
        ruleFixture({
          id: "rule-off",
          pattern: requestTarget,
          useRegex: false,
          targetType: "external",
          targetExternal: upstreamA.target,
        }),
      ],
      disabledRules: ["rule-off"],
    });
    proxy = await startProxy();

    const res = await proxyRequest(proxy.port, { target: requestTarget, host: `127.0.0.1:${upstreamB.port}` });

    // Rule was off → request went straight to B, not to the rule's target A.
    expect(res.status).toBe(200);
    expect(JSON.parse(res.body)).toEqual({ from: "B" });
    expect(upstreamA.requests).toHaveLength(0);
    expect(upstreamB.requests).toHaveLength(1);
  });

  it("passthroughs an absolute-form request when no mock or rule matches", async () => {
    const requestTarget = `http://127.0.0.1:${upstreamA.port}/direct`;
    ws = createWorkspace({ rules: [], mocks: [] });
    proxy = await startProxy();

    const res = await proxyRequest(proxy.port, { target: requestTarget, host: `127.0.0.1:${upstreamA.port}` });

    expect(res.status).toBe(200);
    expect(JSON.parse(res.body)).toEqual({ from: "A" });
  });

  it("returns 400 for a non-localhost relative request", async () => {
    ws = createWorkspace({});
    proxy = await startProxy();

    const res = await proxyRequest(proxy.port, { target: "/just-a-path", host: "external.example.com" });

    expect(res.status).toBe(400);
  });

  it("logs a rule-routed request with via=rule", async () => {
    const requestTarget = `http://127.0.0.1:${upstreamB.port}/data`;
    ws = createWorkspace({
      rules: [
        ruleFixture({
          id: "rule-ext",
          pattern: requestTarget,
          useRegex: false,
          targetType: "external",
          targetExternal: upstreamA.target,
        }),
      ],
    });
    proxy = await startProxy();

    const entries: any[] = [];
    const listener = (e: any) => entries.push(e);
    logEmitter.on("request", listener);

    await proxyRequest(proxy.port, { target: requestTarget, host: `127.0.0.1:${upstreamB.port}` });
    await new Promise((r) => setTimeout(r, 50));

    logEmitter.removeListener("request", listener);
    expect(entries[0]?.via).toBe("rule");
  });
});

// ── Proxy rule scripts ────────────────────────────────────────────────────────

describe("proxy rules — request/response scripts (end-to-end)", () => {
  it("applies a request script to the forwarded request", async () => {
    const requestTarget = `http://127.0.0.1:${upstreamB.port}/scripted`;
    ws = createWorkspace({
      rules: [
        ruleFixture({
          id: "rule-req-script",
          pattern: requestTarget,
          useRegex: false,
          targetType: "external",
          targetExternal: upstreamA.target,
          requestScript: 'lp.request.headers["x-injected"] = "yes";',
        }),
      ],
    });
    proxy = await startProxy();

    const res = await proxyRequest(proxy.port, { target: requestTarget, host: `127.0.0.1:${upstreamB.port}` });

    expect(res.status).toBe(200);
    expect(upstreamA.requests[0]?.headers["x-injected"]).toBe("yes");
  });

  it("applies a response script to the returned body", async () => {
    const requestTarget = `http://127.0.0.1:${upstreamB.port}/rewrite`;
    ws = createWorkspace({
      rules: [
        ruleFixture({
          id: "rule-res-script",
          pattern: requestTarget,
          useRegex: false,
          targetType: "external",
          targetExternal: upstreamA.target,
          responseScript: 'lp.response.body = JSON.stringify({ rewritten: true });',
        }),
      ],
    });
    proxy = await startProxy();

    const res = await proxyRequest(proxy.port, { target: requestTarget, host: `127.0.0.1:${upstreamB.port}` });

    expect(res.status).toBe(200);
    expect(JSON.parse(res.body)).toEqual({ rewritten: true });
  });
});
