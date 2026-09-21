/**
 * P14 WI 7 — a workspace-scoped command reaches **the workspace it names**, not the active one.
 *
 * WHY THIS FILE EXISTS
 * --------------------
 * WI 1's protocol half gave ten commands an optional `workspaceId`. It did not make them *read* it, and
 * nothing in the suite noticed, because the module facade those commands called is **correct for the
 * desktop**: `createProxyServer({ workspaceId: null })` means "the active workspace", which is the only
 * thing a single-workspace desktop has. The bug only exists on a host serving **two** workspaces, and it
 * was found by probing one, not by reading:
 *
 * ```
 * host: ws-A proxy=81 webhook=9101     ws-B proxy=82 webhook=9102
 * server.status                 → { running: false, port: 0 }    ← two members ARE listening
 * server.start {workspaceId: A} → { ok: true }                    ← accepted, IGNORED
 * server.status                 → { running: false, port: 8791 }  ← a THIRD server
 * ```
 *
 * **The failure succeeds**, so every assertion in this file is written to fail on that shape: each one
 * names a workspace whose answer differs from the active one's, and asserts on **identity** — a specific
 * port, a specific map's count — rather than on "something plausible came back". `activeWorkspaceId` is
 * `WS_A` throughout, and the interesting cases address `WS_B`.
 *
 * WHY ITS OWN FILE
 * ----------------
 * `createWorkspaceHost()` registers the whole command surface into whatever registry it is given, and
 * `register()` **throws on a duplicate**. This file passes its own registry rather than letting the host
 * populate the global singleton, so it can run alongside any other suite in its own worker without either
 * double-registering. That is also why the transport is a socket in a temp dir: the host needs an endpoint
 * to exist, but nothing here goes over it — the commands are invoked directly, which is the layer under
 * test.
 */
import { describe, it, expect, beforeAll, afterAll } from "vitest";
import * as fs from "fs";
import * as os from "os";
import * as path from "path";
import * as net from "net";

import { setDataRootOverride } from "@bifurc/engine/store/workspaceFs";
import { setSettingsPathOverride } from "@bifurc/engine/store/appSettings";
import { createWorkspaceHost, type WorkspaceHost } from "@bifurc/engine/runtime/host";
import { CommandRegistry, type CommandContext } from "@bifurc/engine/commands/registry";
import { registerAllEngineCommands } from "@bifurc/engine/registerAll";
import { BLOB_INLINE_THRESHOLD_BYTES } from "@bifurc/protocol";

const WS_A = "scope-ws-a";
const WS_B = "scope-ws-b";
/** Deliberately not a workspace that exists — the `absent` case. */
const WS_GHOST = "scope-ws-ghost";

/** `bus` is only read by handlers that emit; none of these do. The shape is the whole requirement. */
const ctx: CommandContext = { bus: {} as CommandContext["bus"] };

/** Bind port 0 to let the OS pick a free one, then release it. The same trick `deadPort()` uses. */
function freePort(): Promise<number> {
  return new Promise((resolve, reject) => {
    const s = new net.Server();
    s.once("error", reject);
    s.listen(0, "127.0.0.1", () => {
      const addr = s.address();
      const port = typeof addr === "object" && addr ? addr.port : 0;
      s.close(() => resolve(port));
    });
  });
}

/** True when a real `listen()` on `port` succeeds. Not a probe-and-close — see D16 on the TOCTOU race. */
function portIsFree(port: number): Promise<boolean> {
  return new Promise((resolve) => {
    const s = new net.Server();
    const done = (free: boolean): void => {
      s.removeAllListeners();
      s.on("error", () => {});
      s.close(() => resolve(free));
    };
    s.once("error", () => done(false));
    s.once("listening", () => done(true));
    try {
      s.listen(port, "127.0.0.1");
    } catch {
      done(false);
    }
  });
}

function writeWorkspace(dataRoot: string, id: string, name: string): void {
  const dir = path.join(dataRoot, id);
  fs.mkdirSync(dir, { recursive: true });
  fs.writeFileSync(path.join(dir, "workspace.json"), JSON.stringify({ id, name, createdAt: 0 }, null, 2), "utf-8");
}

describe("workspace-scoped commands resolve the named workspace (P14 WI 7)", () => {
  let root: string;
  let dataRoot: string;
  let socketPath: string;
  /** The port the **facade** would bind if a command fell through to it. See the "no third server" case. */
  let configPort: number;
  let host: WorkspaceHost;
  let registry: CommandRegistry;

  /** A runtime's live proxy port, read from the host's own handle rather than from the command under test. */
  function proxyPortOf(workspaceId: string): number {
    const rt = host.runtimes.get(workspaceId);
    if (!rt) throw new Error(`no runtime for ${workspaceId}`);
    const st = rt.status();
    if (st.proxyPort === null) throw new Error(`${workspaceId} has no proxy port`);
    return st.proxyPort;
  }

  function webhookPortOf(workspaceId: string): number {
    const rt = host.runtimes.get(workspaceId);
    if (!rt) throw new Error(`no runtime for ${workspaceId}`);
    const st = rt.status();
    if (st.webhookPort === null) throw new Error(`${workspaceId} has no webhook port`);
    return st.webhookPort;
  }

  beforeAll(async () => {
    root = fs.mkdtempSync(path.join(os.tmpdir(), "bifurc-scope-it-"));
    dataRoot = path.join(root, "data");
    writeWorkspace(dataRoot, WS_A, "Scope A");
    writeWorkspace(dataRoot, WS_B, "Scope B");

    // The facade branch reads `cfg.port`. Probing for a free one rather than hardcoding means the
    // "no third server" assertion below is about behaviour and not about whether 8791 happened to be taken
    // on the machine running the suite.
    configPort = await freePort();

    fs.writeFileSync(
      path.join(root, "app.json"),
      JSON.stringify(
        {
          port: configPort,
          webhookPort: 9101,
          companionPort: 9271,
          minimizeToTray: true,
          tlsEnabled: false,
          tlsCaCertPath: null,
          tlsCaKeyPath: null,
          workspaces: [
            { id: WS_A, name: "Scope A", activeEnvironmentId: null },
            { id: WS_B, name: "Scope B", activeEnvironmentId: null },
          ],
          // **The active workspace is A, and the discriminating assertions address B.** That is the whole
          // design of this file: with A active, a command that ignored `workspaceId` would answer for A,
          // so every B assertion fails on the old code and passes on the new one.
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
        ? `\\\\.\\pipe\\bifurc-scope-it-${process.pid}`
        : path.join(os.tmpdir(), `bifurc-si-${process.pid}.sock`);

    // An explicit registry: `register()` throws on a duplicate, so letting the host populate the global
    // singleton would collide with any other suite sharing this worker.
    registry = new CommandRegistry();
    registerAllEngineCommands(registry);

    host = createWorkspaceHost({
      dataDir: dataRoot,
      registry,
      transport: { kind: "socket", path: socketPath },
      token: "scope-it-token",
      onNotice: () => {},
    });
    await host.start();
  });

  afterAll(async () => {
    await host.stop();
    setDataRootOverride(null);
    setSettingsPathOverride(null);
    try {
      fs.rmSync(root, { recursive: true, force: true });
    } catch {
      /* best-effort */
    }
  });

  it("server.status names the workspace it was asked about, not the active one", () => {
    const a = proxyPortOf(WS_A);
    const b = proxyPortOf(WS_B);
    // Guard the premise: if the allocator ever gave both workspaces the same port, every assertion below
    // would pass for the wrong reason.
    expect(a).not.toBe(b);

    const statusA = registry.invoke("server.status", { workspaceId: WS_A }, ctx) as {
      running: boolean;
      port: number;
      workspaceId?: string;
    };
    const statusB = registry.invoke("server.status", { workspaceId: WS_B }, ctx) as {
      running: boolean;
      port: number;
      workspaceId?: string;
    };

    // The bug's first symptom was `{ running: false, port: 0 }` while both members were listening.
    expect(statusA.running).toBe(true);
    expect(statusB.running).toBe(true);
    // **Identity, not plausibility.** Each answer is the port that workspace's own runtime bound.
    expect(statusA.port).toBe(a);
    expect(statusB.port).toBe(b);
    // And the response says which workspace it describes, so a multiplexed client can match reply to
    // request — `ServerStatusResult` has carried the field for exactly this.
    expect(statusA.workspaceId).toBe(WS_A);
    expect(statusB.workspaceId).toBe(WS_B);
  });

  it("omitting workspaceId resolves to the active workspace — the desktop's contract", () => {
    const status = registry.invoke("server.status", {}, ctx) as { port: number; workspaceId?: string };
    // A is active, so the id-less answer is A's. This is the row that keeps the desktop working, and it is
    // why the parameter is optional rather than required (see `protocol/commands/scope.ts`).
    expect(status.port).toBe(proxyPortOf(WS_A));

    // **And the echo is absent, deliberately.** `workspaceId` is only echoed when the caller supplied one:
    // the field exists so a *multiplexed* client can match reply to request, and a multiplexed client is
    // exactly one that sends an id. Echoing it unconditionally would change the payload of every existing
    // status call including the desktop's, which WI 7's acceptance criterion forbids — and
    // `tests/ipc/handlers.test.ts:764` pins that payload with `toEqual`. That test caught the first version
    // of this code, which echoed always; the test was right and the code was wrong.
    expect(status.workspaceId).toBeUndefined();
    expect(Object.keys(status).sort()).toEqual(["error", "port", "running"]);
  });

  it("server.start { workspaceId: A } binds nothing new — the third server is gone", async () => {
    // Capture the state *before* the call, so "nothing changed" is an assertion rather than a tautology.
    const beforeA = proxyPortOf(WS_A);
    const beforeB = proxyPortOf(WS_B);

    // The exact call that used to return `{ ok: true }` while binding `cfg.port`. `ok` is still true, and
    // that is correct: A is already serving, which is what the caller asked for. What must NOT happen is a
    // bind, so the assertion is about the port rather than about the return value — **`ok: true` is what
    // made this bug invisible for a whole phase.**
    const res = registry.invoke("server.start", { workspaceId: WS_A }, ctx) as { ok: boolean };
    expect(res.ok).toBe(true);

    // The facade branch's port is still free, so no third server took it. A real `listen()`, not a probe.
    expect(await portIsFree(configPort)).toBe(true);

    // And both members are on exactly the ports they had.
    expect(proxyPortOf(WS_A)).toBe(beforeA);
    expect(proxyPortOf(WS_B)).toBe(beforeB);
    expect((registry.invoke("server.status", { workspaceId: WS_A }, ctx) as { port: number }).port).toBe(beforeA);
  });

  it("an unknown workspace is `absent`, never a silent redirect to the active one", () => {
    const status = registry.invoke("server.status", { workspaceId: WS_GHOST }, ctx) as {
      running: boolean;
      port: number;
      error: string | null;
      workspaceId?: string;
    };
    expect(status.running).toBe(false);
    expect(status.port).toBe(0);
    // **This is the assertion the third branch exists for.** Falling through to the facade would report
    // the *active* workspace's port here and claim success — acting on one workspace while the caller
    // asked about another, which is the original bug with a different id.
    expect(status.port).not.toBe(proxyPortOf(WS_A));
    expect(status.error).toMatch(/no runtime/i);
    expect(status.workspaceId).toBe(WS_GHOST);

    // The lifecycle commands agree rather than pretending.
    expect((registry.invoke("server.start", { workspaceId: WS_GHOST }, ctx) as { ok: boolean }).ok).toBe(false);
    expect((registry.invoke("server.stop", { workspaceId: WS_GHOST }, ctx) as { ok: boolean }).ok).toBe(false);
    expect((registry.invoke("proxy.status", { workspaceId: WS_GHOST }, ctx) as { running: boolean }).running).toBe(
      false,
    );
  });

  it("webhookServer.status reports the named workspace's receiver", () => {
    const statusA = registry.invoke("webhookServer.status", { workspaceId: WS_A }, ctx) as {
      running: boolean;
      port: number;
    };
    const statusB = registry.invoke("webhookServer.status", { workspaceId: WS_B }, ctx) as {
      running: boolean;
      port: number;
    };
    expect(statusA.running).toBe(true);
    expect(statusB.running).toBe(true);
    expect(statusA.port).toBe(webhookPortOf(WS_A));
    expect(statusB.port).toBe(webhookPortOf(WS_B));
    expect(statusA.port).not.toBe(statusB.port);
  });

  it("webhook.registerActive writes into the NAMED workspace's map, not the active one's", () => {
    const rtA = host.runtimes.get(WS_A)!;
    const rtB = host.runtimes.get(WS_B)!;
    const beforeA = rtA.webhook.activeCount();
    const beforeB = rtB.webhook.activeCount();

    // Registering for **B** while **A** is active is the discriminating choice: the old code wrote into the
    // facade's map, which means the active workspace's — so this case would land in A and fail here.
    const res = registry.invoke(
      "webhook.registerActive",
      { workspaceId: WS_B, webhookId: "wh-1", urlSuffix: "hook-b" },
      ctx,
    ) as { ok: boolean };
    expect(res.ok).toBe(true);

    expect(rtB.webhook.activeCount()).toBe(beforeB + 1);
    expect(rtA.webhook.activeCount()).toBe(beforeA);

    // And it unregisters from the same place, which is the other half of the pair.
    expect(
      (registry.invoke("webhook.unregisterActive", { workspaceId: WS_B, webhookId: "wh-1" }, ctx) as { ok: boolean })
        .ok,
    ).toBe(true);
    expect(rtB.webhook.activeCount()).toBe(beforeB);
  });

  it("an egress artifact above the inline threshold is readable by the workspace that asked for it", () => {
    /**
     * P14 WI 8. The egress commands' `workspaceId` exists for the **blob half**: above
     * `BLOB_INLINE_THRESHOLD_BYTES` the engine stages an artifact and the client reads it with
     * `blob.read` scoped to its own workspace. Without the field the engine staged host-wide, so a
     * scoped read found nothing — the engine's own export unreadable by the client that requested it.
     *
     * **`capture.shareJson` because its payload size is ours to set.** The `entries` come from the
     * client, so this can cross the threshold without a large fixture on disk or a slow render. The
     * other two (`runner.exportReport`, `audit.export`) are wired identically and asserted at the
     * schema level in `packages/protocol/src/commands/index.test.ts`.
     */
    const big = "x".repeat(BLOB_INLINE_THRESHOLD_BYTES + 1024);

    const res = registry.invoke(
      "capture.shareJson",
      { workspaceId: WS_B, entries: [big] },
      ctx,
    ) as { blobId?: string; inline?: string };

    // **Guard the premise.** If the artifact came back inline, the blob branch was never taken and the
    // two assertions below would pass while testing nothing about namespaces.
    expect(res.blobId).toBeDefined();
    expect(res.inline).toBeUndefined();

    // Readable under B, the workspace that asked.
    const read = registry.invoke("blob.read", { blobId: res.blobId!, offset: 0, workspaceId: WS_B }, ctx) as {
      data: string;
    };
    expect(Buffer.from(read.data, "base64").length).toBeGreaterThan(0);

    // **And NOT under A — the identity assertion.** This is the one that fails if the engine staged
    // host-wide, and it is why the case asserts on a namespace rather than on "a blobId came back".
    expect(() =>
      registry.invoke("blob.read", { blobId: res.blobId!, offset: 0, workspaceId: WS_A }, ctx),
    ).toThrow(/no such blob/i);

    // Release so the case leaves nothing staged behind it.
    registry.invoke("blob.release", { blobId: res.blobId!, workspaceId: WS_B }, ctx);
  });

  it("stopping one workspace leaves its sibling serving — the D10 behaviour", () => {
    const bPort = proxyPortOf(WS_B);

    expect((registry.invoke("server.stop", { workspaceId: WS_A }, ctx) as { ok: boolean }).ok).toBe(true);

    const statusA = registry.invoke("server.status", { workspaceId: WS_A }, ctx) as { running: boolean };
    const statusB = registry.invoke("server.status", { workspaceId: WS_B }, ctx) as { running: boolean; port: number };
    // A is down, B is untouched — and B's *port* is unchanged, so the stop hit the right instance rather
    // than merely hitting something.
    expect(statusA.running).toBe(false);
    expect(statusB.running).toBe(true);
    expect(statusB.port).toBe(bPort);
  });

  it("after host.stop() nothing is served — and the two branches answer differently, deliberately", async () => {
    // `clearRuntimes()` runs in `stop()`. Without it the lookup would keep answering from runtimes the
    // host no longer serves, and `server.status` would report a retained `currentPort` for a workspace that
    // is down — the same stale-answer class as `currentPort` surviving `stop()` one layer below
    // (`proxy/server.ts`'s recorded trap). Called directly here because `afterAll` has not run yet.
    await host.stop();

    // **The non-active workspace reads `absent`, with an explanation.** This is the branch that earns its
    // keep: there is no runtime for B, and there is no in-process engine that could serve it either.
    const b = registry.invoke("server.status", { workspaceId: WS_B }, ctx) as {
      running: boolean;
      port: number;
      error: string | null;
    };
    expect(b.running).toBe(false);
    expect(b.port).toBe(0);
    expect(b.error).toMatch(/no runtime/i);

    // **The active workspace reads through the facade branch instead, and that asymmetry is deliberate.**
    // `resolveRuntimeScope` returns `facade` whenever the target *is* the active workspace, because that is
    // what keeps the desktop working — `createEngine()` binds one proxy through the facade, so on a desktop
    // the facade genuinely is the active workspace's server. A stopped host is indistinguishable from that
    // case here: the registry is empty and the target is the active workspace, so the facade branch answers
    // from an instance that does not exist. The result is honest (`running: false`, `port: 0`) but
    // uninformative (`error: null` — nothing failed), and it is recorded rather than hidden.
    //
    // Not "fixed" by making the facade branch conditional on a live instance, and that is a real decision
    // rather than an oversight: the check would have to reach into `proxy/server.ts`'s private
    // `defaultInstance`, and `tests/integration/settingsMutations.integration.test.ts` drives these same
    // commands through the facade with no workspace id — so the change would alter behaviour for a suite
    // that is currently correct. The state is also transient in practice: `runWorkspaceHost()` exits on the
    // signal that stopped the host.
    const a = registry.invoke("server.status", { workspaceId: WS_A }, ctx) as {
      running: boolean;
      port: number;
      error: string | null;
    };
    expect(a.running).toBe(false);
    expect(a.port).toBe(0);
    expect(a.error).toBeNull();
  });
});
