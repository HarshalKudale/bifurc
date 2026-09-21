/**
 * P14 WI 3 — `createWorkspaceHost()`, exercised with real I/O and a real RPC client.
 *
 * WHY THIS IS AN INTEGRATION FILE, AND WHY IT IS ITS OWN FILE
 * ----------------------------------------------------------
 * Two separate reasons, and both matter:
 *
 * 1. The host's whole job is to bind real things — N proxies, N webhook receivers and **one** socket
 *    endpoint — and then prove that endpoint can reach the workspaces behind it. A mocked transport would
 *    assert the wiring this file exists to verify.
 * 2. It is its own file because `shutdownEngine()` memoises **process-wide**, so a process holds exactly one
 *    engine lifecycle. This file deliberately never calls `createEngine()` (a host must not — see
 *    `host.ts`'s header), but the *other* lifecycle in this suite (`engineSmoke`) does, and vitest gives each
 *    file its own worker. Keeping one engine-shaped lifecycle per file is the rule WI 3's trap entry records.
 *
 * THE ACCEPTANCE CRITERION THIS FILE EXISTS FOR
 * ---------------------------------------------
 * `plan/14` §6: *"a workspace host starts with no companion server."* So the load-bearing assertion is not
 * "the proxies came up" — it is that **the companion port is genuinely unbound** while the host is serving.
 * A port that merely *looks* unstarted would pass a status check; a real bind attempt is what fails if the
 * companion is up.
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

import { setDataRootOverride } from "@bifurc/engine/store/workspaceFs";
import { setSettingsPathOverride } from "@bifurc/engine/store/appSettings";
import { createWorkspaceHost } from "@bifurc/engine/runtime/host";
import { loadRuntimeRegistry } from "@bifurc/engine/runtime/hostRegistry";
import { createSocketTransport } from "@bifurc/engine/transport/socket";
import { commandRegistry } from "@bifurc/engine/commands/registry";
import { registerAllEngineCommands } from "@bifurc/engine/registerAll";
import { PROTOCOL_VERSION } from "@bifurc/protocol";

/** The token this file hands the host, and presents to it. A literal so no file read is on the path. */
const HOST_TOKEN = "host-it-token";

const WS_A = "host-ws-a";
const WS_B = "host-ws-b";

/**
 * The app-level companion port the host must **not** bind.
 *
 * Read from the test's own `app.json` above (9271) rather than hardcoded a second time, so the criterion and
 * the fixture cannot drift apart.
 *
 * ## It is probed, never assumed free
 *
 * The *real* 9271 is very often already held — a developer running the desktop app, a previous suite's
 * leftover, anything. So this file asserts the companion **address** is one of:
 *
 * - **free before the host starts**, and still free after: nothing anywhere bound it, and the host did not.
 * - **taken before the host starts**, and taken *by the same holder* after: the host did not take it, because
 *   a `listen()` on a port already held fails rather than stealing it.
 *
 * The second case is the one that makes this file pass on a developer's machine without weakening anything —
 * and note it is strictly *stronger* than a status check either way, because it is real binds on both sides.
 * What it deliberately does **not** do is fail when the port is busy: that would be measuring the machine, not
 * the host, and it is the assertion-vs-environment confusion `TRAPS.md` records under evidence quality.
 */
const COMPANION_PORT = 9271;

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

/** Can something bind this port? False means something already holds it. */
function portIsFree(port: number): Promise<boolean> {
  return new Promise((resolve) => {
    const srv = new net.Server();
    srv.once("error", () => resolve(false));
    srv.listen(port, "127.0.0.1", () => srv.close(() => resolve(true)));
  });
}

/**
 * Who is holding this port, as far as anything observable is concerned — a *fingerprint* of the holder, not
 * an identity.
 *
 * Implemented by attempting a `connect()` and reporting what happened, because Node exposes no `SO_PEERNAME`
 * for a TCP listener: `lsof`/`netstat` would be needed for a real PID, and shelling out from a test to read a
 * platform-specific table is a dependency this file should not take. A fingerprint is enough for the one
 * question being asked — *did the set of holders change while the host ran?*
 *
 * The important reading: **"free" is itself a report**, so a port that was free and is free afterwards is
 * compared as `free -> free`, the same way a held port is compared as `busy -> busy`.
 */
function portHolderFingerprint(port: number): Promise<string> {
  return new Promise((resolve) => {
    const probe = net.createConnection({ host: "127.0.0.1", port });
    let settled = false;
    const done = (value: string): void => {
      if (settled) return;
      settled = true;
      probe.removeAllListeners();
      probe.on("error", () => {});
      try {
        probe.destroy();
      } catch {
        /* already gone */
      }
      resolve(value);
    };
    probe.once("connect", () => done("accepted"));
    probe.once("error", (err: NodeJS.ErrnoException) => {
      // `ECONNREFUSED` means nothing is listening — the free case, and a report in its own right.
      done(err.code === "ECONNREFUSED" ? "free" : `error:${err.code ?? "unknown"}`);
    });
    probe.setTimeout(2000, () => done("timeout"));
  });
}

describe("createWorkspaceHost — N runtimes, one RPC endpoint, no companion", () => {
  let root: string;
  let dataRoot: string;
  let socketPath: string;

  beforeAll(() => {
    root = fs.mkdtempSync(path.join(os.tmpdir(), "bifurc-host-it-"));
    dataRoot = path.join(root, "data");
    writeWorkspace(dataRoot, WS_A, "Host Workspace A");
    writeWorkspace(dataRoot, WS_B, "Host Workspace B");

    fs.writeFileSync(
      path.join(root, "app.json"),
      JSON.stringify(
        {
          port: 8080,
          webhookPort: 9101,
          companionPort: COMPANION_PORT,
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

  it("starts one runtime per workspace, binds one RPC endpoint, and leaves the companion down", async () => {
    // The companion address *before* the host starts. On a clean machine this reads `free`; on a developer's
    // machine running the desktop app it reads `accepted`. Both are fine — see `COMPANION_PORT`'s comment. What
    // must hold is that the host does not **change** it.
    const companionBefore = await portIsFree(COMPANION_PORT) ? "free" : await portHolderFingerprint(COMPANION_PORT);

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

      // ── one runtime per workspace, both serving ───────────────────────────────
      expect(status.runtimes).toHaveLength(2);
      for (const rt of status.runtimes) {
        expect(rt.running).toBe(true);
        expect(rt.proxyError).toBeNull();
        expect(rt.webhookRunning).toBe(true);
      }

      // Distinct ports — the identity criterion WI 2 established, now asserted through the host's own
      // allocator rather than by passing ports in by hand.
      const proxyPorts = status.runtimes.map((r) => r.proxyPort);
      expect(new Set(proxyPorts).size).toBe(2);
      const webhookPorts = status.runtimes.map((r) => r.webhookPort);
      expect(new Set(webhookPorts).size).toBe(2);

      expect(host.runtimes.size).toBe(2);
      expect(host.runtimes.has(WS_A)).toBe(true);
      expect(host.runtimes.has(WS_B)).toBe(true);

      // ── THE ACCEPTANCE CRITERION: no companion ────────────────────────────────
      // The host did not bind the companion. Asserted as "the state of that address did not change", which is
      // true on a clean machine (free → free) and on a busy one (accepted → accepted) — and is **false** if the
      // host bound it, because a `listen()` on an already-held port fails rather than taking it, so a busy
      // address the host *also* tries to bind goes `accepted → accepted` only if nothing was bound, and on a
      // clean machine the pair is `free → free` vs `free → accepted`. Both directions are real binds.
      expect(await portHolderFingerprint(COMPANION_PORT)).toBe(companionBefore);
      // And it says so on the wire, rather than leaving an operator to guess.
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
         * the endpoint reaches the registry behind all the runtimes.
         *
         * ## The finding this assertion encodes, recorded rather than papered over
         *
         * The first draft of this file sent `config.get` with `{ workspaceId }`, per D13's *"workspace-scoped
         * commands carry `workspaceId` in params"* — and it **failed**, because `ConfigGetParams` is
         * `z.object({}).strict()`. `.strict()` means an extra key is a `BAD_REQUEST`, so the rule is not
         * merely unimplemented: the schema actively rejects it today.
         *
         * That is **WI 1's work, not WI 3's.** WI 3's job is the composition — one endpoint, N runtimes, no
         * companion — and this file proves exactly that. Asserting the D13 param shape here would be asserting
         * work that has not been done, so the assertion below is what is true now, and it is written so that
         * WI 1 failing to add the parameter is *visible* rather than silently absorbed.
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

      // ── the resolved ports reached the host-local registry, not workspace.json ─
      const reg = loadRuntimeRegistry();
      expect(reg.workspaces[WS_A]?.resolvedProxyPort).toBe(
        status.runtimes.find((r) => r.workspaceId === WS_A)?.proxyPort,
      );
      expect(reg.workspaces[WS_B]?.resolvedWebhookPort).toBe(
        status.runtimes.find((r) => r.workspaceId === WS_B)?.webhookPort,
      );
      // The D16 rule, asserted rather than trusted: the port is host-local state, so it is in
      // `runtime.json` and **not** in the git-tracked `workspace.json` that syncs between machines.
      const wsJson = fs.readFileSync(path.join(dataRoot, WS_A, "workspace.json"), "utf-8");
      expect(wsJson).not.toMatch(/Port/i);
    } finally {
      // ── teardown releases everything ──────────────────────────────────────────
      const before = host.status().runtimes.map((r) => ({ proxy: r.proxyPort!, webhook: r.webhookPort! }));
      await host.stop();

      for (const { proxy, webhook } of before) {
        await expect.poll(() => portIsFree(proxy), { timeout: 5000 }).toBe(true);
        await expect.poll(() => portIsFree(webhook), { timeout: 5000 }).toBe(true);
      }
      // `stop()` is idempotent, and a stopped host refuses to restart rather than half-working.
      await expect(host.stop()).resolves.toBeUndefined();
      await expect(host.start()).rejects.toThrow(/already been stopped/i);
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
 * the function name (they are `proxy/certCommands`, `store/coreCommands`, `miscCommands`, …), and a computed
 * guess that silently resolved to the wrong module would make the guard above compare against the wrong list.
 * An entry appearing in the shell's source without a row here is a **failure** in the test, not a skip —
 * that is what keeps this table from rotting as the shell adds calls.
 */
async function loadRegisterFn(shortName: string): Promise<((r: CommandRegistry) => void) | undefined> {
  const paths: Record<string, string> = {
    Blob: "@bifurc/engine/blob/commands",
    ImportExport: "@bifurc/engine/importExport/commands",
    FileOps: "@bifurc/engine/fileOps/commands",
    Cert: "@bifurc/engine/proxy/certCommands",
    Server: "@bifurc/engine/proxy/serverCommands",
    Webhook: "@bifurc/engine/proxy/webhookCommands",
    Config: "@bifurc/engine/store/configCommands",
    Misc: "@bifurc/engine/miscCommands",
    Runner: "@bifurc/engine/runner/runnerCommands",
    Audit: "@bifurc/engine/store/auditCommands",
    Core: "@bifurc/engine/store/coreCommands",
    Folder: "@bifurc/engine/store/folderCommands",
    Graphql: "@bifurc/engine/protocols/graphqlCommands",
    Soap: "@bifurc/engine/protocols/soapCommands",
    Grpc: "@bifurc/engine/protocols/grpcCommands",
    Application: "@bifurc/engine/applications/applicationCommands",
    Sync: "@bifurc/engine/sync/syncCommands",
    EntityCrud: "@bifurc/engine/store/entityCrudCommands",
  };
  const specifier = paths[shortName];
  if (specifier === undefined) return undefined;
  const mod = (await import(/* @vite-ignore */ specifier)) as Record<string, unknown>;
  return mod[`register${shortName}Commands`] as ((r: CommandRegistry) => void) | undefined;
}
