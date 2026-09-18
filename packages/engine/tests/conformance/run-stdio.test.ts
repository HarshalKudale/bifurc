/**
 * The `stdio` runner for the conformance suite.
 *
 * Like `run-in-process.test.ts`, this file holds **no assertions of its own** — it wires a transport
 * to the shared suite and nothing more. `plan/05` lists "conformance suite written per-transport
 * instead of shared" as a risk, and the way to keep that risk retired is to keep every runner this
 * thin.
 *
 * ## Why a `PassThrough` pair and not a spawned child process
 *
 * The suite is written for a `Transport`, and a real `spawnStdioEngine()` child would make every case
 * a process-lifecycle test: a slow CI machine turns "resolves with the handler's value" into a
 * timeout, and a leaked child turns a failing assertion into a hung run. Two `PassThrough` streams
 * carry the same bytes through the same `framing.ts` codec and the same correlation logic, with no
 * process to leak.
 *
 * The cost is real and worth stating: **this does not cover asynchrony.** A `PassThrough` is
 * in-process and, measured on 2026-09-17, delivers synchronously — `bus.emitTyped()` reaches a
 * subscriber before the next line runs:
 *
 * ```
 * MEASUREMENT  delivery immediately after emitTyped: 1; after settle: 1
 * ```
 *
 * That measurement is why this runner worked at all before 2026-09-18, because the suite's
 * `subscribe()` cases used to assert **synchronously**:
 *
 * ```ts
 * bus.emitTyped("server.error", "one");
 * expect(seen).toHaveLength(1);        // no await — fine for in-process and for PassThrough
 * ```
 *
 * A real pipe goes through the OS and is genuinely asynchronous, so that assertion would have failed
 * there. **That was a latent defect in the suite, not in this transport** — `Transport` never promised
 * synchronous delivery — and it was fixed on 2026-09-18 by giving the shared suite a `deliver()`
 * barrier, *before* `run-ws.test.ts` could have failed for a reason that was not its own. The `ws`
 * runner is what made the fix mandatory rather than optional; `run-deferred-delivery.test.ts` was the
 * interim negative control and is deleted now that the real control exists.
 *
 * Note what did **not** change: this file. `make()` is still synchronous and still claims nothing,
 * because the barrier is invisible to a transport that never needed it — which is the property that
 * makes it a fix rather than a rewrite of the programme's most valuable asset.
 *
 * Async behaviour is covered instead by `tests/transport/stdio.test.ts`, which drives a real in-flight
 * request against a peer that disappears mid-request and asserts it **rejects** rather than hanging.
 *
 * ## Capabilities
 *
 * `backpressure` and nothing else. `stdio` has no `hello`, no auth and no replay buffer, so the suite
 * would refuse those claims: `plan/05` work item 4's table is explicit that `stdio` needs no auth —
 * "the pipe is inherited and process-scoped" — so there is no boundary to authenticate across.
 *
 * `backpressure` is earned rather than declared: `createStdioServer` routes its bus subscriptions
 * through `eventPump.ts`, so `log.entry` is coalesced at 100 entries or 250 ms and `server.error` takes
 * the immediate path. Note that this transport needs no `EventLog` for it — the pump is per-session
 * state, and a pipe has exactly one session — which is why the claim does not come with a `log` in
 * `make()` below.
 */
import { afterEach } from "vitest";
import { PassThrough } from "node:stream";
import { createStdioServer, createStdioTransport, type StdioServer } from "../../src/transport/stdio";
import { defineConformanceSuite } from "./protocol.conformance";

/**
 * Servers built by this file's `make()`, so they can be closed.
 *
 * The suite closes the **transport** after each case but knows nothing about the server behind it —
 * `Transport` is the only interface it has. Without this the engine side of every case would stay
 * attached to a dead pipe for the rest of the file. File-scope `afterEach` runs *after* the suite's
 * own (Vitest's hooks are `stack`-ordered), so the transport says goodbye first and the server
 * detaches cleanly rather than being torn out from under a live client.
 */
const servers: StdioServer[] = [];

afterEach(async () => {
  for (const server of servers.splice(0)) await server.close();
});

defineConformanceSuite({
  name: "stdio",

  // Only `backpressure` — see the header. No `log` is passed to the server, because coalescing does not
  // need one and claiming `replay` would.
  capabilities: { backpressure: true },

  make: ({ registry, bus }) => {
    // `toServer` carries client → engine, `toClient` carries engine → client. Two simplex channels
    // rather than one duplex, because that is what a real stdio pair is: the engine's stdin and the
    // engine's stdout are separate streams with separate lifecycles.
    const toServer = new PassThrough();
    const toClient = new PassThrough();
    servers.push(
      createStdioServer({ input: toServer, output: toClient, registry, bus, ctx: { bus } }),
    );
    return createStdioTransport({ input: toServer, output: toClient });
  },
});
