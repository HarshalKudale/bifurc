/**
 * P14 WI 3 — the workspace-host startup shape: `EngineOptions.startCompanion: false`.
 *
 * WHY THIS IS ITS OWN FILE AND NOT A SECOND CASE IN `engineSmoke.integration.test.ts`
 * ----------------------------------------------------------------------------------
 * `shutdownEngine()` is **memoised by design** (P2 work item 3) and the engine is *single-use per
 * process* — `start()` after `stop()` throws rather than appearing to work. That memoisation is
 * process-wide, not per engine instance, so the **second** engine in a process gets a
 * `shutdownEngine()` that has already resolved: its `stop()` awaits a settled promise, `stopServer()`
 * never runs, and the proxy port stays bound. Measured 2026-09-21 — adding this case to
 * `engineSmoke` made the file fail on `portIsFree(proxyPort)` after `stop()`, and the case passed
 * alone. The engineSmoke header already documents the constraint ("a process can only ever run the
 * start→stop cycle *once*"); the fix is one lifecycle file per engine shape, not `vi.resetModules()`,
 * which would hand the test a fresh copy of the store's module-level state and stop it testing what
 * ships.
 *
 * WHAT IS ACTUALLY UNDER TEST
 * ---------------------------
 * `plan/14`'s WI 3 criterion is *"a workspace host starts with no companion server."* So the
 * load-bearing assertions are that the companion port is genuinely never bound **and** that
 * `status().companionPort` reports `null` — a stale `getCompanionPort()` reading would let a host
 * believe it had a bridge it never opened. The proxy must be fully working in the same run, or the
 * test would pass for the wrong reason (an engine that failed to start at all also has no companion).
 *
 * WHY `LOCALAPPDATA` IS OVERRIDDEN
 * --------------------------------
 * On Windows, `workspaceFs.dataRoot()` and `appSettings.settingsPath()` check `%LOCALAPPDATA%`
 * *before* consulting the data root, so `opts.dataDir` alone would send this test's writes to the
 * developer's real `%LOCALAPPDATA%\Bifurc`. Same workaround as `e2e/fixtures/electronApp.ts`.
 */

import { describe, it, expect, beforeAll, afterAll, vi } from "vitest";
import * as fs from "fs";
import * as os from "os";
import * as path from "path";
import * as http from "http";
import * as net from "net";

// `startAutoSync` would otherwise leave a real 30 s poller timer running and hold the process open.
vi.mock("@bifurc/engine/sync/autoSync", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@bifurc/engine/sync/autoSync")>()),
  startAutoSync: vi.fn(),
}));

import { createEngine } from "@bifurc/engine";
import { setDataRoot, resetDataRootForTests } from "@bifurc/engine/store/paths";
import { loadSettings, saveSettings } from "@bifurc/engine/store/appSettings";
import { getFreePort } from "./proxyHarness";

/**
 * Connect to 127.0.0.1 but present `localhost` as the Host, which is what the proxy's own UI is
 * served under. Requesting `127.0.0.1` directly gets a 400 "Not a localhost domain" — the proxy
 * routes on Host, so the header is part of the request, not incidental.
 */
function get(port: number): Promise<{ status: number; body: string }> {
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

/** True when something can bind the port — used to prove a port was never bound, and that a bound one was released. */
function portIsFree(port: number): Promise<boolean> {
  return new Promise((resolve) => {
    const srv = net.createServer();
    srv.once("error", () => resolve(false));
    srv.listen(port, "127.0.0.1", () => srv.close(() => resolve(true)));
  });
}

describe("createEngine with startCompanion: false — the workspace-host shape", () => {
  let root: string;
  let originalLocalAppData: string | undefined;

  beforeAll(() => {
    root = fs.mkdtempSync(path.join(os.tmpdir(), "bifurc-host-startup-"));
    originalLocalAppData = process.env.LOCALAPPDATA;
    process.env.LOCALAPPDATA = root;
  });

  afterAll(() => {
    if (originalLocalAppData === undefined) delete process.env.LOCALAPPDATA;
    else process.env.LOCALAPPDATA = originalLocalAppData;
    resetDataRootForTests();
    try {
      fs.rmSync(root, { recursive: true, force: true });
    } catch {
      /* best-effort */
    }
  });

  it("starts the proxy but never binds the companion port", async () => {
    const proxyPort = await getFreePort();
    const companionPort = await getFreePort();

    setDataRoot(root);
    saveSettings({ ...loadSettings(), port: proxyPort, companionPort });

    const warnings: string[] = [];
    const engine = createEngine({
      dataDir: root,
      startCompanion: false,
      onPreflightWarning: (c) => warnings.push(c.code ?? "unknown"),
    });

    const status = await engine.start();

    // The proxy is up — this is a working engine, not a disabled one.
    expect(status.running).toBe(true);
    expect(status.proxyPort).toBe(proxyPort);
    expect(status.proxyError).toBeNull();

    // And the companion reports nothing, rather than a port it never bound.
    expect(status.companionPort).toBeNull();
    // The port is genuinely free — proof the server was not started and then merely hidden.
    expect(await portIsFree(companionPort)).toBe(true);
    // Preflight was not asked about a port this engine deliberately never opens.
    expect(warnings.filter((w) => w !== "mkcert-unusable")).toEqual([]);

    // It still serves, so the runtime half of the engine is untouched by the option.
    const res = await get(proxyPort);
    expect(res.status).toBe(200);
    expect(res.body).toContain("Bifurc");

    await engine.stop();
    await expect.poll(() => portIsFree(proxyPort), { timeout: 5000 }).toBe(true);
    // Still free after the whole lifecycle — `shutdownEngine()`'s `stopCompanionServer()` is
    // null-guarded, so it must not have bound anything on the way out either.
    expect(await portIsFree(companionPort)).toBe(true);
    expect(engine.status().companionPort).toBeNull();
  }, 30000);
});
