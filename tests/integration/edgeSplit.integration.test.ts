/**
 * `plan/15` work item 3's **named verification step** — the edge split, over real sockets.
 *
 * ## Why this file exists, and why that is worth stating
 *
 * `plan/15` records two suites for WI 3: `tests/proxy/edge.test.ts` (*"new, 40 cases"*) and this one
 * (*"new, 14 cases… the plan's own verification step"*). **Neither existed** — measured 2026-09-23, and the
 * correction is now in the plan's own block. Their absence is the root cause of the 66-case failure mass the
 * split left behind: the product half of WI 3 is real, and the *instrument* half was never written.
 *
 * So this file is written to fail for the reason the split exists, not to describe the implementation.
 *
 * ## What it owns, and what it deliberately does not
 *
 * **`tests/proxy/edge.test.ts` owns the routing *decisions*** — which branch each `Host` takes, the `via`
 * label, the log entry, the bind guard, the instance surface — with `http`/`net` mocked, so a case can
 * fabricate a config the filesystem cannot produce (a `resolveConfig` that throws, a `null` config, a bind
 * that fails with `EADDRINUSE`).
 *
 * **This file owns the behaviour that only real I/O can show**, and there are three kinds:
 *
 *   1. **The two listeners are genuinely two.** A mock can hand both halves the same object; only real
 *      sockets can show that `edgeRequest(edgePort)` and `proxyRequest(port)` reach *different* servers, and
 *      that each refuses the other's traffic with its documented code.
 *   2. **The edge writes raw bytes to `req.socket` and never touches `res`.** That is a measured decision
 *      (`edge.ts`'s header records the probe), and the only instrument that can see it is a real connection
 *      that comes back with a complete, well-framed response.
 *   3. **`resolveConfig` runs per request.** The property is *"a config change needs no invalidation step"*,
 *      which is a statement about time: change the files, make the next request, observe the new answer.
 *
 * **This file supersedes `harnessSplit.integration.test.ts`**, whose header asked to be folded in here
 * rather than kept as a second file asserting the same boundary. Its eight cases are the ones marked
 * `folded from harnessSplit` below.
 *
 * ## The fixtures are absolute-URL by design
 *
 * `plan/15` WI 3 moved the reverse half to the edge, and the forward half matches mocks on the request's
 * **absolute** URL. So the mock's `urlPattern` is absolute (`http://mapped.localhost/api/mock`) and it is
 * declared **fully mocked** (`mockedResponseHeaders`): without that it is a *partial* overlay that still
 * calls an upstream, and the case would be measuring the upstream rather than which listener answered.
 */
import { describe, it, expect, beforeAll, afterAll } from "vitest";
import * as fs from "fs";
import * as path from "path";

import {
  createWorkspace,
  startProxy,
  startUpstream,
  proxyRequest,
  edgeRequest,
  getFreePort,
  mappingFixture,
  mockFixture,
  ruleFixture,
  TEST_WS,
  type RunningProxy,
  type UpstreamServer,
  type WorkspaceFixture,
} from "./proxyHarness";
import { createEdgeRouter } from "@bifurc/engine/proxy/edge";
import { loadConfig } from "@bifurc/engine/workspace/config";
import { setDataRootOverride } from "@bifurc/engine/workspace/workspaceFs";
import { setSettingsPathOverride } from "@bifurc/engine/workspace/appSettings";

/** Write one more mapping into the live workspace's tree — the per-request-resolution probe needs this. */
function addMapping(ws: WorkspaceFixture, mapping: { id: string; domain: string; target: string }): void {
  const dir = path.join(ws.root, "data", TEST_WS, "mappings");
  fs.writeFileSync(
    path.join(dir, `${mapping.id}.json`),
    JSON.stringify({ ...mapping, workspaceId: TEST_WS, enabled: true, label: "", createdAt: 1 }, null, 2),
    "utf-8",
  );
  // The enabled set is a separate file, and a mapping absent from it is loaded-but-disabled — which would
  // make this probe pass for the wrong reason (the 404 staying a 404).
  const enabledPath = path.join(dir, "enabled.json");
  const enabled: string[] = JSON.parse(fs.readFileSync(enabledPath, "utf-8"));
  if (!enabled.includes(mapping.id)) enabled.push(mapping.id);
  fs.writeFileSync(enabledPath, JSON.stringify(enabled, null, 2), "utf-8");
}

describe("plan/15 WI 3 — the edge split, over real sockets", () => {
  let upstream: UpstreamServer;
  let proxy: RunningProxy;
  let ws: WorkspaceFixture;

  beforeAll(async () => {
    upstream = await startUpstream();
    ws = createWorkspace({
      mappings: [
        mappingFixture({ id: "map-mapped", domain: "mapped.localhost", target: `127.0.0.1:${upstream.port}` }),
      ],
      mocks: [
        mockFixture({
          id: "mock-hello",
          urlPattern: "http://mapped.localhost/api/mock",
          responseBody: JSON.stringify({ from: "mock" }),
          // Fully mocked: answers standalone, so neither half needs an upstream to serve it.
          mockedResponseHeaders: ["content-type"],
        }),
      ],
    });
    proxy = await startProxy();
  });

  afterAll(() => {
    proxy?.stop();
    void upstream?.close();
    ws?.cleanup();
  });

  // ── 1. Two listeners, and they are genuinely two ────────────────────────────

  describe("the two listeners", () => {
    it("reports two distinct, live ports (folded from harnessSplit)", () => {
      expect(proxy.port).toBeGreaterThan(0);
      expect(proxy.edgePort).toBeGreaterThan(0);
      expect(proxy.edgePort).not.toBe(proxy.port);
    });

    /**
     * The discriminating case, and the one a collapsed harness cannot pass.
     *
     * `/` with `Host: localhost` is the reverse half's home page: the edge answers **200**. The forward
     * proxy answers every relative request with **400**. Two ports, one URL, two answers — so this fails if
     * the harness ever binds one listener, or if the split is undone in either direction.
     */
    it("answers the SAME request differently on the two ports — the split's whole point", async () => {
      const onEdge = await edgeRequest(proxy.edgePort, { target: "/", host: "localhost" });
      const onForward = await proxyRequest(proxy.port, { target: "/", host: "localhost" });

      expect(onEdge.status).toBe(200);
      expect(onForward.status).toBe(400);
    });
  });

  // ── 2. The edge owns the reverse half ───────────────────────────────────────

  describe("the edge owns the reverse half", () => {
    it("forwards a mapped Host to its target (folded from harnessSplit)", async () => {
      const before = upstream.requests.length;
      const res = await edgeRequest(proxy.edgePort, { target: "/anything", host: "mapped.localhost" });

      expect(res.status).toBe(200);
      expect(JSON.parse(res.body)).toMatchObject({ upstream: true });
      expect(upstream.requests.length).toBe(before + 1);
    });

    it("forwards the request's own path and Host, unchanged", async () => {
      // The mapping decides *where*, not *what*: a reverse proxy that rewrote the path would break every
      // app behind it. Asserted on what the target actually received rather than on the response.
      const before = upstream.requests.length;
      await edgeRequest(proxy.edgePort, { target: "/deep/path?q=1", host: "mapped.localhost" });

      const received = upstream.requests[before];
      expect(received.url).toBe("/deep/path?q=1");
      expect(received.headers.host).toBe("mapped.localhost");
    });

    it("carries a request body through — the edge buffers it before routing", async () => {
      // `edge.ts` reads `req.on("data")` / `req.on("end")` before choosing a branch, so a body has to survive
      // the buffering. A relative target with a body is the shape a browser form post takes.
      const before = upstream.requests.length;
      const res = await edgeRequest(proxy.edgePort, {
        target: "/submit",
        host: "mapped.localhost",
        method: "POST",
        body: JSON.stringify({ hello: "edge" }),
      });

      expect(res.status).toBe(200);
      expect(upstream.requests[before].method).toBe("POST");
      expect(JSON.parse(upstream.requests[before].body)).toEqual({ hello: "edge" });
    });

    it("serves the home page for an unmapped, non-.localhost host (folded from harnessSplit)", async () => {
      const res = await edgeRequest(proxy.edgePort, { target: "/", host: "localhost" });

      expect(res.status).toBe(200);
      expect(res.body).toContain("Bifurc");
    });

    it("lists the active workspace's mappings on that page", async () => {
      // The page's whole content is the mapping table, which is why it had to move with the reverse half:
      // it builds every link from the port it is served on.
      const res = await edgeRequest(proxy.edgePort, { target: "/", host: "localhost" });

      expect(res.body).toContain("mapped.localhost");
    });

    it("404s an unmapped *.localhost domain (folded from harnessSplit)", async () => {
      const res = await edgeRequest(proxy.edgePort, { target: "/", host: "nope.localhost" });

      expect(res.status).toBe(404);
      expect(res.body).toContain("Not Mapped");
    });

    it("serves a mapped domain and the 404 as DIFFERENT rows (folded from harnessSplit)", async () => {
      const mapped = await edgeRequest(proxy.edgePort, { target: "/", host: "mapped.localhost" });
      const unmapped = await edgeRequest(proxy.edgePort, { target: "/", host: "nope.localhost" });

      expect(mapped.status).toBe(200);
      expect(unmapped.status).toBe(404);
    });

    it("serves a mock on a mapped Host without reaching the upstream", async () => {
      // `mockedResponseHeaders` makes this a fully-mocked mock, so a hit here is provably not the upstream.
      const before = upstream.requests.length;
      const res = await edgeRequest(proxy.edgePort, { target: "/api/mock", host: "mapped.localhost" });

      expect(res.status).toBe(200);
      expect(JSON.parse(res.body)).toEqual({ from: "mock" });
      expect(upstream.requests.length).toBe(before);
    });
  });

  // ── 3. The forward proxy owns the forward half, and refuses the reverse one ─

  describe("the forward proxy owns the forward half", () => {
    it("serves absolute-form traffic (folded from harnessSplit)", async () => {
      const res = await proxyRequest(proxy.port, { target: "http://api.test/thing", host: "api.test" });

      // No mapping, rule or mock for `api.test`, so this passthroughs to a host that does not exist. The
      // property is only that the **forward** half answered at all rather than refusing the request shape.
      expect(res.status).toBeGreaterThan(0);
    });

    it("serves a mock when a rule routes the absolute-form request (folded from harnessSplit)", async () => {
      // The forward half matches mocks on the absolute URL, and reaches a *mapping* only through a rule — so
      // this case needs both, which is exactly what the migrated suites do.
      const scoped = createWorkspace({
        mappings: [mappingFixture({ id: "map-api", domain: "api.localhost", target: upstream.target })],
        rules: [
          ruleFixture({
            id: "rule-map",
            pattern: "http://api.test/api/mock",
            targetType: "mapping",
            targetMappingId: "map-api",
          }),
        ],
        mocks: [
          mockFixture({
            id: "mock-scoped",
            urlPattern: "http://api.test/api/mock",
            responseBody: JSON.stringify({ from: "scoped" }),
            mockedResponseHeaders: ["content-type"],
          }),
        ],
      });
      const scopedProxy = await startProxy();
      try {
        const res = await proxyRequest(scopedProxy.port, { target: "http://api.test/api/mock", host: "api.test" });
        expect(res.status).toBe(200);
        expect(JSON.parse(res.body)).toEqual({ from: "scoped" });
      } finally {
        scopedProxy.stop();
        scoped.cleanup();
        /**
         * **Re-point the store back at this file's workspace — `cleanup()` is not local to the fixture.**
         *
         * `createWorkspace()` calls `setDataRootOverride()` / `setSettingsPathOverride()`, and its `cleanup()`
         * resets both to `null`. So creating a second fixture mid-suite silently un-points the store for every
         * case *after* it, and the symptom is a case failing for a reason that has nothing to do with its
         * subject — which is exactly what happened here: the per-request case below read a data root with no
         * workspace in it and saw the new mapping 404.
         *
         * The override is process-global state, so whoever changes it has to put it back. `harnessSplit`'s
         * version of this case got away with not doing so only because it ran last.
         */
        setDataRootOverride(ws.dataRoot);
        setSettingsPathOverride(path.join(ws.root, "app.json"));
      }
    });

    it("no longer answers the reverse half — a *.localhost request is a 400 (folded from harnessSplit)", async () => {
      // The WI 3 boundary, in the direction that catches a regression: pre-split this was a 200 from a module
      // that no longer owns the question.
      const res = await proxyRequest(proxy.port, { target: "/", host: "mapped.localhost" });

      expect(res.status).toBe(400);
    });

    it("refuses the home page too — it moved with the reverse half", async () => {
      // Easy to miss: §3.4's *"loses only the reverse branch"* reads as though `host === "localhost"` stayed.
      // It cannot — `buildHomePage()` interpolates the port it is served on, and the mappings are no longer
      // on this port.
      const res = await proxyRequest(proxy.port, { target: "/", host: "localhost" });

      expect(res.status).toBe(400);
    });

    it("names the edge in the refusal, and never a port number", async () => {
      // Naming a port would thread the edge's port into this module — the one direction of coupling the split
      // exists to remove. The refusal is discoverable instead.
      const res = await proxyRequest(proxy.port, { target: "/", host: "mapped.localhost" });

      expect(res.body).toContain("edge");
      expect(res.body).not.toContain(String(proxy.edgePort));
    });
  });

  // ── 4. The split is the port model, not a refactor ──────────────────────────

  describe("the split is the port model, not a refactor", () => {
    it("re-reads resolveConfig per request — a new mapping is honoured with no restart", async () => {
      /**
       * **This is the case the `resolveConfig`-as-a-function design exists for**, and it is a statement about
       * *time*: `edge.ts` records that re-resolving per request is what makes `workspace.setActive` need no
       * invalidation step. A cached config would keep serving the old mapping table until something
       * remembered to tell the edge — and there is nothing that would.
       *
       * ## Why this builds its own edge instead of reusing `proxy`
       *
       * **`resolveConfig` is a parameter, and what it returns is the host's decision — the two compositions
       * in the tree differ, and the difference is worth pinning rather than papering over:**
       *
       *   - the **engine facade** (`index.ts:286`) passes `() => loadConfig()`, so every request re-reads
       *     `app.json` and the workspace's entity files;
       *   - the **multi-workspace host** (`host.ts`) passes `() => instance.dispatchConfig()`, so every request
       *     re-reads the *instance's* config — which `start()` / `reloadConfig()` set and nothing else does.
       *
       * The harness's `startProxy()` mirrors the host's shape (deliberately — see its header), and this case
       * first asserted the facade's behaviour against it. It failed, correctly: `dispatchConfig()` returned
       * the mapping table `start()` had cached, and the new domain stayed a 404. **The failure was the test's
       * premise, not the code's** — so the case now builds the facade's composition, which is the one whose
       * per-request semantics are the claim being made.
       *
       * The probe writes a mapping file *after* the edge has started and makes a request, with no `stop()` /
       * `start()` and no `reloadConfig()` in between. The first assertion pins the precondition (the domain
       * 404s before the write), so the second cannot pass for the wrong reason.
       */
      const facadeEdge = createEdgeRouter({ resolveConfig: () => loadConfig() });
      const facadePort = await getFreePort();
      facadeEdge.start(facadePort);
      try {
        await expect.poll(() => facadeEdge.isRunning(), { timeout: 5000 }).toBe(true);

        const before = await edgeRequest(facadePort, { target: "/", host: "late.localhost" });
        expect(before.status).toBe(404);

        addMapping(ws, { id: "map-late", domain: "late.localhost", target: upstream.target });

        const after = await edgeRequest(facadePort, { target: "/", host: "late.localhost" });
        expect(after.status).toBe(200);
        expect(JSON.parse(after.body)).toMatchObject({ upstream: true });
      } finally {
        facadeEdge.stop();
      }
    });

    /**
     * ## The other composition, and the difference is real rather than incidental
     *
     * `resolveConfig` is a **parameter**, so "per request" describes *when the function is called*, not what
     * it returns — and the two compositions in the tree return different things:
     *
     *   - the **engine facade** (`index.ts:286`) passes `() => loadConfig()`, so every request re-reads the
     *     files. Pinned by the case above.
     *   - the **multi-workspace host** (`host.ts`) passes `() => instance.dispatchConfig()`, and
     *     `dispatchConfig()` composes the four inputs the instance loaded at `start()` — refreshed only by
     *     `reloadConfig()` (`server.ts:113` says so, and this case is what makes that a measurement rather
     *     than a quotation).
     *
     * **This case exists because the assertion above initially failed against the host's shape and the
     * obvious reading of that failure was wrong.** The cause was a *third* thing entirely — a sibling case's
     * `createWorkspace()` had reset the store override, so the request never saw the file at all. A reader who
     * had taken the first failure at face value would have "fixed" `dispatchConfig()` to re-read on every
     * request, changing D16's resolved-port semantics to satisfy a test that was measuring the wrong
     * workspace. So the two shapes are now pinned *separately*, and each says which one it is.
     *
     * The `reloadConfig()` step is the point, not a convenience: it is the documented way a host's instance
     * picks up a change, and the case fails if the host's edge ever becomes fresh without one (or if the
     * facade's ever stops being).
     */
    it("needs an explicit reloadConfig() on the HOST-shaped edge — the other composition", async () => {
      // `proxy` is the harness's edge, wired the host's way (`() => instance.dispatchConfig()`).
      const before = await edgeRequest(proxy.edgePort, { target: "/", host: "hostlate.localhost" });
      expect(before.status).toBe(404);

      addMapping(ws, { id: "map-hostlate", domain: "hostlate.localhost", target: upstream.target });

      // Still a 404: `dispatchConfig()` composes what `start()` loaded, and no reload has happened.
      const withoutReload = await edgeRequest(proxy.edgePort, { target: "/", host: "hostlate.localhost" });
      expect(withoutReload.status).toBe(404);

      // The documented refresh — and now it is served, with no restart.
      proxy.instance.reloadConfig();

      const afterReload = await edgeRequest(proxy.edgePort, { target: "/", host: "hostlate.localhost" });
      expect(afterReload.status).toBe(200);
      expect(JSON.parse(afterReload.body)).toMatchObject({ upstream: true });
    });

    it("keeps the edge serving when the forward proxy stops — the teardown is asymmetric", async () => {
      /**
       * `server.stop` is a **proxy** lifecycle command; the edge is the host's and outlives any one workspace's
       * proxy. Nothing in the desktop exposes an `edge.stop`, and `runtime/host.ts` tears the edge down in
       * `host.stop()` — a host-level operation.
       *
       * A regression that reached for a shared teardown would take every `*.localhost` domain down with one
       * workspace's proxy, and the user-visible symptom would be "stopping the proxy broke all my mappings".
       * The restart at the end is what keeps this case from leaving the harness in a stopped state for the
       * ones after it.
       */
      proxy.instance.stop();

      const res = await edgeRequest(proxy.edgePort, { target: "/", host: "mapped.localhost" });
      expect(res.status).toBe(200);

      proxy.instance.start(proxy.port);
    });
  });

});
