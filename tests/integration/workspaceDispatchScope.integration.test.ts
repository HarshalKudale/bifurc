/**
 * `plan/17` §1's dispatch rule — **one** forward proxy, serving the **active** workspace, and swapping its
 * in-memory config when the pointer moves.
 *
 * WHAT THIS FILE USED TO ASSERT, AND WHY IT IS THE OPPOSITE NOW
 * -------------------------------------------------------------
 * Until 2026-09-25 this file was the guard for P14 WI 4's gap: a proxy per workspace, each serving **its own**
 * workspace's mocks, mappings and rules *whatever the active pointer said*. The bug it was written for was
 * that a proxy read its state from two places at once — `fullRules` / `enabledSets` through
 * `servedWorkspaceId()` (the instance's own workspace) and `mocks` / `mappings` / `environments` through
 * `loadConfig()`, which is **active-workspace-only by construction** — so `workspaceCfg()` cross-filtered the
 * two and served nothing. Measured before the fix, with only `activeWorkspaceId` differing:
 *
 * ```
 * runtime serves B, active is A:   B's own mock 404    B's own rule 502, upstream hits 0
 * runtime serves B, active is B:   B's own mock 200    B's own rule 200, upstream hits 1
 * ```
 *
 * **Nothing worked and nothing failed loudly**, and the wrong workspace's data did not leak either, because
 * the two halves cancelled — which is why the assertions below check both directions.
 *
 * **`plan/17` §1 inverts the premise.** There is no per-workspace proxy any more: the host owns one forward
 * proxy, and a workspace is a *project* whose switch changes *"which rules, mocks and webhook endpoints are
 * in force"* while *"the ports and the infrastructure stay put"*. So the property is no longer *"B's proxy
 * serves B's data while A is active"* — it is **"the one proxy serves A's data while A is active, and B's
 * once B is active, on the same port"**. The old assertion is not merely obsolete; it now describes a
 * behaviour that would be a **bug**.
 *
 * WHY THE PROPERTY STILL NEEDS A TEST AT ALL
 * ------------------------------------------
 * The swap is an *imperative* call, not a per-request resolution. `plan/17` §5 work item 3 is explicit —
 * *"loads the active workspace's rules and mocks **at start** and **swaps its in-memory config when the
 * active workspace changes** — not per-request resolution"* — and `proxy/server.ts` caches four things at
 * `start()` (`currentConfig`, `fullRules`, `enabledSets`, `wsSlice`) that only `reloadConfig()` refreshes.
 * So "switching workspace changes what the proxy returns" is true **only if** `workspace.setActive` calls
 * `reloadConfig()`, and this file is what makes that a measured fact rather than a reading of the source.
 * It is a live risk, not a hypothetical: P14 removed that call on purpose, and `plan/17` restored it.
 *
 * WHY REAL FILES AND REAL SOCKETS
 * -------------------------------
 * The property is "which workspace's data comes back over the wire", so the entities have to be real files
 * under two real workspace directories and the answers have to be bytes from real sockets. The store's
 * `setDataRootOverride` / `setSettingsPathOverride` are production hooks, not test-only paths.
 *
 * The composition under test is the **desktop's**: `createProxyServer({ workspaceId: null })` +
 * `bindProxyInstance()` + `registerAllEngineCommands()`. That is deliberate — it is the shortest path from
 * `workspace.setActive` to a running listener, and it means this file fails if *either* half of the pairing
 * breaks.
 */

import { describe, it, expect, beforeAll, afterAll } from "vitest";
import * as fs from "fs";
import * as os from "os";
import * as path from "path";

import { setDataRootOverride } from "@bifurc/engine/workspace/workspaceFs";
import { setSettingsPathOverride } from "@bifurc/engine/workspace/appSettings";
import { loadWorkspaceDispatchSlice } from "@bifurc/engine/workspace/config";
import { bindProxyInstance, createProxyServer, type ProxyServerInstance } from "@bifurc/engine/proxy/server";
import { CommandRegistry, type CommandContext } from "@bifurc/engine/kernel/registry";
import { registerAllEngineCommands } from "@bifurc/engine/registerAll";
import { getFreePort, proxyRequest, startUpstream, type UpstreamServer } from "./proxyHarness";

const A = "ws-active";
const B = "ws-other";

const ctx: CommandContext = { bus: {} as CommandContext["bus"] };

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
        /**
         * **`mockedResponseHeaders` is load-bearing, and it was not needed before the split.**
         *
         * `isFullyMocked()` requires every response-header key to appear here, or the mock is a *partial*
         * overlay: the proxy serves the mock's fields but still calls an upstream for the rest. Pre-split the
         * reverse half supplied that upstream from the mapping, so the mock answered 200 without ever
         * declaring itself complete.
         *
         * Post-split the forward proxy has no mapping to fall back on, so a partial mock's upstream lookup is
         * a `replayRequest` to `http://app.localhost/api/b` — which does not resolve, and the case failed
         * with a **502** rather than a 200.
         *
         * Declaring the header mocked is the right fix rather than a workaround: this suite's discriminator
         * is *which mock's body comes back*, and a fully-mocked mock answers standalone with no upstream in
         * the loop at all. That makes the assertion about dispatch scope and nothing else.
         */
        mockedResponseHeaders: ["content-type"],
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

describe("plan/17 §1 — one forward proxy, following the active workspace", () => {
  let root: string;
  let dataRoot: string;
  let upstream: UpstreamServer;
  let proxy: ProxyServerInstance;
  let registry: CommandRegistry;
  let port: number;

  /**
   * Point the pointer at `id` **through the command**, which is the only path that is supposed to swap the
   * proxy's config. Setting `app.json` directly would test the store rather than the wiring, and the wiring
   * is the whole risk here.
   */
  function setActive(id: string): void {
    const res = registry.invoke("workspace.setActive", { id }, ctx) as { ok: boolean };
    expect(res.ok).toBe(true);
  }

  beforeAll(async () => {
    root = fs.mkdtempSync(path.join(os.tmpdir(), "bifurc-dispatch-scope-"));
    dataRoot = path.join(root, "data");
    upstream = await startUpstream((_req, res) => {
      res.writeHead(200, { "content-type": "application/json" });
      res.end(JSON.stringify({ from: "upstream" }));
    });

    // A is the ACTIVE workspace at boot: a mock, a mapping, and deliberately NO rule for api.test.
    writeWorkspace(dataRoot, A, "Active Workspace", {
      mock: { id: "mock-a", urlPattern: "http://app.localhost/api/a", body: JSON.stringify({ from: "A" }) },
      mappingTarget: upstream.target,
    });
    // B has its own mock and the rule A lacks — so "which one is in force" is observable in both directions.
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
        port: 8080, webhookPort: 9101, minimizeToTray: true, tlsEnabled: false,
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

    // The desktop's composition, in four lines: one proxy that follows the pointer, bound to the facade so
    // `workspace.setActive`'s `reloadConfig()` reaches it, with the engine's commands registered.
    proxy = createProxyServer({ workspaceId: null });
    bindProxyInstance(proxy);
    registry = new CommandRegistry();
    registerAllEngineCommands(registry);

    port = await getFreePort();
    proxy.start(port);
    // `start()` is fire-and-forget — it hands off to an async `listen()`. Poll rather than sleep, so a slow
    // bind is waited out and a failed one is not mistaken for a slow one.
    const deadline = Date.now() + 5000;
    while (!proxy.isRunning() && proxy.getError() === null && Date.now() < deadline) {
      await new Promise((r) => setTimeout(r, 10));
    }
    expect(proxy.getError()).toBeNull();
    expect(proxy.isRunning()).toBe(true);
  });

  afterAll(async () => {
    proxy.stop();
    // Reset the binding and the overrides before anything else runs in this worker — a leaked override would
    // redirect every later integration test, the hazard `e2eFixtureContract` records.
    bindProxyInstance(null);
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

  /**
   * ## Absolute-form targets, and why — `plan/15` work item 3
   *
   * These cases used a **relative** target ("/api/b") with a `*.localhost` host, which is what a browser
   * sends to a *reverse* proxy. After the split that shape is refused on this port by design: the reverse half
   * (mappings, `*.localhost`, the home page) is the host-level **edge**, and `proxy/server.ts` answers every
   * relative request with a deliberate 400 naming the edge.
   *
   * What the forward proxy still owns is **mocks and rules**, and both are matched on the request's absolute
   * URL (`matchMock(cfg.mocks, method, url, …)`, where `url` is the raw target when it starts with `http`). So
   * an absolute-form target is not a workaround — it is the shape these two halves are *for*, and the
   * fixtures' own `urlPattern`s are already absolute (`http://app.localhost/api/b`).
   */
  it("with A active: A's mock answers, and B's does not", async () => {
    setActive(A);

    const ownMock = await proxyRequest(port, { target: "http://app.localhost/api/a", host: "app.localhost" });
    expect(ownMock.status).toBe(200);
    expect(JSON.parse(ownMock.body)).toEqual({ from: "A" });

    /**
     * **The assertion is on the body, and only on the body.** A has a mock for `api/a` and B has one for
     * `api/b`, so the discriminator is *which mock answers*: if the proxy were dispatching with B's config
     * this would come back `200 {"from":"B"}`, and that is the one outcome that must never happen.
     *
     * It is deliberately **not** asserted to equal `{from: "upstream"}`. That expectation came from the
     * mapping forwarding the miss to the upstream, and mappings are the edge's now — on the forward proxy a
     * miss falls through to a real passthrough, whose outcome depends on whether anything answers on the
     * host's port 80. A case that passes or fails by what is listening on the machine is the flake class
     * `TESTING.md` records, so the assertion is narrowed to the property rather than the mechanism.
     */
    const otherMock = await proxyRequest(port, { target: "http://app.localhost/api/b", host: "app.localhost" });
    expect(otherMock.body).not.toContain('"from":"B"');

    // A has no rule for api.test, so B's rule must not be reachable while A is active.
    const before = upstream.requests.length;
    const bRule = await proxyRequest(port, { target: "http://api.test/data", host: "api.test" });
    expect(bRule.status).not.toBe(200);
    expect(upstream.requests.length).toBe(before);
  });

  /**
   * **The load-bearing case.** Everything else in this file would pass under the pre-`plan/17` design; this
   * one cannot, because it asserts that the *same socket* changes what it answers.
   *
   * If `workspace.setActive`'s `reloadConfig()` were removed again — as P14 removed it — this is the case that
   * fails, and it fails on the **body** rather than on a status code, which is what makes the diagnosis
   * unambiguous: `{"from":"A"}` coming back for B's own mock URL is the cache not having been swapped.
   */
  it("switching to B swaps the proxy's in-memory config — and the port does not move", async () => {
    setActive(A);
    const portBefore = proxy.getPort();
    const aWhileActive = await proxyRequest(port, { target: "http://app.localhost/api/a", host: "app.localhost" });
    expect(JSON.parse(aWhileActive.body)).toEqual({ from: "A" });

    setActive(B);

    // Same socket, same port — `plan/17` §1: *"the ports and the infrastructure stay put"*.
    expect(proxy.getPort()).toBe(portBefore);

    // ...and B is now the workspace in force, in both directions.
    const bMock = await proxyRequest(port, { target: "http://app.localhost/api/b", host: "app.localhost" });
    expect(bMock.status).toBe(200);
    expect(JSON.parse(bMock.body)).toEqual({ from: "B" });

    const aMock = await proxyRequest(port, { target: "http://app.localhost/api/a", host: "app.localhost" });
    expect(aMock.body).not.toContain('"from":"A"');

    // B's rule — the one A does not have — is reachable now, and it really did go upstream.
    const before = upstream.requests.length;
    const bRule = await proxyRequest(port, { target: "http://api.test/data", host: "api.test" });
    expect(bRule.status).toBe(200);
    expect(JSON.parse(bRule.body)).toEqual({ from: "upstream" });
    expect(upstream.requests.length).toBe(before + 1);

    // Leave the file's end state on A, so the ordering of these cases is not load-bearing for a reader.
    setActive(A);
  });
});
