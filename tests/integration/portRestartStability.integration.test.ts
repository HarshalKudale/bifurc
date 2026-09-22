/**
 * P14 §8 criterion 6 — D16's stickiness, and the restart behaviour around it. **Three cases, and their
 * shapes are the point: they separate the two mechanisms rather than sharing one outcome.**
 *
 * ## What D16 actually requires
 *
 * *"The resolved port is recorded in the host-local registry, so a restart reuses it. A workspace that landed
 * on 81 because 80 was busy **keeps 81** after 80 frees up — a stable URL is worth more to the browser-proxy
 * workflow than reclaiming the base."* The requirement is therefore **not** "the ports come back". It is "the
 * workspace keeps the port it actually bound, *even when a lower one becomes available*" — and those two
 * claims come apart in exactly the third case below.
 *
 * ## History, because the shape of these cases is what makes them trustworthy
 *
 * The criterion was unsatisfiable *as worded* — it said "stop the host … start again", and
 * `workspaceHost.integration.test.ts:331` pins that a stopped host refuses to restart. It was reworded on
 * 2026-09-22 to "a new host lands on the same ports", which **quietly weakened the claim** from stickiness to
 * reproducibility. The code satisfied the substitute and violated the original, so the criterion went green
 * while D16 stayed unimplemented: `resolvedProxyPort` was **write-only** (`recordResolvedPorts()` wrote it,
 * nothing read it), because the allocator had no *soft* input and passing the value as a `preferred*` one
 * would have turned a taken remembered port into a hard startup failure.
 *
 * D16 was implemented the same evening — a soft `remembered*` input on `allocatePorts` — which is what makes
 * the third case possible. **The first two are kept with corrected meanings:** they were always accurate about
 * the code, and only the authority they were measured against changed. See `plan/14` §8 criteria 6a/6b and
 * lesson 3.
 */

import { describe, it, expect, beforeAll, afterAll } from "vitest";
import * as fs from "fs";
import * as os from "os";
import * as path from "path";
import * as net from "net";

import { setDataRootOverride } from "@bifurc/engine/store/workspaceFs";
import { setSettingsPathOverride } from "@bifurc/engine/store/appSettings";
import { createWorkspaceHost, type WorkspaceHost } from "@bifurc/engine/runtime/host";
import { CommandRegistry } from "@bifurc/engine/commands/registry";
import { registerAllEngineCommands } from "@bifurc/engine/registerAll";

const WS_A = "restart-ws-a";
const WS_B = "restart-ws-b";

/** Ports a case holds open, released in its own `finally` so a failure cannot leak into the next case. */
const held: net.Server[] = [];

function writeWorkspace(dataRoot: string, id: string, name: string): void {
  const dir = path.join(dataRoot, id);
  fs.mkdirSync(dir, { recursive: true });
  fs.writeFileSync(path.join(dir, "workspace.json"), JSON.stringify({ id, name, createdAt: 0 }, null, 2), "utf-8");
}

describe("P14 §8 criterion 6 — a new host reproduces the previous ports, and keeps them when D16 says so", () => {
  let root: string;
  let dataRoot: string;
  let socketPath: string;
  let registry: CommandRegistry;
  let hostSeq = 0;

  /** A fresh host over the same data root. The registry is passed in — see the note in the helper below. */
  async function startHost(): Promise<WorkspaceHost> {
    hostSeq += 1;
    const host = createWorkspaceHost({
      dataDir: dataRoot,
      transport: {
        kind: "socket",
        // A distinct path per host: a named pipe left bound by a stopped host would make the next
        // `listen()` fail, and that failure would have nothing to do with the ports under test.
        path: process.platform === "win32" ? `\\\\.\\pipe\\bifurc-restart-${process.pid}-${hostSeq}` : path.join(os.tmpdir(), `bifurc-rs-${process.pid}-${hostSeq}.sock`),
      },
      token: "restart-token",
      // Passed explicitly: `doStart()` would otherwise register into the process-wide singleton, and
      // `register()` throws on a duplicate — which is what makes a second host in one process impossible
      // without this.
      registry,
      onNotice: () => {},
    });
    await host.start();
    return host;
  }

  /** The resolved ports, keyed by workspace id, read from the host's own handle. */
  function portsOf(host: WorkspaceHost): Record<string, [number | null, number | null]> {
    return Object.fromEntries(
      host.status().runtimes.map((r) => [r.workspaceId, [r.proxyPort, r.webhookPort] as [number | null, number | null]]),
    );
  }

  /** Ask the OS for a free port and release it — for the case that needs a *specific* base. */
  async function freePort(): Promise<number> {
    const srv = new net.Server();
    const port = await new Promise<number>((resolve, reject) => {
      srv.once("error", reject);
      srv.listen(0, "127.0.0.1", () => {
        const addr = srv.address();
        resolve(typeof addr === "object" && addr ? addr.port : 0);
      });
    });
    await new Promise<void>((resolve) => srv.close(() => resolve()));
    return port;
  }

  /** Whether a real `listen()` on `port` succeeds. A fixture probe — fine here, never in the product. */
  function portIsFree(port: number): Promise<boolean> {
    return new Promise((resolve) => {
      const srv = new net.Server();
      const done = (free: boolean): void => {
        srv.removeAllListeners();
        srv.on("error", () => {});
        srv.close(() => resolve(free));
      };
      srv.once("error", () => done(false));
      srv.once("listening", () => done(true));
      try {
        srv.listen(port, "127.0.0.1");
      } catch {
        done(false);
      }
    });
  }

  /** Close every held server. Idempotent, so it is safe in a `finally` after an earlier release. */
  async function releaseHeld(): Promise<void> {
    while (held.length) {
      const s = held.pop()!;
      await new Promise<void>((r) => s.close(() => r()));
    }
  }

  beforeAll(() => {
    root = fs.mkdtempSync(path.join(os.tmpdir(), "bifurc-restart-"));
    dataRoot = path.join(root, "data");
    fs.mkdirSync(dataRoot, { recursive: true });
    writeWorkspace(dataRoot, WS_A, "Restart Workspace A");
    writeWorkspace(dataRoot, WS_B, "Restart Workspace B");

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
          workspaces: [
            { id: WS_A, name: "Restart Workspace A", activeEnvironmentId: null },
            { id: WS_B, name: "Restart Workspace B", activeEnvironmentId: null },
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

    socketPath = path.join(os.tmpdir(), `bifurc-restart-unused-${process.pid}.sock`);
    registry = new CommandRegistry();
    registerAllEngineCommands(registry);
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

  it("a second host lands on the same ports — and still does with `runtime.json` deleted", async () => {
    const first = await startHost();
    const firstPorts = portsOf(first);
    await first.stop();

    // The increment is real, not two independent picks: the two workspaces got different ports, and the
    // second is the first plus one. Asserted because "the same ports came back" is uninteresting if both
    // were `null`.
    const a = firstPorts[WS_A];
    const b = firstPorts[WS_B];
    expect(a[0]).not.toBeNull();
    expect(b[0]).not.toBeNull();
    expect(a[0]).not.toBe(b[0]);

    // `runtime.json` now records what the first host resolved. Deleted *before* the second host starts, so
    // the second host cannot possibly be reusing it.
    const registryPath = path.join(dataRoot, "runtime.json");
    expect(fs.existsSync(registryPath), "the first host should have recorded its resolved ports").toBe(true);
    const recorded = JSON.parse(fs.readFileSync(registryPath, "utf-8"));
    expect(recorded.workspaces[WS_A].resolvedProxyPort).toBe(a[0]);
    fs.rmSync(registryPath, { force: true });

    const second = await startHost();
    const secondPorts = portsOf(second);
    await second.stop();

    // **The assertion that separates the two mechanisms.** Identical ports with the record gone means the
    // scan reproduced them; had the remembered value been the input, this would have moved.
    expect(secondPorts).toEqual(firstPorts);
  }, 120000);

  /**
   * **Softness — the whole difference between a remembered port and an override.**
   *
   * The remembered port is occupied before the second host starts, and the two possible rules diverge:
   *
   * - as a `preferred*` **override**, D16's fail-loudly rule applies — `allocatePorts` throws
   *   `PortUnavailableError`, `host.ts` **skips** the workspace, and it does not serve;
   * - as a **soft** input, the bind fails and the scan simply moves on, so the workspace comes up one port
   *   along.
   *
   * So "the workspace still serves, and `skipped` is empty" is evidence for softness and against the
   * preference shape. **These assertions used to be filed as proof that the record was never read at all** —
   * same assertions, opposite conclusion, because the code at that time did neither. The case now guards the
   * rule that replaced the omission, which is why it is worth keeping rather than deleting.
   */
  it("still serves when the remembered port is occupied — a remembered port is soft, never a hard failure", async () => {
    const first = await startHost();
    const remembered = portsOf(first)[WS_A][0]!;
    await first.stop();

    // Hold exactly the port the previous host resolved, so a preference would be an unsatisfiable one.
    const holder = new net.Server();
    await new Promise<void>((resolve, reject) => {
      holder.once("error", reject);
      holder.listen(remembered, "127.0.0.1", () => resolve());
    });
    held.push(holder);

    try {
      const second = await startHost();
      const status = second.status();
      const after = portsOf(second);
      await second.stop();

      // Not skipped: a busy *remembered* port is not a user's override, so D16's fail-loudly rule does not
      // apply to it and the host must not drop the workspace.
      expect(status.skipped).toEqual([]);
      // And it genuinely moved, rather than somehow taking a port that is held.
      expect(after[WS_A][0]).not.toBe(remembered);
      expect(after[WS_A][0]).not.toBeNull();
      expect(after[WS_B][0]).not.toBeNull();
    } finally {
      await releaseHeld();
    }
  }, 120000);

  /**
   * **D16's stickiness — the claim the criterion was originally reaching for, and the one a probe measured as
   * absent on 2026-09-22.**
   *
   * The base is moved to a controlled port with `BIFURC_PROXY_BASE_PORT` so that "free the base" is
   * deterministic and does not depend on what else is running on this machine. The assertion that carries the
   * meaning is **not** `after === before` — case 1 already shows the ports come back. It is that the base is
   * **provably free** at the moment the second host starts, so a scan *would* have taken it, and the workspace
   * keeps its higher port anyway. That is stickiness, and it is the thing D16 asks for.
   */
  it("keeps its incremented port once the base frees up (D16 stickiness)", async () => {
    const base = await freePort();
    const previous = process.env.BIFURC_PROXY_BASE_PORT;
    process.env.BIFURC_PROXY_BASE_PORT = String(base);

    // Hold the base so the first host's scan has to increment.
    const holder = new net.Server();
    await new Promise<void>((resolve, reject) => {
      holder.once("error", reject);
      holder.listen(base, "127.0.0.1", () => resolve());
    });
    held.push(holder);

    try {
      const first = await startHost();
      const before = portsOf(first)[WS_A][0];
      await first.stop();

      expect(before).not.toBeNull();
      expect(before).not.toBe(base);

      // D16's "after 80 frees up".
      await releaseHeld();
      expect(await portIsFree(base), "the base must be free, or the scan could not have taken it").toBe(true);

      const second = await startHost();
      const after = portsOf(second)[WS_A][0];
      await second.stop();

      // **The stickiness assertion.** Without D16 the scan would land this workspace on `base`.
      expect(after).toBe(before);
      expect(after).not.toBe(base);
    } finally {
      if (previous === undefined) delete process.env.BIFURC_PROXY_BASE_PORT;
      else process.env.BIFURC_PROXY_BASE_PORT = previous;
      await releaseHeld();
    }
  }, 120000);
});
