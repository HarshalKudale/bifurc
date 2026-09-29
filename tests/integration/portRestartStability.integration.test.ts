/**
 * P14 §8 criterion 6 — D16's stickiness, and the restart behaviour around it.
 *
 * ## ⚠ SUPERSEDED BY `plan/17` §1 (2026-09-25). WHAT THIS FILE IS NOW, AND WHY IT SURVIVED.
 *
 * D16 gave every workspace its own proxy port and remembered it, so *"a workspace that landed on 81 because
 * 80 was busy keeps 81 after 80 frees up"*. `plan/17` §1 deletes the whole mechanism: a workspace is a
 * **project**, the forward proxy is **one host-level listener on a port the user chose**, and the host-local
 * `runtime.json` that recorded the resolved ports is gone (`plan/17` §5 work item 9). There is no per-workspace
 * port, so there is nothing to be sticky about.
 *
 * The file was kept rather than deleted because the **restart** half of its subject survives and is worth
 * pinning — but the claim is now the opposite shape, and it is important not to read the old meaning into it:
 *
 * - **Before:** the ports came back because a *scan* re-derived them, and stickiness made it re-derive the
 *   *same* ones.
 * - **Now:** the ports come back because they are **settings**, and nothing is allowed to move them. The
 *   interesting failure is no longer "the scan drifted" — it is **"something started scanning"**, because a
 *   silently moved forward-proxy port is a broken browser setting, which is exactly what `plan/17` §1 refuses
 *   to allow: *"the forward proxy does NOT scan. It binds `8080` and **fails if occupied**, naming the port,
 *   so the user enters a working one instead of the app silently incrementing."*
 *
 * So the cases below are, in order: **the ports are reproducible and no registry is written** (the surviving
 * half of the old criterion, plus WI 9's deletion), and then **a busy port fails loudly rather than moving**,
 * once for each host-level listener. The last two are the load-bearing ones and they are discriminating by
 * construction: an implementation that re-added the scan would bind a *different* port and fail them.
 *
 * ## The history, kept because it is the reason this file is careful
 *
 * The criterion was once unsatisfiable *as worded* — it said "stop the host … start again", and
 * `workspaceHost.integration.test.ts` pins that a stopped host refuses to restart. It was reworded on
 * 2026-09-22 to "a new host lands on the same ports", which **quietly weakened the claim** from stickiness to
 * reproducibility. The code satisfied the substitute and violated the original, so the criterion went green
 * while D16 stayed unimplemented: `resolvedProxyPort` was **write-only**, because the allocator had no *soft*
 * input and passing the value as a `preferred*` one would have turned a taken remembered port into a hard
 * startup failure. D16 was implemented the same evening.
 *
 * That episode is why the cases below state *which mechanism* produces the port rather than only that the
 * port is the same — the same lesson, applied to a mechanism that has since been replaced.
 */

import { describe, it, expect, beforeAll, afterAll } from "vitest";
import * as fs from "fs";
import * as os from "os";
import * as path from "path";
import * as net from "net";

import { setDataRootOverride } from "@bifurc/engine/workspace/workspaceFs";
import { setSettingsPathOverride } from "@bifurc/engine/workspace/appSettings";
import { createWorkspaceHost, type WorkspaceHost } from "@bifurc/engine/runtime/host";
import { CommandRegistry } from "@bifurc/engine/kernel/registry";
import { registerAllEngineCommands } from "@bifurc/engine/registerAll";

const WS_A = "restart-ws-a";
const WS_B = "restart-ws-b";

/**
 * The two host-level ports every host in this file is told to use.
 *
 * Chosen by the OS rather than hardcoded, because `plan/17` §1 removed the scan: a value colliding with
 * something on the developer's machine is no longer absorbed, it is a reported bind failure — which the
 * cases below *deliberately* produce, so they need a base they know is free.
 */
let EDGE_PORT: number;
let FORWARD_PROXY_PORT: number;

/** Ports a case holds open, released in its own `finally` so a failure cannot leak into the next case. */
const held: net.Server[] = [];

function writeWorkspace(dataRoot: string, id: string, name: string): void {
  const dir = path.join(dataRoot, id);
  fs.mkdirSync(dir, { recursive: true });
  fs.writeFileSync(path.join(dir, "workspace.json"), JSON.stringify({ id, name, createdAt: 0 }, null, 2), "utf-8");
}

describe("P14 §8 criterion 6 — a new host reproduces the ports, and a busy one fails loudly instead of moving", () => {
  let root: string;
  let dataRoot: string;
  let registry: CommandRegistry;
  let hostSeq = 0;
  /** Every notice every host in this file emitted, so a case can assert what the operator was told. */
  let notices: string[] = [];

  /** A fresh host over the same data root. The registry is passed in — see the note in the helper below. */
  async function startHost(): Promise<WorkspaceHost> {
    hostSeq += 1;
    const host = createWorkspaceHost({
      dataDir: dataRoot,
      transport: {
        kind: "socket",
        // A distinct path per host: a named pipe left bound by a stopped host would make the next
        // `listen()` fail, and that failure would have nothing to do with the ports under test.
        path:
          process.platform === "win32"
            ? `\\\\.\\pipe\\bifurc-restart-${process.pid}-${hostSeq}`
            : path.join(os.tmpdir(), `bifurc-rs-${process.pid}-${hostSeq}.sock`),
      },
      token: "restart-token",
      // Passed explicitly: `doStart()` would otherwise register into the process-wide singleton, and
      // `register()` throws on a duplicate — which is what makes a second host in one process impossible
      // without this.
      registry,
      onNotice: (m) => notices.push(m),
    });
    await host.start();
    return host;
  }

  /**
   * The host's two host-level ports, read from its own handle.
   *
   * Replaces a `portsOf(host)` keyed by workspace id: there is no per-workspace port any more, so a map keyed
   * by workspace would be N copies of one number. `null` means *that listener is not up* — which under
   * `plan/17` §1 is a reported failure rather than an allocation, so the cases below read it deliberately.
   */
  function portsOf(host: WorkspaceHost): { edge: number | null; forwardProxy: number | null } {
    const st = host.status();
    return { edge: st.edgePort, forwardProxy: st.forwardProxyPort };
  }

  /** Ask the OS for a free port and release it. */
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

  /**
   * Two ports the OS reports free, and that **differ from each other**.
   *
   * The retry is load-bearing: `freePort()` releases the socket before returning, so the OS may hand the same
   * ephemeral port back on the next call — and since neither listener scans, two equal values would mean the
   * second one fails to bind, which would look like a defect in the code under test.
   */
  async function twoFreePorts(): Promise<[number, number]> {
    const first = await freePort();
    let second = await freePort();
    while (second === first) second = await freePort();
    return [first, second];
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

  /** Hold `port` for the duration of a case, so the host's bind of it must fail. */
  async function hold(port: number): Promise<void> {
    const holder = new net.Server();
    await new Promise<void>((resolve, reject) => {
      holder.once("error", reject);
      holder.listen(port, "127.0.0.1", () => resolve());
    });
    held.push(holder);
  }

  /** Close every held server. Idempotent, so it is safe in a `finally` after an earlier release. */
  async function releaseHeld(): Promise<void> {
    while (held.length) {
      const s = held.pop()!;
      await new Promise<void>((r) => s.close(() => r()));
    }
  }

  beforeAll(async () => {
    [EDGE_PORT, FORWARD_PROXY_PORT] = await twoFreePorts();

    root = fs.mkdtempSync(path.join(os.tmpdir(), "bifurc-restart-"));
    dataRoot = path.join(root, "data");
    fs.mkdirSync(dataRoot, { recursive: true });
    writeWorkspace(dataRoot, WS_A, "Restart Workspace A");
    writeWorkspace(dataRoot, WS_B, "Restart Workspace B");

    fs.writeFileSync(
      path.join(root, "app.json"),
      JSON.stringify(
        {
          // Both host-level ports are stated, because they are settings the user owns — see `EDGE_PORT`.
          port: EDGE_PORT,
          forwardProxyPort: FORWARD_PROXY_PORT,
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

    registry = new CommandRegistry();
    registerAllEngineCommands(registry);
  });

  afterEach(() => {
    notices = [];
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

  it("a second host lands on the same ports — and no `runtime.json` is written at all", async () => {
    const first = await startHost();
    const firstPorts = portsOf(first);
    await first.stop();

    // The non-vacuity guard, and under `plan/17` §1 a real precondition: `null` means the listener is **not
    // up**, so "the same ports came back" would be trivially true of two failures.
    expect(firstPorts.edge).toBe(EDGE_PORT);
    expect(firstPorts.forwardProxy).toBe(FORWARD_PROXY_PORT);

    // **`plan/17` §5 work item 9's assertion.** The host-local registry is deleted: it existed to remember
    // *which port each workspace landed on*, and with one host-level listener per port there is nothing per
    // workspace to remember. Asserting the **absence** is the point — a re-introduced `runtime.json` would be
    // a second home for a port that now has exactly one (the setting).
    expect(
      fs.existsSync(path.join(dataRoot, "runtime.json")),
      "the host must not write a port registry any more (plan/17 §5 work item 9)",
    ).toBe(false);

    const second = await startHost();
    const secondPorts = portsOf(second);
    await second.stop();

    // **Weaker than it used to mean, and stated so.** Before `plan/17` this separated a scan from a
    // remembered value; now it says only that nothing *moves* the ports — which is the property §1 actually
    // cares about, because a moved forward-proxy port is a browser setting that no longer works.
    expect(secondPorts).toEqual(firstPorts);
  }, 120000);

  /**
   * **The rule that replaced the scan, for the forward proxy.**
   *
   * `plan/17` §1: *"the forward proxy does NOT scan. It binds `8080` and **fails if occupied**, naming the
   * port, so the user enters a working one instead of the app silently incrementing."* This case occupies the
   * configured port and asserts exactly that: no silent increment, a named failure, and **the host still
   * starts** — because "fails" means the *bind* fails, not that the process refuses to run. A host that
   * exited would turn one taken port into an outage for every workspace.
   *
   * **Discriminating by construction:** an implementation that re-added the scan would bind a different port,
   * so `forwardProxyPort` would be a number and the first assertion would fail. That is the mutation this
   * case is aimed at.
   */
  it("a busy forward-proxy port fails loudly and names it — it does not silently increment", async () => {
    await hold(FORWARD_PROXY_PORT);

    try {
      const host = await startHost();
      const status = host.status();
      await host.stop();

      // Not moved: `null`, because nothing is answering on the port it was told to use.
      expect(status.forwardProxyPort).toBeNull();
      expect(status.forwardProxyError).not.toBeNull();

      // And the operator was told **which** port and **why it matters**, because the browser's proxy setting
      // points at it. Without the name, "the proxy is down" is unattributable.
      const joined = notices.join("\n");
      expect(joined).toMatch(new RegExp(`forward proxy FAILED to bind ${FORWARD_PROXY_PORT}`));
      expect(joined).toMatch(/browser/i);
      // A working alternative is **named**, which is what makes `plan/17`'s instruction to the user
      // (*"enter a working one instead of the app silently incrementing"*) followable at all.
      expect(joined).toMatch(/Port \d+ is free/);

      // **The host still serves.** The edge is the other host-level listener and it is unaffected, which is
      // the whole difference between a reported bind failure and a refusal to start.
      expect(status.edgePort).toBe(EDGE_PORT);
      expect(status.edgeError).toBeNull();
      expect(status.workspaces.map((w) => w.workspaceId)).toEqual([WS_A, WS_B]);
    } finally {
      await releaseHeld();
    }
  }, 120000);

  /**
   * The same rule for the **edge**, and it is a separate case rather than a second assertion above because
   * the two listeners fail differently in a way that matters: the edge is the reverse path itself, so a host
   * whose edge is down answers for no `*.localhost` mapping at all. `host.ts` gives it its own notice for that
   * reason, and this asserts the notice says so.
   */
  it("a busy edge port is reported as a failure, and the forward proxy still serves", async () => {
    await hold(EDGE_PORT);

    try {
      const host = await startHost();
      const status = host.status();
      await host.stop();

      expect(status.edgePort).toBeNull();
      expect(status.edgeError).not.toBeNull();

      const joined = notices.join("\n");
      expect(joined).toMatch(/edge on .* FAILED to bind/);
      // The consequence, stated as a consequence rather than as routine — see `host.ts`'s notice.
      expect(joined).toMatch(/no \*\.localhost mapping will resolve/);

      // The forward proxy is independent, so it is up: one busy port does not take the other down.
      expect(status.forwardProxyPort).toBe(FORWARD_PROXY_PORT);
      expect(status.forwardProxyError).toBeNull();
    } finally {
      await releaseHeld();
    }
  }, 120000);

  it("releases both host-level ports on stop, so the next host can take them", async () => {
    const host = await startHost();
    expect(portsOf(host)).toEqual({ edge: EDGE_PORT, forwardProxy: FORWARD_PROXY_PORT });
    await host.stop();

    // The release half, asserted by real binds rather than by a status flag: a `stop()` that only cleared the
    // handle would leave the sockets bound and the *next* host would fail to bind for a reason that pointed at
    // the wrong case.
    await expect.poll(() => portIsFree(EDGE_PORT), { timeout: 5000 }).toBe(true);
    await expect.poll(() => portIsFree(FORWARD_PROXY_PORT), { timeout: 5000 }).toBe(true);
  }, 120000);
});
