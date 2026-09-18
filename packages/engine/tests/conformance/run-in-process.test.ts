/**
 * P4 work item 6 — the `in-process` runner for the shared conformance suite.
 *
 * `plan/05`'s "How to start" step 1: define `Transport`, implement `in-process`, and stand up the
 * conformance suite skeleton — the fastest possible feedback loop, and the one that forces the
 * interface to be right before I/O complicates it.
 *
 * This file contains **no assertions of its own**. That is the point: the suite is transport-agnostic
 * and lives in `protocol.conformance.ts`, so `run-stdio.test.ts` and `run-ws.test.ts` will be
 * runners of the same length as this one. `plan/05` lists "conformance suite written per-transport
 * instead of shared" as a risk, and a runner that started adding its own cases would be exactly that
 * risk materialising.
 *
 * ## Capabilities are not declared here
 *
 * They are deliberately omitted rather than spelled out as `false`: `defineConformanceSuite` defaults
 * every capability to false and **throws** if a runner claims one the suite cannot verify. Declaring
 * nothing is therefore the honest statement for `in-process`, which has no handshake, no auth, no
 * replay buffer and **no coalescing** — those protect a boundary this transport does not have, and
 * batching is a property of a *wire*.
 *
 * The last one is worth spelling out, because it is not the same claim as "no batch shape". This
 * transport **does** deliver `event.log.entry` as the protocol's `LogEntryBatchEvent`, one entry to a
 * batch, because the envelope belongs to `@bifurc/protocol` rather than to the batching — a client must
 * not have to read `{entries}` over a socket and a bare entry here. What it lacks is the *coalescing*
 * (many entries into one frame), and that is what `backpressure: false` means. The shared suite has an
 * **ungated** case asserting the shape on every runner, so the two cannot drift apart.
 *
 * ## Which commands are tested
 *
 * Not declared here either. The every-command matrix is derived from `COMMAND_FIXTURES` in
 * `@bifurc/protocol`, which the protocol's own schema test asserts covers `COMMANDS` exactly — so
 * this runner gets all 93 commands, and would get a 94th automatically.
 */
import { createInProcessTransport } from "../../src/transport/inProcess";
import { defineConformanceSuite } from "./protocol.conformance";

defineConformanceSuite({
  name: "in-process",

  make: ({ registry, bus, log }) =>
    // `bus` is passed both as the event source and inside `ctx`, mirroring what the shell's
    // `ipcMain` adapter passes — so a handler that reaches for `ctx.bus` sees the same bus the
    // transport is delivering from, not the process-wide singleton.
    //
    // `log` is the suite's own, so the `seq` every case observes comes from the same authority a
    // reconnecting client would replay from. A transport that made its own log would still pass every
    // case here — they assert monotonicity, not identity — which is exactly why the log is wired in
    // rather than left to the transport's default.
    createInProcessTransport(registry, { bus, ctx: { bus }, log }),
});
