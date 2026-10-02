/**
 * END-TO-END — "Disable cache": the headers a browser sends to revalidate (`If-None-Match`, `If-Modified-Since`,
 * `Cache-Control`) must not reach the target while the policy is on, so the server answers with a body instead of 304.
 */
import { afterEach, describe, expect, it } from "vitest";
import { setCacheDisabled } from "@bifurc/engine/utils/cachePolicy";
import {
  createWorkspace, startProxy, startUpstream, edgeRequest, mappingFixture,
  type RunningProxy, type UpstreamServer, type WorkspaceFixture,
} from "./proxyHarness";

let ws: WorkspaceFixture;
let proxy: RunningProxy;
let upstream: UpstreamServer;
let seen: Record<string, unknown> = {};

afterEach(() => {
  setCacheDisabled(true);
  proxy?.stop();
  ws?.cleanup();
});

async function send(): Promise<void> {
  upstream = await startUpstream((req, res) => {
    seen = req.headers;
    res.writeHead(200, { "content-type": "text/plain" });
    res.end("fresh");
  });
  ws = createWorkspace({ mappings: [mappingFixture({ id: "map-app", domain: "app.localhost", target: upstream.target })] });
  proxy = await startProxy();
  const res = await edgeRequest(proxy.edgePort, {
    target: "/x", host: "app.localhost",
    headers: { "if-none-match": '"abc"', "if-modified-since": "Wed, 01 Jan 2025 00:00:00 GMT", "cache-control": "max-age=0", accept: "*/*" },
  });
  expect(res.body).toBe("fresh");
}

describe("disable cache (end-to-end)", () => {
  it("strips the cache headers before the request reaches the target", async () => {
    await send();
    expect(seen["if-none-match"]).toBeUndefined();
    expect(seen["if-modified-since"]).toBeUndefined();
    expect(seen["cache-control"]).toBeUndefined();
    expect(seen["accept"]).toBe("*/*");
  });

  it("forwards them untouched once switched off", async () => {
    setCacheDisabled(false);
    await send();
    expect(seen["if-none-match"]).toBe('"abc"');
    expect(seen["cache-control"]).toBe("max-age=0");
  });
});
