/**
 * P4 work item 6 — the conformance suite.
 *
 * One suite, run against **every** transport. `plan/05` calls this "the highest-value test asset in
 * the programme", because it is what lets the CLI (P8), the web UI (P7) and the Docker image (P9) be
 * added without re-testing the protocol each time. It lists "conformance suite written per-transport
 * instead of shared" as a risk, so the shape here matters as much as the coverage: `defineConformanceSuite()`
 * takes a factory, and a new transport is added as a **runner**, never as a new suite.
 *
 * ## Who owns what
 *
 * The suite owns the registry, the bus and the handlers; the runner owns only the transport. That
 * split is deliberate: `CommandRegistry.register()` **throws on a double registration**, so a runner
 * that pre-registered handlers would collide with any test that wants to install its own — and the
 * most interesting cases here (a handler that resolves a failure-shaped value, a handler that throws)
 * all need exactly that.
 *
 * So `make()` receives a ready registry and bus and returns a transport wired to them, and the suite
 * passes each test the handler it wants:
 *
 * ```ts
 * // packages/engine/tests/conformance/run-in-process.test.ts
 * defineConformanceSuite({
 *   name: "in-process",
 *   make: ({ registry, bus }) => createInProcessTransport(registry, { bus, ctx: { bus } }),
 * });
 * ```
 *
 * Note what a runner does **not** declare: which commands to test. The every-command matrix is built
 * from `COMMAND_FIXTURES` in `@bifurc/protocol`, and `packages/protocol/src/commands/index.test.ts`
 * asserts that table covers `COMMANDS` exactly. So the matrix covers every command by construction,
 * and adding a command to the protocol widens every transport's matrix automatically.
 *
 * ## Capabilities, and why the suite refuses to be lied to
 *
 * `in-process` has no handshake, no auth, no replay buffer and no backpressure — those are work items
 * 3 and 4, and they protect a *boundary* that `in-process` does not have. So the suite is
 * capability-gated: an unsupported capability is emitted as a **skipped** case naming the work item
 * that will implement it, which keeps the gap visible in the test output instead of absent from it.
 *
 * The gate is deliberately two-sided. A transport may not claim a capability the suite cannot yet
 * verify: `defineConformanceSuite()` **throws** at definition time if `capabilities.replay` is true,
 * because a suite that skips a case a transport claims to pass is worse than one that skips it
 * honestly — it would let a `ws` adapter ship with replay marked green and untested.
 *
 * ## What this suite deliberately does NOT assert
 *
 * **Reference identity.** `in-process` passes values by reference; `stdio` and `ws` serialise them.
 * A suite that asserted `toBe` would pass here and fail there, so every value assertion is structural
 * (`toEqual`). The same reasoning rules out asserting on class instances, `undefined`-vs-missing
 * keys, and `Date`/`Map` payloads.
 */
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  COMMAND_FIXTURES,
  EngineError,
  EVENT_NAMES,
  LogEntryBatchEventSchema,
  type CommandAction,
  type LogEntryBatchEvent,
} from "@bifurc/protocol";
// Relative rather than `@bifurc/engine/*`, deliberately. `packages/engine`'s `exports` map wildcards
// to `./dist/*.d.ts`, so the specifier would typecheck this suite against **build output** — and a
// suite that typechecks against stale `dist` is the failure mode `vitest.config.ts` already warns
// about for the runtime alias ("a source edit that was not rebuilt is silently not tested"). A
// relative path reaches the same source file the test actually runs, with no build in between.
import { CommandRegistry, type CommandContext } from "../../src/commands/registry";
import { EngineEventBus, ENGINE_EVENT_NAMES } from "../../src/eventBus";
import { EventLog, DEFAULT_MAX_EVENTS } from "../../src/transport/eventLog";
import {
  commandIsAllowed,
  HELLO_ACTION,
  requiredScopeFor,
  type Scope,
} from "../../src/transport/auth";
import type { EngineEvent, Transport } from "../../src/transport/types";
// Type-only, so nothing of the proxy module is loaded — it is here so the `log.entry` fixture below is
// compile-checked against the shape the bus really emits.
import type { RequestLogEntry } from "../../src/proxy/logEmitter";
import {
  assertBridgeIsTotal,
  BUS_EVENTS_NOT_ON_THE_WIRE,
  BUS_NAME_BY_WIRE_NAME,
  wireNameFor,
} from "../../src/transport/types";

/**
 * Reserved for the suite's own handler tests, because those need to *install* a handler and
 * `register()` throws on a duplicate.
 *
 * It is `config.get` because it takes no params, so a valid payload is the empty object — the
 * cheapest command to drive with an arbitrary handler. It is therefore excluded from the
 * every-command matrix; `COMMAND_FIXTURES` still carries its fixture, and the protocol's own schema
 * test still exercises it.
 */
export const RESERVED_COMMAND = "config.get";

/**
 * The every-command matrix: every command in the frozen protocol except the reserved one, each with
 * its own valid and near-miss-invalid payload from `COMMAND_FIXTURES`.
 *
 * Derived rather than declared. A runner cannot narrow it, and it cannot fall behind the protocol:
 * `packages/protocol/src/commands/index.test.ts` fails if `COMMAND_FIXTURES` and `COMMANDS` disagree,
 * so the only way to add a command without widening this matrix is to fail the protocol's own suite.
 */
const MATRIX: [CommandAction, { valid: unknown; invalid: unknown }][] = (
  Object.entries(COMMAND_FIXTURES) as [CommandAction, { valid: unknown; invalid: unknown }][]
).filter(([cmd]) => cmd !== RESERVED_COMMAND);

/**
 * A floor, not a count to maintain. Its job is to catch the matrix being silently emptied by a
 * refactor — the failure mode where "100% of commands covered" turns out to be a claim about an empty
 * list. It is deliberately far below the real number (93 at P4) so it never needs updating.
 */
const MINIMUM_MATRIX_COMMANDS = 50;

/**
 * The channel the delivery barrier sends its sentinel on.
 *
 * `entity.changed` because no case in this suite uses it as its *subject*: it is a low-frequency event
 * with a small structured payload, so it is cheap to emit and nothing is already listening to it.
 * If a future case does take it as a subject, the sentinel will land in that case's array and the case
 * will fail loudly — the safe direction. A sentinel that polluted a case *silently* would be worse
 * than no sentinel at all, and that is the only reason this is a named constant rather than a literal.
 */
const SENTINEL_EVENT = "event.entity.changed";

/**
 * The documented cap on one `log.entry` batch.
 *
 * A literal rather than the pump's `DEFAULT_MAX_BATCH`, deliberately. This suite asserts the
 * **contract**, and the contract's number is `plan/02`'s "up to 100 entries or 250 ms, whichever
 * first" — restated in `packages/protocol/src/events.ts`. Importing the implementation's constant
 * would make a change to it invisible here, which is the one thing this assertion must not be.
 */
const LOG_ENTRY_BATCH_CAP = 100;

/**
 * A `log.entry` bus payload: the engine's internal `RequestLogEntry`, capture bodies and all.
 *
 * Deliberately the **heavy** shape rather than the protocol's light `LogEntryEvent`. The wire shape is
 * what the cases below assert, and a fixture that already looked like the wire shape would let them
 * pass with no projection in place — which is exactly the defect worth catching, because batching the
 * internal shape puts up to ~50 MB in one frame.
 */
function logEntryPayload(n: number): RequestLogEntry {
  return {
    id: `req-${n}`,
    ts: 1_000 + n,
    method: "GET",
    url: `https://example.test/${n}`,
    host: "example.test",
    status: 200,
    via: "proxy",
    target: null,
    durationMs: 3,
    reqHeaders: {},
    reqBody: "",
    resHeaders: {},
    resBody: "a-captured-body-that-must-not-reach-the-wire",
    resStatus: 200,
  };
}

/** The protocol's wire shape for `n`, stated independently of the engine's projection. */
function wireLogEntry(n: number): Record<string, unknown> {
  return {
    id: `req-${n}`,
    timestamp: 1_000 + n,
    method: "GET",
    url: `https://example.test/${n}`,
    status: 200,
    durationMs: 3,
  };
}

/** Features a transport may or may not have. All four are false for `in-process`. */
export interface ConformanceCapabilities {
  /** A `hello` handshake must succeed before any request (work item 4). */
  handshake: boolean;
  /** Requests can be refused for want of a token or a scope (work item 4). */
  auth: boolean;
  /** Events can be replayed from a client-supplied `lastSeq`, or `resyncRequired` returned (item 3). */
  replay: boolean;
  /**
   * High-frequency events are coalesced and `server.error` is never dropped (item 3).
   *
   * Claiming it means the payload of `event.log.entry` is the protocol's `LogEntryBatchEvent` and that
   * a flood arrives in batches of at most 100 rather than one delivery per entry. The two cases that
   * verify it are gated on this flag, so a transport that forwards every entry honestly declares
   * `false` rather than passing a case that asserts nothing.
   */
  backpressure: boolean;
}

/**
 * Capabilities this suite can actually verify today. Claiming anything else is a definition-time
 * error — see the header. Flip a flag here **in the same change** that adds its cases.
 *
 * `handshake` and `auth` became verifiable with P4 work item 4: `createAuthenticatedTransport` wraps
 * any transport with the `hello` handshake, the token check and the scope gate, so a transport can
 * claim them by *being* that wrapper. `replay` joined them with work item 3, on the same principle —
 * the decorator takes a `log` and answers `lastSeq` from it.
 *
 * `backpressure` was the last to close, and it is the one that needed a **design decision** rather
 * than a mechanism. Coalescing is invisible to a client by construction: it sees the same events
 * whether the engine sent one frame or a hundred, which is the point of coalescing and also the
 * reason no case written against `Transport` could distinguish a coalescing engine from one that
 * forwards every entry. The alternative was a frame counter handed to the suite — a testing hook in a
 * production contract, which this suite's header rules out in the same words it uses for `barrier()`.
 *
 * What made it verifiable instead is `event.log.entry`'s payload: it is `LogEntryBatchEvent`
 * (`{entries}`), the shape `@bifurc/protocol` has carried since P1, so a subscriber *sees the
 * batching*. Three batches of 100 arrive as three `EngineEvent`s rather than 250, and that is an
 * assertion the interface can make. `eventPump.ts`'s header works the trade through in full.
 */
const VERIFIABLE: Readonly<Record<keyof ConformanceCapabilities, boolean>> = {
  handshake: true,
  auth: true,
  replay: true,
  backpressure: true,
};

export interface ConformanceDeps {
  registry: CommandRegistry;
  /**
   * An **isolated** bus, not the `bus` singleton: Vitest shares a module registry across files in a
   * worker, so asserting listener accounting against the singleton makes one test's leak look like
   * the next test's bug.
   */
  bus: EngineEventBus;
  /**
   * An **isolated** event log over `bus` — the single `seq` authority the transports now share.
   *
   * The suite owns it, for the same reason it owns the registry and the bus: it is infrastructure, not
   * a transport detail. A runner wires it into whatever needs it (`createInProcessTransport(…, { log })`,
   * `createStdioServer({ …, log })`) and a runner that has no use for it ignores it.
   *
   * Fresh per test, so one case's `seq` values cannot leak into the next — the failure that would
   * produce is a case asserting `[1, 2, 3]` and seeing `[4, 5, 6]`, which reads like an off-by-N bug
   * in the transport.
   */
  log: EventLog;
}

export interface ConformanceOptions {
  name: string;
  capabilities?: Partial<ConformanceCapabilities>;
  /**
   * Build a transport wired to the supplied registry and bus. Called once per test.
   *
   * `scopes`, when supplied, asks for a session holding **only** those scopes. Only the `auth` cases
   * pass it, and only a transport with authorisation needs to honour it: the harness's own session
   * must hold every scope or the command matrix could not run, and a session that can do everything
   * cannot demonstrate a refusal. A transport that ignores the second argument is fine — a
   * `make(deps)` function satisfies this signature.
   *
   * **May return a promise**, and `harness()` awaits it. That is not a convenience: a transport with a
   * real channel has to *bind* one, and binding a TCP port is asynchronous — `ws` cannot know its own
   * URL until the OS has assigned one. A synchronous `make` would force the `ws` runner to either use
   * a fixed port (collision-prone, and a second concurrent run fails) or to hand the client a
   * `Promise<string>` as its URL, which is an odd shape for a production API that exists because a
   * *caller* knows where the engine is. `await` on a plain `Transport` is a no-op, so the three
   * synchronous runners are unaffected — and that is asserted, not assumed: their counts must be
   * byte-identical before and after.
   */
  make(deps: ConformanceDeps, scopes?: readonly Scope[]): Transport | Promise<Transport>;
  /**
   * Required when the runner claims `handshake` or `auth`. `defineConformanceSuite` throws at
   * definition time otherwise, for the same reason it refuses an unverifiable capability: a runner
   * that claims auth without saying how to authenticate would silently skip the cases that matter.
   */
  auth?: ConformanceAuth;
}

/**
 * How the suite authenticates against a transport that has a handshake.
 *
 * Deliberately just the payload. The suite performs the handshake itself — `hello` is a protocol
 * message, not a transport detail — so the runner does not get to hand it a "successful" result to
 * trust, which is what makes the negative cases (bad version, bad token) expressible at all.
 */
export interface ConformanceAuth {
  /**
   * A valid `hello` request body for this transport, with `overrides` merged over it.
   *
   * The overrides parameter is what makes the replay cases expressible: `lastSeq` is a property of the
   * *client's history*, so the suite has to be able to say "this client has already seen up to seq N"
   * without the runner knowing anything about replay. A runner whose `hello` ignores the argument
   * simply cannot claim `replay` — which `defineConformanceSuite` enforces by refusing the capability
   * when no `log` is available.
   */
  hello: (overrides?: Record<string, unknown>) => Record<string, unknown>;
}

export function defineConformanceSuite(opts: ConformanceOptions): void {
  const capabilities: ConformanceCapabilities = {
    handshake: false,
    auth: false,
    replay: false,
    backpressure: false,
    ...opts.capabilities,
  };

  for (const key of Object.keys(capabilities) as (keyof ConformanceCapabilities)[]) {
    if (capabilities[key] && !VERIFIABLE[key]) {
      throw new Error(
        `Conformance suite for "${opts.name}" claims capability "${key}", but this suite cannot ` +
          `verify it yet (see VERIFIABLE in protocol.conformance.ts). Either implement the cases ` +
          `first, or declare the capability false — a green run that skips a claimed capability is ` +
          `worse than an honest skip.`,
      );
    }
  }

  // A runner that claims a handshake has to say how to perform one, or its cases would be emitted as
  // skips *despite* the claim — the exact "green run that skips a claimed capability" the gate above
  // exists to prevent, arriving by a different door.
  if ((capabilities.handshake || capabilities.auth) && opts.auth === undefined) {
    throw new Error(
      `Conformance suite for "${opts.name}" claims a handshake or auth capability but supplies no ` +
        `\`auth\` option. Pass \`{ hello: () => … }\` so the suite can drive the exchange and its ` +
        `failure paths.`,
    );
  }

  // A replay is requested by a `lastSeq` on the `hello` message, so a transport with no handshake has
  // nowhere to put one. Without this, a runner claiming `replay` alone would emit the replay cases as
  // skips *despite* the claim and the run would look green — the same hole the `auth` guard above
  // closes, arriving through a different door.
  if (capabilities.replay && !capabilities.handshake) {
    throw new Error(
      `Conformance suite for "${opts.name}" claims "replay" without "handshake". A replay is ` +
        `requested by a "lastSeq" on the hello message, so a transport with no handshake has ` +
        `nowhere to put one.`,
    );
  }

  // The matrix must not contain the command the suite reserves for its own handler tests, or the
  // harness would register it twice and `register()` throws on a duplicate. It is filtered out above;
  // asserted here so a future change to that derivation cannot quietly reintroduce it.
  if (MATRIX.some(([cmd]) => cmd === RESERVED_COMMAND)) {
    throw new Error(
      `Conformance suite for "${opts.name}": the command matrix contains "${RESERVED_COMMAND}", ` +
        `which is reserved for the suite's own handler tests.`,
    );
  }

  describe(`transport conformance — ${opts.name}`, () => {
    /** Every transport created, so a mid-test failure cannot leave listeners attached. */
    const live: Transport[] = [];
    /** Every log created, closed after the transports that subscribe to it. */
    const logs: EventLog[] = [];
    afterEach(async () => {
      for (const t of live.splice(0)) await t.close();
      // After the transports, not before: a transport closing its own subscriptions must find a live
      // log. Closing it first would make `close()` a no-op on an already-dead log and hide a leak
      // rather than report one.
      for (const log of logs.splice(0)) log.close();
    });

    /**
     * A fresh registry + isolated bus + transport.
     *
     * `handler` is the implementation of `RESERVED_COMMAND`; every command in the matrix is
     * registered with a handler that echoes the command name, so the matrix asserts *reachability*
     * without inventing per-command behaviour it would then be testing instead of the transport.
     *
     * `{ matrix: false }` registers **only** `RESERVED_COMMAND`. The `UNKNOWN_COMMAND` case needs it:
     * once the matrix registers every protocol command there is no longer any command that is *known
     * but unregistered*, and that distinction is the whole point of the case. Making the sparse
     * registry explicit is better than deleting the case, which would silently stop distinguishing
     * "no handler for this" from "not a command at all".
     */
    async function harness(
      // `ctx` is threaded through because one case is *about* the command context — the session
      // identity a handler can see. Every other handler ignores both parameters.
      handler: (_params: unknown, ctx: CommandContext) => unknown = () => ({ port: 8080 }),
      // Named `local` rather than `opts`: `opts` is the enclosing `ConformanceOptions`, and shadowing
      // it made `opts.make` below resolve to this bag instead of the transport factory.
      local: { matrix?: boolean; authenticated?: boolean; scopes?: readonly Scope[] } = {},
    ): Promise<ConformanceDeps & { transport: Transport }> {
      const registry = new CommandRegistry();
      const bus = new EngineEventBus();
      const log = new EventLog({ bus });
      logs.push(log);
      registry.register(RESERVED_COMMAND, handler);
      if (local.matrix !== false) {
        for (const [cmd] of MATRIX) {
          registry.register(cmd, () => ({ reached: cmd }));
        }
      }
      const transport = await opts.make({ registry, bus, log }, local.scopes);
      live.push(transport);

      // Authenticate unless the case is *about* being unauthenticated. Only a runner that declared an
      // `auth` option has anything to authenticate against, so this is a no-op for the others — which
      // is what keeps the pre-auth transports' cases unchanged.
      //
      // Pushed onto `live` first, so a handshake that throws mid-case still gets closed by the
      // afterEach rather than leaking a session.
      if (opts.auth !== undefined && local.authenticated !== false) {
        await transport.request(HELLO_ACTION, opts.auth.hello());
      }

      return { transport, registry, bus, log };
    }

    /**
     * The delivery barrier: emit through this, and await it before asserting on what the emit should
     * have produced.
     *
     * The `Transport` interface promises **delivery**, not **synchronous** delivery — but these cases
     * used to assert immediately after the emit, which silently required the latter. That held for
     * both transports that exist, and it is why the gap went unnoticed: `in-process` calls the
     * listener directly, and `PassThrough` in flowing mode emits `data` from inside `write()`
     * (measured, not assumed — `plan/05` records the probe). A TCP socket will never behave that way.
     * The bytes are queued and a later tick delivers them, so the `ws` runner would have failed these
     * cases for a reason that is not a defect in `ws`. This is a suite bug that was waiting for its
     * third transport to expose it.
     *
     * The barrier neither sleeps nor polls for the event under test. It emits a **sentinel** on
     * `event.entity.changed` — a channel no case here uses as its subject — and waits for *that* to
     * arrive. Delivery is FIFO within a session, so a sentinel arriving proves every event emitted
     * before it has already been delivered; in the negative cases it proves the event was *skipped*
     * rather than merely slow. A `setTimeout(0)` would not do: a socket's bytes can take several
     * ticks, and a timeout that is *usually* long enough is a flaky test.
     *
     * The sentinel consumes a `seq`, but only its own subscription sees it, so a case asserting
     * `[1, 2, 3]` is unaffected — the sentinel takes 4.
     *
     * ## And the emit is **passed in**, not made before the call — the other half of the same defect
     *
     * The sentinel alone is not enough over a socket, and this is worth stating because the first
     * version of this function looked correct and was not. `subscribe()` is **synchronous**, so for
     * `in-process` (a direct call) and `PassThrough` (flowing mode writes synchronously) the
     * subscription is live by the time the next line runs. Neither is true of a real channel:
     * `subscribe()` on a `ws` client can only *queue* a control frame, and the engine has to receive
     * it, attach its log listener, and only then can an event be recorded at all.
     *
     * Retention makes that fatal rather than merely slow. The log's retention is **lazy** — an event
     * nobody is listening for is never numbered and never buffered — so an emit that lands before the
     * subscribe frame is processed is not delayed, it is **dropped**. And the shape the cases used,
     *
     * ```ts
     * transport.subscribe(["event.server.error"], cb);
     * bus.emitTyped("server.error", "x");        // ← races the control frame
     * await deliver(transport, bus);             // ← too late to help
     * ```
     *
     * cannot be repaired by anything inside this function, because the emit has already happened by
     * the time it is called. So the emit is now a **callback**: the barrier subscribes its sentinel,
     * waits for the transport to apply everything it has been given, and only then runs the caller's
     * emit. Making the ordering structural rather than a rule to remember is deliberate — a helper you
     * must call in the right order is a helper that will eventually be called in the wrong one, and
     * the failure is a *timeout*, which points at the transport rather than at the test.
     *
     * The wait is a command round trip. Frames are ordered within a session, so a response proves
     * every control frame sent before it has already been handled — the only ordering guarantee the
     * `Transport` interface offers, and deliberately not a new method on it: a `barrier()` on
     * `Transport` would be a testing hook in a production contract, which is what `plan/05`'s
     * "`in-process` must not be a special case" rule exists to avoid. The rejection is swallowed on
     * purpose — the barrier needs the round trip to **complete**, not to succeed, so a case whose
     * reserved handler throws must not silently lose its barrier.
     */
    async function deliver(
      transport: Transport,
      bus: EngineEventBus,
      emit?: () => void,
    ): Promise<void> {
      const arrived: EngineEvent[] = [];
      transport.subscribe([SENTINEL_EVENT], (e) => arrived.push(e));
      await transport.request(RESERVED_COMMAND, {}).catch(() => undefined);
      emit?.();
      bus.emitTyped("entity.changed", { wsId: "sentinel", kind: "sentinel", action: "updated" });
      await vi.waitFor(() => expect(arrived).toHaveLength(1));
    }

    // ── capability gates ─────────────────────────────────────────────────────
    //
    // Declared here, above every `describe` that uses them, rather than next to the block they were
    // written for. `describe` callbacks run at **collection** time, so a gate declared below the first
    // `describe` that references it is a temporal-dead-zone error, not a missing case — and the case it
    // takes down is whichever one happens to come first, which points nowhere near the cause.
    //
    // Gated with `capabilities.x ? it : it.skip` rather than emitted unconditionally. A transport with
    // no boundary — `in-process`, `stdio` — genuinely has no handshake, and asserting one would be
    // inventing a requirement rather than testing a property. The skip is still *counted*, so the gap
    // is visible in the totals.

    const auth = opts.auth;
    const handshakeIt = capabilities.handshake ? it : it.skip;
    const authIt = capabilities.auth ? it : it.skip;
    const replayIt = capabilities.replay ? it : it.skip;
    const backpressureIt = capabilities.backpressure ? it : it.skip;
    /**
     * The inverse of `handshakeIt`, and the reason it exists rather than a plain `it`: the *absence* of
     * a session is a property only a transport with no handshake can demonstrate. Run it everywhere and
     * the three auth-claiming runners fail it for being correct.
     */
    const noHandshakeIt = capabilities.handshake ? it.skip : it;

    // ── the event-name bridge ────────────────────────────────────────────────
    //
    // Transport-independent, but asserted here because every transport depends on it: a subscription
    // that silently never fires is the failure mode `plan/05` singles out, and its cause is this
    // mapping rather than any adapter.

    describe("the bus-name ↔ wire-name bridge", () => {
      it("maps every protocol event to a bus event", () => {
        for (const wire of EVENT_NAMES) {
          expect(BUS_NAME_BY_WIRE_NAME.get(wire), `no bus name for ${wire}`).toBeDefined();
        }
      });

      it("follows the `event.` prefix rule, so the table is derived rather than hand-maintained", () => {
        for (const wire of EVENT_NAMES) {
          expect(BUS_NAME_BY_WIRE_NAME.get(wire)).toBe(wire.replace(/^event\./, ""));
        }
      });

      it("has no wire name for every bus event still listed as unmapped", () => {
        for (const busName of BUS_EVENTS_NOT_ON_THE_WIRE) {
          expect(wireNameFor(busName)).toBeUndefined();
        }
      });

      it("does not list an event as unmapped once the protocol names it", () => {
        // The list is the engine's promise that an event is *deliberately* not deliverable. Leaving a
        // name in it after the protocol gains a wire name for it is the dangerous direction: the event
        // becomes deliverable and `BUS_EVENTS_NOT_ON_THE_WIRE` would still claim it is not.
        //
        // The branch that enforces this inside `assertBridgeIsTotal` reads the module-level list, so
        // it cannot be triggered from out here — this asserts the property the list must hold instead
        // of pretending to exercise the branch.
        for (const busName of BUS_EVENTS_NOT_ON_THE_WIRE) {
          expect(BUS_NAME_BY_WIRE_NAME.has(`event.${busName}`), `${busName} now has a wire name`).toBe(false);
        }
      });

      it("rejects a bridge that is missing a bus event (the guard has teeth)", () => {
        // Negative control. Without it, a change that made `assertBridgeIsTotal` a no-op would keep
        // every other case in this describe green.
        expect(() => assertBridgeIsTotal(["sync.status"])).toThrow(/bridge is incomplete/i);
      });

      it("tolerates a bus event the protocol does not care about", () => {
        // The check requires every *wire* name to have a bus name; it does not require the reverse.
        // The bus may carry an event no client can subscribe to, and that is what
        // BUS_EVENTS_NOT_ON_THE_WIRE documents — so an extra key must not be an error.
        expect(() => assertBridgeIsTotal([...ENGINE_EVENT_NAMES, "some.internal.event"])).not.toThrow();
      });

      it("accepts the engine's real event list", () => {
        expect(() => assertBridgeIsTotal(ENGINE_EVENT_NAMES)).not.toThrow();
      });
    });

    // ── request() ────────────────────────────────────────────────────────────

    describe("request()", () => {
      it("resolves with the handler's value", async () => {
        const { transport } = await harness();
        await expect(transport.request(RESERVED_COMMAND, {})).resolves.toEqual({ port: 8080 });
      });

      it("resolves with a failure-shaped value UNINSPECTED — the envelope rule", async () => {
        // The single most important assertion in the suite. Most legacy handlers report their
        // failures as RESOLVED values and the renderer branches on them, so a transport that turned
        // `{ok:false}` into a rejection would break `window.api` for every one of them.
        const { transport } = await harness(() => ({ ok: false, error: "nope" }));
        await expect(transport.request(RESERVED_COMMAND, {})).resolves.toEqual({ ok: false, error: "nope" });
      });

      it("resolves with undefined when the handler returns nothing", async () => {
        const { transport } = await harness(() => undefined);
        await expect(transport.request(RESERVED_COMMAND, {})).resolves.toBeUndefined();
      });

      it("awaits an async handler", async () => {
        const { transport } = await harness(async () => ({ port: 1234 }));
        await expect(transport.request(RESERVED_COMMAND, {})).resolves.toEqual({ port: 1234 });
      });

      it("rejects UNKNOWN_COMMAND for a command that is known but unregistered", async () => {
        // `server.status` is a real protocol command that this harness deliberately leaves
        // unregistered — so this distinguishes "the registry has no handler for it" from "the name is
        // not in the protocol at all", which is the next case.
        const { transport } = await harness(undefined, { matrix: false });
        await expect(transport.request("server.status", {})).rejects.toMatchObject({
          code: "UNKNOWN_COMMAND",
        });
      });

      it("rejects UNKNOWN_COMMAND for a string that is not a protocol command at all", async () => {
        const { transport } = await harness();
        await expect(transport.request("not.a.real.command", {})).rejects.toMatchObject({
          code: "UNKNOWN_COMMAND",
        });
      });

      it("rejects BAD_REQUEST when the payload fails the frozen schema", async () => {
        const { transport } = await harness();
        // The reserved command takes no params, so a non-object payload must fail its schema.
        await expect(transport.request(RESERVED_COMMAND, { unexpected: true })).rejects.toMatchObject({
          code: "BAD_REQUEST",
        });
      });

      it("rejects ENGINE_ERROR when the handler throws", async () => {
        const { transport } = await harness(() => {
          throw new Error("boom");
        });
        await expect(transport.request(RESERVED_COMMAND, {})).rejects.toMatchObject({
          code: "ENGINE_ERROR",
          message: "boom",
        });
      });

      it("rejects ENGINE_ERROR when the handler rejects", async () => {
        const { transport } = await harness(async () => {
          throw new Error("async boom");
        });
        await expect(transport.request(RESERVED_COMMAND, {})).rejects.toMatchObject({ code: "ENGINE_ERROR" });
      });

      it("rejects ENGINE_ERROR when a handler throws a non-Error", async () => {
        const { transport } = await harness(() => {
          throw "just a string";
        });
        await expect(transport.request(RESERVED_COMMAND, {})).rejects.toMatchObject({
          code: "ENGINE_ERROR",
          message: "just a string",
        });
      });

      it("preserves the code of an EngineError a handler threw deliberately", async () => {
        const { transport } = await harness(() => {
          throw new EngineError("CONFLICT", "already exists");
        });
        await expect(transport.request(RESERVED_COMMAND, {})).rejects.toMatchObject({
          code: "CONFLICT",
          message: "already exists",
        });
      });

      it("never rejects synchronously", async () => {
        // A caller writing `transport.request(...).catch(...)` must not have to guard the call
        // itself. Every transport is async, so this is a contract rather than a detail.
        const { transport } = await harness();
        const p = transport.request("not.a.real.command", {});
        expect(p).toBeInstanceOf(Promise);
        await expect(p).rejects.toBeInstanceOf(EngineError);
      });
    });

    // ── subscribe() ──────────────────────────────────────────────────────────

    describe("subscribe()", () => {
      it("delivers an event with its wire name and payload", async () => {
        const { transport, bus } = await harness();
        const seen: EngineEvent[] = [];
        transport.subscribe(["event.server.error"], (e) => seen.push(e));

        await deliver(transport, bus, () => bus.emitTyped("server.error", "upstream died"));

        expect(seen).toHaveLength(1);
        expect(seen[0]!.event).toBe("event.server.error");
        expect(seen[0]!.payload).toBe("upstream died");
      });

      it("starts seq at 1 and increases it monotonically", async () => {
        const { transport, bus } = await harness();
        const seen: EngineEvent[] = [];
        transport.subscribe(["event.server.error"], (e) => seen.push(e));

        await deliver(transport, bus, () => {
          bus.emitTyped("server.error", "one");
          bus.emitTyped("server.error", "two");
          bus.emitTyped("server.error", "three");
        });

        expect(seen.map((e) => e.seq)).toEqual([1, 2, 3]);
      });

      it("shares one seq counter across subscriptions, so a session has a single order", async () => {
        const { transport, bus } = await harness();
        const a: EngineEvent[] = [];
        const b: EngineEvent[] = [];
        transport.subscribe(["event.server.error"], (e) => a.push(e));
        transport.subscribe(["event.log.chunk"], (e) => b.push(e));

        await deliver(transport, bus, () => {
          bus.emitTyped("server.error", "x");
          bus.emitTyped("log.chunk", { logId: "l1", chunk: "hi", done: false });
          bus.emitTyped("server.error", "y");
        });

        expect(a.map((e) => e.seq)).toEqual([1, 3]);
        expect(b.map((e) => e.seq)).toEqual([2]);
      });

      it("does not deliver an event the caller did not subscribe to", async () => {
        const { transport, bus } = await harness();
        const seen: EngineEvent[] = [];
        transport.subscribe(["event.server.error"], (e) => seen.push(e));

        // The barrier is what makes this a real assertion rather than a race: without it, "nothing
        // arrived" is indistinguishable from "nothing has arrived *yet*". The unsubscribed emit goes
        // through it too, or the case would be *vacuous* rather than wrong — the event would be
        // dropped for want of a subscription and `toHaveLength(0)` would pass for the wrong reason.
        await deliver(transport, bus, () =>
          bus.emitTyped("log.chunk", { logId: "l1", chunk: "hi", done: false }),
        );

        expect(seen).toHaveLength(0);
      });

      it("throws UNSUPPORTED for an engine event with no wire name yet", async () => {
        // A silently accepted subscription is indistinguishable from "nothing has happened yet".
        // `process.statusChange` is the one bus event still without a protocol name.
        const { transport } = await harness();
        expect(() => transport.subscribe(["event.process.statusChange"], () => {})).toThrow(/cannot deliver/i);
      });

      it("delivers every event the protocol names, including the two added in P4", async () => {
        // `process.output` and `settings.changed` existed on the bus since P2 but had no wire name, so
        // no client could subscribe to them. This is the assertion that they are reachable now — one
        // event per direction of the change: a structured payload, and an empty one.
        const { transport, bus } = await harness();
        const output: EngineEvent[] = [];
        const settings: EngineEvent[] = [];
        transport.subscribe(["event.process.output"], (e) => output.push(e));
        transport.subscribe(["event.settings.changed"], (e) => settings.push(e));

        await deliver(transport, bus, () => {
          bus.emitTyped("process.output", { appId: "a1", stream: "stdout", data: "hello", ts: 1 });
          bus.emitTyped("settings.changed", {});
        });

        expect(output).toHaveLength(1);
        expect(output[0]!.event).toBe("event.process.output");
        expect(output[0]!.payload).toEqual({ appId: "a1", stream: "stdout", data: "hello", ts: 1 });
        expect(settings).toHaveLength(1);
        expect(settings[0]!.event).toBe("event.settings.changed");
      });

      it("delivers `log.entry` as a protocol batch, on every transport", async () => {
        /**
         * The shape is the protocol's, not the batching's — `eventPump.ts`'s header works that decision
         * through in full. Two consequences are asserted, and neither is decorative:
         *
         *  - **A batch of one is still a batch.** `in-process` has no wire and therefore no coalescing,
         *    but a client must not have to read `{entries}` over a socket and a bare entry in-process.
         *    This case is **ungated** for exactly that reason: it holds whether or not the transport
         *    claims `backpressure`, because the envelope belongs to `@bifurc/protocol` rather than to
         *    the batching. It is also the case that keeps `in-process` honest — without it, the
         *    transports could drift into two payload shapes for one event name.
         *  - **The capture bodies do not cross.** The bus payload is the engine's `RequestLogEntry`,
         *    carrying the captured request and response bodies (base64, the response capped at 512 KB).
         *    The wire entry is six fields. A batch of 100 unprojected entries is up to ~50 MB in one
         *    frame, so the projection is what makes coalescing affordable rather than harmful.
         */
        const { transport, bus } = await harness();
        const seen: EngineEvent[] = [];
        transport.subscribe(["event.log.entry"], (e) => seen.push(e));

        await deliver(transport, bus, () => bus.emitTyped("log.entry", logEntryPayload(1)));

        // One emit, one delivery — the barrier's sentinel is not this subscription's event.
        expect(seen).toHaveLength(1);
        expect(seen[0]!.event).toBe("event.log.entry");

        const parsed = LogEntryBatchEventSchema.safeParse(seen[0]!.payload);
        expect(parsed.success, parsed.success ? "" : JSON.stringify(parsed.error.issues)).toBe(true);
        expect(parsed.success && parsed.data.entries).toEqual([wireLogEntry(1)]);
        expect(
          JSON.stringify(seen[0]!.payload),
          "the captured body must not be on the wire",
        ).not.toContain("must-not-reach-the-wire");
      });

      it("throws UNSUPPORTED for a name that is not an event at all", async () => {
        const { transport } = await harness();
        expect(() => transport.subscribe(["event.not.real"], () => {})).toThrow(/cannot deliver/i);
      });

      it("attaches nothing when one name in the batch is bad", async () => {
        // All-or-nothing: attaching as we go would leave the first subscription live after the
        // throw, so a caller that catches and retries would silently accumulate duplicates.
        const { transport, bus } = await harness();
        expect(() => transport.subscribe(["event.server.error", "event.not.real"], () => {})).toThrow();
        expect(bus.listenerCount("server.error")).toBe(0);
      });

      it("treats an empty subscription list as a no-op", async () => {
        const { transport, bus } = await harness();
        const off = transport.subscribe([], () => {});
        expect(typeof off).toBe("function");
        expect(bus.eventNames()).toHaveLength(0);
      });

      it("stops delivery after unsubscribe, and the unsubscribe is idempotent", async () => {
        const { transport, bus } = await harness();
        const seen: EngineEvent[] = [];
        const off = transport.subscribe(["event.server.error"], (e) => seen.push(e));

        await deliver(transport, bus, () => bus.emitTyped("server.error", "before"));

        off();
        off(); // idempotent
        await deliver(transport, bus, () => bus.emitTyped("server.error", "after"));

        expect(seen.map((e) => e.payload)).toEqual(["before"]);
      });

      it("unsubscribing one subscription leaves the others live", async () => {
        const { transport, bus } = await harness();
        const a: EngineEvent[] = [];
        const b: EngineEvent[] = [];
        const offA = transport.subscribe(["event.server.error"], (e) => a.push(e));
        transport.subscribe(["event.server.error"], (e) => b.push(e));

        offA();
        await deliver(transport, bus, () => bus.emitTyped("server.error", "x"));

        expect(a).toHaveLength(0);
        expect(b).toHaveLength(1);
      });

      it("stops delivering once unsubscribed, rather than merely looking detached", async () => {
        // The bus is long-lived; a transport that leaked *subscriptions* would accumulate them across
        // sessions until Node's 11-listener warning fires, and the leak would then be blamed on
        // whatever subscribed last.
        //
        // The assertion is **behavioural** rather than `bus.listenerCount(...)`, and that changed with
        // P4 work item 3. The listener accounting moved into the `EventLog`, which attaches one
        // retention listener per event name on first subscribe and then **holds it for the engine's
        // life** — deliberately, because a buffer that only filled while someone was listening would
        // replay an incomplete history after a blip. So `listenerCount` is no longer a proxy for "does
        // this session leak?"; it is 1 by design, and asserting 0 here would now be asserting that
        // replay is broken.
        //
        // What still matters at this layer is that a detached subscription stops firing, and the
        // accounting is asserted where it now lives — `tests/transport/eventLog.test.ts` covers
        // `subscriberCount` and "one bus listener per name however many subscribers there are".
        const { transport, bus } = await harness();
        const seen: EngineEvent[] = [];
        const off = transport.subscribe(["event.server.error", "event.log.chunk"], (e) => seen.push(e));

        await deliver(transport, bus, () => bus.emitTyped("server.error", "while subscribed"));
        expect(seen).toHaveLength(1);

        off();
        await deliver(transport, bus, () => bus.emitTyped("server.error", "after unsubscribe"));
        expect(seen, "an unsubscribed callback received an event").toHaveLength(1);
      });
    });

    // ── close() ──────────────────────────────────────────────────────────────

    describe("close()", () => {
      it("detaches every subscription", async () => {
        const { transport, bus } = await harness();
        const seen: EngineEvent[] = [];
        transport.subscribe(["event.server.error"], (e) => seen.push(e));

        await transport.close();
        bus.emitTyped("server.error", "after close");

        expect(seen).toHaveLength(0);
        // No `bus.listenerCount(...)` assertion, for the reason given in the unsubscribe case above:
        // the retention listener belongs to the shared log and outlives the session by design. What
        // "detached" means for a closed session is that its callbacks stop firing — and there is
        // nothing further it *could* subscribe with, because `subscribe()` now throws (the case below).
        //
        // No `deliver()` barrier either: this asserts an absence, and an absence cannot be made more
        // true by waiting. The suite's header notes the same about the other negative case.
      });

      it("is idempotent", async () => {
        const { transport } = await harness();
        await transport.close();
        await expect(transport.close()).resolves.toBeUndefined();
      });

      it("makes request() reject rather than silently doing nothing", async () => {
        const { transport } = await harness();
        await transport.close();
        await expect(transport.request(RESERVED_COMMAND, {})).rejects.toBeInstanceOf(EngineError);
      });

      it("makes subscribe() throw rather than silently doing nothing", async () => {
        const { transport } = await harness();
        await transport.close();
        expect(() => transport.subscribe(["event.server.error"], () => {})).toThrow(/closed/i);
      });
    });

    // ── the every-command matrix ─────────────────────────────────────────────

    describe("every command in the frozen protocol", () => {
      it("covers a substantial set of commands", () => {
        // Guards against the matrix silently becoming a no-op, which is how a "100% of commands
        // covered" claim turns out to be a claim about an empty list. A floor, not a count.
        expect(MATRIX.length).toBeGreaterThan(MINIMUM_MATRIX_COMMANDS);
      });

      it("covers commands from more than one namespace", () => {
        // A matrix that quietly narrowed to one namespace would still pass the floor above.
        const namespaces = new Set(MATRIX.map(([cmd]) => cmd.split(".")[0]));
        expect(namespaces.size).toBeGreaterThan(5);
      });

      it("reaches every command with its own valid payload", async () => {
        const { transport } = await harness();
        for (const [cmd, fixture] of MATRIX) {
          // `resolves`, not `rejects`: the assertion is that the *transport* did not fail. A handler
          // may legitimately resolve a failure-shaped value, which the envelope rule keeps as data,
          // so this checks reachability rather than success.
          await expect(transport.request(cmd, fixture.valid), `${cmd} with a valid payload`).resolves.toEqual({
            reached: cmd,
          });
        }
      });

      it("rejects BAD_REQUEST for every command's near-miss invalid payload", async () => {
        const { transport } = await harness();
        for (const [cmd, fixture] of MATRIX) {
          // A near-miss, not `null`. `null` fails every schema for reasons that have nothing to do
          // with the transport, so a transport that let *everything* through would still pass a
          // null-based check. `COMMAND_FIXTURES`' invalid payloads are otherwise well-formed — a bad
          // enum member, a missing required field, a negative offset — so a BAD_REQUEST here is
          // evidence that the frozen schema actually ran on the far side of the transport.
          await expect(transport.request(cmd, fixture.invalid), `${cmd} with an invalid payload`).rejects.toMatchObject({
            code: "BAD_REQUEST",
          });
        }
      });
    });

    // ── replay (work item 3) ─────────────────────────────────────────────────
    //
    // The most likely source of "works locally, broken remotely" bugs, per `plan/05`'s risks table,
    // and the reason the whole event log exists. What is being tested is not the log — that has its
    // own 30 cases — but that a `lastSeq` survives the round trip through a transport and comes back
    // as either the events the client missed or an explicit refusal to pretend.

    describe("replay from lastSeq", () => {
      /**
       * Attach retention for `event.server.error` and immediately leave.
       *
       * This is not a test artefact — it is the blip the feature exists for. Retention is **lazy**, so
       * an event nobody ever subscribed to is never numbered and there is nothing to replay; the first
       * subscription is what starts the buffer, and it deliberately outlives that subscription. Written
       * as one line because `log.subscribe(...)()` is exactly "someone was listening, then the
       * connection dropped".
       */
      function openTheWindow(log: EventLog): void {
        log.subscribe(["event.server.error"], () => {})();
      }

      replayIt("delivers the events a reconnecting client missed, and says from where", async () => {
        const { transport, bus, log } = await harness(undefined, { authenticated: false });
        openTheWindow(log);
        bus.emitTyped("server.error", "missed-1");
        bus.emitTyped("server.error", "missed-2");

        // `lastSeq: 0` is the honest "I have seen nothing" — it is a valid value, not a sentinel.
        const response = await transport.request(HELLO_ACTION, auth!.hello({ lastSeq: 0 }));
        expect(response).toMatchObject({ replayedFrom: 0 });

        const seen: EngineEvent[] = [];
        transport.subscribe(["event.server.error"], (e) => seen.push(e));
        await deliver(transport, bus);

        expect(seen.map((e) => e.payload)).toEqual(["missed-1", "missed-2"]);
        // The engine's own seqs, not a client-side recount. A transport that numbered events on arrival
        // would produce the same `[1, 2]` here and diverge the moment the client subscribed to a
        // subset — which is the bug this asserts against.
        expect(seen.map((e) => e.seq)).toEqual([1, 2]);
      });

      replayIt("replays only what the client has not seen", async () => {
        const { transport, bus, log } = await harness(undefined, { authenticated: false });
        openTheWindow(log);
        for (const p of ["one", "two", "three"]) bus.emitTyped("server.error", p);

        await transport.request(HELLO_ACTION, auth!.hello({ lastSeq: 2 }));

        const seen: EngineEvent[] = [];
        transport.subscribe(["event.server.error"], (e) => seen.push(e));
        await deliver(transport, bus);

        // Not `["one","two","three"]`: replaying what the client already has would duplicate every
        // event on every reconnect, and for a streamed `log.chunk` that is corrupted output rather
        // than a cosmetic repeat.
        expect(seen.map((e) => e.payload)).toEqual(["three"]);
      });

      replayIt("does not replay to a client that sent no lastSeq", async () => {
        // The fourth state, and the one an over-eager implementation collapses into the others: a
        // **first** connection has nothing to catch up on. Replaying here would send a fresh client
        // the whole buffer, which for `log.entry` is a burst of history it never asked for.
        const { transport, bus, log } = await harness(undefined, { authenticated: false });
        openTheWindow(log);
        bus.emitTyped("server.error", "before-anyone-connected");

        const response = await transport.request(HELLO_ACTION, auth!.hello());
        expect(response).not.toHaveProperty("replayedFrom");
        expect(response).not.toHaveProperty("resyncRequired");

        const seen: EngineEvent[] = [];
        transport.subscribe(["event.server.error"], (e) => seen.push(e));
        await deliver(transport, bus, () => bus.emitTyped("server.error", "live"));

        expect(seen.map((e) => e.payload)).toEqual(["live"]);
      });

      replayIt("refuses with resyncRequired once the buffer has moved past the client", async () => {
        // `plan/05`: "If `lastSeq` is outside the buffer, the engine sets `resyncRequired` and the
        // client re-fetches authoritative state via normal commands rather than replaying events."
        // The alternative — replaying the part that survives — leaves the client believing it is
        // current, which is the permanently-wrong-UI failure the plan calls out.
        const { transport, bus, log } = await harness(undefined, { authenticated: false });
        openTheWindow(log);

        // Force an eviction through the real bound rather than reaching into the log's internals.
        // One past `maxEvents` is enough to drop seq 1, which is the event the client would need.
        for (let i = 0; i <= DEFAULT_MAX_EVENTS; i += 1) bus.emitTyped("server.error", `e${i}`);
        expect(log.oldestSeq, "the precondition: seq 1 must have been evicted").toBe(2);

        const response = await transport.request(HELLO_ACTION, auth!.hello({ lastSeq: 0 }));
        expect(response).toMatchObject({ resyncRequired: true });
        expect(response).not.toHaveProperty("replayedFrom");

        // And the session is still usable — a resync is a state to recover from, not a disconnect.
        const seen: EngineEvent[] = [];
        transport.subscribe(["event.server.error"], (e) => seen.push(e));
        await deliver(transport, bus, () => bus.emitTyped("server.error", "live"));
        expect(seen.map((e) => e.payload)).toEqual(["live"]);
      });

      replayIt("refuses a lastSeq ahead of anything the engine ever sent", async () => {
        // A client that claims to have seen seq 99 has either corrupted its own state or is talking to
        // the wrong engine. Replaying nothing would leave it believing it is up to date.
        const { transport, bus, log } = await harness(undefined, { authenticated: false });
        openTheWindow(log);
        bus.emitTyped("server.error", "one");

        const response = await transport.request(HELLO_ACTION, auth!.hello({ lastSeq: 99 }));
        expect(response).toMatchObject({ resyncRequired: true });
      });

      replayIt("advertises events.replay, so a client knows lastSeq is meaningful", async () => {
        // Without the capability a client has no reason to send `lastSeq`, and an engine that can
        // replay would silently never do it — a failure that looks like "replay is broken" and gets
        // debugged in the wrong file.
        const { transport } = await harness(undefined, { authenticated: false });
        const response = (await transport.request(HELLO_ACTION, auth!.hello())) as {
          capabilities?: string[];
        };
        expect(response.capabilities).toContain("events.replay");
      });

      replayIt("serves a second subscription from the same resume point", async () => {
        // A client that subscribes in two calls — the shape a UI has when a second panel opens — must
        // not get the backlog for whichever call came first and nothing for the other. Consuming the
        // resume point on the first `subscribe()` would leave the second set permanently stale while
        // looking perfectly connected.
        const { transport, bus, log } = await harness(undefined, { authenticated: false });
        log.subscribe(["event.server.error", "event.log.chunk"], () => {})();
        bus.emitTyped("server.error", "missed-error");
        bus.emitTyped("log.chunk", { logId: "l1", chunk: "missed-chunk", done: false });

        await transport.request(HELLO_ACTION, auth!.hello({ lastSeq: 0 }));

        const errors: EngineEvent[] = [];
        const chunks: EngineEvent[] = [];
        transport.subscribe(["event.server.error"], (e) => errors.push(e));
        transport.subscribe(["event.log.chunk"], (e) => chunks.push(e));
        await deliver(transport, bus);

        expect(errors.map((e) => e.payload)).toEqual(["missed-error"]);
        expect(chunks.map((e) => e.payload)).toEqual([
          { logId: "l1", chunk: "missed-chunk", done: false },
        ]);
      });

      replayIt("does not duplicate an event that arrives while the backlog is delivered", async () => {
        // The ordering hazard: the backlog is a snapshot taken before the live path is attached, so an
        // event emitted *during* delivery must arrive exactly once — from the live path, not from both.
        // Asserted with an emit from inside the callback, which is the only way to hit the window
        // without a socket.
        const { transport, bus, log } = await harness(undefined, { authenticated: false });
        openTheWindow(log);
        bus.emitTyped("server.error", "in-the-backlog");
        await transport.request(HELLO_ACTION, auth!.hello({ lastSeq: 0 }));

        const seen: EngineEvent[] = [];
        let reentered = false;
        transport.subscribe(["event.server.error"], (e) => {
          seen.push(e);
          if (!reentered) {
            reentered = true;
            bus.emitTyped("server.error", "during-delivery");
          }
        });
        await deliver(transport, bus);

        expect(seen.map((e) => e.payload)).toEqual(["in-the-backlog", "during-delivery"]);
      });
    });

    // ── capabilities this transport does not have (yet) ──────────────────────
    //
    // Emitted as skips so the gap is visible in the run output and counted in the suite's totals,
    // rather than being invisible. Each names the work item that will implement it.

    // ── handshake and auth (work item 4) ─────────────────────────────────────
    //
    // The suite drives the exchange itself from `auth.hello()`, so it can express the negative cases.
    // A runner that supplied a "do the handshake" callback would be handing the suite a result to
    // trust, and the bad-token case could not be written at all. The gates themselves are declared at
    // the top of this describe — see the note there.

    describe("the hello handshake", () => {
      handshakeIt("accepts a valid hello and opens a session", async () => {
        const { transport } = await harness(undefined, { authenticated: false });
        const response = await transport.request(HELLO_ACTION, auth!.hello());
        // Structural, not identity: a serialising transport returns a copy. The point is that a
        // session came back and the transport is usable, which the cases below then rely on.
        expect(response).toBeTruthy();
        expect(typeof response).toBe("object");
      });

      handshakeIt("tells a handler which session invoked it", async () => {
        // The handler *is* the probe: it is registered as `RESERVED_COMMAND`, so whatever it returns is
        // what the command context held at dispatch time — no new channel, and nothing the transport
        // could satisfy by special-casing this case.
        const { transport } = await harness((_params, ctx) => ctx.session ?? null);
        const seen = await transport.request(RESERVED_COMMAND, {});

        expect(seen).toMatchObject({
          // `expect.any(String)` rather than `toBeDefined()`: a placeholder would satisfy the latter
          // while identifying nobody, and the whole point of the field is attribution.
          sessionId: expect.any(String),
          clientName: expect.any(String),
          clientVersion: expect.any(String),
        });
        expect((seen as { sessionId: string }).sessionId).not.toBe("");
      });

      noHandshakeIt("gives a handler no session, because nothing established one", async () => {
        const { transport } = await harness((_params, ctx) => ctx.session ?? null);
        // Not a gap to be "fixed" by defaulting: an invented session id would put a fiction in the
        // audit log. `in-process` and `stdio` have no identity to report — the caller of an in-process
        // transport *is* the engine, and a stdio pipe is inherited and process-scoped.
        await expect(transport.request(RESERVED_COMMAND, {})).resolves.toBeNull();
      });

      handshakeIt("refuses a major protocol version mismatch", async () => {
        const { transport } = await harness(undefined, { authenticated: false });
        const hello = auth!.hello();
        const major = String(hello.protocolVersion ?? "1.0.0").split(".");
        major[0] = String(Number(major[0]) + 1);

        await expect(
          transport.request(HELLO_ACTION, { ...hello, protocolVersion: major.join(".") }),
        ).rejects.toMatchObject({ code: "UNSUPPORTED" });
      });

      handshakeIt("refuses a missing or invalid token", async () => {
        for (const token of [undefined, "definitely-not-the-token"]) {
          const { transport } = await harness(undefined, { authenticated: false });
          await expect(
            transport.request(HELLO_ACTION, { ...auth!.hello(), token }),
            `token ${String(token)} must be refused`,
          ).rejects.toMatchObject({ code: "UNAUTHORIZED" });
        }
      });
    });

    describe("authorisation", () => {
      authIt("refuses a command outside the session's scopes", async () => {
        // A **narrow** session, not the harness's own. The harness session holds every scope, because
        // the command matrix has to be able to run every command through it — and a session that can
        // do everything cannot demonstrate a refusal. The command is picked from the real table
        // rather than hard-coded, so a re-scoping cannot make this case vacuous.
        const narrow: readonly Scope[] = ["read"];
        const { transport } = await harness(undefined, { scopes: narrow });

        const forbidden = MATRIX.map(([cmd]) => cmd).filter(
          (cmd) => !commandIsAllowed(new Set(narrow), cmd),
        );
        expect(
          forbidden.length,
          "a read-only session can reach every command — the case would assert nothing",
        ).toBeGreaterThan(0);

        await expect(transport.request(forbidden[0]!, {})).rejects.toMatchObject({ code: "FORBIDDEN" });
      });

      authIt("refuses every namespace before the handshake", async () => {
        // `plan/05` work item 5's rule is universal — "Never fall back to unauthenticated" — so it is
        // sampled across namespaces rather than spot-checked. A per-command exemption would be the
        // exact hole this case exists to close.
        //
        // A **fresh transport per command**: the first refusal closes the session, so reusing one
        // would make every iteration after the first assert the closed-session path instead, and the
        // loop would silently stop testing what it claims to.
        const sample = [...new Set(MATRIX.map(([cmd]) => cmd.split(".")[0]))]
          .map((ns) => MATRIX.find(([cmd]) => cmd.startsWith(`${ns}.`))![0])
          .filter((cmd) => cmd !== undefined);
        expect(sample.length).toBeGreaterThan(5);

        for (const cmd of sample) {
          const { transport } = await harness(undefined, { authenticated: false });
          await expect(
            transport.request(cmd, {}),
            `${cmd} must be refused before the handshake`,
          ).rejects.toMatchObject({ code: "UNAUTHORIZED" });
        }
      });
    });

    // ── backpressure ─────────────────────────────────────────────────────────
    //
    // Item 3's coalescing half, and the last capability to become verifiable. The difficulty was
    // never the mechanism — it was finding an assertion a *forwarding* transport would fail, because
    // coalescing is invisible to a client by construction. The batch payload is what makes it visible:
    // see `VERIFIABLE`'s comment and `eventPump.ts`'s header.

    describe("backpressure", () => {
      backpressureIt("coalesces `log.entry` under flood instead of one delivery per entry", async () => {
        /**
         * The assertion has to *distinguish* coalescing from forwarding, and that is the whole of this
         * case's value. "Every entry arrived" passes either way; "fewer deliveries than entries" is
         * true only if batching happened. A transport that declared `backpressure: true` while
         * forwarding each entry fails here, which is what makes the claim worth anything.
         *
         * The flood is one synchronous burst, so the delivery count is deterministic rather than
         * timing-dependent: the pump fills and flushes at the cap twice and holds the remainder, and
         * the barrier's sentinel — an immediate event — flushes that remainder before it is delivered.
         * Nothing here waits on the 250 ms window, which is asserted directly in
         * `tests/transport/eventPump.test.ts` where the scheduler can be driven by hand.
         */
        const { transport, bus } = await harness();
        const deliveries: EngineEvent[] = [];
        transport.subscribe(["event.log.entry"], (e) => deliveries.push(e));

        const flood = 250;
        await deliver(transport, bus, () => {
          for (let n = 1; n <= flood; n += 1) bus.emitTyped("log.entry", logEntryPayload(n));
        });

        const sizes = deliveries.map((e) => {
          const parsed = LogEntryBatchEventSchema.safeParse(e.payload);
          expect(parsed.success, "every delivery must be a batch payload").toBe(true);
          return parsed.success ? parsed.data.entries.length : -1;
        });

        expect(
          deliveries.length,
          "coalescing is the claim: strictly fewer deliveries than entries",
        ).toBeLessThan(flood);
        expect(deliveries.length, `${flood} entries at a cap of ${LOG_ENTRY_BATCH_CAP}`).toBe(
          Math.ceil(flood / LOG_ENTRY_BATCH_CAP),
        );
        for (const size of sizes) {
          expect(size).toBeGreaterThan(0);
          expect(size).toBeLessThanOrEqual(LOG_ENTRY_BATCH_CAP);
        }
        expect(
          sizes.reduce((a, b) => a + b, 0),
          "and no entry is lost to the batching",
        ).toBe(flood);

        // Order across the batch boundaries. A coalescer that dropped, duplicated or reordered entries
        // could still satisfy every count above, and this is what rules that out.
        const ids = deliveries.flatMap((e) =>
          (e.payload as LogEntryBatchEvent).entries.map((entry) => entry.id),
        );
        expect(ids).toEqual(Array.from({ length: flood }, (_, i) => `req-${i + 1}`));
      });

      backpressureIt("never drops `server.error` under a `log.entry` flood", async () => {
        /**
         * "Never dropped" is a consequence of "never queued": `server.error` takes the immediate path,
         * so there is no priority comparison that could be got wrong. Asserted rather than
         * asserted-about, because this is the half of the acceptance criterion that is a *guarantee*
         * rather than an optimisation.
         *
         * Interleaved rather than sequential on purpose. A sequential emit would prove only that an
         * error survives a *past* flood; the failure this guards against is an error lost while entries
         * are still accumulating, so the error has to be emitted into the middle of one.
         *
         * The payload is the bare string the bus types `server.error` as. Note that
         * `@bifurc/protocol`'s `ServerErrorEventSchema` says `{error: string}` — a pre-existing
         * mismatch, recorded in `plan/05` rather than fixed here, because the larger half of it is that
         * **nothing in the engine emits this event at all**: every `bus.emitTyped("server.error", …)`
         * in the repository is a test. Making the engine produce it is a wiring change in the
         * proxy/startup path, and it is what would decide which of the two shapes is right.
         */
        const { transport, bus } = await harness();
        const errors: EngineEvent[] = [];
        const entries: EngineEvent[] = [];
        transport.subscribe(["event.server.error"], (e) => errors.push(e));
        transport.subscribe(["event.log.entry"], (e) => entries.push(e));

        const flood = 250;
        const everyNth = 10;
        await deliver(transport, bus, () => {
          for (let n = 1; n <= flood; n += 1) {
            bus.emitTyped("log.entry", logEntryPayload(n));
            if (n % everyNth === 0) bus.emitTyped("server.error", `failure ${n}`);
          }
        });

        const expected = flood / everyNth;
        expect(errors, "every `server.error` arrives").toHaveLength(expected);
        expect(errors.map((e) => e.payload)).toEqual(
          Array.from({ length: expected }, (_, i) => `failure ${(i + 1) * everyNth}`),
        );
        expect(
          entries.reduce((n, e) => n + (e.payload as LogEntryBatchEvent).entries.length, 0),
          "and the flood is not sacrificed to protect them",
        ).toBe(flood);
      });
    });
  });
}
