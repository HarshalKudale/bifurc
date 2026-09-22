/**
 * P14 WI 4's gap — a proxy serves **its own** workspace's mocks, mappings and rules, whatever the active
 * pointer says. Closed 2026-09-22.
 *
 * WHAT WAS WRONG, AND WHY THIS FILE IS THE GUARD FOR IT
 * ----------------------------------------------------
 * A proxy instance read its state from two places at once: `fullRules` and `enabledSets` through
 * `servedWorkspaceId()` (the instance's own workspace) and `mocks` / `mappings` / `environments` through
 * `loadConfig()` — which is **active-workspace-only by construction**. `workspaceCfg()` then filtered the
 * active workspace's entities by the instance's enabled sets. Measured with only `activeWorkspaceId`
 * differing, before the fix:
 *
 * ```
 * runtime serves B, active is A:   B's own mock 404    B's own rule 502, upstream hits 0
 * runtime serves B, active is B:   B's own mock 200    B's own rule 200, upstream hits 1
 * ```
 *
 * **Nothing worked and nothing failed loudly** — and note that the wrong workspace's data did *not* leak
 * either, because the two halves cancelled. So a test that only looked for cross-talk would have found
 * nothing; the assertions below check **both** directions on purpose.
 *
 * WHY THE EXISTING SUITE COULD NOT CATCH IT
 * ----------------------------------------
 * `workspaceRuntime.integration.test.ts`'s "both genuinely serve" step asserts `getHomePage(port)` — the
 * proxy's own UI page, which is served regardless of any rule, mock or mapping. `plan/14` §8's first
 * criterion reads *"each serving its own rules"*, and **a home page is not a rule**. The instrument chosen
 * for *ports* and *readiness* in that file was chosen for *convenience* for *content*; this file is the
 * content instrument.
 *
 * WHY REAL FILES AND REAL SOCKETS
 * -------------------------------
 * The property is "which workspace's data comes back over the wire", so the entities have to be real files
 * under two real workspace directories and the answers have to be bytes from real sockets. The store's
 * `setDataRootOverride` / `setSettingsPathOverride` are production hooks, not test-only paths.
 */

import { describe, it, expect, beforeAll, afterAll } from "vitest";
import * as fs from "fs";
import * as os from "os";
import * as path from "path";

import { setDataRootOverride } from "@bifurc/engine/store/workspaceFs";
import { setSettingsPathOverride } from "@bifurc/engine/store/appSettings";
import { loadWorkspaceDispatchSlice } from "@bifurc/engine/store/config";
import { createWorkspaceRuntime, type WorkspaceRuntime } from "@bifurc/engine/runtime/workspaceRuntime";
import { registerRuntime, clearRuntimes } from "@bifurc/engine/runtime/runtimeRegistry";
import { getFreePort, proxyRequest, startUpstream, type UpstreamServer } from "./proxyHarness";

const A = "ws-active";
const B = "ws-other";

/** Write one workspace's tree: a mock, a mapping for `app.localhost`, and optional rules. */
function writeWorkspace(
  dataRoot: string,
  wsId: string,
  name: string,
  opts: {
    mock: { id: string; urlPattern: string; body: string };
    mappingTarget: string;
    rules?: any[];
  },
): void {
  const dir = path.join(dataRoot, wsId);
  for (const sub of ["mappings", "rules", "mocks", "environments", "requests", "sockets", "webhooks", "capture"]) {
    fs.mkdirSync(path.join(dir, sub), { recursive: true });
  }
  fs.writeFileSync(
    path.join(dir, "workspace.json"),
    JSON.stringify({ id: wsId, name, createdAt: 0, activeEnvironmentId: null }, null, 2),
  );

  fs.writeFileSync(path.join(dir, "mocks", "enabled.json"), JSON.stringify([opts.mock.id]));
  fs.writeFileSync(
    path.join(dir, "mocks", `${opts.mock.id}.json`),
    JSON.stringify(
      {
        id: opts.mock.id, workspaceId: wsId, name: opts.mock.id, method: "GET",
        urlPattern: opts.mock.urlPattern, useRegex: false, capturedHeaders: {}, capturedBody: "",
        responseStatus: 200, responseHeaders: { "content-type": "application/json" },
        responseBody: opts.mock.body, responseBodyEncoding: "utf8", responseDelay: 0,
        streamingMode: "none", createdAt: 1,
      },
      null, 2,
    ),
  );
  fs.writeFileSync(path.join(dir, "mocks", "index.json"), JSON.stringify({ folders: [], order: [opts.mock.id] }));

  fs.writeFileSync(path.join(dir, "mappings", "enabled.json"), JSON.stringify(["map-app"]));
  fs.writeFileSync(
    path.join(dir, "mappings", "map-app.json"),
    JSON.stringify(
      { id: "map-app", workspaceId: wsId, domain: "app.localhost", target: opts.mappingTarget, enabled: true, label: "", createdAt: 1 },
      null, 2,
    ),
  );
  fs.writeFileSync(path.join(dir, "mappings", "index.json"), JSON.stringify({ folders: [], order: ["map-app"] }));

  const rules = opts.rules ?? [];
  fs.writeFileSync(path.join(dir, "rules", "enabled.json"), JSON.stringify(rules.map((r) => r.id)));
  for (const r of rules) fs.writeFileSync(path.join(dir, "rules", `${r.id}.json`), JSON.stringify(r, null, 2));
  fs.writeFileSync(
    path.join(dir, "rules", "names.json"),
    JSON.stringify(Object.fromEntries(rules.map((r) => [r.id, { name: r.name, url: r.pattern }]))),
  );
  fs.writeFileSync(path.join(dir, "rules", "index.json"), JSON.stringify({ folders: [], order: rules.map((r) => r.id) }));

  fs.writeFileSync(
    path.join(dir, "environments", "__global__.json"),
    JSON.stringify({ id: "__global__", name: "Global", variables: [], createdAt: 0, workspaceId: wsId }),
  );
}

describe("P14 WI 4 — a proxy serves its own workspace, whatever is active", () => {
  let root: string;
  let dataRoot: string;
  let upstream: UpstreamServer;
  let runtimeB: WorkspaceRuntime;
  let runtimeA: WorkspaceRuntime;
  let portB: number;
  let portA: number;

  beforeAll(async () => {
    root = fs.mkdtempSync(path.join(os.tmpdir(), "bifurc-dispatch-scope-"));
    dataRoot = path.join(root, "data");
    upstream = await startUpstream((_req, res) => {
      res.writeHead(200, { "content-type": "application/json" });
      res.end(JSON.stringify({ from: "upstream" }));
    });

    // A is the ACTIVE workspace and has a mock, a mapping, and deliberately NO rule for api.test.
    writeWorkspace(dataRoot, A, "Active Workspace", {
      mock: { id: "mock-a", urlPattern: "http://app.localhost/api/a", body: JSON.stringify({ from: "A" }) },
      mappingTarget: upstream.target,
    });
    // B is the one both runtimes under test serve.
    writeWorkspace(dataRoot, B, "Other Workspace", {
      mock: { id: "mock-b", urlPattern: "http://app.localhost/api/b", body: JSON.stringify({ from: "B" }) },
      mappingTarget: upstream.target,
      rules: [{
        id: "rule-b", workspaceId: B, name: "B Rule", pattern: "http://api.test/data", useRegex: false,
        targetType: "external", targetMappingId: "", targetExternal: upstream.target,
        requestScript: "", responseScript: "", createdAt: 1, folderId: null,
      }],
    });

    fs.writeFileSync(
      path.join(root, "app.json"),
      JSON.stringify({
        port: 8080, webhookPort: 9101, companionPort: 9271, minimizeToTray: true, tlsEnabled: false,
        tlsCaCertPath: null, tlsCaKeyPath: null,
        workspaces: [
          { id: A, name: "Active Workspace", activeEnvironmentId: null },
          { id: B, name: "Other Workspace", activeEnvironmentId: null },
        ],
        activeWorkspaceId: A,
        hasSeenWelcome: true, zoomLevel: 0, zoomLevelSetByUser: false, themeId: null,
      }, null, 2),
    );

    setDataRootOverride(dataRoot);
    setSettingsPathOverride(path.join(root, "app.json"));

    portB = await getFreePort();
    runtimeB = createWorkspaceRuntime({ workspaceId: B, proxyPort: portB, webhookPort: await getFreePort(), ownsCA: true });
    registerRuntime(runtimeB);
    await runtimeB.start();

    portA = await getFreePort();
    runtimeA = createWorkspaceRuntime({ workspaceId: A, proxyPort: portA, webhookPort: await getFreePort(), ownsCA: true });
    registerRuntime(runtimeA);
    await runtimeA.start();
  });

  afterAll(async () => {
    await runtimeB.stop();
    await runtimeA.stop();
    clearRuntimes();
    // Reset the overrides before anything else runs in this worker — a leak would redirect every later
    // integration test, the hazard `e2eFixtureContract` records.
    setDataRootOverride(null);
    setSettingsPathOverride(null);
    await upstream.close();
    try { fs.rmSync(root, { recursive: true, force: true }); } catch { /* best-effort */ }
  });

  it("the slice loader scopes to the id it is given, not to the active pointer", () => {
    // The unit-level statement of the same fact, so a failure points at the loader rather than at HTTP.
    const forB = loadWorkspaceDispatchSlice(B);
    const forA = loadWorkspaceDispatchSlice(A);

    expect(forB.workspaceId).toBe(B);
    expect(forA.workspaceId).toBe(A);
    expect(forB.mocks.map((m) => m.id)).toEqual(["mock-b"]);
    expect(forA.mocks.map((m) => m.id)).toEqual(["mock-a"]);
    expect(forB.environments.map((e) => e.id)).toEqual(["__global__"]);
  });

  it("B serves B's mock and B's rule while A is the active workspace", async () => {
    const ownMock = await proxyRequest(portB, { target: "/api/b", host: "app.localhost" });
    expect(ownMock.status).toBe(200);
    expect(JSON.parse(ownMock.body)).toEqual({ from: "B" });

    const before = upstream.requests.length;
    const ownRule = await proxyRequest(portB, { target: "http://api.test/data", host: "api.test" });
    expect(ownRule.status).toBe(200);
    expect(JSON.parse(ownRule.body)).toEqual({ from: "upstream" });
    expect(upstream.requests.length).toBe(before + 1);
  });

  it("B does NOT serve A's mock — asserted in the other direction on purpose", async () => {
    // The two halves of the old config *cancelled*, so nothing leaked and a cross-talk-only assertion would
    // have passed before the fix. This one is here so the file states both directions explicitly.
    //
    // Note the assertion is on the **body**, not the status: B has its own mapping for `app.localhost`, so
    // `/api/a` legitimately answers 200 — by forwarding to the upstream, because B has no mock for it. That
    // is itself evidence B's own config is in force; what must never happen is A's mock body coming back.
    const otherMock = await proxyRequest(portB, { target: "/api/a", host: "app.localhost" });
    expect(otherMock.body).not.toContain('"from":"A"');
    expect(JSON.parse(otherMock.body)).toEqual({ from: "upstream" });
  });

  it("A's runtime serves A's mock, not B's — the same assertion from the other side", async () => {
    const ownMock = await proxyRequest(portA, { target: "/api/a", host: "app.localhost" });
    expect(ownMock.status).toBe(200);
    expect(JSON.parse(ownMock.body)).toEqual({ from: "A" });

    // A has no rule for api.test, so B's rule must not be reachable through A's port.
    const before = upstream.requests.length;
    const bRule = await proxyRequest(portA, { target: "http://api.test/data", host: "api.test" });
    expect(bRule.status).not.toBe(200);
    expect(upstream.requests.length).toBe(before);
  });
});
