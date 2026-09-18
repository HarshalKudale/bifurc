/**
 * P4 work item 4 — the authenticated runner for the shared conformance suite.
 *
 * The decorator over `in-process`. This is what makes auth **verified** rather than merely written:
 * `createAuthenticatedTransport` is a `Transport`, so it goes through the whole shared suite like every
 * other transport, and the `handshake`/`auth` capabilities become real assertions instead of the
 * skips they were.
 *
 * ## The session wiring
 *
 * `make` builds **one context per session** and hands the same object to the inner transport and to the
 * decorator, because the session id does not exist until the handshake and the inner transport has
 * already captured its context by then. See the comment in `make` — and note that a runner which skips
 * this fails the suite's session-identity case, which is the intended pressure.
 *
 * ## Why `in-process` underneath
 *
 * The point of the decorator is that the auth mechanism is independent of the channel it protects.
 * Running it over the cheapest transport keeps the auth cases fast and — more importantly — means the
 * handshake, the token check and the scope gate are all exercised **before `ws` exists**. `ws` is
 * gated on this work item ("an unauthenticated remote RPC surface is a remote code execution hole"),
 * and a gate that can only be satisfied by building the thing it gates is not a gate.
 *
 * When `run-ws.test.ts` lands it will claim the same two capabilities and supply the same `auth`
 * payload over a socket — a runner, not a second suite.
 *
 * ## The scopes argument
 *
 * `make` honours the suite's optional `scopes` by building a session that holds only those. The
 * suite's own session holds every scope (the command matrix runs every command through it), so the
 * FORBIDDEN case needs a *narrow* session to have anything to refuse — which is the whole reason the
 * argument exists.
 *
 * ## No assertions of its own
 *
 * Same rule as the other three runners. If a case is specific to *this* composition rather than to
 * authenticated transports in general, it belongs in `tests/transport/auth.test.ts`, which is where
 * the mechanism's own 43 cases live.
 *
 * ## Why `backpressure` is not claimed
 *
 * Because there is no wire underneath: the decorator wraps `in-process`, which coalesces nothing. The
 * batch *shape* still holds here — it is the protocol's, not the batching's — and the suite's ungated
 * case asserts it, so this runner is not exempt from it. Only the coalescing is absent, and
 * `backpressure: false` is what says so.
 */
import { PROTOCOL_VERSION } from "@bifurc/protocol";
import type { CommandContext } from "../../src/commands/registry";
import { createAuthenticatedTransport } from "../../src/transport/auth";
import { createInProcessTransport } from "../../src/transport/inProcess";
import { defineConformanceSuite } from "./protocol.conformance";

const TOKEN = "conformance-token";

defineConformanceSuite({
  name: "in-process + authenticated",

  capabilities: { handshake: true, auth: true, replay: true },

  make: ({ registry, bus, log }, scopes) => {
    /**
     * One context per session, and the **same object** goes to both halves.
     *
     * That is not incidental: the decorator learns the session id during the handshake, by which time
     * the inner transport has already captured its context, so the identity reaches handlers only by
     * mutation of this one object. Handing each half its own `{ bus }` would leave the session
     * invisible and look correct in every other respect.
     *
     * The wiring is the runner's job, not the suite's. A transport claiming `auth` is claiming it
     * *establishes* a session — which is not the same as claiming it tells the command context about
     * it, and the suite's "tells a handler which session invoked it" case is what holds the runner to
     * the second half.
     */
    const ctx: CommandContext = { bus };
    const inner = createInProcessTransport(registry, { bus, ctx, log });
    return createAuthenticatedTransport(inner, {
      token: TOKEN,
      engineVersion: "0.0.0-conformance",
      // Undefined -> the decorator's default (SHELL_SCOPES), which is what the matrix needs.
      scopesFor: scopes === undefined ? undefined : () => new Set(scopes),
      // The suite's own log, so the `seq` a reconnecting client presents is the one the engine issued.
      // The decorator advertises `events.replay` off the back of this being present.
      log,
      onSession: (identity) => {
        ctx.session = identity;
      },
    });
  },

  auth: {
    // The suite drives the handshake from this, so it can also drive the *bad* ones — and, for the
    // replay cases, the `lastSeq` of a client that has already seen something.
    hello: (overrides) => ({
      protocolVersion: PROTOCOL_VERSION,
      clientName: "cli",
      clientVersion: "0.0.0-conformance",
      token: TOKEN,
      ...overrides,
    }),
  },
});
