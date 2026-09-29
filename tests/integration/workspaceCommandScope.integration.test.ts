/**
 * The `server.*` / `webhookServer.*` commands, after `plan/17`: they act on the host's **one** listener, and
 * `workspaceId` is an **echo** rather than a selector.
 *
 * WHAT THIS FILE USED TO ASSERT, AND WHY IT CANNOT NOW
 * ----------------------------------------------------
 * Until 2026-09-25 this was `workspaceCommandScope.integration.test.ts`, the guard for P14 WI 7's fix. The bug
 * it was written for was real and was **succeeding silently** on a two-workspace host:
 *
 * ```
 * host: ws-A proxy=81 webhook=9101     ws-B proxy=82 webhook=9102
 * server.status                 → { running: false, port: 0 }    ← two members ARE listening
 * server.start {workspaceId: A} → { ok: true }                    ← accepted, IGNORED
 * server.status                 → { running: false, port: 8791 }  ← a THIRD server
 * ```
 *
 * P14's fix was `resolveRuntimeScope()`: a three-way union (`runtime` / `facade` / `absent`) that routed a
 * `workspaceId` to the right one of N servers, with `absent` for a workspace this host does not serve.
 *
 * **`plan/17` §1 removes the premise of every assertion in that file.** There is now exactly **one** forward
 * proxy per host, on a port the user owns, serving whichever workspace is active — so a `workspaceId` cannot
 * select a server, and "which of the N?" is not a question the code is able to ask. The registry, the union
 * and the `absent` case were deleted with the model; these cases were rewritten rather than adapted, because
 * an adapted version would assert routing that no longer exists.
 *
 * WHAT REPLACES IT — AND WHY EACH CASE CAN STILL FAIL
 * --------------------------------------------------
 * Three properties are worth pinning now, and each has a way to break:
 *
 * 1. **The commands reach the host's listener.** `bindProxyInstance()` / `bindWebhookInstance()` point the
 *    module facade at the instances the host created. If that binding were dropped, `server.status` would
 *    create a *lazily defaulted* instance and report `{ running: false, port: 0 }` — **the exact symptom of
 *    the P14 bug**, arriving by a new route. So this is asserted against the host's own handle.
 * 2. **`workspaceId` is echoed, and only when it was sent.** A multiplexed client matches response to
 *    request by it; returning it unconditionally would change the desktop's payload, which
 *    `tests/ipc/handlers.test.ts` pins with `toEqual`.
 * 3. **The webhook is off until started.** `plan/17` §1 makes it a *feature, not infrastructure*. A host that
 *    opened that port at boot would be adding a listener this product has never had.
 */

import { describe, it, expect, beforeAll, afterAll } from "vitest";
import * as fs from "fs";
import * as os from "os";
import * as path from "path";
import * as net from "net";

import { setDataRootOverride } from "@bifurc/engine/workspace/workspaceFs";
import { setSettingsPathOverride } from "@bifurc/engine/workspace/appSettings";
import { createWorkspaceHost, type WorkspaceHost } from "@bifurc/engine/runtime/host";
import { CommandRegistry, type CommandContext } from "@bifurc/engine/kernel/registry";
import { registerAllEngineCommands } from "@bifurc/engine/registerAll";

const WS_A = "scope-ws-a";
const WS_B = "scope-ws-b";
/** Deliberately not a workspace that exists — under P14 this was the `absent` case. */
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

function writeWorkspace(dataRoot: string, id: string, name: string): void {
  const dir = path.join(dataRoot, id);
  fs.mkdirSync(dir, { recursive: true });
  fs.writeFileSync(path.join(dir, "workspace.json"), JSON.stringify({ id, name, createdAt: 0 }, null, 2), "utf-8");
}

/**
 * Poll until `pred` holds, or the deadline passes.
 *
 * **Required, not a convenience.** `startServer()` and `startWebhookServer()` are fire-and-forget — they hand
 * off to an async `listen()` and return — so `isRunning()` is still `false` on the very next line. Asserting
 * it directly would be asserting that the OS completed a bind within zero microseconds, which is the flake
 * class `TESTING.md` records. Polling rather than sleeping also means a *slow* bind is waited out while a
 * *failed* one is not mistaken for a slow one: the caller still asserts the value afterwards, so a bind that
 * never lands fails on the assertion rather than passing on the timeout.
 */
async function waitUntil(pred: () => boolean, timeoutMs = 5000): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (!pred() && Date.now() < deadline) {
    await new Promise((resolve) => setTimeout(resolve, 10));
  }
}

describe("plan/17 — the server commands act on the host's one listener", () => {
  let root: string;
  let dataRoot: string;
  let socketPath: string;
  /** The port the host's **forward proxy** is configured for. One number, for every workspace. */
  let forwardProxyPort: number;
  /** The port the host's **edge** is configured for — deliberately a different one. */
  let edgePort: number;
  /** The webhook setting, so "started on the configured port" is a real assertion rather than a default. */
  let webhookPort: number;
  let host: WorkspaceHost;
  let registry: CommandRegistry;

  beforeAll(async () => {
    root = fs.mkdtempSync(path.join(os.tmpdir(), "bifurc-scope-it-"));
    dataRoot = path.join(root, "data");
    writeWorkspace(dataRoot, WS_A, "Scope A");
    writeWorkspace(dataRoot, WS_B, "Scope B");

    // Every port is probed rather than hardcoded, so a failure is about behaviour rather than about whether
    // a fixed number happened to be taken on the machine running the suite.
    edgePort = await freePort();
    forwardProxyPort = await freePort();
    webhookPort = await freePort();

    fs.writeFileSync(
      path.join(root, "app.json"),
      JSON.stringify(
        {
          // The **edge** owns `port` (`plan/15` §3.1) and the **forward proxy** has its own setting
          // (`plan/17` §1). Keeping them different is what makes "the command reported the forward proxy's
          // port, not the edge's" an assertion rather than a coincidence.
          port: edgePort,
          forwardProxyPort,
          webhookPort,
          companionPort: 4747,
          minimizeToTray: true,
          tlsEnabled: false,
          tlsCaCertPath: null,
          tlsCaKeyPath: null,
          workspaces: [
            { id: WS_A, name: "Scope A", activeEnvironmentId: null },
            { id: WS_B, name: "Scope B", activeEnvironmentId: null },
          ],
          // **The active workspace is A, and several assertions address B.** Under P14 that was the whole
          // design of this file — with A active, a command that ignored `workspaceId` answered for A. It is
          // kept because the *inverse* now has to hold: the answer must be the same for both ids, because
          // there is one listener and the id is not a selector.
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

  // ── the binding: the commands reach the host's listener ──────────────────────

  it("server.status reports the host's forward proxy, on its configured port", () => {
    const status = registry.invoke("server.status", {}, ctx) as {
      running: boolean;
      port: number;
      error: string | null;
    };

    // Read against the host's own handle rather than against a literal, so this fails if the binding is
    // dropped and the command reports a lazily created instance instead. `{ running: false, port: 0 }` is
    // what that failure looks like, and it is the P14 bug's own symptom.
    expect(host.forwardProxy).not.toBeNull();
    expect(status.running).toBe(true);
    expect(status.port).toBe(forwardProxyPort);
    expect(status.port).toBe(host.forwardProxy!.getPort());
    expect(status.error).toBeNull();
  });

  it("the same answer for A, for B, and for a workspace that does not exist", () => {
    const ports = [WS_A, WS_B, WS_GHOST, undefined].map((workspaceId) => {
      const params = workspaceId === undefined ? {} : { workspaceId };
      return (registry.invoke("server.status", params, ctx) as { port: number }).port;
    });

    // **This is the assertion P14's fix would fail, inverted.** There, four ids had to produce *different*
    // answers; now they must all produce the *same* one, because the id selects nothing. A regression that
    // reintroduced routing would split these — and a `ghost` id would go back to being refused.
    expect(new Set(ports).size).toBe(1);
    expect(ports[0]).toBe(forwardProxyPort);
  });

  it("echoes workspaceId only when the caller sent one — the pinned payload", () => {
    const withId = registry.invoke("server.status", { workspaceId: WS_B }, ctx) as { workspaceId?: string };
    expect(withId.workspaceId).toBe(WS_B);

    // Absent, not present-and-undefined: `tests/ipc/handlers.test.ts` pins the desktop's payload with
    // `toEqual`, so a key that appeared unconditionally would be a user-visible change to `window.api`.
    const withoutId = registry.invoke("server.status", {}, ctx) as Record<string, unknown>;
    expect("workspaceId" in withoutId).toBe(false);
  });

  it("a workspaceId naming nothing is echoed rather than refused", () => {
    // Under P14 this returned `{ running: false, port: 0, error: "No runtime is registered for workspace …" }`
    // — the `absent` case. There is no routing left to be absent from, so the honest answer is the one the
    // host can give: its proxy, with the id echoed back. Asserted explicitly so a re-introduced refusal is a
    // decision rather than a silent return.
    const status = registry.invoke("server.status", { workspaceId: WS_GHOST }, ctx) as {
      running: boolean;
      port: number;
      error: string | null;
      workspaceId?: string;
    };
    expect(status.workspaceId).toBe(WS_GHOST);
    expect(status.running).toBe(true);
    expect(status.port).toBe(forwardProxyPort);
    expect(status.error).toBeNull();
  });

  // ── lifecycle: one proxy, whatever id the caller names ───────────────────────

  it("server.stop then server.start act on the one proxy, and it comes back on its configured port", async () => {
    expect((registry.invoke("server.stop", { workspaceId: WS_B }, ctx) as { ok: boolean }).ok).toBe(true);
    expect(host.forwardProxy!.isRunning()).toBe(false);

    // Stopped, and the command says so rather than reporting a stale port.
    expect((registry.invoke("server.status", {}, ctx) as { running: boolean }).running).toBe(false);

    expect((registry.invoke("server.start", { workspaceId: WS_A }, ctx) as { ok: boolean }).ok).toBe(true);
    await waitUntil(() => host.forwardProxy!.isRunning());
    expect(host.forwardProxy!.isRunning()).toBe(true);
    expect(host.forwardProxy!.getPort()).toBe(forwardProxyPort);
  });

  it("proxy.status is the same listener as server.status, with the narrower payload", () => {
    // Two commands, two payloads, one `isRunning()` — the distinction `serverCommands.ts`'s header records.
    expect((registry.invoke("proxy.status", { workspaceId: WS_A }, ctx) as { running: boolean }).running).toBe(
      host.forwardProxy!.isRunning(),
    );
  });

  // ── the webhook: host-level, and off until the user starts it ────────────────

  it("the host does NOT start the webhook — plan/17 §1's off-by-default rule", () => {
    const status = registry.invoke("webhookServer.status", {}, ctx) as {
      running: boolean;
      error: string | null;
    };

    // `plan/17` §1: the webhook *"is a **feature, not infrastructure**: it is **off by default**, the user
    // starts it, and the user owns the port."* A host that opened it at boot would add a listening port this
    // product has never had.
    expect(status.running).toBe(false);
    expect(status.error).toBeNull();
    expect(host.status().webhookPort).toBeNull();
  });

  it("webhookServer.start binds the host's receiver on the configured port", async () => {
    expect((registry.invoke("webhookServer.start", {}, ctx) as { ok: boolean }).ok).toBe(true);

    await waitUntil(() => host.webhook!.isRunning());
    expect(host.webhook!.isRunning()).toBe(true);
    expect(host.webhook!.getPort()).toBe(webhookPort);
    expect(host.status().webhookPort).toBe(webhookPort);

    const status = registry.invoke("webhookServer.status", { workspaceId: WS_A }, ctx) as {
      running: boolean;
      port: number;
      workspaceId?: string;
    };
    expect(status.running).toBe(true);
    expect(status.port).toBe(webhookPort);
    // The same echo rule as `server.status`, and asserted here because the two commands must agree about when
    // the key is present — which is why `commands/echoWorkspace.ts` is one function rather than two copies.
    expect(status.workspaceId).toBe(WS_A);

    expect((registry.invoke("webhookServer.stop", {}, ctx) as { ok: boolean }).ok).toBe(true);
    expect(host.webhook!.isRunning()).toBe(false);
  });
});
