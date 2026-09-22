/**
 * P14 §8 criterion 18 — `runWorkspaceHost()`'s **signal** path.
 *
 * ## Why this file exists, and why the instrument it replaces was not enough
 *
 * `runtime/host.ts:370-401` wires `SIGTERM`/`SIGINT` → `host.stop()` → `process.exit(143 | 130)`. The audit
 * of §8 found that the criterion *read* as met because `tests/shutdown.test.ts` was cited for it — and that
 * file exercises **`shutdownEngine()`**, a different entry point entirely. So the mechanism existed with
 * nothing proving it ran. That is the same shape as criterion 1, where a liveness assertion stood in for a
 * content one: **an instrument answering a neighbouring question.**
 *
 * ## What makes this a test of the real thing rather than of a mock
 *
 * It calls `runWorkspaceHost()` itself and then invokes **the listener that function actually registered**
 * with `process.on`, discovered by diffing `process.listeners("SIGTERM")` across the call. A test that
 * called a hand-rolled `shutdown()` would be asserting a function nobody ships; a test that called
 * `process.emit("SIGTERM")` would also fire every unrelated listener in the worker.
 *
 * ## The two things it is careful about
 *
 * 1. **`process.exit` is stubbed.** Unstubbed, the teardown would kill the worker mid-file and the failure
 *    would read as a crash rather than as an assertion.
 * 2. **The listeners are removed again.** `process.on` in a function under test leaks by construction, so
 *    leaving them registered would give the *next* case a second, stale teardown.
 *
 * ## What is asserted, and what deliberately is not
 *
 * The criterion says "stops every runtime's servers **and flushes**". The stop half is asserted here by
 * **real binds**: each runtime's proxy and webhook ports must be free after the signal. The flush half is
 * not asserted — the event log's flush is `WorkspaceRuntime.stop()`'s own contract and is covered by the
 * runtime suite; restating it here through a signal would be a second, weaker copy. Named rather than
 * quietly skipped.
 */

import { describe, it, expect, beforeAll, afterAll, afterEach, vi } from "vitest";
import * as fs from "fs";
import * as os from "os";
import * as path from "path";
import * as net from "net";

import { setDataRootOverride } from "@bifurc/engine/store/workspaceFs";
import { setSettingsPathOverride } from "@bifurc/engine/store/appSettings";
import { runWorkspaceHost } from "@bifurc/engine/runtime/host";
import { commandRegistry } from "@bifurc/engine/commands/registry";
import { registerAllEngineCommands } from "@bifurc/engine/registerAll";

const HOST_TOKEN = "host-signal-token";
const WS_A = "sig-ws-a";

let root: string;
let dataRoot: string;
let socketPath: string;

/** Can something bind this port? False means something still holds it. */
function portIsFree(port: number): Promise<boolean> {
  return new Promise((resolve) => {
    const srv = new net.Server();
    srv.once("error", () => resolve(false));
    srv.listen(port, "127.0.0.1", () => srv.close(() => resolve(true)));
  });
}

/** One workspace's tree — the same layout the host suite writes, for the same reason. */
function writeWorkspace(dir: string, wsId: string, name: string): void {
  for (const sub of ["mappings", "rules", "mocks", "environments", "requests", "sockets", "webhooks", "capture"]) {
    fs.mkdirSync(path.join(dir, sub), { recursive: true });
  }
  fs.writeFileSync(
    path.join(dir, "workspace.json"),
    JSON.stringify({ id: wsId, name, createdAt: 1704067200000, activeEnvironmentId: null }, null, 2),
    "utf-8",
  );
  for (const kind of ["mappings", "rules", "mocks"]) {
    fs.writeFileSync(path.join(dir, kind, "enabled.json"), JSON.stringify([], null, 2), "utf-8");
  }
  fs.writeFileSync(path.join(dir, "rules", "names.json"), JSON.stringify({}, null, 2), "utf-8");
  fs.writeFileSync(path.join(dir, "rules", "index.json"), JSON.stringify({ folders: [], order: [] }, null, 2), "utf-8");
  fs.writeFileSync(path.join(dir, "mocks", "index.json"), JSON.stringify({ folders: [], order: [] }, null, 2), "utf-8");
  fs.writeFileSync(
    path.join(dir, "environments", "__global__.json"),
    JSON.stringify({ id: "__global__", name: "Global", variables: [], createdAt: 0, workspaceId: wsId }, null, 2),
    "utf-8",
  );
}

/**
 * The most recently started host, so a case that fails mid-way cannot contaminate the one after it.
 *
 * This is not tidiness. Both cases bind the same proxy base and the same RPC socket path, so a first case
 * that fails before its signal fires leaves a live host holding them — and the second case then fails for a
 * reason that points at the *first* case. That was observed while mutation-testing this file: with the
 * teardown removed, the SIGTERM case failed on the port assertion (correctly) and the SIGINT case failed
 * too, for a reason that had nothing to do with SIGINT. A failure that names the wrong case is worse than
 * no failure.
 */
let activeHost: Awaited<ReturnType<typeof runWorkspaceHost>> | null = null;

/**
 * The listeners a host run adds, plus the handle — and the teardown that removes them again.
 *
 * ## Why the registry is passed explicitly
 *
 * `host.ts`'s `doStart()` registers the command surface only when `opts.registry === undefined`, against the
 * **module-level** `commandRegistry`. That is a singleton, so a second `runWorkspaceHost()` in one process
 * would re-register onto it and throw `Command "blob.put" is already registered.` — measured, not assumed:
 * the SIGINT case below failed on exactly that before this was added.
 *
 * That is a real constraint and not a defect to route around: one host per process is the design (one RPC
 * endpoint per host, and `shutdownEngine()` memoises process-wide too). A second host is still startable —
 * it just has to be handed a registry that has already been populated, which is what `beforeAll` does. The
 * host is told what it serves rather than being allowed to re-declare it, which is P3's own rule.
 */
async function startHostAndCaptureSignals(): Promise<{
  host: Awaited<ReturnType<typeof runWorkspaceHost>>;
  sigterm: (() => void)[];
  sigint: (() => void)[];
  ports: number[];
}> {
  // A no-op on the happy path, where the signal already tore the previous host down (`stop()` is
  // idempotent). It only does work when a case failed — see `activeHost`.
  await activeHost?.stop().catch(() => {});
  activeHost = null;

  const beforeTerm = process.listeners("SIGTERM");
  const beforeInt = process.listeners("SIGINT");

  const host = await runWorkspaceHost({
    dataDir: dataRoot,
    transport: { kind: "socket", path: socketPath },
    token: HOST_TOKEN,
    registry: commandRegistry,
    onNotice: () => {},
  });
  activeHost = host;

  const sigterm = process.listeners("SIGTERM").filter((l) => !beforeTerm.includes(l)) as unknown as (() => void)[];
  const sigint = process.listeners("SIGINT").filter((l) => !beforeInt.includes(l)) as unknown as (() => void)[];

  const ports = host.status().runtimes.flatMap((r) => [r.proxyPort!, r.webhookPort!]);
  return { host, sigterm, sigint, ports };
}

function removeListeners(sigterm: (() => void)[], sigint: (() => void)[]): void {
  for (const l of sigterm) process.removeListener("SIGTERM", l as unknown as NodeJS.SignalsListener);
  for (const l of sigint) process.removeListener("SIGINT", l as unknown as NodeJS.SignalsListener);
}

describe("runWorkspaceHost() — the signal path (P14 §8 criterion 18)", () => {
  const exitCalls: number[] = [];

  beforeAll(() => {
    root = fs.mkdtempSync(path.join(os.tmpdir(), "bifurc-host-sig-"));
    dataRoot = path.join(root, "data");
    fs.mkdirSync(dataRoot, { recursive: true });
    writeWorkspace(dataRoot, WS_A, "Signal Workspace A");

    fs.writeFileSync(
      path.join(root, "app.json"),
      JSON.stringify(
        {
          companionEnabled: false,
          companionPort: 9271,
          minimizeToTray: true,
          tlsEnabled: false,
          tlsCaCertPath: null,
          tlsCaKeyPath: null,
          workspaces: [{ id: WS_A, name: "Signal Workspace A", activeEnvironmentId: null }],
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

    socketPath =
      process.platform === "win32"
        ? `\\\\.\\pipe\\bifurc-host-sig-${process.pid}`
        : path.join(os.tmpdir(), `bifurc-hs-${process.pid}.sock`);

    // Populated **once**, here, rather than left to the first host: the registration throws on a second
    // call (see `startHostAndCaptureSignals`), so doing it explicitly makes both cases independent of which
    // runs first.
    registerAllEngineCommands(commandRegistry);

    // Stubbed **before any host starts**, because the teardown calls it asynchronously and an unstubbed
    // `process.exit` would end the worker mid-case — reported as a crash, not as an assertion.
    vi.spyOn(process, "exit").mockImplementation(((code?: number) => {
      exitCalls.push(code ?? 0);
      return undefined as never;
    }) as unknown as (code?: number) => never);
  });

  afterEach(() => {
    exitCalls.length = 0;
  });

  afterAll(() => {
    vi.restoreAllMocks();
    setDataRootOverride(null);
    setSettingsPathOverride(null);
    try {
      fs.rmSync(root, { recursive: true, force: true });
    } catch {
      /* best-effort */
    }
  });

  it("SIGTERM stops every runtime's servers, then exits 143 — and a second signal is ignored", async () => {
    const { sigterm, sigint, ports } = await startHostAndCaptureSignals();
    try {
      // The wiring itself: one listener per signal, registered by `runWorkspaceHost`.
      expect(sigterm).toHaveLength(1);
      expect(sigint).toHaveLength(1);
      // The runtime is genuinely serving before the signal, or "the ports are free afterwards" would be
      // true for the wrong reason.
      for (const p of ports) expect(await portIsFree(p), `port ${p} should be held while serving`).toBe(false);

      sigterm[0]();
      // `shutdown()` is `void`-called and async, so the exit code arrives on a later tick.
      await expect.poll(() => exitCalls.length, { timeout: 20000 }).toBeGreaterThan(0);

      expect(exitCalls).toEqual([143]);

      // **The teardown half, asserted by real binds rather than by a status flag.** If the signal only
      // called `process.exit` without stopping the runtimes, these would stay held.
      for (const p of ports) {
        await expect.poll(() => portIsFree(p), { timeout: 10000 }).toBe(true);
      }

      // Idempotence: `shuttingDown` is what stops a second signal from re-entering teardown and exiting
      // twice. Without the guard the second call would push a second code onto `exitCalls`.
      sigterm[0]();
      await new Promise((r) => setTimeout(r, 200));
      expect(exitCalls).toEqual([143]);
    } finally {
      removeListeners(sigterm, sigint);
    }
  }, 90000);

  it("SIGINT exits 130 — the shell convention for a signal-killed process", async () => {
    const { sigterm, sigint } = await startHostAndCaptureSignals();
    try {
      expect(sigint).toHaveLength(1);
      sigint[0]();
      await expect.poll(() => exitCalls.length, { timeout: 20000 }).toBeGreaterThan(0);
      // `128 + signo`: 128 + 2. Asserted as a distinct number from the SIGTERM case, because an
      // implementation that hardcoded one code would satisfy either test alone.
      expect(exitCalls).toEqual([130]);
    } finally {
      removeListeners(sigterm, sigint);
    }
  }, 90000);
});
