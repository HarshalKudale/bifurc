/**
 * The last open P5 acceptance criterion: **"Client works over `in-process`, `stdio` and `ws`."**
 *
 * ## What this file is, and what it deliberately is not
 *
 * It is a **client-level smoke over real transports**, not a second conformance suite. The engine's
 * `packages/engine/tests/conformance/` already proves each transport exhaustively against the shared
 * `protocol.conformance.ts` — every command, the envelope semantics, replay, backoff, auth. Repeating
 * any of that here would be the "written per-transport instead of shared" risk `plan/05` names,
 * one layer up.
 *
 * So each transport gets exactly one question: **does a `createClient()` built over it actually
 * work?** That is a different question from the conformance suite's, and it is the one P5 has to
 * answer — the suite drives the `Transport` interface directly, and nothing so far has driven the
 * 132-method surface through a socket.
 *
 * ## Why `stdio` uses a `PassThrough` pair
 *
 * For the same reason `run-stdio.test.ts` does: a spawned child would turn every case into a
 * process-lifecycle test, where a slow machine reports a timeout instead of a result and a leaked
 * child turns a failure into a hang. The bytes still go through the same `framing.ts` codec and the
 * same correlation logic. The trade is documented at length in that runner — briefly, it does not
 * cover asynchrony, and `packages/engine/tests/transport/stdio.test.ts` covers that instead.
 *
 * ## Why `ws` performs a handshake here and the others do not
 *
 * `ws` is the only one of the three built with `auth`, and therefore the only one whose first request has to
 * be `hello`. It used to be justified as the only transport that *crosses a trust boundary*, and that
 * justification is now weaker than it was: since **2026-09-25** the boundary is not a credential — the
 * listener binds loopback unconditionally and the upgrade is refused unless the page's `Origin` is local.
 * What the handshake still buys is the **session** — its `sessionId`, its scopes, and the `lastSeq` replay
 * decision — which is why it is configured here and not on the other two.
 *
 * The handshake is not a special method: it is a plain `request("hello", …)` (`HELLO_ACTION` in
 * `transport/auth/authenticated.ts`), which is why it can be driven from a raw `Transport` before the client
 * exists. Doing it *before* `createClient()` keeps the client itself unaware of auth, which is the property
 * P6 needs — the renderer must not know the transport is remote.
 */
import { afterEach, describe, expect, it } from "vitest";
import { PassThrough } from "node:stream";
import { PROTOCOL_VERSION } from "@bifurc/protocol";
import { CommandRegistry } from "@bifurc/engine/commands/registry";
import { EngineEventBus } from "@bifurc/engine/eventBus";
import { EventLog } from "@bifurc/engine/transport/eventLog";
import { createInProcessTransport } from "@bifurc/engine/transport/inProcess";
import { createStdioServer, createStdioTransport, type StdioServer } from "@bifurc/engine/transport/stdio";
import { createWsServer, createWsTransport, type WsServer } from "@bifurc/engine/transport/ws";
import type { Transport } from "@bifurc/engine/transport/types";
import { createClient } from "../src/index";
import type { ClientLocal } from "../src/local";

/** Servers, so a mid-test failure cannot leave a listener or an attached session behind. */
const servers: (StdioServer | WsServer)[] = [];
const logs: EventLog[] = [];
const transports: Transport[] = [];

afterEach(async () => {
  // Transports first: the client says goodbye, then the server tears down a detached session.
  for (const t of transports.splice(0)) await t.close().catch(() => undefined);
  for (const s of servers.splice(0)) await s.close();
  for (const log of logs.splice(0)) log.close();
});

/**
 * A registry with the handful of commands these cases touch.
 *
 * Deliberately small. Registering the whole `COMMAND_FIXTURES` matrix would be re-running the
 * conformance suite's job through a client, and the point here is the *channel*, not the commands.
 * `config.get` is the one that matters most: three `window.api` listers cannot work without it.
 */
function engineRegistry(): CommandRegistry {
  const registry = new CommandRegistry();
  registry.register("config.get", () => ({ activeWorkspaceId: "ws-1", theme: "dark" }));
  registry.register("server.status", () => ({ running: true, port: 8080, error: null }));
  registry.register("entity.list", () => ({ entities: [{ id: "wsdl-1" }] }));
  /**
   * The frozen `{id, entity}` envelope, with the **created** entity inside it — `entityCrudCommands.ts:413`
   * returns `{ id: created.id, entity: created }`, not the params it was handed.
   *
   * This fixture used to be `({ id: "created", ...params })`, which put `entity` *and* `kind` at the top
   * level — so the client's missing unwrap was invisible here: the envelope happened to carry the id the
   * assertion looked for. Modelling the real shape is what makes the assertion below a test of the
   * client rather than of the fixture.
   */
  registry.register("entity.create", (params) => {
    const { entity } = params as { entity: Record<string, unknown> };
    return { id: "created", entity: { ...entity, id: "created" } };
  });
  return registry;
}

/** The smallest `ClientLocal` that satisfies the interface; these cases never call it. */
function stubLocal(): ClientLocal {
  const no = () => Promise.reject(new Error("not used in this file"));
  return {
    checkUpdate: no as never,
    openExternal: no as never,
    setTitleBarOverlay: no as never,
    getTheme: no as never,
    setTheme: no as never,
    tlsInstallCA: no as never,
    // `local` as of 2026-09-20 — see `surface.ts`. Present because the interface demands them, not
    // because these cases reach them.
    tlsRemoveCert: no as never,
    tlsCertStatus: no as never,
    openFileDialog: no as never,
    pickFilePath: no as never,
    pickFolderPath: no as never,
    platform: "win32" as never,
    isFirstLaunch: no as never,
    completeFirstLaunch: no as never,
    getZoomLevel: no as never,
    setZoomLevel: no as never,
  };
}

/**
 * The assertions every transport must satisfy, so the three cases cannot drift apart.
 *
 * A shared body rather than three copies: the failure mode this guards against is "the `ws` case
 * asserts less than the others and nobody notices", which is exactly how a transport-specific
 * exception gets baked in.
 */
async function assertTheClientWorks(transport: Transport): Promise<void> {
  const client = createClient(transport, { local: stubLocal() });

  // A plain read over the transport.
  await expect(client.serverStatus()).resolves.toEqual({ running: true, port: 8080, error: null });

  // A read whose payload the client builds itself.
  await expect(client.getConfig()).resolves.toEqual({ activeWorkspaceId: "ws-1", theme: "dark" });

  /**
   * The composite one, and the reason this case is worth having on every transport: `listWsdls`
   * needs `config.get` to resolve the active workspace *and then* `entity.list`, so it exercises two
   * round trips and the client's own internal plumbing rather than one command's reachability.
   */
  await expect(client.listWsdls()).resolves.toEqual([{ id: "wsdl-1" }]);

  /**
   * A mutation, to prove the write path is not special-cased anywhere.
   *
   * `id` proves the round trip reached the engine and came back with the server's id; `name` proves the
   * client **unwrapped** `entity.create`'s `{id, entity}` envelope. Both are needed: the old assertion was
   * `toMatchObject({ id: "created" })` alone, against a fixture that leaked the envelope, so it was green
   * *because of* the defect it existed to catch. Drop the unwrap in `src/index.ts` and `name` goes
   * `undefined` — which is how this case was found (2026-09-20).
   */
  await expect(client.addMock({ name: "m" } as never)).resolves.toMatchObject({
    id: "created",
    name: "m",
  });

  await client.close();
}

describe("createClient over a real in-process transport", () => {
  it("drives the surface through the registry", async () => {
    const bus = new EngineEventBus();
    const log = new EventLog({ bus });
    logs.push(log);
    const transport = createInProcessTransport(engineRegistry(), { bus, ctx: { bus }, log });
    transports.push(transport);

    await assertTheClientWorks(transport);
  });
});

describe("createClient over a real stdio transport", () => {
  it("drives the surface through a PassThrough pipe", async () => {
    const bus = new EngineEventBus();
    // Two simplex channels rather than one duplex, because that is what a real stdio pair is: the
    // engine's stdin and stdout are separate streams with separate lifecycles.
    const toServer = new PassThrough();
    const toClient = new PassThrough();
    servers.push(
      createStdioServer({ input: toServer, output: toClient, registry: engineRegistry(), bus, ctx: { bus } }),
    );
    const transport = createStdioTransport({ input: toServer, output: toClient });
    transports.push(transport);

    await assertTheClientWorks(transport);
  });
});

describe("createClient over a real ws transport", () => {
  it("drives the surface through a socket, after the hello handshake", async () => {
    const bus = new EngineEventBus();
    const log = new EventLog({ bus });
    logs.push(log);

    const server = await createWsServer({
      registry: engineRegistry(),
      bus,
      log,
      // No token: it was the authentication half of this config and was removed on 2026-09-25. What is left
      // is the handshake's *session* half — identity and scopes — which is still required.
      auth: { engineVersion: "0.0.0-client-test" },
    });
    servers.push(server);

    // The URL, not host/port: the server resolved `port: 0` to whatever the OS assigned.
    const transport = createWsTransport(server.url);
    transports.push(transport);

    /**
     * The handshake, driven on the raw transport so `createClient` never learns about auth.
     *
     * Without this the decorator refuses the first real request, which is the whole point of the
     * gate — so if this file ever passes with the handshake removed, the gate has stopped working.
     *
     * The `token` this call used to carry is gone rather than merely ignored: `HelloRequestSchema` is a
     * plain `z.object`, so unknown keys are **stripped**, and a stale field would pass silently.
     */
    await transport.request("hello", {
      protocolVersion: PROTOCOL_VERSION,
      clientName: "web-ui",
      clientVersion: "0.0.0-client-test",
    });

    await assertTheClientWorks(transport);
  });
});
