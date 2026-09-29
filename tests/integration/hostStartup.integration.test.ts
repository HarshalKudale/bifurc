/**
 * P14 WI 3 — the workspace-host startup shape.
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
 * `plan/14`'s WI 3 criterion, as amended 2026-09-23: *"a workspace host starts with no app-level
 * singleton server."*
 *
 * **This file used to assert the criterion by its old wording** — `startCompanion: false`, a free
 * port that stays free, and `status().companionPort === null`. The companion server and its port
 * were **retired outright** (not relocated) while implementing `plan/16`, so `startCompanion` and
 * `companionPort` no longer exist: `EngineOptions` has no such option and `EngineStatus` has no such
 * field. A test asserting them would not compile, and — worse — a test asserting their *absence* is
 * now vacuous, because there is nothing left that could be absent.
 *
 * So the option and the port field are simply gone from this file. What survives, and is what the
 * criterion was always *for*, is the assertion underneath the old ones: **the engine's own
 * app-level singleton server is the edge router, and a lifecycle's placement of it is real binds**,
 * not a status flag. `status().edgePort` therefore has to be reported while the engine serves, and
 * the edge's socket has to be genuinely released by `stop()` — the same "prove it with a real bind"
 * standard the old companion port was held to, applied to the server that actually exists.
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
vi.mock("@bifurc/engine/gitops/autoSync", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@bifurc/engine/gitops/autoSync")>()),
  startAutoSync: vi.fn(),
}));

import { createEngine } from "@bifurc/engine";
import { setDataRoot, resetDataRootForTests } from "@bifurc/engine/workspace/paths";
import { loadSettings, saveSettings } from "@bifurc/engine/workspace/appSettings";
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

describe("createEngine — the workspace-host startup shape (P14 WI 3, as amended)", () => {
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

  it("starts the proxy and the edge, and releases both on stop", async () => {
    // Three distinct ports, de-duped explicitly. `getFreePort()` binds 0 and reads the assignment
    // back, so what it returns is a *reservation*, not a guarantee — and nothing makes three
    // back-to-back calls distinct. The sibling files that need this de-dup the same way: two ports
    // in `proxyHarness.ts:435`, three in `settingsMutations.integration.test.ts:325-328`. A collision
    // would surface as an `EADDRINUSE` that reads like a busy machine rather than a defect here.
    const proxyPort = await getFreePort();
    let edgePort = await getFreePort();
    while (edgePort === proxyPort) edgePort = await getFreePort();
    let hookPort = await getFreePort();
    while (hookPort === proxyPort || hookPort === edgePort) hookPort = await getFreePort();

    setDataRoot(root);
    // `port` is the edge's (`plan/15` §6.2), `webhookPort` the webhook receiver's, `forwardProxyPort`
    // the forward proxy's. The last was omitted here on the belief that it was "not settable" — it is
    // (`runtime/host.ts:474` reads it straight off settings), and leaving it out made this test depend
    // on the machine's 8080 being free. Measured 2026-09-28: with a running `electron.exe` (the app
    // itself) holding 80 and 8080, the proxy failed to bind and `status.running` was false. Pinning it
    // makes the test hermetic *and* gives the release assertion below its subject — `portIsFree(proxyPort)`
    // was checking a port nothing had bound.
    saveSettings({ ...loadSettings(), port: edgePort, webhookPort: hookPort, forwardProxyPort: proxyPort });

    const warnings: string[] = [];
    const engine = createEngine({
      dataDir: root,
      onPreflightWarning: (c) => warnings.push(c.code ?? "unknown"),
    });

    const status = await engine.start();

    // The proxy is up — this is a working engine, not a disabled one.
    expect(status.running).toBe(true);
    expect(status.proxyError).toBeNull();

    // The engine's app-level singleton server is the edge, and it is really listening. Asserted as a
    // port that is **bound**, not merely reported: a stale `getPort()` reading would let a caller
    // believe a router it never opened was serving every mapping on the host.
    expect(status.edgePort).toBe(edgePort);
    expect(status.edgeError).toBeNull();
    expect(await portIsFree(edgePort)).toBe(false);
    // Preflight was not asked about a port this engine does not open.
    expect(warnings.filter((w) => w !== "mkcert-unusable")).toEqual([]);

    // It still serves, so the runtime half of the engine is untouched. The request goes to the **edge**,
    // not `proxyPort`: `plan/15` WI 3 moved the home page onto the edge with the rest of the reverse half,
    // so a bare `GET /` is the edge's row 5. Asking the forward proxy for it is a deliberate 400.
    const res = await get(edgePort);
    expect(res.status).toBe(200);
    expect(res.body).toContain("Bifurc");

    await engine.stop();
    await expect.poll(() => portIsFree(proxyPort), { timeout: 5000 }).toBe(true);
    // And the edge is genuinely released rather than flagged stopped — `shutdownEngine()`'s
    // `edge.stop()` is what makes this true, and a null-guarded no-op would leave the socket bound.
    await expect.poll(() => portIsFree(edgePort), { timeout: 5000 }).toBe(true);
    expect(engine.status().edgePort).toBeNull();
  }, 30000);
});
