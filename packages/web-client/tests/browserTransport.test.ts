/**
 * The browser transport, driven against a **real engine server over a real socket**.
 *
 * ## Why this suite exists, and why it is shaped this way
 *
 * This adapter is the only part of P7 that is testable *without* a browser UI: everything else in the
 * phase is blocked behind a client that can connect. So it is tested the way P14's instruments are —
 * against a real listener, not a mock — because the claims worth making here are all about the wire:
 *
 *  - a frame this module encodes is a frame the engine's server actually parses;
 *  - a frame the engine sends back is one this module actually decodes;
 *  - the `seq` a subscriber receives is the engine's, not a local recount;
 *  - a request in flight when the peer dies **rejects rather than hangs**.
 *
 * A mocked `WebSocket` can assert none of those: it would be asserting that this file agrees with
 * itself.
 *
 * ## No `webSocketFactory` here, deliberately
 *
 * The production default is the platform's own `WebSocket`. Node 22 provides one, so this suite uses
 * it — which means the tests exercise the same code path a browser does, rather than an injected
 * stand-in. The factory exists for hosts that provide none; it is not used to make the suite easier.
 *
 * ## Determinism
 *
 * A real socket is asynchronous, so "wait a bit" is the obvious way to write these and the wrong one.
 * Every case either waits on a promise the engine itself resolves (a handler signalling it was
 * entered) or uses `vi.waitFor`, which retries until a condition holds rather than assuming a
 * duration. Nothing here asserts on *how long* anything takes.
 */
import { createServer as createHttpServer } from "node:http";
import { createServer as createTcpServer, type AddressInfo, type Socket } from "node:net";
import { afterEach, describe, expect, it, vi } from "vitest";
import { CommandRegistry, type CommandContext } from "@bifurc/engine/kernel/registry";
import { EngineEventBus } from "@bifurc/engine/kernel/eventBus";
import { EventLog } from "@bifurc/engine/transport/eventLog";
import { createWsServer, type WsServer } from "@bifurc/engine/transport/ws";
import type { Transport } from "@bifurc/protocol";
import { createBrowserTransport } from "../src/index";

/**
 * A registry with real command names, so a typo is an `UNKNOWN_COMMAND` rather than a passing test.
 *
 * `overrides` exists for the one case that needs a handler which **never settles** — a request can
 * only be said to be in flight if the handler is still running.
 */
function makeRegistry(
  overrides: Record<string, (params: unknown, ctx: CommandContext) => unknown> = {},
): CommandRegistry {
  const registry = new CommandRegistry();
  registry.register("config.get", overrides["config.get"] ?? (() => ({ port: 8080 })));
  registry.register(
    "env.setActive",
    overrides["env.setActive"] ?? (() => ({ reached: "env.setActive" })),
  );
  return registry;
}

const servers: WsServer[] = [];
const transports: Transport[] = [];

/**
 * Bind a real server and point a browser transport at it, registering both for teardown.
 *
 * An `EventLog` per pair, for the reason the conformance suite gives: Vitest shares a module registry,
 * so asserting against the process-wide singleton would make one test's leak look like the next
 * test's bug.
 */
async function pair(registry: CommandRegistry = makeRegistry()) {
  const bus = new EngineEventBus();
  const log = new EventLog({ bus });
  const server = await createWsServer({ registry, bus, log });
  servers.push(server);

  const transport = createBrowserTransport({ url: server.url });
  transports.push(transport);
  return { server, transport, bus, log };
}

afterEach(async () => {
  for (const t of transports.splice(0)) await t.close();
  for (const s of servers.splice(0)) await s.close();
});

describe("browser transport — the wire", () => {
  it("round-trips a request over a real socket", async () => {
    // The load-bearing claim of the whole package: a frame this module encodes is one the engine's
    // server parses, and the reply is one this module decodes. If the text/binary choice in
    // `channel.ts` were wrong, this is where it shows.
    const { transport } = await pair();
    await expect(transport.request("config.get", {})).resolves.toEqual({ port: 8080 });
  });

  it("preserves the code of a failure the engine reports", async () => {
    // The envelope rule, across a real boundary: a *thrown* handler is an envelope-level failure and
    // keeps its code, so a client can branch per failure rather than seeing ENGINE_ERROR for
    // everything.
    const { transport } = await pair();
    await expect(transport.request("no.such.command", {})).rejects.toMatchObject({
      code: "UNKNOWN_COMMAND",
    });
  });

  it("delivers an event to a subscription, carrying the ENGINE's seq", async () => {
    const { transport, bus } = await pair();

    const seen: number[] = [];
    transport.subscribe(["event.server.error"], (e) => seen.push(e.seq));
    // A round trip, so the subscription is applied before anything is emitted. Without this the test
    // would race the control frame and pass or fail by timing.
    await transport.request("config.get", {});

    bus.emitTyped("server.error", "a");
    bus.emitTyped("server.error", "b");

    await vi.waitFor(() => expect(seen).toHaveLength(2));
    // The engine's own numbering, not a client-side recount — which would also produce `[1, 2]` here
    // and would therefore be indistinguishable *in this case*. The conformance suite's subset
    // assertion is what separates them; this case only pins that events arrive at all.
    expect(seen).toEqual([1, 2]);
  });

  it("queues a subscribe issued before the socket is open, rather than dropping it", async () => {
    // The failure this guards is a subscription that *succeeds* and never fires — indistinguishable
    // from "nothing has happened yet", and the one `plan/05` singles out. `subscribe()` is
    // synchronous, so it necessarily runs before the socket can be open: the control frame has to go
    // into the session's outbox and be replayed by `opened()`.
    const { transport, bus } = await pair();

    const seen: unknown[] = [];
    // No `await` before this line: the socket cannot possibly be open yet.
    transport.subscribe(["event.server.error"], (e) => seen.push(e.payload));

    await transport.request("config.get", {});
    bus.emitTyped("server.error", "queued");

    await vi.waitFor(() => expect(seen).toEqual(["queued"]));
  });
});

describe("browser transport — a peer that goes away", () => {
  it("rejects a request that is in flight when the server dies, rather than hanging", async () => {
    // The classic serialising-transport bug, and invisible until something crashes: a request in
    // flight when the peer dies must settle, not leave the caller awaiting a promise nobody will
    // ever resolve.
    let entered!: () => void;
    const reached = new Promise<void>((resolve) => {
      entered = resolve;
    });

    const { server, transport } = await pair(
      makeRegistry({
        "config.get": () => {
          entered();
          return new Promise(() => {}); // never settles
        },
      }),
    );

    const inFlight = transport.request("config.get", {});
    // The handler is running, so the request is genuinely in flight rather than merely queued.
    await reached;

    await server.close();

    await expect(inFlight).rejects.toBeInstanceOf(Error);
  });

  it("reports an unexpected peer death through onFatal", async () => {
    const fatals: Error[] = [];
    const { server } = await pair();
    const transport = createBrowserTransport({ url: server.url, onFatal: (e) => fatals.push(e) });
    transports.push(transport);

    await transport.request("config.get", {});
    await server.close();

    await vi.waitFor(() => expect(fatals).toHaveLength(1));
  });

  it("does NOT report a caller-initiated close as a fault", async () => {
    // The other half, and the half that is easy to get wrong: `dispose()` must detach the socket's
    // handlers *before* closing it, or the `close` it triggers re-enters the failure path and every
    // orderly shutdown is reported as a crash to whatever is watching.
    const fatals: Error[] = [];
    const { server } = await pair();
    const transport = createBrowserTransport({ url: server.url, onFatal: (e) => fatals.push(e) });
    transports.push(transport);

    await transport.request("config.get", {});
    await transport.close();

    // Give the close a chance to be mis-reported: assert on the settled state, not on an instant.
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(fatals).toEqual([]);
  });

  it("reports a connect failure distinctly from a dropped session", async () => {
    // Both are failures, and they are *different* failures: plan/08's work item 2 gives them different
    // UI ("Connecting" with the handshake result, versus "Disconnected" with reconnect), and only one
    // of them is worth retrying. So the distinction has to survive to the caller rather than collapsing
    // into "something went wrong".
    //
    // The address is a real port that was genuinely free a moment ago — bound and then released —
    // rather than a hardcoded number that might belong to something else on this machine.
    const bus = new EngineEventBus();
    const probe = await createWsServer({
      registry: makeRegistry(),
      bus,
      log: new EventLog({ bus }),
    });
    const deadUrl = probe.url;
    await probe.close();

    const fatals: Error[] = [];
    const transport = createBrowserTransport({
      url: deadUrl,
      // Short, and not because this case is in a hurry. A released port is not a *guaranteed* refusal:
      // under the full suite another file's listener can take it in the gap, and if that listener
      // accepts without answering, the connect ends on the timeout rather than on a refusal. Both are
      // connect failures and both are what this case is about — the bound keeps the second one well
      // inside the test timeout, so the assertion is what decides rather than the clock.
      connectTimeoutMs: 2_000,
      onFatal: (e) => fatals.push(e),
    });
    transports.push(transport);

    await expect(transport.request("config.get", {})).rejects.toBeInstanceOf(Error);
    await vi.waitFor(() => expect(fatals).toHaveLength(1));
    // Assert the shape that carries the distinction rather than one spelling of it: the engine was
    // never reached, said one of the two ways that can happen — and never as "the session you had is
    // gone", which is the message the UI answers with "reconnect".
    expect(fatals[0].message).toMatch(/Could not connect|Timed out after/);
    expect(fatals[0].message).not.toMatch(/Session closed/);
  });

  it("gives up on a peer that accepts the socket and then never answers", async () => {
    // The third way a connect can end, and the one the socket itself never reports: a listener that
    // completes the TCP handshake and then says nothing produces neither `open` nor `close`. Without a
    // timer `ensureReady()` never settles and the request waits forever — which is how this case first
    // failed on 2026-09-22, as a 15 s test timeout rather than a failed assertion, when a released
    // ephemeral port in the case above had been re-bound by another file's listener.
    //
    // A bare `node:net` server rather than a WebSocket one, because the point is a peer that speaks
    // TCP and nothing else: that is deterministic, where racing a released port is not. Before the
    // timeout existed this case hung; the assertion below is the thing that would catch its removal.
    const held: Socket[] = [];
    const silent = createTcpServer((socket) => {
      // Deliberately never write: the connection stays open and the upgrade is never answered.
      // Held so teardown can end it — `close()` alone waits for its connections to end, and this one is
      // designed not to.
      held.push(socket);
    });
    await new Promise<void>((resolve) => silent.listen(0, "127.0.0.1", () => resolve()));
    const port = (silent.address() as AddressInfo).port;

    const fatals: Error[] = [];
    const transport = createBrowserTransport({
      url: `ws://127.0.0.1:${port}`,
      connectTimeoutMs: 250,
      onFatal: (e) => fatals.push(e),
    });
    transports.push(transport);

    try {
      // The whole claim is that it *settles*, and that it settles as a connect failure.
      await expect(transport.request("config.get", {})).rejects.toThrow(/Timed out after 250ms/);
      await vi.waitFor(() => expect(fatals).toHaveLength(1));
      expect(fatals[0].message).toMatch(/Timed out after 250ms connecting to ws:\/\//);
    } finally {
      for (const socket of held) socket.destroy();
      await new Promise<void>((resolve) => silent.close(() => resolve()));
    }
  });

  it("reports a handshake the peer refuses as a connect failure", async () => {
    // The other half of `open === false`, and the half a *released* port cannot be relied on to
    // produce: something **is** listening, it just is not an engine. On a busy machine that is how a
    // wrong port usually looks — a different service on the number the user typed — and it has to be
    // reported as "the engine was never reached", never as a lost session, because that is the message
    // work item 2's UI answers with "reconnect".
    //
    // A plain HTTP server is the deterministic form of it: the upgrade is answered with a non-101
    // response, so the handshake fails without the socket ever opening. The case above passes on the
    // timeout branch on this machine, so without this one the "Could not connect" wording would have no
    // deterministic coverage at all.
    const notAnEngine = createHttpServer();
    /**
     * The refusal is written by hand, and that is **measured rather than assumed**.
     *
     * An `http.Server` with no `'upgrade'` listener neither answers an upgrade request nor closes it —
     * the socket is left in limbo. This case was first written with an ordinary `'request'` handler and
     * it failed on the **10 s connect timeout**, i.e. it reproduced the very hang the case above is
     * about instead of the refusal it was meant to pin. Only writing the non-101 response makes
     * "listening, but not an engine" deterministic.
     */
    notAnEngine.on("upgrade", (_req, socket) => {
      socket.write("HTTP/1.1 426 Upgrade Required\r\ncontent-length: 0\r\nconnection: close\r\n\r\n");
      socket.end();
    });
    await new Promise<void>((resolve) => notAnEngine.listen(0, "127.0.0.1", () => resolve()));
    const port = (notAnEngine.address() as AddressInfo).port;

    const fatals: Error[] = [];
    const transport = createBrowserTransport({
      url: `ws://127.0.0.1:${port}`,
      onFatal: (e) => fatals.push(e),
    });
    transports.push(transport);

    try {
      await expect(transport.request("config.get", {})).rejects.toThrow(/Could not connect/);
      await vi.waitFor(() => expect(fatals).toHaveLength(1));
      expect(fatals[0].message).toMatch(/Could not connect to ws:\/\//);
    } finally {
      notAnEngine.closeAllConnections();
      await new Promise<void>((resolve) => notAnEngine.close(() => resolve()));
    }
  });
});
