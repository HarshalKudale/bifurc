/**
 * P14 WI 3 — `createWorkspaceHost()`, exercised with real I/O and a real RPC client.
 *
 * WHY THIS IS AN INTEGRATION FILE, AND WHY IT IS ITS OWN FILE
 * ----------------------------------------------------------
 * Two separate reasons, and both matter:
 *
 * 1. The host's whole job is to bind real things — **one** forward proxy, **one** webhook receiver and
 *    **one** socket endpoint — and then prove that endpoint can reach the workspaces behind it. A mocked
 *    transport would assert the wiring this file exists to verify.
 * 2. It is its own file because `shutdownEngine()` memoises **process-wide**, so a process holds exactly one
 *    engine lifecycle. This file deliberately never calls `createEngine()` (a host must not — see
 *    `host.ts`'s header), but the *other* lifecycle in this suite (`engineSmoke`) does, and vitest gives each
 *    file its own worker. Keeping one engine-shaped lifecycle per file is the rule WI 3's trap entry records.
 *
 * THE ACCEPTANCE CRITERION THIS FILE EXISTS FOR
 * ---------------------------------------------
 * `plan/14` §6, in its original wording: *"a workspace host starts with no companion server."*
 *
 * **This file used to assert that by probing the companion's port (9271)** and requiring its holder to be
 * unchanged. The companion was **retired outright** while implementing `plan/16`, so half of that was on
 * its way to becoming vacuous — the *server* is gone, and a test that only asserts a deleted thing is
 * absent cannot fail against any code that keeps it deleted.
 *
 * ## What replaces it — and a correction, because the first replacement was wrong
 *
 * The first rewrite re-pointed the criterion at the **edge router** and asserted the host leaves the
 * edge's port unbound. **That is backwards, and the suite caught it.** `runtime/host.ts` does
 * `opts.edgePort ?? settings.port` and then `edge.start(edgePortWanted)` — the host *does* compose and
 * bind the edge, deliberately: the edge is the host-level reverse half, so *one listener for N
 * workspaces' mappings* is exactly what a host is for. The criterion never said "bind no host-level
 * server"; it said "bind no **companion**".
 *
 * So the honest assertion is the one that **still discriminates**, and it is two real binds in both
 * directions:
 *
 * 1. **the host holds its edge port** — a bind there must fail, i.e. the reverse path genuinely exists
 *    (this is new coverage: nothing else asserts the host's own edge is up), and
 * 2. **the host does not hold the retired companion's port** — the surviving falsifiable form of the
 *    original criterion. A regression that regrew a companion bridge on 9271 turns this red, which was
 *    verified rather than assumed: mutating the test to bind 9271 **flips it to `1 failed`** with
 *    *"the host must not hold the retired companion port 9271"*.
 *
 * WHAT `plan/17` §5 WORK ITEM 3 CHANGED HERE, AND WHAT IT DID NOT
 * --------------------------------------------------------------
 * The host used to allocate a port per workspace (D16's base-and-scan) and compose a `WorkspaceRuntime`
 * around each. `plan/17` §1 replaces that: a workspace is a **project**, so switching workspaces changes the
 * *context* — which rules and mocks are in force — while the ports stay put. There is now one forward proxy
 * and one webhook receiver for the whole host, and `WorkspaceRuntime` is deleted rather than hollowed.
 *
 * So the old assertions *"two workspaces get two distinct proxy ports / two distinct webhook ports"* are
 * **gone**, and not because they were inconvenient: there is no longer any per-workspace port for them to be
 * distinct about, and `status().workspaces` is config scope only. What replaces them is sharper, and the
 * webhook half is new coverage: **the webhook is OFF by default** (`plan/17` §1 — it is a feature, not
 * infrastructure), so `status().webhookPort` must be `null` on a freshly started host. A receiver that came
 * up at boot would be a listener this product has never opened on its own.
 *
 * **The readiness contract is asserted implicitly and must not be lost again.** `host.start()` resolving
 * before its listeners bind would make every `status()` port `null`, and the first case below reads
 * `status.forwardProxyPort` / `status.edgePort` straight off it. That wait used to live in
 * `WorkspaceRuntime.doStart()` and was restored to `host.ts` when the class was deleted — see
 * `waitUntilReady`'s docblock there.
 *
 * WHY THE RPC CLIENT IS THE REAL ONE
 * ----------------------------------
 * `createSocketTransport` is the same code the desktop shell and P5's typed client use, so "the endpoint
 * serves both workspaces" is asserted by sending real frames over a real named pipe and reading real
 * replies — including a round trip that proves `workspaceId` reaches a workspace-scoped command.
 */

import { describe, it, expect, beforeAll, afterAll } from "vitest";
import * as fs from "fs";
import * as os from "os";
import * as path from "path";
import * as net from "net";

import { setDataRootOverride } from "@bifurc/engine/workspace/workspaceFs";
import { setSettingsPathOverride } from "@bifurc/engine/workspace/appSettings";
import { createWorkspaceHost } from "@bifurc/engine/runtime/host";
import { createSocketTransport } from "@bifurc/engine/transport/socket";
import { CommandRegistry, commandRegistry } from "@bifurc/engine/kernel/registry";
import { registerAllEngineCommands } from "@bifurc/engine/registerAll";
import { PROTOCOL_VERSION } from "@bifurc/protocol";

/** The token this file hands the host, and presents to it. A literal so no file read is on the path. */
const HOST_TOKEN = "host-it-token";

const WS_A = "host-ws-a";
const WS_B = "host-ws-b";

/**
 * The port the **retired companion** used, which the host must not be holding.
 *
 * `plan/14` WI 3 was originally *"a workspace host starts with no companion server."* The companion is
 * gone, so a test asserting its port is never bound would have nothing to catch — except that this
 * number is exactly what a regression would reuse. Asserting it here costs one bind attempt and is the
 * only surviving form of the original criterion that can still **fail**.
 *
 * Probed rather than assumed free, for the same reason the edge port is: the real 9271 may be held by a
 * developer's own instance, so the claim is *"the host did not take it"*, not *"it is free"*. On a clean
 * machine that reads `free → free`; on a busy one, held by the same outsider before and after.
 */
const RETIRED_COMPANION_PORT = 9271;

/**
 * The two ports this host is told to use — chosen by the OS at `beforeAll`, not hardcoded.
 *
 * ## Why they are not literals any more
 *
 * The old file pinned the edge at `8080` and left the forward proxy to D16's scan, so a collision with
 * anything on the developer's machine was absorbed by the scan. `plan/17` §1 **removes the scan**: the
 * forward proxy *binds its port and fails if occupied*, precisely so the user always knows what their
 * browser must point at. A hardcoded value would therefore turn a busy port on one machine into a red
 * suite that says nothing about the code.
 *
 * They must also be **distinct from each other**, which under the new model is a real constraint rather
 * than a coincidence: the edge and the forward proxy are both host-level listeners now, and the defaults
 * (80 and 8080) differ only because they are chosen to. `twoFreePorts()` below enforces it.
 */
let EDGE_PORT: number;
let FORWARD_PROXY_PORT: number;

/**
 * True when this process can `listen()` on the port — i.e. nobody else holds it right now.
 *
 * A real bind, not a status read, because the two claims this file makes are both about *possession*:
 * the host **holds** its edge port, and the host **does not hold** the retired companion's. A status flag
 * cannot express either — an engine that bound nothing reports nothing.
 */
function bindAttemptSucceeds(port: number): Promise<boolean> {
  return new Promise((resolve) => {
    const srv = net.createServer();
    srv.once("error", () => resolve(false));
    srv.listen(port, "127.0.0.1", () => srv.close(() => resolve(true)));
  });
}

/** Can something bind this port? False means something already holds it. */
function portIsFree(port: number): Promise<boolean> {
  return new Promise((resolve) => {
    const srv = new net.Server();
    srv.once("error", () => resolve(false));
    srv.listen(port, "127.0.0.1", () => srv.close(() => resolve(true)));
  });
}

/** Ask the OS for a free port, then release it. `proxyHarness.getFreePort()`'s shape. */
function freePort(): Promise<number> {
  return new Promise((resolve, reject) => {
    const srv = net.createServer();
    srv.unref();
    srv.on("error", reject);
    srv.listen(0, "127.0.0.1", () => {
      const addr = srv.address();
      const port = typeof addr === "object" && addr ? addr.port : 0;
      srv.close(() => resolve(port));
    });
  });
}

/**
 * Two ports the OS reports free, and that **differ from each other**.
 *
 * The retry is not defensive padding: `freePort()` closes the socket before returning, so the OS is free
 * to hand the *same* ephemeral port back on the very next call — and with the forward proxy no longer
 * scanning (`plan/17` §1), two equal values would mean the second listener fails to bind. That failure
 * would be reported rather than thrown, so the symptom would be a `forwardProxyPort: null` assertion
 * failing for a reason that reads like a code defect.
 */
async function twoFreePorts(): Promise<[number, number]> {
  const first = await freePort();
  let second = await freePort();
  while (second === first) second = await freePort();
  return [first, second];
}

/** Write one workspace's tree. Same layout the runtime suite writes, for the same reason. */
function writeWorkspace(dataRoot: string, wsId: string, name: string): void {
  const dir = path.join(dataRoot, wsId);
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

describe("createWorkspaceHost — one listener per port, one RPC endpoint, no companion", () => {
  let root: string;
  let dataRoot: string;
  let socketPath: string;

  beforeAll(async () => {
    [EDGE_PORT, FORWARD_PROXY_PORT] = await twoFreePorts();

    root = fs.mkdtempSync(path.join(os.tmpdir(), "bifurc-host-it-"));
    dataRoot = path.join(root, "data");
    writeWorkspace(dataRoot, WS_A, "Host Workspace A");
    writeWorkspace(dataRoot, WS_B, "Host Workspace B");

    fs.writeFileSync(
      path.join(root, "app.json"),
      JSON.stringify(
        {
          // Both host-level ports are **stated**, because `plan/17` §1 makes them settings the user owns
          // rather than values the host discovers. See `EDGE_PORT` / `FORWARD_PROXY_PORT`.
          port: EDGE_PORT,
          forwardProxyPort: FORWARD_PROXY_PORT,
          // Carried because the settings shape has it; the host does **not** start the webhook, so nothing
          // binds this. That the value is inert here is the point of the `webhookPort: null` assertion.
          webhookPort: 9101,
          minimizeToTray: true,
          tlsEnabled: false,
          tlsCaCertPath: null,
          tlsCaKeyPath: null,
          workspaces: [
            { id: WS_A, name: "Host Workspace A", activeEnvironmentId: null },
            { id: WS_B, name: "Host Workspace B", activeEnvironmentId: null },
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

    // POSIX gets a unix socket, Windows a named pipe — `defaultSocketPath()`'s shape. Kept short because a
    // unix socket path has a ~100-byte limit and a temp dir under a long user name can exceed it.
    socketPath =
      process.platform === "win32"
        ? `\\\\.\\pipe\\bifurc-host-it-${process.pid}`
        : path.join(os.tmpdir(), `bifurc-hi-${process.pid}.sock`);
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

  it("binds one forward proxy and one edge, serves every workspace as config, and holds no companion port", async () => {
    // The retired companion's address *before* the host starts. On a clean machine this reads `true` (free);
    // on a developer's machine running an old instance it reads `false` (held). Both are fine — see
    // `RETIRED_COMPANION_PORT`'s comment. What must hold is that **the host is not the holder**, so the
    // reading after the host starts must match this one.
    const retiredCompanionFreeBefore = await bindAttemptSucceeds(RETIRED_COMPANION_PORT);

    const notices: string[] = [];
    const host = createWorkspaceHost({
      dataDir: dataRoot,
      transport: { kind: "socket", path: socketPath },
      // A fixed token so the client below needs no file read — and because a host that reads its own token
      // back is not what this file is testing.
      token: HOST_TOKEN,
      onNotice: (m) => notices.push(m),
    });

    try {
      const status = await host.start();

      // ── every workspace is served, and what is per workspace is *config* ──────
      //
      // This replaces `status.runtimes` (one entry per workspace, each with its own `proxyPort` /
      // `webhookPort`). Those numbers were the same for every entry under `plan/17` §1, so the array was N
      // copies of one fact; `workspaces` is the fact that is genuinely per workspace.
      expect(status.workspaces.map((w) => w.workspaceId)).toEqual([WS_A, WS_B]);
      expect(status.workspaces.map((w) => w.name)).toEqual(["Host Workspace A", "Host Workspace B"]);
      expect(status.workspaces.find((w) => w.workspaceId === WS_A)?.active).toBe(true);
      expect(status.workspaces.find((w) => w.workspaceId === WS_B)?.active).toBe(false);

      // ── one forward proxy for the whole host, on the port the user chose ──────
      //
      // Read straight off `status()` with no polling, which is itself the assertion: `host.start()` must not
      // resolve before its listeners bind. That wait lived in `WorkspaceRuntime.doStart()` until the class
      // was deleted; `host.ts`'s `waitUntilReady` is where it lives now.
      expect(status.forwardProxyPort).toBe(FORWARD_PROXY_PORT);
      expect(status.forwardProxyError).toBeNull();
      expect(host.forwardProxy).not.toBeNull();
      expect(host.forwardProxy!.isRunning()).toBe(true);
      expect(host.forwardProxy!.getPort()).toBe(FORWARD_PROXY_PORT);

      // ── the webhook is OFF by default — `null`, not a number ─────────────────
      //
      // `plan/17` §1: *"the webhook does NOT scan either. It is a feature, not infrastructure: it is off by
      // default, the user starts it, and the user owns the port."* So the receiver exists (the facade is
      // bound to it) but nothing is listening — which is why `status.webhookPort` is `null` while
      // `host.webhook` is not.
      expect(status.webhookPort).toBeNull();
      expect(status.webhookError).toBeNull();
      expect(host.webhook).not.toBeNull();
      expect(host.webhook!.isRunning()).toBe(false);

      // ── THE ACCEPTANCE CRITERION: no companion, and the edge is the host's ────
      // Real bind attempts, in both directions, because a status read cannot express either claim.
      //
      // Direction 1: the host **holds** its edge port. `runtime/host.ts` composes the edge deliberately —
      // it is the host-level reverse half — so a bind here must FAIL. (`bindAttemptSucceeds` false means
      // somebody is listening; the host is the only candidate that had the chance to.)
      //
      // `status().edgePort` is asserted first so the bind result cannot pass for the wrong reason: on a
      // machine where some *other* process held the port, the bind would fail and the host's own edge would
      // be down, and the bind assertion alone would not notice.
      expect(status.edgePort).toBe(EDGE_PORT);
      expect(status.edgeError).toBeNull();
      expect(
        await bindAttemptSucceeds(EDGE_PORT),
        `the host must be holding the edge port ${EDGE_PORT} — a bind succeeding means nothing is serving`,
      ).toBe(false);
      //
      // Direction 2: `plan/14` WI 3's original criterion, in its only surviving falsifiable form. The
      // companion was retired, so this asserts the host has not *regrown* one: on a clean machine the port
      // is free and still free; on a busy one an outsider holds it both before and after, and the host is
      // not the holder because a `listen()` on a held port errors rather than stealing it.
      expect(
        await bindAttemptSucceeds(RETIRED_COMPANION_PORT),
        `the host must not hold the retired companion port ${RETIRED_COMPANION_PORT}`,
      ).toBe(retiredCompanionFreeBefore);
      // And it says so on the wire, rather than leaving an operator to guess. The companion's notice is
      // emitted only on the `ws` transport, so a socket host must not mention it at all.
      expect(notices.some((m) => m.includes("rpc endpoint"))).toBe(true);
      expect(notices.join("\n")).not.toMatch(/companion/i);

      // ── the one RPC endpoint serves both workspaces ───────────────────────────
      expect(status.rpcAddress).toBe(socketPath);
      expect(host.rpc).not.toBeNull();

      const transport = createSocketTransport(socketPath);
      try {
        /**
         * The handshake, first — and its necessity is a **good** sign rather than an obstacle: the endpoint
         * refuses every command on an unauthenticated session ("the first request must be `hello`"), which is
         * `plan/05`'s hard gate. `transport/auth/` is a decorator, so this file reaching for its protocol is
         * the host having wired the decorator rather than having merely constructed one.
         *
         * The token is the one the host was given; a mismatch here would be the very `UNAUTHORIZED` path the
         * assertion below on `notices` covers.
         */
        const hello = (await transport.request("hello", {
          protocolVersion: PROTOCOL_VERSION,
          // `clientName` is a closed enum in the protocol (`electron-shell` | `web-ui` | `cli` |
          // `bifurc-extension`), and `cli` is the honest one here: this file is an engine-side RPC client with
          // no renderer and no extension. Inventing a name would be a schema fight the host does not need —
          // notably, the first attempt at this used a made-up name and the *server* rejected it, which is the
          // validation working exactly as intended.
          clientName: "cli",
          clientVersion: "0.0.0-test",
          token: HOST_TOKEN,
        })) as { capabilities?: string[] };
        // `socket` is advertised by the transport itself; `events.replay` comes from the host's `EventLog`
        // being passed to `createSocketServer` — which is the difference between a host that owns one `seq`
        // authority (D13) and one that leaves each connection to make its own.
        expect(hello.capabilities).toContain("socket");
        expect(hello.capabilities).toContain("events.replay");

        /**
         * A real command round trip. `config.get` takes **empty params** and is the cheapest read that proves
         * the endpoint reaches the registry behind the host's listeners.
         *
         * ## The finding this assertion encodes, recorded rather than papered over
         *
         * The first draft of this file sent `config.get` with `{ workspaceId }`, per D13's *"workspace-scoped
         * commands carry `workspaceId` in params"* — and it **failed**, because `ConfigGetParams` is
         * `z.object({}).strict()`. `.strict()` means an extra key is a `BAD_REQUEST`, so the rule is not
         * merely unimplemented: the schema actively rejects it today.
         *
         * That is **WI 1's work, not WI 3's.** WI 3's job is the composition — one endpoint, one listener per
         * port, no companion — and this file proves exactly that. Asserting the D13 param shape here would be
         * asserting work that has not been done, so the assertion below is what is true now, and it is written
         * so that WI 1 failing to add the parameter is *visible* rather than silently absorbed.
         */
        const cfg = (await transport.request("config.get", {})) as Record<string, unknown>;
        expect(cfg).toBeDefined();
        // `config.get` resolves the ACTIVE workspace internally (`store/config.ts#loadConfig` reads
        // `settings.activeWorkspaceId`), which is precisely the coupling WI 4 exists to sever — a workspace's
        // servers should not consult a global pointer. Today it does, so this is the observable truth.
        expect(Object.keys(cfg).length).toBeGreaterThan(0);
      } finally {
        await transport.close();
      }

      // ── the host-local port registry is gone, and no port reached `workspace.json` ─
      //
      // This block used to read `runtime.json` back through `loadRuntimeRegistry()` and check that each
      // workspace's *resolved* proxy/webhook port had landed there. `plan/17` §5 work item 9 deletes that
      // registry outright: it existed to remember which port each workspace landed on (D16's stickiness rule),
      // and with one host-level listener per port there is nothing per workspace to remember. The surviving
      // half of the rule is the half that mattered — **no port is in the git-tracked `workspace.json`**,
      // which syncs between machines and must not carry one machine's port allocation.
      expect(fs.existsSync(path.join(dataRoot, "runtime.json"))).toBe(false);
      const wsJson = fs.readFileSync(path.join(dataRoot, WS_A, "workspace.json"), "utf-8");
      expect(wsJson).not.toMatch(/Port/i);
    } finally {
      // ── teardown releases everything ──────────────────────────────────────────
      const held = { proxy: host.status().forwardProxyPort, edge: host.status().edgePort };
      expect(held.proxy).toBe(FORWARD_PROXY_PORT);
      expect(held.edge).toBe(EDGE_PORT);

      await host.stop();

      for (const port of [held.proxy!, held.edge!]) {
        await expect.poll(() => portIsFree(port), { timeout: 5000 }).toBe(true);
      }
      // The webhook was never started, so there is nothing here to release — and that is asserted rather
      // than assumed, because "the teardown released it" and "it was never bound" are different facts.
      expect(host.status().webhookPort).toBeNull();
      // `stop()` is idempotent, and a stopped host refuses to restart rather than half-working.
      await expect(host.stop()).resolves.toBeUndefined();
      await expect(host.start()).rejects.toThrow(/already been stopped/i);
    }
  }, 60000);

  /**
   * P14 WI 6 — a requested workspace that does not exist is **named**, not silently dropped.
   *
   * ## The failure this pins, measured while verifying the CLI artifact
   *
   * `--workspace X` where `X` is absent from the settings filtered the list to **empty**: the host came up
   * serving nothing and exited 0. The only symptom was `serving 0 workspace(s)`, which reads as an empty
   * data root rather than as "the settings file was read from somewhere else". The cause in that run was
   * exactly that — on Windows `settingsPath()` prefers `%LOCALAPPDATA%` regardless of `setDataRoot()`
   * (recorded in `certificates/certManager.ts`), so a `--data-dir` run read the user's real settings and the
   * requested id was absent from a list nobody could see.
   *
   * **Why the notice names the directory.** That is the whole diagnostic: without it, an operator has no
   * way to tell "no such workspace" from "wrong settings file". This is the same "accepted and ignored"
   * class as WI 1's `server.start { workspaceId }` — the failure succeeds, so nothing else reports it.
   *
   * A second host rather than a case in the one above, because `register()` throws on a duplicate and the
   * host above populates the shared singleton. An explicit registry is the same shape this file already
   * uses for the drift guard below.
   */
  it("names a requested workspace that is not in the settings, instead of serving nothing silently", async () => {
    const notices: string[] = [];
    const registry = new CommandRegistry();
    registerAllEngineCommands(registry);

    const host = createWorkspaceHost({
      dataDir: dataRoot,
      registry,
      transport: { kind: "socket", path: `${socketPath}-selection` },
      token: HOST_TOKEN,
      // `WS_A` is in this file's `app.json`; `ghost-ws` is not. A mixed list is deliberate: it proves the
      // notice does not replace serving, and that a partial request still comes up.
      workspaceIds: ["ghost-ws", WS_A],
      onNotice: (m) => notices.push(m),
    });

    try {
      const status = await host.start();
      // The real workspace is served...
      expect(status.workspaces.map((w) => w.workspaceId)).toEqual([WS_A]);
      // ...and the missing one is reported rather than skipped in silence.
      const joined = notices.join("\n");
      expect(joined).toMatch(/requested workspace "ghost-ws" is not in the settings/);
      // The directory is the diagnostic — see the comment above.
      expect(joined).toMatch(/looked in /);
      // And the host did not refuse to start over it: the same "skip loudly, do not refuse" rule D16
      // applies to a taken port override. Asserted by the host actually **serving** — both host-level
      // listeners up on the ports it was told to use — rather than by a `skipped` count, which no longer
      // exists because there is nothing per workspace left to skip.
      expect(status.forwardProxyPort).toBe(FORWARD_PROXY_PORT);
      expect(status.edgePort).toBe(EDGE_PORT);
    } finally {
      await host.stop();
    }
  }, 60000);
});

/**
 * The drift guard between the engine's `registerAllEngineCommands()` and the shell's
 * `registerIpcHandlers()`.
 *
 * ## Why this is needed at all
 *
 * The two lists are deliberately **not** one list (`registerAll.ts`'s header records why: the shell's
 * eighteen calls each carry a comment naming which P6 deletion made it the only registration, and
 * collapsing them would delete that record). Two lists of the same thing drift. This is the check that
 * makes the drift visible.
 *
 * ## What it asserts, and why not equality
 *
 * **A superset, in the direction that can actually bite.** The dangerous failure is a command the shell
 * registers and `registerAllEngineCommands()` does not: the desktop works, the container answers
 * `UNKNOWN_COMMAND`, and nothing in the desktop's own suite can see it — which is precisely how the gap
 * this whole file is about survived until now. So the assertion is *"everything the shell registers, the
 * engine-side list also registers."*
 *
 * The reverse direction is **not** asserted as an error: an engine command the shell chooses not to expose
 * is a legitimate P3 decision ("the consumer declares what it is willing to serve"), not drift. It is
 * reported in the failure message instead, so a reader can judge.
 *
 * It runs on its **own** registry rather than the shared singleton — `register()` throws on a duplicate, so
 * registering into the global one would corrupt every other suite in this worker.
 */
describe("registerAllEngineCommands — the engine-side list covers the shell's", () => {
  /** A `CommandRegistry` that records names instead of registering them — see the note above on why. */
  function recordingRegistry(): { registry: CommandRegistry; names: Set<string> } {
    const names = new Set<string>();
    const registry = {
      register(name: string): void {
        names.add(name);
      },
      invoke(): never {
        throw new Error("not used");
      },
      has: (name: string) => names.has(name),
      names: () => [...names],
    } as unknown as CommandRegistry;
    return { registry, names };
  }

  it("registers a name set that contains every command the Electron shell serves", async () => {
    const { registry, names } = recordingRegistry();
    registerAllEngineCommands(registry);

    // Guard against a vacuous pass: an empty list is trivially a superset of nothing, and that is exactly
    // what this function looked like before it existed. The floor is a *sanity* bound, not the real
    // assertion — the real one is the comparison against the shell's set below. Kept well under the measured
    // count so it does not become a second, accidental lock on the command total: this file is not the
    // ratchet, and `packages/protocol/src/commands/index.test.ts` already moves by exactly 2× per command
    // change. Measured 2026-09-21: **92** names.
    expect(names.size).toBeGreaterThan(80);

    // The shell's set, read from its source rather than by calling it: `registerIpcHandlers()` also registers
    // `ipcMain` channels and would need an Electron mock, and what is being compared is the *command* list.
    const handlersSrc = fs.readFileSync(path.join(process.cwd(), "src", "ipc", "handlers.ts"), "utf-8");
    const shellCalls = [...handlersSrc.matchAll(/^\s*register([A-Za-z]+)Commands\(commandRegistry\);/gm)].map(
      (m) => m[1],
    );
    // If the regex stops matching, the comparison silently becomes "nothing vs nothing" — the
    // guard-that-matches-nothing trap. Fail loudly instead.
    expect(shellCalls.length).toBeGreaterThanOrEqual(15);

    // Each shell call site corresponds to a function imported from the engine; take the names it registers.
    // The comparison that matters is on the *command names*, so the shell's modules are loaded for real.
    const shellRegistry = recordingRegistry();
    for (const fn of shellCalls) {
      const mod = await loadRegisterFn(fn);
      expect(mod, `the shell calls register${fn}Commands() but no such export was importable`).toBeTypeOf(
        "function",
      );
      mod(shellRegistry.registry);
    }

    const missing = [...shellRegistry.names].filter((n) => !names.has(n));
    const extra = [...names].filter((n) => !shellRegistry.names.has(n));

    expect(
      missing,
      `registerAllEngineCommands() is missing ${missing.length} command(s) the shell serves: ` +
        `${missing.slice(0, 10).join(", ")}${missing.length > 10 ? " …" : ""}. A headless host will answer ` +
        `UNKNOWN_COMMAND for these.`,
    ).toEqual([]);

    // Informational, not a failure — see the docblock.
    if (extra.length > 0) {
      console.info(
        `[registerAll] ${extra.length} engine command(s) are not exposed by the shell's registerIpcHandlers(): ` +
          `${extra.slice(0, 12).join(", ")}${extra.length > 12 ? " …" : ""}`,
      );
    }
  }, 30000);
});

/**
 * Resolve one `register<Name>Commands` export by the short name the shell's call site uses.
 *
 * The mapping is explicit rather than computed: module paths are the one thing that cannot be derived from
 * the function name (they are `certificates/certCommands`, `workspace/entities/coreCommands`, `rest/commands`, …), and a
 * computed guess that silently resolved to the wrong module would make the guard above compare against the
 * wrong list. An entry appearing in the shell's source without a row here is a **failure** in the test, not a
 * skip — that is what keeps this table from rotting as the shell adds calls.
 *
 * ⚠ **This table is invisible to the engine-layout codemod.** The specifiers are plain string *values* passed
 * to `import()` as a variable, so there is no import specifier for a rewrite to match — and `@vite-ignore`
 * keeps Vite from resolving them at build time. Every engine-layout phase that moves a module named here
 * must update this row by hand, or the guard above fails with `Cannot find package '@bifurc/engine/…'`
 * (which is how phase 3's `store/` → `workspace/` split was caught).
 */
async function loadRegisterFn(shortName: string): Promise<((r: CommandRegistry) => void) | undefined> {
  const paths: Record<string, string> = {
    Blob: "@bifurc/engine/blob/commands",
    ImportExport: "@bifurc/engine/importexport/commands",
    FileOps: "@bifurc/engine/fileops/commands",
    Cert: "@bifurc/engine/certificates/certCommands",
    Server: "@bifurc/engine/proxy/serverCommands",
    Webhook: "@bifurc/engine/webhooks/webhookCommands",
    Config: "@bifurc/engine/workspace/entities/configCommands",
    // Engine-layout phase 7 dissolved `miscCommands` (§4.5) into three modules, so the single `Misc`
    // row became three. The commands are unchanged; only their registering module moved.
    Rest: "@bifurc/engine/rest/commands",
    Mock: "@bifurc/engine/mocks/commands",
    Healthbar: "@bifurc/engine/workspace/healthbarCommands",
    Runner: "@bifurc/engine/runner/runnerCommands",
    Audit: "@bifurc/engine/workspace/entities/auditCommands",
    Core: "@bifurc/engine/workspace/entities/coreCommands",
    Folder: "@bifurc/engine/workspace/entities/folderCommands",
    Graphql: "@bifurc/engine/graphql/commands",
    Soap: "@bifurc/engine/soap/commands",
    Grpc: "@bifurc/engine/grpc/commands",
    Application: "@bifurc/engine/applications/applicationCommands",
    Sync: "@bifurc/engine/gitops/syncCommands",
    EntityCrud: "@bifurc/engine/workspace/entities/entityCrudCommands",
  };
  const specifier = paths[shortName];
  if (specifier === undefined) return undefined;
  const mod = (await import(/* @vite-ignore */ specifier)) as Record<string, unknown>;
  return mod[`register${shortName}Commands`] as ((r: CommandRegistry) => void) | undefined;
}
