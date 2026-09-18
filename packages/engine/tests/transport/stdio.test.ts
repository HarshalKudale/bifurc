/**
 * `stdio` behaviour that the conformance suite **cannot** express.
 *
 * The suite is transport-agnostic by design: it is handed a `Transport` and knows nothing about pipes,
 * processes or stream lifecycles. That is exactly what makes it reusable across `in-process`, `stdio`
 * and `ws` — and it is also why several things worth asserting about a pipe cannot live there:
 *
 *  - what happens to a request that is **in flight when the peer disappears**;
 *  - `onFatal` firing on a broken session but not on a clean shutdown;
 *  - the server half being driven with raw frames the client would never send;
 *  - a graceful `close()` releasing the *server's* bus listeners.
 *
 * Those live here. Everything that is about the `Transport` contract rather than about pipes lives in
 * `tests/conformance/run-stdio.test.ts` and is asserted once, for every transport.
 *
 * ## The harness, and the one thing it does not model
 *
 * A `PassThrough` pair carries real bytes through the real `framing.ts` codec with no process to leak.
 * What it does **not** model is timing: measured on 2026-09-17, a `PassThrough` in flowing mode emits
 * `data` synchronously from `write()`, so a round trip here completes within one tick where a real
 * child process would take milliseconds and several. The cases below therefore never assert on *how
 * long* something takes, only on the order of causes and effects — which is what makes them stable.
 *
 * `toClient.end()` is the stand-in for "the engine's stdout closed": it is what a crashed child, a
 * killed process and a closed pipe all look like from the client's side.
 */
import { describe, expect, it } from "vitest";
import { PassThrough } from "node:stream";
import { EngineError, isKnownCommand } from "@bifurc/protocol";
import { CommandRegistry } from "../../src/commands/registry";
import { EngineEventBus } from "../../src/eventBus";
import { encodeFrame } from "../../src/transport/framing";
import { createStdioServer, createStdioTransport, RESERVED_ACTIONS } from "../../src/transport/stdio";
import type { Transport } from "../../src/transport/types";

const RESERVED = "config.get";

/** Let the pipe drain. Several immediate ticks, since each hop is one tick. */
async function settle(rounds = 6): Promise<void> {
  for (let i = 0; i < rounds; i++) await new Promise((r) => setImmediate(r));
}

interface Harness {
  transport: Transport;
  bus: EngineEventBus;
  /** The client → engine pipe, for writing raw frames the client would never send. */
  toServer: PassThrough;
  /** The engine → client pipe, for simulating the engine dying. */
  toClient: PassThrough;
  fatals: Error[];
}

function harness(handler: () => unknown = () => ({ port: 8080 })): Harness {
  const registry = new CommandRegistry();
  const bus = new EngineEventBus();
  registry.register(RESERVED, handler);
  registry.register("env.setActive", () => ({ reached: "env.setActive" }));

  const toServer = new PassThrough();
  const toClient = new PassThrough();
  createStdioServer({ input: toServer, output: toClient, registry, bus, ctx: { bus } });
  const fatals: Error[] = [];
  const transport = createStdioTransport({
    input: toServer,
    output: toClient,
    onFatal: (e) => fatals.push(e),
  });
  return { transport, bus, toServer, toClient, fatals };
}

/** Read the single frame a raw exchange produced. */
async function readFrame(toClient: PassThrough): Promise<Record<string, unknown>> {
  const chunks: Buffer[] = [];
  toClient.on("data", (c: Buffer) => chunks.push(c));
  await settle();
  expect(chunks).toHaveLength(1);
  const header = chunks[0]!.readUInt32BE(0);
  return JSON.parse(chunks[0]!.subarray(4, 4 + header).toString("utf8")) as Record<string, unknown>;
}

describe("stdio — a dying peer", () => {
  it("REJECTS a request that was in flight when the peer went away", async () => {
    // The classic stdio-transport bug, and the reason this file exists: the engine dies mid-request
    // and the caller awaits a promise nobody will ever settle. A CLI that hangs forever with no error
    // is far worse than one that exits with a message.
    const { transport, toClient } = harness(async () => {
      await new Promise((r) => setTimeout(r, 1000));
      return { port: 1 };
    });

    const inFlight = transport.request(RESERVED, {});
    await settle(2);
    toClient.end();

    await expect(inFlight).rejects.toBeInstanceOf(EngineError);
  });

  it("calls onFatal exactly once when the peer goes away", async () => {
    const { toClient, fatals } = harness();
    await settle(2);
    toClient.end();
    await settle(4);
    expect(fatals).toHaveLength(1);
  });

  it("does NOT call onFatal for a caller-initiated close", async () => {
    // A clean shutdown that looked like a crash to whatever is watching `onFatal` would make every
    // graceful exit log an error.
    const { transport, toClient, fatals } = harness();
    await settle(2);
    await transport.close();
    toClient.end();
    await settle(4);
    expect(fatals).toHaveLength(0);
  });

  it("kills the session on a frame that is not valid JSON, rather than desynchronising it", async () => {
    const { transport, toClient } = harness(async () => {
      await new Promise((r) => setTimeout(r, 1000));
      return { port: 8080 };
    });

    const inFlight = transport.request(RESERVED, {});
    await settle(2);
    // An honest length prefix with a body that is not JSON. Frame boundaries are still aligned, but
    // we cannot know what the sender meant — so the session ends rather than acting on half a message.
    const body = Buffer.from("{not json", "utf8");
    const header = Buffer.allocUnsafe(4);
    header.writeUInt32BE(body.length, 0);
    toClient.write(Buffer.concat([header, body]));

    await expect(inFlight).rejects.toBeInstanceOf(EngineError);
    await expect(transport.request(RESERVED, {})).rejects.toBeInstanceOf(EngineError);
  });

  it("kills the session on an oversized frame", async () => {
    const { transport, toClient } = harness();
    await settle(2);
    const header = Buffer.allocUnsafe(4);
    header.writeUInt32BE(0xffffffff, 0);
    toClient.write(header);

    await expect(transport.request(RESERVED, {})).rejects.toBeInstanceOf(EngineError);
  });
});

describe("stdio — correlation", () => {
  it("correlates concurrent in-flight requests by id, not by arrival order", async () => {
    // A slow request must not block or steal the answer to a fast one sent behind it.
    const registry = new CommandRegistry();
    const bus = new EngineEventBus();
    registry.register(RESERVED, async () => {
      await new Promise((r) => setTimeout(r, 5));
      return { port: 8080 };
    });
    registry.register("env.setActive", () => ({ reached: "env.setActive" }));
    const toServer = new PassThrough();
    const toClient = new PassThrough();
    createStdioServer({ input: toServer, output: toClient, registry, bus, ctx: { bus } });
    const transport = createStdioTransport({ input: toServer, output: toClient });

    const [slow, fast] = await Promise.all([
      transport.request(RESERVED, {}),
      transport.request("env.setActive", { id: "env-1" }),
    ]);

    expect(slow).toEqual({ port: 8080 });
    expect(fast).toEqual({ reached: "env.setActive" });
  });

  it("rejects BAD_REQUEST for an unframeable payload but keeps the session usable", async () => {
    // A caller's mistake must not kill a healthy session: a cyclic payload cannot be framed, but that
    // says nothing about the pipe.
    const { transport } = harness();
    const cyclic: Record<string, unknown> = {};
    cyclic.self = cyclic;

    await expect(transport.request(RESERVED, cyclic)).rejects.toMatchObject({ code: "BAD_REQUEST" });
    await expect(transport.request(RESERVED, {})).resolves.toEqual({ port: 8080 });
  });
});

describe("stdio — the server half, driven with raw frames", () => {
  it("refuses a subscribe for an unknown event with UNSUPPORTED", async () => {
    const { toServer, toClient } = harness();
    const frame = readFrame(toClient);
    toServer.write(encodeFrame({ id: "raw-1", action: "subscribe", payload: { events: ["event.not.real"] } }));

    expect(await frame).toMatchObject({
      id: "raw-1",
      ok: false,
      error: { code: "UNSUPPORTED" },
    });
  });

  it("refuses a subscribe for an event with no wire name yet with UNSUPPORTED", async () => {
    // `process.statusChange` is a real bus event with no protocol name — subscribing must be refused
    // rather than accepted and never fired, which would be debugged as a broken engine.
    const { toServer, toClient } = harness();
    const frame = readFrame(toClient);
    toServer.write(
      encodeFrame({ id: "raw-2", action: "subscribe", payload: { events: ["event.process.statusChange"] } }),
    );

    expect(await frame).toMatchObject({ ok: false, error: { code: "UNSUPPORTED" } });
  });

  it("answers BAD_REQUEST for a malformed subscribe payload", async () => {
    const { toServer, toClient } = harness();
    const frame = readFrame(toClient);
    toServer.write(encodeFrame({ id: "raw-3", action: "subscribe", payload: { events: "not-an-array" } }));

    expect(await frame).toMatchObject({ ok: false, error: { code: "BAD_REQUEST" } });
  });

  it("answers BAD_REQUEST for a malformed unsubscribe payload", async () => {
    const { toServer, toClient } = harness();
    const frame = readFrame(toClient);
    toServer.write(encodeFrame({ id: "raw-4", action: "unsubscribe", payload: { events: 42 } }));

    expect(await frame).toMatchObject({ ok: false, error: { code: "BAD_REQUEST" } });
  });

  it("treats an unsubscribe for a name it is not subscribed to as a no-op, not an error", async () => {
    const { toServer, toClient } = harness();
    const frame = readFrame(toClient);
    toServer.write(encodeFrame({ id: "raw-5", action: "unsubscribe", payload: { events: ["event.server.error"] } }));

    expect(await frame).toMatchObject({ id: "raw-5", ok: true });
  });

  it("kills the session on a frame with no id or action, and tells the client", async () => {
    // The protocol has no notification frame (`RpcRequestSchema` requires `id`), so a frame that is
    // neither a request nor an event is a peer bug. It cannot be answered — there is no id to answer
    // to — so the session ends. Crucially the *client* must find out: a server that quietly stopped
    // responding would leave every request in flight hanging forever, with no error and no timeout.
    const registry = new CommandRegistry();
    const bus = new EngineEventBus();
    registry.register(RESERVED, () => ({ port: 8080 }));
    const toServer = new PassThrough();
    const toClient = new PassThrough();
    const serverFatals: Error[] = [];
    createStdioServer({
      input: toServer,
      output: toClient,
      registry,
      bus,
      ctx: { bus },
      onFatal: (e) => serverFatals.push(e),
    });
    const clientFatals: Error[] = [];
    const transport = createStdioTransport({
      input: toServer,
      output: toClient,
      onFatal: (e) => clientFatals.push(e),
    });
    await settle(2);

    toServer.write(encodeFrame({ hello: "there" }));
    await settle(4);

    expect(serverFatals).toHaveLength(1);
    // The server ended its output, so the client's stream ends and it learns the session is over.
    expect(clientFatals).toHaveLength(1);
    await expect(transport.request(RESERVED, {})).rejects.toBeInstanceOf(EngineError);
  });
});

describe("stdio — server shutdown semantics", () => {
  it("close() does NOT end the output stream", async () => {
    // A graceful close must leave the process's stdout usable: the engine is going to exit anyway, and
    // a server that closed its own stdout would take the shell's console with it.
    const toServer = new PassThrough();
    const toClient = new PassThrough();
    const registry = new CommandRegistry();
    const bus = new EngineEventBus();
    registry.register(RESERVED, () => ({ port: 8080 }));
    const server = createStdioServer({ input: toServer, output: toClient, registry, bus, ctx: { bus } });

    await server.close();

    expect(server.closed).toBe(true);
    expect(toClient.writableEnded).toBe(false);
    expect(toClient.destroyed).toBe(false);
  });

  it("a fatal frame error DOES end the output stream, so the peer is told", async () => {
    const toServer = new PassThrough();
    const toClient = new PassThrough();
    const registry = new CommandRegistry();
    const bus = new EngineEventBus();
    registry.register(RESERVED, () => ({ port: 8080 }));
    const server = createStdioServer({ input: toServer, output: toClient, registry, bus, ctx: { bus } });
    await settle(2);

    // A frame that cannot be correlated, and so cannot be answered.
    toServer.write(encodeFrame({ hello: "there" }));
    await settle(4);

    expect(server.closed).toBe(true);
    expect(server.error).toBeInstanceOf(EngineError);
    expect(toClient.writableEnded).toBe(true);
  });

  it("close() after a fatal error is a no-op and preserves the error", async () => {
    const toServer = new PassThrough();
    const toClient = new PassThrough();
    const registry = new CommandRegistry();
    const bus = new EngineEventBus();
    registry.register(RESERVED, () => ({ port: 8080 }));
    const server = createStdioServer({ input: toServer, output: toClient, registry, bus, ctx: { bus } });
    await settle(2);

    toServer.write(encodeFrame({ hello: "there" }));
    await settle(4);
    const reason = server.error;
    await server.close();

    expect(server.error).toBe(reason);
  });
});

describe("stdio — reserved action names", () => {
  it("does not shadow any real protocol command", () => {
    // `subscribe`/`unsubscribe` are reserved as *bare* names precisely because every real command is
    // namespaced (`config.get`, `blob.put`). If that ever stops being true, `stdio.ts` throws at
    // import time rather than silently routing the command into transport control.
    for (const reserved of RESERVED_ACTIONS) {
      expect(isKnownCommand(reserved), `"${reserved}" must not be a real command`).toBe(false);
    }
  });
});
