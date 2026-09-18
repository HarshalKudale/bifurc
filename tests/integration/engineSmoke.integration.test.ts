/**
 * The P2 work item 8 acceptance criterion, exercised end to end:
 *
 *   "Engine starts from a bare Node script with `--data-dir`, serves, and shuts down cleanly."
 *
 * This suite runs in plain Node — no Electron, no renderer, no shell. It calls the package's real
 * public entry point, `createEngine()`, against a real temp data directory, then makes a real HTTP
 * request through the proxy it started and shuts it down.
 *
 * WHY IT IS ONE BIG `it()` RATHER THAN SEVERAL SMALL ONES
 * ------------------------------------------------------
 * `shutdownEngine()` is **memoised by design** (P2 work item 3) — the shell quitting, a supervisor
 * restarting a crashed engine and a SIGTERM handler may all ask for teardown, in any order, and
 * only the first may do work. The consequence is that a process can only ever run the start→stop
 * cycle *once*. Splitting these assertions across tests would therefore require `vi.resetModules()`
 * between them, which would hand each test a fresh copy of the store's module-level state and stop
 * it testing the thing that actually ships. So: one engine, one lifecycle, all assertions inside it.
 *
 * WHY `LOCALAPPDATA` IS OVERRIDDEN
 * --------------------------------
 * On Windows, `workspaceFs.dataRoot()` and `appSettings.settingsPath()` check `%LOCALAPPDATA%`
 * *before* consulting the data root, so `opts.dataDir` alone would send this test's writes to the
 * developer's real `%LOCALAPPDATA%\Bifurc`. `e2e/fixtures/electronApp.ts` works around the same
 * branch the same way. On macOS and Linux the branch does not exist and `dataDir` is used directly.
 */

import { describe, it, expect, beforeAll, afterAll, vi } from "vitest";
import * as fs from "fs";
import * as os from "os";
import * as path from "path";
import * as http from "http";
import * as net from "net";

// `startAutoSync` would otherwise leave a real 30s poller timer running and hold the process open.
// Everything else runs for real: real temp data root, real directories, real `git init`, real
// sockets. Mocking the store layer here would test nothing — the point of the extraction is that
// the same on-disk effects still happen.
vi.mock("@bifurc/engine/sync/autoSync", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@bifurc/engine/sync/autoSync")>()),
  startAutoSync: vi.fn(),
}));

import { createEngine, createInProcessTransport } from "@bifurc/engine";
import { setDataRoot, resetDataRootForTests } from "@bifurc/engine/store/paths";
import { loadSettings, saveSettings } from "@bifurc/engine/store/appSettings";
import { commandRegistry } from "@bifurc/engine/commands/registry";
import { bus } from "@bifurc/engine/eventBus";
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

/** True when something can bind the port — used to prove teardown actually released it. */
function portIsFree(port: number): Promise<boolean> {
  return new Promise((resolve) => {
    const srv = net.createServer();
    srv.once("error", () => resolve(false));
    srv.listen(port, "127.0.0.1", () => srv.close(() => resolve(true)));
  });
}

describe("createEngine — the bare-Node acceptance criterion", () => {
  let root: string;
  let originalLocalAppData: string | undefined;

  beforeAll(() => {
    root = fs.mkdtempSync(path.join(os.tmpdir(), "bifurc-engine-smoke-"));
    // See the header: required on Windows, harmless elsewhere.
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

  it("starts headless, serves over a real socket, and shuts down cleanly", async () => {
    const proxyPort = await getFreePort();
    const companionPort = await getFreePort();

    // Persist the two ports we want, then let the engine read them back through its own bootstrap.
    // Written with the production `saveSettings`, not a fixture, so the test exercises the same
    // path a real CLI would.
    setDataRoot(root);
    saveSettings({ ...loadSettings(), port: proxyPort, companionPort });

    const warnings: string[] = [];
    const engine = createEngine({
      dataDir: root,
      // Preflight is advisory by design; collect it so an unexpected finding is visible in the
      // assertion below rather than printed and lost.
      onPreflightWarning: (c) => warnings.push(c.code ?? "unknown"),
    });

    // ── status() is safe before start() ──────────────────────────────────────
    expect(engine.status().running).toBe(false);
    expect(engine.status().dataDir).toBe(root);

    // ── start() ──────────────────────────────────────────────────────────────
    const status = await engine.start();
    expect(status.running).toBe(true);
    expect(status.proxyPort).toBe(proxyPort);
    expect(status.companionPort).toBe(companionPort);
    expect(status.proxyError).toBeNull();
    // The engine bootstrapped a workspace to serve — proof bootstrapWorkspaces() ran.
    expect(status.settings.activeWorkspaceId).toBeTruthy();
    expect(status.settings.workspaces.length).toBeGreaterThan(0);

    // The workspace really is on disk under the data root we asked for.
    expect(fs.existsSync(root)).toBe(true);

    // ── the registry and bus are the engine's own singletons, not copies ─────
    // Identity, not population: `createEngine()` deliberately does not register any commands. The
    // ~112 commands are registered by the *consumer* — today that is the shell's in-process
    // registration layer (`src/ipc/handlers.ts` and friends), which P6 replaces with the RPC
    // client. Whoever dispatches is responsible for registering, and the engine only guarantees
    // that the registry it hands out is the one shared singleton.
    expect(engine.registry).toBe(commandRegistry);
    expect(typeof engine.registry.invoke).toBe("function");
    expect(typeof engine.registry.isRegistered).toBe("function");
    expect(engine.bus).toBe(bus);
    expect(typeof engine.bus.emitTyped).toBe("function");

    // ── it actually serves ───────────────────────────────────────────────────
    const res = await get(proxyPort);
    expect(res.status).toBe(200);
    expect(res.body).toContain("Bifurc");

    // ── P4 item 3: one event log, and it outlives the session that filled it ──
    //
    // The acceptance criterion is "replay works inside the buffer", and the property that makes it
    // work is that `seq` belongs to the **engine**, not to a session. A reconnect is a new session, so
    // a per-session counter would make a client's `lastSeq` refer to events from a session that no
    // longer exists. Exercised here, through the public API and two real transports, because that
    // scope is the one thing about replay that cannot be seen from inside a single session's tests.
    expect(engine.log.newestSeq).toBe(0);
    expect(engine.log.retainedNames()).toEqual([]); // retention is lazy: nothing subscribed yet

    const before = createInProcessTransport(engine.registry, { bus: engine.bus, log: engine.log });
    const gapFiller: string[] = [];
    const off = before.subscribe(["event.server.error"], (e) => gapFiller.push(String(e.payload)));
    engine.bus.emitTyped("server.error", "while-connected");
    await vi.waitFor(() => expect(gapFiller).toEqual(["while-connected"]));

    // The blip: the session goes away and the engine keeps working.
    off();
    await before.close();
    engine.bus.emitTyped("server.error", "during-the-blip-1");
    engine.bus.emitTyped("server.error", "during-the-blip-2");
    expect(engine.log.newestSeq).toBe(3);

    // The reconnect: a **new** transport, a new session, and the engine's own seqs in the backlog.
    const after = createInProcessTransport(engine.registry, { bus: engine.bus, log: engine.log });
    const replayed: { seq: number; payload: unknown }[] = [];
    after.subscribe(["event.server.error"], (e) => replayed.push({ seq: e.seq, payload: e.payload }));

    // `replayFrom(1)` because the client saw up to seq 1 before the blip — the number it would put on
    // `hello.lastSeq`. A per-session counter would have restarted at 1 here and replayed the wrong
    // two events, or none at all.
    const outcome = engine.log.replayFrom(1, ["event.server.error"]);
    expect(outcome.kind === "replay" && outcome.events.map((e) => e.payload)).toEqual([
      "during-the-blip-1",
      "during-the-blip-2",
    ]);
    expect(outcome.kind === "replay" && outcome.events.map((e) => e.seq)).toEqual([2, 3]);
    await after.close();

    // ── start() is idempotent ────────────────────────────────────────────────
    const second = await engine.start();
    expect(second.proxyPort).toBe(proxyPort);
    expect(await portIsFree(proxyPort)).toBe(false); // still bound exactly once

    // ── stop() ───────────────────────────────────────────────────────────────
    await engine.stop();
    expect(engine.status().running).toBe(false);
    expect(engine.status().proxyPort).toBeNull();

    // The socket is genuinely released, not just flagged as stopped.
    await expect.poll(() => portIsFree(proxyPort), { timeout: 5000 }).toBe(true);

    // ── stop() is idempotent ─────────────────────────────────────────────────
    await expect(engine.stop()).resolves.toBeUndefined();

    // ── the engine is single-use, and says so instead of silently half-working
    await expect(engine.start()).rejects.toThrow(/already been stopped/i);

    // No unexpected preflight findings on a clean temp dir with free ports. `mkcert-unusable` is
    // tolerated: it depends on the host having mkcert, which CI images may not.
    expect(warnings.filter((w) => w !== "mkcert-unusable")).toEqual([]);
  }, 30000);
});
