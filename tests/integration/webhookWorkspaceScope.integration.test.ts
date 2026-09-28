/**
 * P14 §8 criterion 10 — the webhook receiver routes by `(workspaceId, urlSuffix)`.
 *
 * ## ⚠ THIS CRITERION IS RETIRED BY `plan/17` §1 (2026-09-25). WHAT REPLACES IT, AND WHY.
 *
 * The criterion required **one receiver per workspace on its own port**, so that "a suffix registered in A
 * is not served by B's port" was a claim two real ports could settle. `plan/17` §1 removes the premise: a
 * workspace is a **project**, so switching workspaces changes the *context* while the ports stay put —
 * therefore there is now **one webhook receiver per host**, on **one port the user owns**, with **one active
 * map**. Two ports do not exist to compare, so the cross-port assertion is not merely failing, it is
 * **unstatable**.
 *
 * Deleting the file would leave the change unasserted in the place that used to be its most careful guard.
 * So it is **inverted** instead — the same treatment `workspaceDispatchScope` got — and it now pins three
 * things:
 *
 * 1. the receiver is **host-level**: every registered suffix is served on the *one* port,
 * 2. the receiver is **off by default** (`plan/17` §1 — a feature, not infrastructure), so the port does not
 *    exist until something starts it, and
 * 3. **the known gap, pinned rather than implied.** One active map means *switching workspace does not retire
 *    the previous workspace's active webhooks*, and `workspaceId` is now an **echo** rather than a routing
 *    key. `plan/17` §1 lists *"which webhook endpoints are in force"* among the things a switch changes, so
 *    this is a deliberate, recorded limit and **not** the intended end state — closing it needs the active map
 *    keyed by workspace, which `createWebhookServer()` does not do.
 *
 * ## Why pinning a gap is worth a test rather than a comment
 *
 * A comment describing a gap is read once. An assertion describing it **fails the moment someone changes the
 * behaviour**, which forces the change to be deliberate: either the gap is closed and this file is updated to
 * say so, or it was closed by accident and the failure says what was lost. That is the same reasoning the
 * "accepted and ignored" entries elsewhere in this suite use.
 *
 * ## What is kept from the original, and why it still earns its place
 *
 * The **control**. The old file's whole method was *identity, not plausibility* — a receiver that answers
 * `404` to everything would satisfy "B must not serve A's suffix", so it asserted a `200` for the receiver's
 * *own* suffix first. That discipline survives verbatim: every `200` below is paired with a `404` for a
 * suffix nobody registered, so "the receiver serves it" cannot pass vacuously.
 *
 * ## Why its own file
 *
 * Same reason as `workspaceCommandScope`: `createWorkspaceHost()` registers the whole command surface into
 * the registry it is given, and `register()` throws on a duplicate, so this passes its own registry rather
 * than populating the process-wide singleton.
 */

import { describe, it, expect, beforeAll, afterAll } from "vitest";
import * as fs from "fs";
import * as os from "os";
import * as path from "path";
import * as http from "http";
import * as net from "net";

import { setDataRootOverride } from "@bifurc/engine/store/workspaceFs";
import { setSettingsPathOverride } from "@bifurc/engine/store/appSettings";
import { createWorkspaceHost, type WorkspaceHost } from "@bifurc/engine/runtime/host";
import { CommandRegistry, type CommandContext } from "@bifurc/engine/commands/registry";
import { registerAllEngineCommands } from "@bifurc/engine/registerAll";

const WS_A = "hook-ws-a";
const WS_B = "hook-ws-b";

/** Suffixes so each registration has its own subject — the control needs one nobody registered. */
const SUFFIX_A = "orders/created";
const SUFFIX_B = "invoices/paid";
/** Never registered anywhere. The control that keeps every `200` below from passing vacuously. */
const SUFFIX_NEVER = "nothing/registered";

/** `bus` is only read by handlers that emit; none of these do. The shape is the whole requirement. */
const ctx: CommandContext = { bus: {} as CommandContext["bus"] };

function writeWorkspace(dataRoot: string, id: string, name: string): void {
  const dir = path.join(dataRoot, id);
  fs.mkdirSync(dir, { recursive: true });
  fs.writeFileSync(path.join(dir, "workspace.json"), JSON.stringify({ id, name, createdAt: 0 }, null, 2), "utf-8");
}

/** A real POST to the real receiver. `status` is the whole assertion; the body is kept for diagnostics. */
function postWebhook(port: number, suffix: string): Promise<{ status: number; body: string }> {
  return new Promise((resolve, reject) => {
    const req = http.request(
      {
        host: "127.0.0.1",
        port,
        path: `/bifurc/webhooks/${suffix}`,
        method: "POST",
        headers: { "content-type": "application/json" },
      },
      (res) => {
        let body = "";
        res.on("data", (c) => (body += c));
        res.on("end", () => resolve({ status: res.statusCode ?? 0, body }));
      },
    );
    req.on("error", reject);
    req.end("{}");
  });
}

/** Ask the OS for a free port, then release it — the receiver's port is the *user's*, so it is stated. */
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

describe("the webhook receiver is host-level, on one port, with one active map (plan/17 §1)", () => {
  let root: string;
  let dataRoot: string;
  let socketPath: string;
  let host: WorkspaceHost;
  let registry: CommandRegistry;
  let receiverPort: number;

  /** The one port, or a loud failure — `null` here means the receiver is not up and every POST would lie. */
  function theOnePort(): number {
    const port = host.status().webhookPort;
    if (port === null) throw new Error("the host-level receiver has no port — it was never started");
    return port;
  }

  beforeAll(async () => {
    root = fs.mkdtempSync(path.join(os.tmpdir(), "bifurc-hook-scope-"));
    dataRoot = path.join(root, "data");
    fs.mkdirSync(dataRoot, { recursive: true });
    writeWorkspace(dataRoot, WS_A, "Hook Workspace A");
    writeWorkspace(dataRoot, WS_B, "Hook Workspace B");

    fs.writeFileSync(
      path.join(root, "app.json"),
      JSON.stringify(
        {
          minimizeToTray: true,
          tlsEnabled: false,
          tlsCaCertPath: null,
          tlsCaKeyPath: null,
          workspaces: [
            { id: WS_A, name: "Hook Workspace A", activeEnvironmentId: null },
            { id: WS_B, name: "Hook Workspace B", activeEnvironmentId: null },
          ],
          // The active workspace is A, and one registration below claims **B** — the same asymmetry WI 7's
          // probe used. Under `plan/17` the claim is inert, which is exactly what case 3 asserts.
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
        ? `\\\\.\\pipe\\bifurc-hook-scope-${process.pid}`
        : path.join(os.tmpdir(), `bifurc-hs2-${process.pid}.sock`);

    registry = new CommandRegistry();
    registerAllEngineCommands(registry);
    host = createWorkspaceHost({
      dataDir: dataRoot,
      transport: { kind: "socket", path: socketPath },
      token: "hook-scope-token",
      registry,
      onNotice: () => {},
    });
    await host.start();
  }, 90000);

  afterAll(async () => {
    await host?.stop().catch(() => {});
    setDataRootOverride(null);
    setSettingsPathOverride(null);
    try {
      fs.rmSync(root, { recursive: true, force: true });
    } catch {
      /* best-effort */
    }
  });

  it("is OFF by default — the host binds no webhook port until something starts it", async () => {
    // The precondition for everything below, and `plan/17` §1's own rule: *"it is off by default, the user
    // starts it, and the user owns the port."* Asserted before the start so the case that follows cannot be
    // read as "the host opened it for us".
    expect(host.status().webhookPort).toBeNull();
    expect(host.webhook).not.toBeNull();
    expect(host.webhook!.isRunning()).toBe(false);
    expect(await registry.invoke("webhookServer.status", {}, ctx)).toMatchObject({ running: false, error: null });

    // Start it on a port we chose — the user owns it, so the command honours what it is given.
    receiverPort = await freePort();
    expect(await registry.invoke("webhookServer.start", { port: receiverPort }, ctx)).toEqual({ ok: true });

    // `startWebhookServer()` is fire-and-forget (it hands off to an async `listen()`), so readiness is
    // polled rather than assumed — the same reason `host.ts` has its own `waitUntilReady`.
    await expect.poll(() => host.status().webhookPort, { timeout: 5000 }).toBe(receiverPort);
    expect(await registry.invoke("webhookServer.status", {}, ctx)).toMatchObject({
      running: true,
      port: receiverPort,
      error: null,
    });
  }, 60000);

  it("serves every registered suffix on the ONE port — and still 404s an unregistered one", async () => {
    // One suffix claiming each workspace, registered **through the command layer**. Under the retired
    // criterion these two ended up in two different maps on two different ports; now they share one.
    expect(
      await registry.invoke("webhook.registerActive", { workspaceId: WS_A, webhookId: "hook-a", urlSuffix: SUFFIX_A }, ctx),
    ).toEqual({ ok: true });
    expect(
      await registry.invoke("webhook.registerActive", { workspaceId: WS_B, webhookId: "hook-b", urlSuffix: SUFFIX_B }, ctx),
    ).toEqual({ ok: true });

    const port = theOnePort();

    // Both suffixes, one port. This is the whole shape change: the old file's *"A's suffix is 404 at B"* is
    // unstatable when there is no B port.
    const a = await postWebhook(port, SUFFIX_A);
    expect(a.status, `the receiver should serve a registered suffix: ${a.body}`).toBe(200);
    const b = await postWebhook(port, SUFFIX_B);
    expect(b.status, `and every other registered one: ${b.body}`).toBe(200);

    // **The control, without which both `200`s above are worthless.** A receiver that answered `200` to
    // everything — or one whose `activeCount` was ignored — would satisfy the two assertions and fail this.
    const never = await postWebhook(port, SUFFIX_NEVER);
    expect(never.status, `an unregistered suffix must 404: ${never.body}`).toBe(404);
  }, 60000);

  it("keeps ONE active map — the known gap, pinned so it cannot drift silently", async () => {
    // `plan/17` §1 says switching workspace changes *"which webhook endpoints are in force"*. It does not,
    // and this is where that shows: the active map lives inside `createWebhookServer()` and is **per
    // instance**, and there is now one instance — so both workspaces' registrations land in it.
    //
    // If a future change keys the map by workspace (which is what closing the gap requires), this assertion
    // fails, and the failure message says what it used to mean. That is the point of pinning it.
    expect(
      host.webhook!.activeCount(),
      "one host-level receiver means one active map: switching workspace does not retire the previous " +
        "workspace's webhooks (plan/17 §1's recorded gap — see runtime/host.ts step 3c)",
    ).toBe(2);

    // Unregistering A's must leave B's alone. This half is not about the gap: it is the surviving form of
    // the old file's *worst* bug — a registration writing into the wrong map — and it still holds, because
    // there is only one map for it to be wrong about.
    expect(await registry.invoke("webhook.unregisterActive", { workspaceId: WS_A, webhookId: "hook-a" }, ctx)).toEqual({
      ok: true,
    });
    expect(host.webhook!.activeCount()).toBe(1);
    expect((await postWebhook(theOnePort(), SUFFIX_A)).status).toBe(404);
    expect((await postWebhook(theOnePort(), SUFFIX_B)).status).toBe(200);
  }, 60000);

  it("treats `workspaceId` as an echo, not a routing key — a ghost id registers and is served", async () => {
    // P14 WI 1 gave these commands an optional `workspaceId` and WI 7 made them *read* it, because resolving
    // the wrong runtime was the bug. `plan/17` §1 deletes the question: with one instance there is nothing to
    // resolve, so the id is carried through `echoWorkspace()` and nothing else. A ghost id is therefore
    // accepted and its suffix is served on the same port as everyone else's.
    //
    // Discriminating against the old behaviour: under the scope union, a `workspaceId` naming no runtime
    // resolved to `absent` and the registration would not have reached a live receiver at all.
    const ghost = "ghost-ws";
    expect(
      await registry.invoke(
        "webhook.registerActive",
        { workspaceId: ghost, webhookId: "hook-ghost", urlSuffix: "ghost/incoming" },
        ctx,
      ),
    ).toEqual({ ok: true });

    expect((await postWebhook(theOnePort(), "ghost/incoming")).status).toBe(200);
    // And the id comes back unchanged rather than being normalised away or resolved to the active workspace.
    expect(await registry.invoke("webhookServer.status", { workspaceId: ghost }, ctx)).toMatchObject({
      workspaceId: ghost,
    });
  }, 60000);
});
