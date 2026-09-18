/**
 * The `ws` runner for the conformance suite — P4 work item 2, step 3.
 *
 * Like the other four, this file holds **no assertions of its own**. It binds a server, points a
 * client at it, and hands the pair to the shared suite. `plan/05` lists "conformance suite written
 * per-transport instead of shared" as a risk, and the way to keep that risk retired is to keep every
 * runner this thin.
 *
 * ## Why a real socket and not a stub
 *
 * The `stdio` runner uses a `PassThrough` pair because a spawned child would make every case a
 * process-lifecycle test. That shortcut is **not** available here, and deliberately so: `PassThrough`
 * delivers synchronously, so it could not have found the two defects this runner did — see below.
 * A `ws` runner that did not open a port would be testing the framing and nothing else, which is the
 * one thing WebSocket already gives us.
 *
 * The cost is real: this runner is the slowest of the five and it depends on ephemeral ports. That is
 * the price of being the first transport in the suite that is genuinely asynchronous.
 *
 * ## What it found, which is why it is worth the cost
 *
 * `plan/05` predicted one of these ("a TCP socket is never synchronous") and the suite's `deliver()`
 * barrier was built for it. Running it found that the barrier was **half** the fix:
 *
 * 1. It awaited *delivery* but not *subscription establishment*. `subscribe()` is synchronous, so for
 *    `in-process` and `PassThrough` the subscription is live by the next line — and the suite's
 *    "subscribe, then emit locally" pattern silently depended on that. Over a socket, `subscribe()`
 *    can only queue a control frame. Because the log's retention is **lazy**, an emit that lands
 *    before the frame is processed is not delayed, it is *dropped*, and the case times out with no
 *    evidence about why. `deliver()` now awaits a command round trip before emitting.
 * 2. `make()` had to become async-capable, because binding a port is asynchronous and `ws` cannot
 *    know its own URL until the OS has assigned one.
 *
 * Neither is a defect in `ws`. Both are the suite having been written against the only two transports
 * that existed, which is exactly the failure mode a third transport is for.
 *
 * ## Capabilities
 *
 * All four verifiable ones are claimed, and each is earned rather than declared:
 *
 * - `handshake` / `auth` — every connection is wrapped in `createAuthenticatedTransport`, so the
 *   `hello` exchange, the token check and the scope gate are exercised **through a socket** rather
 *   than over `in-process` (which is what `run-authenticated.test.ts` does).
 * - `replay` — the server is given the suite's own `EventLog`, so `lastSeq` means the engine's `seq`.
 * - `backpressure` — the connection's bus subscriptions go through a per-session `eventPump`, so
 *   `log.entry` is coalesced and `server.error` is never queued. **One pump per connection, not one
 *   per server**: a pump's queue holds frames for one channel, so a shared one would hand each client
 *   the other's events. That is the opposite of the `EventLog`, which is deliberately shared.
 */
import { afterEach } from "vitest";
import { PROTOCOL_VERSION } from "@bifurc/protocol";
import { createWsServer, createWsTransport, type WsServer } from "../../src/transport/ws";
import { defineConformanceSuite } from "./protocol.conformance";

const TOKEN = "conformance-ws-token";

/**
 * Servers built by this file's `make()`, so they can be closed.
 *
 * The suite closes the **transport** after each case but knows nothing about the server behind it —
 * `Transport` is the only interface it has. Without this, every case would leave a listening socket
 * and an attached session behind for the rest of the file, and the process would not exit.
 *
 * File-scope `afterEach` runs *after* the suite's own (Vitest's hooks are stack-ordered), so the
 * client says goodbye first and the server tears down a session that has already detached.
 */
const servers: WsServer[] = [];

afterEach(async () => {
  for (const server of servers.splice(0)) await server.close();
});

defineConformanceSuite({
  name: "ws",

  capabilities: { handshake: true, auth: true, replay: true, backpressure: true },

  make: async ({ registry, bus, log }, scopes) => {
    const server = await createWsServer({
      registry,
      bus,
      // The suite's own log, so the `seq` a reconnecting client presents is the one the engine
      // issued. The decorator advertises `events.replay` off the back of this being present.
      log,
      auth: {
        token: TOKEN,
        engineVersion: "0.0.0-conformance",
        // Undefined -> the decorator's default (SHELL_SCOPES), which is what the matrix needs.
        // A narrow set is only asked for by the FORBIDDEN case, which needs a session that can be
        // refused something.
        scopesFor: scopes === undefined ? undefined : () => new Set(scopes),
      },
      // Loopback and an ephemeral port: the default. Nothing here opts into the remote path, which
      // is the point — `tests/transport/ws.test.ts` is where the bind guards are asserted.
    });
    servers.push(server);
    // The URL, not the host/port pair: the server resolved `port: 0` to whatever the OS assigned, and
    // that resolved value is the only one that identifies this listener.
    return createWsTransport(server.url);
  },

  auth: {
    // The suite drives the handshake from this, so it can drive the *bad* ones too — and, for the
    // replay cases, the `lastSeq` of a client that has already seen something.
    hello: (overrides) => ({
      protocolVersion: PROTOCOL_VERSION,
      clientName: "web-ui",
      clientVersion: "0.0.0-conformance",
      token: TOKEN,
      ...overrides,
    }),
  },
});
