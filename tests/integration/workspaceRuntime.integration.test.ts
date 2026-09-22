/**
 * P14 WI 3 — `createWorkspaceRuntime()`, exercised with real I/O.
 *
 * WHY THIS IS AN INTEGRATION FILE AND NOT A UNIT ONE
 * --------------------------------------------------
 * The runtime's entire value proposition is that **two of them coexist in one process**: separate
 * ports, separate listening state, separate errors, and — the part `plan/14` §8 names as the criterion
 * — *"starting runtime B does not observe runtime A's readiness."* A symmetric `vi.mock("net")` harness
 * cannot show that: it hands every `createServer()` call the same object (the exact fixture artifact
 * that produced two false failures while writing `tests/proxy/server.test.ts`'s isolation cases). So
 * this file boots real servers on real sockets against a real two-workspace data root, and asserts on
 * bytes that came back over the wire.
 *
 * WHY TWO WORKSPACES, BUILT BY HAND RATHER THAN BY `proxyHarness.createWorkspace()`
 * -------------------------------------------------------------------------------
 * The harness writes exactly one workspace (its `TEST_WS`) and points the store at it. A runtime per
 * workspace is the thing under test, so this file writes its own two-workspace tree — the format is
 * the harness's, deliberately, but the cardinality is the point.
 *
 * WHY THE HOOKS ARE `setDataRootOverride` / `setSettingsPathOverride`
 * -------------------------------------------------------------------
 * The production store exposes both precisely so a test can point it at a throwaway directory; nothing
 * here is a test-only code path.
 */

import { describe, it, expect, beforeAll, afterAll } from "vitest";
import * as fs from "fs";
import * as os from "os";
import * as path from "path";
import * as http from "http";
import * as net from "net";

import { setDataRootOverride } from "@bifurc/engine/store/workspaceFs";
import { setSettingsPathOverride } from "@bifurc/engine/store/appSettings";
import { createWorkspaceRuntime } from "@bifurc/engine/runtime/workspaceRuntime";
import { getFreePort } from "./proxyHarness";

const WS_A = "ws-a";
const WS_B = "ws-b";

/**
 * Write one workspace's directory tree. Mirrors `proxyHarness.createWorkspace()`'s layout, narrowed to
 * what a booting proxy actually reads: the workspace dir, its `workspace.json`, and the three entity
 * dirs with `enabled.json` so `bootstrapEnabledSet()` finds an empty-but-valid set rather than a
 * missing file.
 */
function writeWorkspace(dataRoot: string, wsId: string, name: string): void {
  const dir = path.join(dataRoot, wsId);
  for (const sub of ["mappings", "rules", "mocks", "environments", "requests", "sockets", "webhooks", "capture"]) {
    fs.mkdirSync(path.join(dir, sub), { recursive: true });
  }
  fs.writeFileSync(
    path.join(dir, "workspace.json"),
    JSON.stringify(
      { id: wsId, name, createdAt: 1704067200000, activeEnvironmentId: null },
      null,
      2,
    ),
    "utf-8",
  );
  for (const kind of ["mappings", "rules", "mocks"]) {
    fs.writeFileSync(path.join(dir, kind, "enabled.json"), JSON.stringify([], null, 2), "utf-8");
  }
  fs.writeFileSync(
    path.join(dir, "rules", "names.json"),
    JSON.stringify({}, null, 2),
    "utf-8",
  );
  fs.writeFileSync(
    path.join(dir, "rules", "index.json"),
    JSON.stringify({ folders: [], order: [] }, null, 2),
    "utf-8",
  );
  fs.writeFileSync(path.join(dir, "mocks", "index.json"), JSON.stringify({ folders: [], order: [] }, null, 2), "utf-8");
  fs.writeFileSync(
    path.join(dir, "environments", "__global__.json"),
    JSON.stringify({ id: "__global__", name: "Global", variables: [], createdAt: 0, workspaceId: wsId }, null, 2),
    "utf-8",
  );
}

/** GET the proxy's own UI page, presenting `localhost` as Host — how the proxy routes to its home page. */
function getHomePage(port: number): Promise<{ status: number; body: string }> {
  return new Promise((resolve, reject) => {
    const req = http.get(
      { host: "127.0.0.1", port, path: "/", headers: { host: `localhost:${port}` } },
      (res) => {
        let body = "";
        res.setEncoding("utf8");
        res.on("data", (c) => (body += c));
        res.on("end", () => resolve({ status: res.statusCode ?? 0, body }));
      },
    );
    req.on("error", reject);
    req.setTimeout(5000, () => req.destroy(new Error("request timed out")));
  });
}

/** True when something can bind the port — proves a port is genuinely released (or never taken). */
function portIsFree(port: number): Promise<boolean> {
  return new Promise((resolve) => {
    const srv = new net.Server();
    srv.once("error", () => resolve(false));
    srv.listen(port, "127.0.0.1", () => srv.close(() => resolve(true)));
  });
}

describe("createWorkspaceRuntime — two runtimes in one process, real sockets", () => {
  let root: string;
  let dataRoot: string;

  beforeAll(() => {
    root = fs.mkdtempSync(path.join(os.tmpdir(), "bifurc-runtime-it-"));
    dataRoot = path.join(root, "data");
    writeWorkspace(dataRoot, WS_A, "Workspace A");
    writeWorkspace(dataRoot, WS_B, "Workspace B");

    // A tiny app.json so the store resolves; the runtime does not read the port from here (the host
    // allocates), but `loadConfig()` runs inside `proxy.start()` and expects the file to exist.
    fs.writeFileSync(
      path.join(root, "app.json"),
      JSON.stringify(
        {
          port: 8080,
          webhookPort: 9101,
          companionPort: 9271,
          minimizeToTray: true,
          tlsEnabled: false,
          tlsCaCertPath: null,
          tlsCaKeyPath: null,
          workspaces: [
            { id: WS_A, name: "Workspace A", activeEnvironmentId: null },
            { id: WS_B, name: "Workspace B", activeEnvironmentId: null },
          ],
          activeWorkspaceId: WS_A,
          hasSeenWelcome: true,
          zoomLevel: 0,
          zoomLevelSetByUser: false,
          themeId: null,
        },
        null,
        2,
      ),
      "utf-8",
    );

    setDataRootOverride(dataRoot);
    setSettingsPathOverride(path.join(root, "app.json"));
  });

  afterAll(() => {
    setDataRootOverride(null);
    setSettingsPathOverride(null);
    try {
      fs.rmSync(root, { recursive: true, force: true });
    } catch {
      /* best-effort */
    }
  });

  it("runs two workspaces side by side, each serving its own proxy, and stops independently", async () => {
    const [proxyA, proxyB, webhookA, webhookB] = await Promise.all([
      getFreePort(),
      getFreePort(),
      getFreePort(),
      getFreePort(),
    ]);

    const a = createWorkspaceRuntime({ workspaceId: WS_A, proxyPort: proxyA, webhookPort: webhookA });
    const b = createWorkspaceRuntime({ workspaceId: WS_B, proxyPort: proxyB, webhookPort: webhookB });

    // ── status() is safe before start() ───────────────────────────────────────
    expect(a.status().running).toBe(false);
    expect(a.status().proxyPort).toBeNull();
    expect(a.status().workspaceId).toBe(WS_A);

    // ── both start, and each reports only its own state ───────────────────────
    const [statusA, statusB] = await Promise.all([a.start(), b.start()]);

    expect(statusA.running).toBe(true);
    expect(statusA.proxyPort).toBe(proxyA);
    expect(statusA.proxyError).toBeNull();
    expect(statusA.webhookRunning).toBe(true);
    expect(statusA.webhookPort).toBe(webhookA);

    expect(statusB.running).toBe(true);
    expect(statusB.proxyPort).toBe(proxyB);
    expect(statusB.proxyError).toBeNull();
    expect(statusB.webhookRunning).toBe(true);
    expect(statusB.webhookPort).toBe(webhookB);

    // The identity criterion: neither observed the other's ports or readiness. A shared module
    // global would make both of these equal, which is the trap WI 2's trap entry names.
    expect(statusA.proxyPort).not.toBe(statusB.proxyPort);
    expect(statusA.webhookPort).not.toBe(statusB.webhookPort);

    // ── both are up and answering over the wire ───────────────────────────────
    //
    // **This is a LIVENESS check, not the "each serving its own rules" criterion.** The home page is served
    // regardless of any rule, mock or mapping, so it cannot fail for a config-scoping defect — and for a
    // while it was cited as though it could, which is why `plan/14` §8's first criterion read as met while a
    // runtime serving a non-active workspace served nothing at all (closed 2026-09-22). **The content half
    // lives in `tests/integration/workspaceDispatchScope.integration.test.ts`**, which asserts each runtime
    // serves its own mock and its own rule and *not* the other's.
    const [homeA, homeB] = await Promise.all([getHomePage(proxyA), getHomePage(proxyB)]);
    expect(homeA.status).toBe(200);
    expect(homeA.body).toContain("Bifurc");
    expect(homeB.status).toBe(200);
    expect(homeB.body).toContain("Bifurc");

    // ── stopping A releases A's ports and leaves B listening ──────────────────
    await a.stop();

    expect(a.status().running).toBe(false);
    expect(a.status().proxyPort).toBeNull();
    // B never noticed.
    expect(b.status().running).toBe(true);
    expect(b.status().proxyPort).toBe(proxyB);
    await expect.poll(() => portIsFree(proxyA), { timeout: 5000 }).toBe(true);
    await expect.poll(() => portIsFree(webhookA), { timeout: 5000 }).toBe(true);
    expect(await portIsFree(proxyB)).toBe(false); // still bound — exactly once

    // And B still answers over the wire after A's teardown.
    const homeBAfter = await getHomePage(proxyB);
    expect(homeBAfter.status).toBe(200);

    // ── stop() is idempotent; start() after stop() is refused, not silently half-working ──
    await expect(b.stop()).resolves.toBeUndefined();
    await expect(b.stop()).resolves.toBeUndefined();
    await expect(b.start()).rejects.toThrow(/already been stopped/i);

    await expect.poll(() => portIsFree(proxyB), { timeout: 5000 }).toBe(true);
    await expect.poll(() => portIsFree(webhookB), { timeout: 5000 }).toBe(true);
  }, 30000);

  it("reports a bind failure on the losing runtime only, without throwing from start()", async () => {
    const taken = await getFreePort();
    const proxyB = await getFreePort();
    const webhookA = await getFreePort();
    const webhookB = await getFreePort();

    // Hold the port so A's proxy bind genuinely fails.
    const blocker = new net.Server();
    await new Promise<void>((resolve, reject) => {
      blocker.once("error", reject);
      blocker.listen(taken, "127.0.0.1", () => resolve());
    });

    const a = createWorkspaceRuntime({ workspaceId: WS_A, proxyPort: taken, webhookPort: webhookA });
    const b = createWorkspaceRuntime({ workspaceId: WS_B, proxyPort: proxyB, webhookPort: webhookB });

    try {
      // Neither start rejects: a busy port is reported, not thrown — the pre-P14 contract.
      const statusA = await a.start();
      const statusB = await b.start();

      expect(statusA.running).toBe(false);
      expect(statusA.proxyError).toMatch(/in use|EADDRINUSE/i);
      expect(statusA.proxyPort).toBeNull();

      // B is unaffected by A's failure — the cross-runtime half of the criterion.
      expect(statusB.running).toBe(true);
      expect(statusB.proxyPort).toBe(proxyB);
      expect(statusB.proxyError).toBeNull();

      // A's webhook still came up: the two servers in one runtime are independent.
      expect(statusA.webhookRunning).toBe(true);
      expect(statusA.webhookPort).toBe(webhookA);
    } finally {
      await a.stop();
      await b.stop();
      await new Promise<void>((resolve) => blocker.close(() => resolve()));
    }
  }, 30000);
});
