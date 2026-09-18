/**
 * The `socket` runner for the conformance suite — P4 work item 2, step 4.
 *
 * Like the other four, this file holds **no assertions of its own**. It binds a server on a real
 * unix socket / named pipe, points a client at it, and hands the pair to the shared suite. `plan/05`
 * lists "conformance suite written per-transport instead of shared" as a risk, and the way to keep
 * that risk retired is to keep every runner this thin.
 *
 * ## Why this runner is cheap, when the `ws` runner was not
 *
 * `run-ws.test.ts` records what the suite cost the *first* time it met a genuinely asynchronous
 * transport: `deliver()` awaited delivery but not subscription establishment, and `make()` had to
 * become async-capable. Both are now properties of the suite rather than of any transport, so this
 * runner is a runner — bind, connect, hand over — and the plan predicted exactly this: *"the suite is
 * ready for `socket` in the way it was not ready for `ws`."*
 *
 * It is nonetheless the second transport to be asynchronous for real reasons, and it is deliberately
 * **not** a `PassThrough` pair like `stdio`'s. A `PassThrough` delivers synchronously, so it would
 * have re-hidden the very class of defect the `ws` runner was built to expose — and this is now the
 * transport the desktop shell actually uses, which makes "it passes over a stub" a worthless claim.
 *
 * ## Capabilities
 *
 * All four verifiable ones are claimed, and each is earned rather than declared:
 *
 * - `handshake` / `auth` — every connection is wrapped in `createAuthenticatedTransport`, so the
 *   `hello` exchange, the token check and the scope gate are exercised **through a unix socket**
 *   rather than over `in-process` (which is what `run-authenticated.test.ts` does).
 * - `replay` — the server is given the suite's own `EventLog`, so `lastSeq` means the engine's `seq`.
 * - `backpressure` — the connection's bus subscriptions go through a per-session `eventPump`, so
 *   `log.entry` is coalesced and `server.error` is never queued. Per connection, not per server, for
 *   the reason `ws.ts` gives at the same seam.
 *
 * ## A fresh path per case, because a unix socket is a filesystem entry
 *
 * `defaultSocketPath()` is deliberately *not* used here. It is derived from the pid, and Vitest runs
 * several test files in one process, so two files would compute the same path and the second would be
 * refused by the stale-socket guard — a failure that looks like a transport bug and is not one. The
 * per-file tag keeps each runner's paths disjoint; the counter keeps each case's disjoint from the
 * last, so a case that fails to clean up cannot poison the next one. `defaultSocketPath()` itself is
 * asserted in `tests/transport/socket.test.ts`, where it can be checked without binding anything.
 */
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach } from "vitest";
import { PROTOCOL_VERSION } from "@bifurc/protocol";
import {
  createSocketServer,
  createSocketTransport,
  type SocketServer,
} from "../../src/transport/socket";
import { defineConformanceSuite } from "./protocol.conformance";

const TOKEN = "conformance-socket-token";

/** Distinct per test file, so parallel files cannot collide on a path. */
const FILE_TAG = Math.random().toString(36).slice(2, 8);
let pathCounter = 0;

function uniqueSocketPath(): string {
  const name = `bifurc-conformance-${FILE_TAG}-${pathCounter++}`;
  return process.platform === "win32" ? `\\\\.\\pipe\\${name}` : join(tmpdir(), `${name}.sock`);
}

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
const servers: SocketServer[] = [];

afterEach(async () => {
  for (const server of servers.splice(0)) await server.close();
});

defineConformanceSuite({
  name: "socket",

  capabilities: { handshake: true, auth: true, replay: true, backpressure: true },

  make: async ({ registry, bus, log }, scopes) => {
    const server = await createSocketServer({
      path: uniqueSocketPath(),
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
    });
    servers.push(server);
    // `server.path`, not the local variable: the server is the thing that actually bound, and the
    // stale-socket rule means the two could in principle differ. (They do not today — `close()` is
    // what removes the entry, and it unlinks exactly the path it bound.)
    return createSocketTransport(server.path);
  },

  auth: {
    // The suite drives the handshake from this, so it can drive the *bad* ones too — and, for the
    // replay cases, the `lastSeq` of a client that has already seen something.
    //
    // `electron-shell` rather than `web-ui`: this is D3's desktop transport, and the name is recorded
    // per session so the engine can log which build is connected. It does not select the scopes —
    // `scopesFor` does, and it defaults to the shell's — but the recorded name should be true.
    hello: (overrides) => ({
      protocolVersion: PROTOCOL_VERSION,
      clientName: "electron-shell",
      clientVersion: "0.0.0-conformance",
      token: TOKEN,
      ...overrides,
    }),
  },
});
