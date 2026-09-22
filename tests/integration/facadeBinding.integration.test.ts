/**
 * P14 WI 7 — the module facades are **bound** to the runtime the desktop adopts.
 *
 * WHY THIS FILE EXISTS, AND WHY IT IS THE ONLY GUARD FOR THIS
 * ----------------------------------------------------------
 * `src/main.ts` adopts one `WorkspaceRuntime` and then calls `bindProxyInstance(runtime.proxy)` /
 * `bindWebhookInstance(runtime.webhook)`. Every module function in `proxy/server.ts` and
 * `proxy/webhookServer.ts` then acts on **that** instance — which is the whole point, because 23 engine
 * modules import `reloadConfig()` from `proxy/server.ts` and would otherwise be updating a lazily created
 * *phantom* whose config the running proxy never sees.
 *
 * Nothing else can observe that. `startServer(port)` succeeding is not evidence: an unbound facade binds
 * its own instance just as happily. **So every assertion below is discriminating** — each one fails if the
 * facade acts on an instance other than the runtime's:
 *
 * - `startServer(port)` must make **the runtime's** proxy listen, not a second one.
 * - `reloadConfig()` must reach **the runtime's** proxy, so a rule written after boot is served.
 * - `stopServer()` must release **the runtime's** proxy, not leave it listening.
 * - `startWebhookServer(port)` must reach **the runtime's** webhook.
 * - and **binding must not start anything** — the property §8's "one proxy port" criterion depends on.
 *
 * TWO MUTATIONS ARE NEEDED TO SHOW THAT, NOT ONE
 * ----------------------------------------------
 * Disabling the binding entirely (making both `bind*Instance` no-ops) fails four of these six. It does
 * **not** fail the `reloadConfig` case, because it moves the listener too: `startServer` then binds the
 * facade's own instance, and `reloadConfig` reloads the same one, so the rule is served and the assertion
 * passes for the wrong reason. The `reloadConfig` case needs its own mutation — making `reloadConfig()`
 * use an instance of its own while the binding still governs `startServer` — which fails it with 502,
 * because the rule lands in an instance nobody is serving from. **A test that survives one mutation is
 * not thereby weak; it may just be aimed at a different mechanism.**
 *
 * WHY REAL SOCKETS
 * ----------------
 * The same reason `workspaceRuntime.integration.test.ts` gives: the property under test is "which object is
 * listening", and a mocked `net` hands every `createServer()` call the same object, which cannot show it.
 * `portIsFree()` is the identity instrument — a port is either bindable or it is not.
 *
 * WHAT THIS FILE IS NOT
 * ---------------------
 * An earlier version put the runtime on a *different* workspace than the active pointer, to make the
 * `reloadConfig` case discriminating by *which workspace* got reloaded. That version failed — and the
 * failure was a genuine defect, but not in this work item: `serverUtils.ts`'s `workspaceCfg()` filters
 * dispatch by `cfg.activeWorkspaceId` rather than by the instance's own workspace, so a runtime serving a
 * non-active workspace reads its own rules and then discards them. That is a **WI 4** gap, it is recorded
 * in `plan/14` and `TRAPS.md`, and it is deliberately **not** fixed here — different blast radius. This
 * file therefore stays on the active workspace and does not depend on it.
 */

import { describe, it, expect, beforeAll, afterAll, afterEach } from "vitest";
import * as fs from "fs";
import * as path from "path";
import * as net from "net";

import { createWorkspaceRuntime, type WorkspaceRuntime } from "@bifurc/engine/runtime/workspaceRuntime";
import { registerRuntime, clearRuntimes } from "@bifurc/engine/runtime/runtimeRegistry";
import {
  bindProxyInstance,
  startServer,
  stopServer,
  isRunning,
  getPort,
  reloadConfig,
} from "@bifurc/engine/proxy/server";
import {
  bindWebhookInstance,
  startWebhookServer,
  stopWebhookServer,
  isWebhookServerRunning,
  getWebhookPort,
} from "@bifurc/engine/proxy/webhookServer";
import {
  createWorkspace,
  getFreePort,
  proxyRequest,
  startUpstream,
  TEST_WS,
  type UpstreamServer,
  type WorkspaceFixture,
} from "./proxyHarness";

/** True when something can bind the port — i.e. **nothing is listening on it**. */
function portIsFree(port: number): Promise<boolean> {
  return new Promise((resolve) => {
    const srv = new net.Server();
    srv.once("error", () => resolve(false));
    srv.listen(port, "127.0.0.1", () => srv.close(() => resolve(true)));
  });
}

/** Give `listen()` a tick to bind before the first request — the harness's own 50 ms. */
const settle = (): Promise<void> => new Promise((r) => setTimeout(r, 50));

/** The rule the workspace gains *after* boot, so `reloadConfig()` is the only way it can go live. */
const LATE_RULE_ID = "rule-late";
const LATE_TARGET = "http://api.test/data";

/**
 * Write the late rule into the workspace's tree, exactly as the store's writers would: the entity file
 * plus the three index files. `enabled.json` is what makes it live — the proxy reads `fullRules` from the
 * entity files and the *set* from `enabled.json`.
 */
function writeLateRule(ws: WorkspaceFixture, targetExternal: string): void {
  const rulesDir = path.join(ws.dataRoot, TEST_WS, "rules");
  fs.writeFileSync(
    path.join(rulesDir, `${LATE_RULE_ID}.json`),
    JSON.stringify(
      {
        id: LATE_RULE_ID,
        workspaceId: TEST_WS,
        name: "Late Rule",
        pattern: LATE_TARGET,
        useRegex: false,
        targetType: "external",
        targetMappingId: "",
        targetExternal,
        requestScript: "",
        responseScript: "",
        createdAt: 1,
        folderId: null,
      },
      null,
      2,
    ),
    "utf-8",
  );
  fs.writeFileSync(path.join(rulesDir, "enabled.json"), JSON.stringify([LATE_RULE_ID], null, 2), "utf-8");
  fs.writeFileSync(
    path.join(rulesDir, "names.json"),
    JSON.stringify({ [LATE_RULE_ID]: { name: "Late Rule", url: LATE_TARGET } }, null, 2),
    "utf-8",
  );
  fs.writeFileSync(
    path.join(rulesDir, "index.json"),
    JSON.stringify({ folders: [], order: [LATE_RULE_ID] }, null, 2),
    "utf-8",
  );
}

describe("P14 WI 7 — the bound facades act on the desktop's runtime", () => {
  let ws: WorkspaceFixture;
  let upstream: UpstreamServer;
  let runtime: WorkspaceRuntime;
  let proxyPort: number;
  let webhookPort: number;

  beforeAll(async () => {
    // No rules at boot: the rule below is written later, so only `reloadConfig()` can put it in play.
    ws = createWorkspace({});
    upstream = await startUpstream((_req, res) => {
      res.writeHead(200, { "content-type": "application/json", "x-upstream": "bound" });
      res.end(JSON.stringify({ from: "bound" }));
    });
    proxyPort = await getFreePort();
    webhookPort = await getFreePort();

    runtime = createWorkspaceRuntime({
      workspaceId: TEST_WS,
      proxyPort,
      webhookPort,
      // The single-runtime case, which is what `src/main.ts` passes. Asserted here so a change to the
      // default cannot silently alter whether the desktop's proxy unloads the shared CA on stop.
      ownsCA: true,
    });
    registerRuntime(runtime);
    bindProxyInstance(runtime.proxy);
    bindWebhookInstance(runtime.webhook);
  });

  afterEach(() => {
    stopWebhookServer();
    stopServer();
  });

  afterAll(async () => {
    await runtime.stop();
    // Unbind before releasing the overrides: a leak here would redirect every later integration test in
    // this worker, the same hazard `e2eFixtureContract` records for the data root.
    bindProxyInstance(null);
    bindWebhookInstance(null);
    clearRuntimes();
    await upstream.close();
    ws.cleanup();
  });

  it("binding starts nothing — neither server, and no port is opened", async () => {
    expect(runtime.proxy.isRunning()).toBe(false);
    expect(runtime.webhook.isRunning()).toBe(false);
    expect(isRunning()).toBe(false);
    expect(isWebhookServerRunning()).toBe(false);

    // The load-bearing one: `runtime.start()` would have bound both, and the webhook port is one this
    // product has never opened at boot.
    expect(await portIsFree(proxyPort)).toBe(true);
    expect(await portIsFree(webhookPort)).toBe(true);
  });

  it("startServer() binds the RUNTIME's proxy — not a second instance", async () => {
    startServer(proxyPort);
    await settle();

    // Under a facade that ignored the binding, `runtime.proxy` would still be idle and a separate
    // instance would hold the port. This is the assertion that distinguishes the two.
    expect(runtime.proxy.isRunning()).toBe(true);
    expect(runtime.proxy.getPort()).toBe(proxyPort);
    expect(isRunning()).toBe(true);
    expect(getPort()).toBe(proxyPort);
    expect(runtime.status().proxyPort).toBe(proxyPort);
  });

  it("reloadConfig() reaches the RUNTIME's proxy, so a post-boot rule is served", async () => {
    startServer(proxyPort);
    await settle();

    // Able to fail: the rule does not exist yet, so the upstream must not be reached. If the proxy
    // re-read rules per request this assertion would fail — and it is the premise of the next one.
    const before = await proxyRequest(proxyPort, { target: LATE_TARGET, host: "api.test" });
    expect(upstream.requests).toHaveLength(0);
    expect(before.body).not.toContain('"from":"bound"');

    writeLateRule(ws, upstream.target);
    reloadConfig();

    const after = await proxyRequest(proxyPort, { target: LATE_TARGET, host: "api.test" });

    // **Discriminating, but only because the *listening* instance is the runtime's.** Mutation-checked
    // with a `reloadConfig`-specific change (make it use an instance of its own while `startServer` still
    // honours the binding): this fails with 502, because the rule is reloaded into an instance nobody is
    // serving from. See the header on why the coarser mutation does not show it.
    expect(after.status).toBe(200);
    expect(JSON.parse(after.body)).toEqual({ from: "bound" });
    expect(upstream.requests).toHaveLength(1);

    // Clean up so the file's other cases start from the same place.
    fs.rmSync(path.join(ws.dataRoot, TEST_WS, "rules", `${LATE_RULE_ID}.json`));
    fs.writeFileSync(path.join(ws.dataRoot, TEST_WS, "rules", "enabled.json"), JSON.stringify([], null, 2), "utf-8");
    reloadConfig();
  });

  it("stopServer() releases the RUNTIME's proxy — it is not left listening", async () => {
    startServer(proxyPort);
    await settle();
    expect(runtime.proxy.isRunning()).toBe(true);

    stopServer();

    // A facade acting on a phantom would leave the runtime's proxy bound, and the port would stay taken.
    expect(runtime.proxy.isRunning()).toBe(false);
    expect(isRunning()).toBe(false);
    expect(await portIsFree(proxyPort)).toBe(true);
  });

  it("startWebhookServer() binds the RUNTIME's webhook, and only when asked", async () => {
    expect(await portIsFree(webhookPort)).toBe(true);

    startWebhookServer(webhookPort);
    await settle();

    expect(runtime.webhook.isRunning()).toBe(true);
    expect(runtime.status().webhookRunning).toBe(true);
    expect(runtime.status().webhookPort).toBe(webhookPort);
    expect(isWebhookServerRunning()).toBe(true);
    expect(getWebhookPort()).toBe(webhookPort);
  });

  it("a lazily created default from before the binding does not win over the bound instance", async () => {
    // `bootstrapWorkspaces()` runs before `src/main.ts` creates the runtime and calls `reloadConfig()`,
    // which creates the facade's lazy default instance. It is never bound and never started, but it
    // exists — so the binding must take precedence over it, and this is that assertion.
    bindProxyInstance(null);
    reloadConfig(); // creates the lazy default
    expect(isRunning()).toBe(false);

    bindProxyInstance(runtime.proxy);
    startServer(proxyPort);
    await settle();

    expect(runtime.proxy.isRunning()).toBe(true);
    expect(getPort()).toBe(proxyPort);

    stopServer();
    expect(runtime.proxy.isRunning()).toBe(false);
    expect(await portIsFree(proxyPort)).toBe(true);
  });
});
