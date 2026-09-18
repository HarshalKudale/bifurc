/**
 * `ws` behaviour that the conformance suite **cannot** express.
 *
 * The suite is transport-agnostic by design: it is handed a `Transport` and knows nothing about
 * sockets, ports or TLS. That is exactly what makes it reusable — and it is also why several things
 * worth asserting about a *listener* cannot live there:
 *
 *  - the **bind guards** (work item 5's "never default to `0.0.0.0`" and "TLS beyond loopback"),
 *    which are properties of `createWsServer`, not of a `Transport`;
 *  - what happens to a request that is **in flight when the peer disappears**;
 *  - what happens to a **malformed frame from one peer**, and whether it takes the engine with it;
 *  - whether a disconnected session **releases its `EventLog` subscriptions** — the leak class that
 *    bit the `stdio` server in P4;
 *  - the **frame-size ceiling**, which is enforced by the socket layer rather than by any code here.
 *
 * Everything that is about the `Transport` contract rather than about sockets lives in
 * `tests/conformance/run-ws.test.ts` and is asserted once, for every transport.
 *
 * ## Why these cases are deterministic without sleeping
 *
 * A real socket is asynchronous, so "wait a bit" is the obvious way to write these and the wrong one.
 * Every case below either (a) waits on a promise the engine itself resolves — a handler that signals
 * it was entered, a socket's `close` event — or (b) uses `vi.waitFor`, which retries until a condition
 * holds rather than assuming a duration. Nothing here asserts on *how long* anything takes.
 */
import { afterEach, describe, expect, it, vi } from "vitest";
import { WebSocket } from "ws";
import { PROTOCOL_VERSION } from "@bifurc/protocol";
import { CommandRegistry, type CommandContext } from "../../src/commands/registry";
import { EngineEventBus } from "../../src/eventBus";
import { EventLog } from "../../src/transport/eventLog";
import type { Transport } from "../../src/transport/types";
import {
  CLOSE_CODE_POLICY,
  createWsServer,
  createWsTransport,
  DEFAULT_WS_HOST,
  type WsServer,
  type WsServerOptions,
} from "../../src/transport/ws";

/**
 * A registry with two real commands, so a name typo is a `UNKNOWN_COMMAND` rather than a passing test.
 *
 * The handler takes the command context because one case is about what the *server* puts in it — the
 * session identity and the peer address. Every other caller ignores both parameters.
 */
function makeRegistry(
  handler: (_params: unknown, ctx: CommandContext) => unknown = () => ({ port: 8080 }),
): CommandRegistry {
  const registry = new CommandRegistry();
  registry.register("config.get", handler);
  registry.register("env.setActive", () => ({ reached: "env.setActive" }));
  return registry;
}

const servers: WsServer[] = [];
const transports: Transport[] = [];

/**
 * Bind a server and point a client at it, registering both for teardown.
 *
 * The `bus` is created here unless the caller supplies one, so a case that needs to emit on it gets
 * the same instance the server was wired to — an isolated one per test, for the reason the conformance
 * suite gives: Vitest shares a module registry, so asserting against the process-wide singleton makes
 * one test's leak look like the next test's bug.
 */
async function pair(
  options: Partial<Omit<WsServerOptions, "registry" | "bus">> & { bus?: EngineEventBus } = {},
  transportOptions: Parameters<typeof createWsTransport>[1] = {},
): Promise<{ server: WsServer; transport: Transport; bus: EngineEventBus }> {
  const bus = options.bus ?? new EngineEventBus();
  const server = await createWsServer({ registry: makeRegistry(), ...options, bus });
  servers.push(server);
  const transport = createWsTransport(server.url, transportOptions);
  transports.push(transport);
  return { server, transport, bus };
}

afterEach(async () => {
  for (const t of transports.splice(0)) await t.close();
  for (const s of servers.splice(0)) await s.close();
});

describe("ws — the bind guards (work item 5)", () => {
  it("binds loopback on an ephemeral port by default", async () => {
    // The whole of the access control for an unauthenticated engine, and `plan/05`'s first rule:
    // "Never default to 0.0.0.0."
    const { server } = await pair();
    expect(server.host).toBe(DEFAULT_WS_HOST);
    expect(server.secure).toBe(false);
    // `port: 0` is resolved by the OS, and the resolved value is the only one that identifies this
    // listener — which is why `createWsServer` is async.
    expect(server.port).toBeGreaterThan(0);
    expect(server.url).toBe(`ws://127.0.0.1:${server.port}`);
  });

  it("REFUSES a non-loopback bind without an explicit opt-in", async () => {
    const registry = makeRegistry();
    const bus = new EngineEventBus();
    for (const host of ["0.0.0.0", "::", "192.168.1.10"]) {
      await expect(
        createWsServer({ registry, bus, host }),
        `${host} must not bind without an opt-in`,
      ).rejects.toThrow(/loopback by default/i);
    }
  });

  it("REFUSES a non-loopback bind in the clear, even with the opt-in", async () => {
    // The second guard, and the one that is easy to forget: opting into exposure is not the same as
    // opting into exposure *unencrypted*. `plan/05` work item 5: "TLS on the channel, not optional."
    // Until this is satisfiable, remote mode is unreachable — which is the honest state, and the
    // reason a pass-through `tls` option with no test behind it is acceptable.
    const registry = makeRegistry();
    const bus = new EngineEventBus();
    await expect(
      createWsServer({ registry, bus, host: "0.0.0.0", allowNonLoopback: true }),
    ).rejects.toThrow(/requires TLS/i);
  });
});

describe("ws — a dying peer", () => {
  it("REJECTS a request that was in flight when the peer went away", async () => {
    // The classic serialising-transport bug: the engine dies mid-request and the caller awaits a
    // promise nobody will ever settle. A CLI that hangs forever with no error is far worse than one
    // that exits with a message.
    let entered: () => void = () => {};
    const enteredPromise = new Promise<void>((resolve) => {
      entered = resolve;
    });

    const bus = new EngineEventBus();
    const server = await createWsServer({
      registry: makeRegistry(() => {
        entered();
        // Never settles, so the request is genuinely in flight when the server goes away.
        return new Promise(() => {});
      }),
      bus,
    });
    servers.push(server);
    const transport = createWsTransport(server.url);
    transports.push(transport);

    const inFlight = transport.request("config.get", {});
    // Waiting on the engine's own signal rather than a timeout: the request is in flight when this
    // resolves, which is the precondition the case is about.
    await enteredPromise;

    await server.close();

    await expect(inFlight).rejects.toBeInstanceOf(Error);
  });

  it("does NOT call onFatal for a caller-initiated close", async () => {
    // A clean shutdown reported as a fault would make every orderly exit look like a crash to whatever
    // is watching `onFatal`.
    const fatals: Error[] = [];
    const { transport } = await pair({}, { onFatal: (e) => fatals.push(e) });
    await transport.request("config.get", {});
    await transport.close();
    expect(fatals).toHaveLength(0);
  });

  it("calls onFatal when the engine goes away", async () => {
    const fatals: Error[] = [];
    const { server, transport } = await pair({}, { onFatal: (e) => fatals.push(e) });
    await transport.request("config.get", {});
    await server.close();
    await vi.waitFor(() => expect(fatals).toHaveLength(1));
  });
});

describe("ws — the engine keeps serving", () => {
  it("serves commands with no auth configured, on loopback", async () => {
    // The `companionServer.ts` configuration, kept deliberately: auth is optional because the bind
    // guard is what makes an unauthenticated engine unreachable from anywhere but this machine. This
    // is the case that would fail if someone made `auth` mandatory, which is a reasonable-looking
    // change that would break the companion extension.
    const { transport } = await pair();
    await expect(transport.request("config.get", {})).resolves.toEqual({ port: 8080 });
  });

  it("closes only the connection that sent a malformed frame", async () => {
    // `stdio` treats a frame it cannot correlate as fatal to the *process*, because a stdio engine has
    // exactly one peer. A WebSocket engine has many, so one broken client must not take down the
    // process serving the rest — this is the case that would fail if the frame handler threw, or if it
    // reused `stdio`'s `fail()`-the-whole-server shape.
    const { server, transport } = await pair();
    await expect(transport.request("config.get", {})).resolves.toEqual({ port: 8080 });

    const raw = new WebSocket(server.url);
    const closed = new Promise<number>((resolve) => {
      raw.on("close", (code) => resolve(code));
    });
    raw.on("error", () => {});
    await new Promise<void>((resolve) => raw.on("open", () => resolve()));
    raw.send("this is not an RpcRequest envelope");

    expect(await closed).toBe(CLOSE_CODE_POLICY);

    // And the first client is untouched.
    await expect(transport.request("config.get", {})).resolves.toEqual({ port: 8080 });
  });

  it("enforces the frame-size ceiling rather than buffering whatever it is sent", async () => {
    // Work item 5's "Request size: ... at the frame level". `ws` defaults to 100 MiB, which is a large
    // allocation to hand a peer that controls the length; the ceiling is what stops one.
    const { transport } = await pair({ maxPayloadBytes: 1024 });
    await expect(
      transport.request("config.get", { padding: "x".repeat(4096) }),
    ).rejects.toBeInstanceOf(Error);
  });
});

describe("ws — event delivery", () => {
  it("carries the ENGINE's seq, so a subset subscription sees gaps", async () => {
    // The assertion that distinguishes "the engine numbered this" from "the client counted arrivals".
    // `stdio`'s client numbers locally, which is sound only because `stdio` has no replay. A `ws`
    // client that did the same would compute a `lastSeq` the engine does not recognise, and the
    // conformance suite cannot catch it — its replay case emits exactly the events the client
    // subscribed to, so both numberings agree there. Here they do not.
    const bus = new EngineEventBus();
    const log = new EventLog({ bus });
    const { transport } = await pair({ bus, log });

    /**
     * Start retention for `log.chunk` without subscribing this client to it.
     *
     * This is not scene-setting — it is the precondition the case is about. Retention is **lazy**, so
     * an event nobody has ever subscribed to is never numbered and consumes no `seq`. Without this
     * line the two `server.error` events would be numbered 1 and 2 and the assertion below would pass
     * for a reason that has nothing to do with where the numbering comes from — a vacuous test.
     *
     * `log.subscribe(...)()` is the one-line form of "someone was listening, then the connection
     * dropped", which is the blip the retention guarantee exists for. The conformance suite's
     * `openTheWindow()` helper does the same thing, for the same reason.
     */
    log.subscribe(["event.log.chunk"], () => {})();

    const seen: number[] = [];
    transport.subscribe(["event.server.error"], (e) => seen.push(e.seq));
    // A round trip, so the subscription is applied before anything is emitted — the same barrier the
    // conformance suite's `deliver()` uses, and for the same reason.
    await transport.request("config.get", {});

    bus.emitTyped("server.error", "a"); // seq 1
    bus.emitTyped("log.chunk", { logId: "l1", chunk: "x", done: false }); // seq 2, not subscribed
    bus.emitTyped("server.error", "b"); // seq 3

    await vi.waitFor(() => expect(seen).toHaveLength(2));
    // A client-side recount would produce `[1, 2]`.
    expect(seen).toEqual([1, 3]);
  });

  it("releases the session's log subscriptions when the client disconnects", async () => {
    // The leak class that bit the `stdio` server in P4: an attached transport does not own the pipe, so
    // a graceful close left the engine's bus listeners attached with nobody to receive them, and the
    // engine accumulated one per closed session until Node's warning fired. `subscriberCount` is where
    // that is measured now that the `EventLog` owns retention.
    const bus = new EngineEventBus();
    const log = new EventLog({ bus });
    const { transport } = await pair({ bus, log });

    transport.subscribe(["event.server.error"], () => {});
    await vi.waitFor(() => expect(log.subscriberCount("event.server.error")).toBe(1));

    await transport.close();
    await vi.waitFor(() => expect(log.subscriberCount("event.server.error")).toBe(0));
  });
});

describe("ws — the auth consequence (work item 5)", () => {
  it("answers UNAUTHORIZED and then closes the session", async () => {
    // "Auth failure returns UNAUTHORIZED and closes. Never fall back to unauthenticated." Two halves,
    // and both matter: the reply must be flushed *before* the close (a peer disconnected without being
    // told sees a network fault instead of a rejected credential), and the session must really be gone
    // afterwards rather than merely unauthorised-looking.
    const fatals: Error[] = [];
    const { transport } = await pair(
      { auth: { token: "the-right-token", engineVersion: "0.0.0-test" } },
      { onFatal: (e) => fatals.push(e) },
    );

    await expect(
      transport.request("hello", {
        protocolVersion: PROTOCOL_VERSION,
        clientName: "cli",
        clientVersion: "0.0.0-test",
        token: "definitely-not-the-token",
      }),
    ).rejects.toMatchObject({ code: "UNAUTHORIZED" });

    // The close is what proves there is no unauthenticated fallback path: waiting for it here rather
    // than assuming it has already happened is what makes the next assertion deterministic.
    await vi.waitFor(() => expect(fatals).toHaveLength(1));
    await expect(transport.request("config.get", {})).rejects.toBeInstanceOf(Error);
  });

  it("keeps a FORBIDDEN session open, because it is a scope error and not an identity failure", async () => {
    // The other half of the split `authenticated.ts` makes. Closing here would turn a scope error into a
    // disconnect and make a mis-scoped client look like a network fault.
    const fatals: Error[] = [];
    const { transport } = await pair(
      {
        auth: {
          token: "the-right-token",
          engineVersion: "0.0.0-test",
          scopesFor: () => new Set(["read"] as const),
        },
      },
      { onFatal: (e) => fatals.push(e) },
    );

    await transport.request("hello", {
      protocolVersion: PROTOCOL_VERSION,
      clientName: "bifurc-extension",
      clientVersion: "0.0.0-test",
      token: "the-right-token",
    });

    // `env.setActive` requires `write`; a `read` session must be refused and must stay connected.
    await expect(transport.request("env.setActive", {})).rejects.toMatchObject({ code: "FORBIDDEN" });
    await expect(transport.request("config.get", {})).resolves.toEqual({ port: 8080 });
    expect(fatals).toHaveLength(0);
  });

  it("tells a handler which session invoked it, and from which address", async () => {
    // This case lives here rather than in the shared suite because of `peer`: only the **server** sees
    // the connection, so only the server can fill it in, and the suite is handed a `Transport` and
    // knows nothing about addresses. The suite's own session case asserts the transport-independent
    // half (`sessionId`/`clientName`/`clientVersion`).
    const bus = new EngineEventBus();
    const server = await createWsServer({
      registry: makeRegistry((_params, ctx) => ctx.session ?? null),
      bus,
      auth: { token: "the-right-token", engineVersion: "0.0.0-test" },
    });
    servers.push(server);
    const transport = createWsTransport(server.url);
    transports.push(transport);

    await transport.request("hello", {
      protocolVersion: PROTOCOL_VERSION,
      clientName: "cli",
      clientVersion: "0.0.0-test",
      token: "the-right-token",
    });

    const seen = await transport.request("config.get", {});
    expect(seen).toMatchObject({
      sessionId: expect.any(String),
      clientName: "cli",
      clientVersion: "0.0.0-test",
      // Matched by shape rather than by exact string: whether loopback resolves to `::1` or
      // `127.0.0.1` depends on the machine. What matters is that it is a real address and not the
      // `"unknown"` placeholder the server falls back to — which over loopback it never is, and a
      // placeholder would quietly turn an audit entry into an unattributable one.
      peer: expect.stringMatching(/^(::1|127\.0\.0\.1|::ffff:127\.0\.0\.1)$/),
    });
  });
});
