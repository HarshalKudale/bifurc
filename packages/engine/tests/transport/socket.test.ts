/**
 * `socket` behaviour that the conformance suite **cannot** express.
 *
 * The suite is transport-agnostic by design: it is handed a `Transport` and knows nothing about paths,
 * permissions or the filesystem. That is exactly what makes it reusable — and it is also why several
 * things worth asserting about a *listener* cannot live there:
 *
 *  - the **stale-socket rule**, which is the difference between "the app restarts" and "the app will
 *    not start and nothing says why";
 *  - the **`0600` assertion**, which is the whole of this transport's access control (`plan/05`'s auth
 *    table) and therefore the one property that must not be taken on trust;
 *  - that a path which exists and is **not** a socket is refused rather than deleted — the failure
 *    mode here is a user losing a file;
 *  - what happens to a request that is **in flight when the peer disappears**;
 *  - what happens to a **malformed frame from one peer**, and whether it takes the engine with it;
 *  - whether a disconnected session **releases its `EventLog` subscriptions** — the leak class that
 *    bit the `stdio` server in P4;
 *  - the **frame-size ceiling**, enforced by `framing.ts` rather than by any code in `socket.ts`.
 *
 * Everything that is about the `Transport` contract rather than about sockets lives in
 * `tests/conformance/run-socket.test.ts` and is asserted once, for every transport.
 *
 * ## Why these cases are deterministic without sleeping
 *
 * A real socket is asynchronous, so "wait a bit" is the obvious way to write these and the wrong one.
 * Every case below either (a) waits on a promise the engine itself resolves — a handler that signals
 * it was entered, a socket's `close` event — or (b) uses `vi.waitFor`, which retries until a condition
 * holds rather than assuming a duration. Nothing here asserts on *how long* anything takes.
 *
 * ## The POSIX-only cases
 *
 * A unix domain socket is a filesystem entry, so the permissions, stale-path and unlink cases have no
 * Windows counterpart: Node treats a path on Windows as a named pipe, which has no file, no mode and
 * nothing to leave behind. Those cases are `skipIf(win32)`, which means they do **not** run on the
 * Windows development machine this was written on — they are asserted here so CI on Linux and macOS
 * exercises them, and so the intent is recorded even where it cannot be executed. Nothing in
 * `socket.ts` is *only* reachable through them: the bind path, the framing and the session lifecycle
 * are all covered by the cases that do run everywhere.
 */
import { existsSync, readFileSync, statSync, unlinkSync, writeFileSync } from "node:fs";
import { createConnection, createServer } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { PROTOCOL_VERSION } from "@bifurc/protocol";
import { CommandRegistry } from "../../src/commands/registry";
import { EngineEventBus } from "../../src/eventBus";
import { EventLog } from "../../src/transport/eventLog";
import { encodeFrame, MAX_FRAME_BYTES } from "../../src/transport/framing";
import type { Transport } from "../../src/transport/types";
import {
  createSocketServer,
  createSocketTransport,
  defaultSocketPath,
  type SocketServer,
  type SocketServerOptions,
} from "../../src/transport/socket";

const isWindows = process.platform === "win32";

/** Distinct per test file, so parallel files cannot collide on a path. */
const FILE_TAG = Math.random().toString(36).slice(2, 8);
let pathCounter = 0;

function uniqueSocketPath(): string {
  const name = `bifurc-socket-test-${FILE_TAG}-${pathCounter++}`;
  return isWindows ? `\\\\.\\pipe\\${name}` : join(tmpdir(), `${name}.sock`);
}

/** A registry with two real commands, so a name typo is a `UNKNOWN_COMMAND` rather than a passing test. */
function makeRegistry(handler: () => unknown = () => ({ port: 8080 })): CommandRegistry {
  const registry = new CommandRegistry();
  registry.register("config.get", handler);
  registry.register("env.setActive", () => ({ reached: "env.setActive" }));
  return registry;
}

const servers: SocketServer[] = [];
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
  options: Partial<Omit<SocketServerOptions, "registry" | "bus" | "path">> & {
    bus?: EngineEventBus;
  } = {},
  transportOptions: Parameters<typeof createSocketTransport>[1] = {},
): Promise<{ server: SocketServer; transport: Transport; bus: EngineEventBus }> {
  const bus = options.bus ?? new EngineEventBus();
  const server = await createSocketServer({
    path: uniqueSocketPath(),
    registry: makeRegistry(),
    ...options,
    bus,
  });
  servers.push(server);
  const transport = createSocketTransport(server.path, transportOptions);
  transports.push(transport);
  return { server, transport, bus };
}

afterEach(async () => {
  for (const t of transports.splice(0)) await t.close();
  for (const s of servers.splice(0)) await s.close();
});

describe("socket — the path", () => {
  it("names a per-pid path, as a pipe on Windows and a socket file elsewhere", () => {
    // `plan/07` is where this naming scheme comes from, and both ends need it: P6 spawns the engine
    // and connects, and P8's CLI probes the same path to decide whether to attach instead of spawn.
    // Two copies of the scheme is how an attach-mode client ends up unable to find the engine it just
    // started, which is why it lives in `socket.ts` and is asserted rather than assumed.
    const path = defaultSocketPath(4242);
    if (isWindows) {
      expect(path).toBe("\\\\.\\pipe\\bifurc-4242");
    } else {
      expect(path).toMatch(/bifurc-4242\.sock$/);
      // `$XDG_RUNTIME_DIR` when it is set, because the spec requires that directory to be `0700` and
      // per-user — which makes the socket private before the `chmod` below has to do anything.
      const runtime = process.env.XDG_RUNTIME_DIR;
      if (runtime !== undefined && runtime.length > 0) expect(path).toBe(join(runtime, "bifurc-4242.sock"));
      else expect(path).toBe(join(tmpdir(), "bifurc-4242.sock"));
    }
  });

  it("refuses an empty path rather than binding something arbitrary", async () => {
    // A default here would be worse than an error: an engine that invented a path would make a test
    // that forgot to pass a temp one collide with the developer's running engine.
    await expect(createSocketServer({ path: "", registry: makeRegistry() })).rejects.toThrow(
      /non-empty socket path/,
    );
  });

  it.skipIf(isWindows)("REFUSES a path that exists and is not a socket, and does NOT delete it", async () => {
    // The one case here whose failure mode is data loss. Without the `isSocket()` check, an engine
    // starting up would `unlink()` whatever happened to be at the path — and a user who mistyped a
    // path in a config file would lose a document, silently, on next launch.
    const path = uniqueSocketPath();
    writeFileSync(path, "definitely not a socket");

    await expect(
      createSocketServer({ path, registry: makeRegistry() }),
    ).rejects.toThrow(/not a socket/);

    // Still there, still intact.
    expect(existsSync(path)).toBe(true);
    expect(readFileSync(path, "utf8")).toBe("definitely not a socket");
    unlinkSync(path);
  });

  it.skipIf(isWindows)("unlinks a STALE socket file and binds over it", async () => {
    // A crashed engine leaves the entry behind, and `bind()` then fails with `EADDRINUSE` for as long
    // as the file exists — the classic "the app will not start and nothing says why".
    //
    // The premise is asserted rather than assumed: a raw `net.Server` closed with `close()` leaves its
    // socket file on disk, which is exactly how the corpse gets there in the first place.
    const path = uniqueSocketPath();
    const raw = createServer();
    await new Promise<void>((resolve) => raw.listen(path, () => resolve()));
    await new Promise<void>((resolve) => raw.close(() => resolve()));
    expect(existsSync(path)).toBe(true);

    const server = await createSocketServer({ path, registry: makeRegistry(), bus: new EngineEventBus() });
    servers.push(server);
    expect(existsSync(path)).toBe(true);

    // And it really is serving, not merely present.
    const transport = createSocketTransport(path);
    transports.push(transport);
    await expect(transport.request("config.get", {})).resolves.toEqual({ port: 8080 });
  });

  it.skipIf(isWindows)("REFUSES to bind over a socket another engine is listening on", async () => {
    // The other half of the stale-socket rule: a live entry must be reported loudly, not unlinked.
    // Unlinking it would leave the running engine unreachable while a second one took its name — a
    // silent split brain, and far worse than a startup failure.
    const path = uniqueSocketPath();
    const first = await createSocketServer({ path, registry: makeRegistry(), bus: new EngineEventBus() });
    servers.push(first);

    await expect(
      createSocketServer({ path, registry: makeRegistry(), bus: new EngineEventBus() }),
    ).rejects.toThrow(/already listening/);

    // The live engine is untouched by the probe.
    const transport = createSocketTransport(path);
    transports.push(transport);
    await expect(transport.request("config.get", {})).resolves.toEqual({ port: 8080 });
  });

  it.skipIf(isWindows)("is 0600 by the time createSocketServer() resolves", async () => {
    // `plan/05`'s auth table: "socket / named pipe — Filesystem permissions (0600). Token additionally
    // recommended." This is the whole of the access control when no token is configured, so it is
    // asserted rather than assumed — and it is asserted *after* `await`, because the guarantee the
    // caller has is that no window exists in which a server built from this one is reachable.
    const { server } = await pair();
    const mode = statSync(server.path).mode & 0o777;
    // The load-bearing half: nothing for group or other.
    expect(mode & 0o077).toBe(0);
    expect(mode).toBe(0o600);
  });

  it.skipIf(isWindows)("removes the socket file on close()", async () => {
    // Leaving it behind would make the *next* start pay for this one's cleanup — and the next start
    // may be a different user, who cannot unlink a file they do not own.
    const server = await createSocketServer({
      path: uniqueSocketPath(),
      registry: makeRegistry(),
      bus: new EngineEventBus(),
    });
    expect(existsSync(server.path)).toBe(true);
    await server.close();
    expect(existsSync(server.path)).toBe(false);
  });
});

describe("socket — a dying peer", () => {
  it("REJECTS a request that was in flight when the peer went away", async () => {
    // The classic serialising-transport bug: the engine dies mid-request and the caller awaits a
    // promise nobody will ever settle. A CLI that hangs forever with no error is far worse than one
    // that exits with a message.
    let entered: () => void = () => {};
    const enteredPromise = new Promise<void>((resolve) => {
      entered = resolve;
    });

    const bus = new EngineEventBus();
    const server = await createSocketServer({
      registry: makeRegistry(() => {
        entered();
        // Never settles, so the request is genuinely in flight when the server goes away.
        return new Promise(() => {});
      }),
      path: uniqueSocketPath(),
      bus,
    });
    servers.push(server);
    const transport = createSocketTransport(server.path);
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

describe("socket — the engine keeps serving", () => {
  it("serves commands with no auth configured, on a 0600 socket", async () => {
    // The `companionServer.ts` configuration, kept deliberately: auth is optional because the `0600`
    // mode is what makes an unauthenticated engine unreachable by anyone but this user. This is the
    // case that would fail if someone made `auth` mandatory, which is a reasonable-looking change that
    // would break every embedded client.
    const { transport } = await pair();
    await expect(transport.request("config.get", {})).resolves.toEqual({ port: 8080 });
  });

  it("closes only the connection that sent a frame it cannot correlate", async () => {
    // `stdio` treats a frame it cannot correlate as fatal to the *process*, because a stdio engine has
    // exactly one peer. A socket engine has many — the shell, the CLI, a probe — so one broken client
    // must not take down the process serving the rest. This is the case that would fail if the frame
    // handler threw, or if it reused `stdio`'s `fail()`-the-whole-server shape.
    const { server, transport } = await pair();
    await expect(transport.request("config.get", {})).resolves.toEqual({ port: 8080 });

    const raw = createConnection(server.path);
    await new Promise<void>((resolve) => raw.once("connect", () => resolve()));
    raw.on("error", () => {});
    const closed = new Promise<void>((resolve) => raw.once("close", () => resolve()));
    // Valid framing, valid JSON, not an `RpcRequest` — no string `id`, so there is nothing to
    // correlate a reply to and no way to answer at all.
    raw.write(encodeFrame({ not: "an RpcRequest envelope" }));
    await closed;

    // And the first client is untouched.
    await expect(transport.request("config.get", {})).resolves.toEqual({ port: 8080 });
  });

  it("refuses an oversized frame WITHOUT waiting for its body, and reports it", async () => {
    // Work item 5's "Request size: ... at the frame level". The length prefix is attacker-controlled
    // the moment this runs over a socket, so a peer that declares 4 GiB must be rejected *now* rather
    // than after it has sent 4 GiB — which is why this case writes four bytes and nothing else, and
    // still expects the connection to end.
    //
    // It also pins the deliberate divergence from `ws`: `ws` closes without reporting, because its
    // framing lives inside the library and a close code is all it has. Here the framing is ours, so
    // there is a real error to hand to the audit hook.
    const fatals: Error[] = [];
    const { server, transport } = await pair({ onFatal: (e) => fatals.push(e) });

    const raw = createConnection(server.path);
    await new Promise<void>((resolve) => raw.once("connect", () => resolve()));
    raw.on("error", () => {});
    const closed = new Promise<void>((resolve) => raw.once("close", () => resolve()));

    const header = Buffer.alloc(4);
    header.writeUInt32BE(MAX_FRAME_BYTES + 1, 0);
    raw.write(header);
    await closed;

    await vi.waitFor(() => expect(fatals).toHaveLength(1));
    // `frameFailure` keeps the peer's fault distinguishable from the engine's.
    expect(fatals[0]).toMatchObject({ code: "BAD_REQUEST" });

    // One peer's garbage did not stop the engine.
    await expect(transport.request("config.get", {})).resolves.toEqual({ port: 8080 });
  });
});

describe("socket — event delivery", () => {
  it("carries the ENGINE's seq, so a subset subscription sees gaps", async () => {
    // The assertion that distinguishes "the engine numbered this" from "the client counted arrivals".
    // `seq` is engine-scoped (`eventLog.ts`), so a session subscribed to a subset sees gaps — events
    // it did not subscribe to still consume a number. The conformance suite cannot catch a client-side
    // recount, because its replay case emits exactly the events the client subscribed to and both
    // numberings agree there. Here they do not.
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
    // The leak class that bit the `stdio` server in P4: an attached transport does not own the socket's
    // other end, so a graceful close left the engine's bus listeners attached with nobody to receive
    // them, and the engine accumulated one per closed session until Node's warning fired.
    // `subscriberCount` is where that is measured now that the `EventLog` owns retention.
    const bus = new EngineEventBus();
    const log = new EventLog({ bus });
    const { transport } = await pair({ bus, log });

    transport.subscribe(["event.server.error"], () => {});
    await vi.waitFor(() => expect(log.subscriberCount("event.server.error")).toBe(1));

    await transport.close();
    await vi.waitFor(() => expect(log.subscriberCount("event.server.error")).toBe(0));
  });
});

describe("socket — the auth consequence (work item 5)", () => {
  it("answers UNAUTHORIZED and then closes the session", async () => {
    // "Auth failure returns UNAUTHORIZED and closes. Never fall back to unauthenticated." Both halves
    // matter, and the ordering matters more here than on `ws`, not less: a WebSocket can carry a close
    // code, whereas a plain socket has no way to say *why* it went away. The reply is the only signal
    // the peer gets, so a peer disconnected without it cannot tell a rejected credential from a crash.
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
    // The other half of the split `authenticated.ts` makes, and the reason `IDENTITY_FAILURE_CODES`
    // holds only `UNAUTHORIZED` and `UNSUPPORTED`. Closing here would turn a scope error into a
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

  it("advertises the socket capability, so a client can tell which transport it is on", async () => {
    // `Capability.SOCKET` is what `Capability.WS` is for the WebSocket transport. Without it a client
    // would have to infer the transport from what it happened to construct, and the `hello` response
    // is the only place capabilities are reported.
    const { transport } = await pair({
      auth: { token: "the-right-token", engineVersion: "0.0.0-test" },
    });

    const response = (await transport.request("hello", {
      protocolVersion: PROTOCOL_VERSION,
      clientName: "electron-shell",
      clientVersion: "0.0.0-test",
      token: "the-right-token",
    })) as { capabilities: string[] };

    expect(response.capabilities).toContain("socket");
  });
});
