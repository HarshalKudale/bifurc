/**
 * P14 §8 criterion 10 — the webhook receiver routes by `(workspaceId, urlSuffix)`.
 *
 * ## What the routing key actually *is*, and why this file asserts it through a socket
 *
 * WI 2 deliberately left the routing key as WI 1's job rather than keying the active map by
 * `(workspaceId, urlSuffix)` — and `webhookServer.ts`'s header records the consequence: the map lives
 * *inside* `createWebhookServer()`, so on a host it is **per instance**, and one receiver per workspace is
 * what makes the pair the key. `webhookCommands.ts` then resolves the runtime from `params.workspaceId`
 * before touching the map, which is the WI 7 fix.
 *
 * So the claim is not "the map has the right key" — it is **"a suffix registered in A is not served by B's
 * port"**, and the only instrument for that is a real POST to two real ports. Asserted on **identity**: the
 * same suffix that A answers `200` is `404` at B, and the converse with a second suffix. A test that
 * asserted "B returns 404 for something" would pass against a receiver that answered 404 for everything.
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

import { setDataRootOverride } from "@bifurc/engine/store/workspaceFs";
import { setSettingsPathOverride } from "@bifurc/engine/store/appSettings";
import { createWorkspaceHost, type WorkspaceHost } from "@bifurc/engine/runtime/host";
import { CommandRegistry, type CommandContext } from "@bifurc/engine/commands/registry";
import { registerAllEngineCommands } from "@bifurc/engine/registerAll";

const WS_A = "hook-ws-a";
const WS_B = "hook-ws-b";

/** Two suffixes so each direction has its own subject — the control needs one A has never seen. */
const SUFFIX_A = "orders/created";
const SUFFIX_B = "invoices/paid";

/** `bus` is only read by handlers that emit; none of these do. The shape is the whole requirement. */
const ctx: CommandContext = { bus: {} as CommandContext["bus"] };

function writeWorkspace(dataRoot: string, id: string, name: string): void {
  const dir = path.join(dataRoot, id);
  fs.mkdirSync(dir, { recursive: true });
  fs.writeFileSync(path.join(dir, "workspace.json"), JSON.stringify({ id, name, createdAt: 0 }, null, 2), "utf-8");
}

/** A real POST to a real receiver. `status` is the whole assertion; the body is kept for diagnostics. */
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

describe("the webhook receiver routes by (workspaceId, urlSuffix) (P14 §8 criterion 10)", () => {
  let root: string;
  let dataRoot: string;
  let socketPath: string;
  let host: WorkspaceHost;
  let registry: CommandRegistry;

  function webhookPortOf(workspaceId: string): number {
    const rt = host.runtimes.get(workspaceId);
    if (!rt) throw new Error(`no runtime for ${workspaceId}`);
    const st = rt.status();
    if (st.webhookPort === null) throw new Error(`${workspaceId} has no webhook port`);
    return st.webhookPort;
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
          companionEnabled: false,
          companionPort: 9271,
          minimizeToTray: true,
          tlsEnabled: false,
          tlsCaCertPath: null,
          tlsCaKeyPath: null,
          workspaces: [
            { id: WS_A, name: "Hook Workspace A", activeEnvironmentId: null },
            { id: WS_B, name: "Hook Workspace B", activeEnvironmentId: null },
          ],
          // The active workspace is A, and the interesting cases address **B** — the same asymmetry WI 7's
          // probe used, so a command that ignored `workspaceId` would answer from A and be caught.
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

    // One suffix on each workspace, registered **through the command layer** with an explicit workspaceId —
    // which is the layer WI 7 fixed and the reason the two maps end up different.
    expect(await registry.invoke("webhook.registerActive", { workspaceId: WS_A, webhookId: "hook-a", urlSuffix: SUFFIX_A }, ctx)).toEqual({ ok: true });
    expect(await registry.invoke("webhook.registerActive", { workspaceId: WS_B, webhookId: "hook-b", urlSuffix: SUFFIX_B }, ctx)).toEqual({ ok: true });
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

  it("serves each workspace's own suffix, and 404s the other's — in both directions", async () => {
    const aPort = webhookPortOf(WS_A);
    const bPort = webhookPortOf(WS_B);
    // The ports must differ, or "A served it and B did not" would be a claim about one receiver.
    expect(aPort).not.toBe(bPort);

    // A's own suffix: 200 on A. This is the precondition that makes the 404 below meaningful — without it,
    // a receiver that answered 404 for everything would satisfy the cross-workspace half.
    const aOwn = await postWebhook(aPort, SUFFIX_A);
    expect(aOwn.status, `A should serve its own suffix: ${aOwn.body}`).toBe(200);

    // A's suffix at B: 404. **Identity, not plausibility** — the same suffix, the other port.
    const aAtB = await postWebhook(bPort, SUFFIX_A);
    expect(aAtB.status, `B must not serve A's suffix: ${aAtB.body}`).toBe(404);

    // The converse, so the pair is symmetric rather than one-directional luck.
    const bOwn = await postWebhook(bPort, SUFFIX_B);
    expect(bOwn.status, `B should serve its own suffix: ${bOwn.body}`).toBe(200);

    const bAtA = await postWebhook(aPort, SUFFIX_B);
    expect(bAtA.status, `A must not serve B's suffix: ${bAtA.body}`).toBe(404);
  }, 60000);

  it("keeps the active-set budget per workspace, not per host", async () => {
    // `MAX_ACTIVE_WEBHOOKS` is enforced against `activeWebhookSuffixes`, which is per instance — so a
    // webhook A registered must not be counted against B's budget. Asserted on the map's own count rather
    // than by filling the budget, because the count is the thing the budget is computed from.
    expect(host.runtimes.get(WS_A)!.webhook.activeCount()).toBe(1);
    expect(host.runtimes.get(WS_B)!.webhook.activeCount()).toBe(1);

    // Unregistering A's webhook must leave B's alone — the map is the instance's, and this is the
    // mutation that the WI 2 header says was the *worst* version of the bug (A's registration writing into
    // the active workspace's map).
    expect(await registry.invoke("webhook.unregisterActive", { workspaceId: WS_A, webhookId: "hook-a" }, ctx)).toEqual({ ok: true });
    expect(host.runtimes.get(WS_A)!.webhook.activeCount()).toBe(0);
    expect(host.runtimes.get(WS_B)!.webhook.activeCount()).toBe(1);
    expect((await postWebhook(webhookPortOf(WS_B), SUFFIX_B)).status).toBe(200);
  }, 60000);
});
