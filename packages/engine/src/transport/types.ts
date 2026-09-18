/**
 * P4 work item 2 — the `Transport` abstraction.
 *
 * The shell needs **embedded**, **spawned** and **remote** to be the same interface, so this file
 * defines one interface and lets each transport be a thin adapter over it. Nothing here is
 * Electron-aware and nothing here opens a socket: `inProcess.ts` is the first adapter, and
 * `stdio` / `ws` / `socket` follow in later work items.
 *
 * ## The two decisions worth reading before changing anything
 *
 * ### 1. `request()` resolves with the handler's value **uninspected**
 *
 * This is not a style choice — it is the load-bearing envelope rule validated by the P0 spike and
 * restated at the top of `@bifurc/protocol`'s `envelope.ts`:
 *
 *   > A legacy handler that *resolves* — with ANY shape, including `{ok:false, error:"..."}` — is an
 *   > envelope-level SUCCESS (`ok:true`); the value rides through untouched in `data`. Only a
 *   > THROWN/REJECTED handler becomes an envelope-level failure.
 *
 * Most existing handlers report their failures as **resolved** values, and the renderer already
 * branches on those. So a transport that inspected a resolved value and "helpfully" turned
 * `{ok:false}` into a rejection would break `window.api` compatibility for every one of them — the
 * exact opposite of what P4 is for. `request()` therefore has a single failure path: the registry
 * *threw*. A resolved `{ok:false}` comes back to the caller as data, verbatim.
 *
 * ### 2. Event names are bridged in exactly one place, and the bridge is checked at import time
 *
 * The engine's `EventBus` (`../eventBus.ts`) keys events by their **bus** name — `"sync.status"`,
 * `"log.entry"` — while `@bifurc/protocol`'s `EVENT_NAMES` uses the **wire** name —
 * `"event.sync.status"`, `"event.log.entry"`. Today the two differ by exactly the `event.` prefix,
 * and it is tempting to write `"event." + busKey` inline. That would be a silent-corruption bug the
 * day an event does not follow the rule: the subscription would succeed and the event would simply
 * never arrive, which is the "works locally, broken remotely" class of defect `plan/05` calls out.
 *
 * So the mapping is an explicit table derived from the protocol's own list, and `assertBridgeIsTotal()`
 * throws **at import time** if the two sides ever stop lining up. A wrong mapping should fail loudly
 * at startup, not quietly at 3am.
 */
import {
  EngineError,
  isKnownCommand,
  makeError,
  EVENT_NAMES,
  type ErrorCodeValue,
  type EventName,
  type RpcError,
} from "@bifurc/protocol";

/** The four transports `plan/05` work item 2 requires. Only `in-process` exists so far. */
export type TransportKind = "in-process" | "stdio" | "ws" | "socket";

/**
 * An event as it crosses the transport, matching `@bifurc/protocol`'s `EventEnvelope`.
 *
 * `seq` is assigned by the **`EventLog`** (`./eventLog.ts`), which is engine-scoped: one counter per
 * engine, shared by every session, incremented once per retained event whether or not anyone is
 * subscribed.
 *
 * This replaced an earlier "per-session, gap-free" note, which was wrong in a way that only shows up
 * on reconnect. `plan/05`'s own `hello` sketch presents `lastSeq` with no session id to disambiguate
 * it, so a per-session counter would give the same number two different meanings across a reconnect —
 * the client's `lastSeq: 48210` would refer to a session that no longer exists. An engine-scoped
 * counter is the only kind that survives a reconnect, and `eventLog.ts`'s header works through the
 * trade in full. The short version: a session subscribed to only *some* events now sees gaps, because
 * events it did not subscribe to still consume a number.
 */
export interface EngineEvent<T = unknown> {
  /** The **wire** name, e.g. `"event.log.entry"`. Never the bus name. */
  event: EventName;
  seq: number;
  payload: T;
}

export interface Transport {
  readonly kind: TransportKind;
  /**
   * Invoke a command and resolve with its result.
   *
   * Rejects with an `EngineError` (`@bifurc/protocol`) carrying one of:
   *  - `UNKNOWN_COMMAND` — nothing is registered under that name;
   *  - `BAD_REQUEST` — the payload failed the command's frozen Zod schema;
   *  - `ENGINE_ERROR` — the handler itself threw or rejected;
   *  - whatever code the handler threw, if it threw an `EngineError` of its own.
   *
   * A handler that *resolved* is never a rejection, whatever it resolved with — see the header.
   */
  request(cmd: string, payload: unknown): Promise<unknown>;
  /**
   * Subscribe to one or more **wire** event names. Returns an unsubscribe function that is safe to
   * call more than once, and that detaches only this subscription.
   *
   * Throws `UNSUPPORTED` for a name this transport cannot deliver, rather than accepting the
   * subscription and never firing — a silent no-op here is indistinguishable from "nothing has
   * happened yet" and would be debugged as a broken engine.
   */
  subscribe(events: string[], cb: (e: EngineEvent) => void): () => void;
  /** Idempotent. After this, `request()` and `subscribe()` reject rather than silently doing nothing. */
  close(): Promise<void>;
}

// ── the bus-name ↔ wire-name bridge ──────────────────────────────────────────

/**
 * Wire name → bus name. Derived from the protocol's `EVENT_NAMES` by stripping the `event.` prefix,
 * then asserted against the bus's real key set by `assertBridgeIsTotal()` below.
 */
export const BUS_NAME_BY_WIRE_NAME: ReadonlyMap<string, string> = new Map(
  EVENT_NAMES.map((wire) => [wire, wire.replace(/^event\./, "")]),
);

export const WIRE_NAME_BY_BUS_NAME: ReadonlyMap<string, EventName> = new Map(
  EVENT_NAMES.map((wire) => [wire.replace(/^event\./, ""), wire]),
);

/** The wire name for a bus event, or `undefined` if the event is not on the wire yet. */
export function wireNameFor(busName: string): EventName | undefined {
  return WIRE_NAME_BY_BUS_NAME.get(busName);
}

/**
 * Engine bus events that deliberately have **no** wire name yet. Listed explicitly rather than left
 * to be discovered, because "this event is not delivered" is otherwise indistinguishable from
 * "this event is delivered but nothing subscribes to it".
 *
 * `plan/05` work item 3 has now added `event.process.output` and `event.settings.changed` to the
 * protocol, so this list is down to one entry.
 *
 * `process.statusChange` is the remainder and it is a **real decision still to make**, not an
 * oversight: it overlaps `sync.entityStatus` (the plan's risks table lists "Event ordering races
 * between `entity.changed` and `sync.entityStatus`"), and the bus payload is
 * `{appId, status, [key: string]: unknown}` — an open-ended shape that is not a wire contract yet.
 * Putting a `[key: string]: unknown` on the wire would freeze it as one, so it stays engine-internal
 * until something needs it.
 */
export const BUS_EVENTS_NOT_ON_THE_WIRE: readonly string[] = ["process.statusChange"] as const;

/**
 * Fails at import time if the bridge is not total in both directions.
 *
 * Two things are checked, and both are real failure modes rather than paranoia:
 *
 *  1. **Every wire name has a bus name.** If the protocol gains `event.foo.bar` and the bus has no
 *     `foo.bar`, a subscription would be accepted and never fire.
 *  2. **No two wire names collapse to the same bus name.** `event.a` and `event.a` cannot, but a
 *     future `event.sync.status` plus a literal bus key `sync.status` would — and then one of the
 *     two subscriptions would receive the other's events.
 *
 * It deliberately does **not** require the bus's key set to be covered: `BUS_EVENTS_NOT_ON_THE_WIRE`
 * is the explicit list of the remainder, and it is checked too, so that adding a bus event without
 * deciding its wire name is a test failure rather than an omission.
 */
export function assertBridgeIsTotal(busKeys: readonly string[]): void {
  const busKeySet = new Set(busKeys);

  for (const [wire, bus] of BUS_NAME_BY_WIRE_NAME) {
    if (!busKeySet.has(bus)) {
      throw new Error(
        `Transport event bridge is incomplete: protocol event "${wire}" maps to bus event ` +
          `"${bus}", which the engine bus does not emit.`,
      );
    }
  }

  if (WIRE_NAME_BY_BUS_NAME.size !== BUS_NAME_BY_WIRE_NAME.size) {
    throw new Error(
      "Transport event bridge is not one-to-one: two protocol events collapse onto the same bus event.",
    );
  }

  for (const bus of BUS_EVENTS_NOT_ON_THE_WIRE) {
    if (WIRE_NAME_BY_BUS_NAME.has(bus)) {
      throw new Error(
        `Bus event "${bus}" is listed in BUS_EVENTS_NOT_ON_THE_WIRE but the protocol now has a wire ` +
          `name for it. Remove it from that list — and give it a conformance case.`,
      );
    }
  }
}

// ── transport control ────────────────────────────────────────────────────────

/**
 * Action names that mean *transport control* rather than *invoke a command*.
 *
 * Bare names, with no `.`, because every real command is namespaced (`config.get`, `env.setActive`,
 * `blob.put`). The check below makes that convention load-bearing instead of merely conventional.
 *
 * It lives here rather than in `stdio.ts`, where it was born, because `ws` uses the identical two
 * names — and a constant with a guard that two transports must agree on is exactly the kind of thing
 * that drifts when it has two homes. Adding a third reserved name in one file and not the other would
 * produce a transport that silently shadows a command, which is the failure the guard exists to
 * prevent. (`hello` is the third reserved name and it is owned by `auth/authenticated.ts`, because it
 * is an envelope-level message rather than transport control.)
 */
export const RESERVED_ACTIONS = ["subscribe", "unsubscribe"] as const;

/**
 * Refuse to load if the protocol ever grows a command with a reserved name.
 *
 * At **module scope**, so it is a startup failure rather than a per-call one: shadowing a real command
 * is a protocol/engine conflict that no amount of runtime checking can make safe.
 */
for (const reserved of RESERVED_ACTIONS) {
  if (isKnownCommand(reserved)) {
    throw new Error(
      `Transport control action "${reserved}" is now a real @bifurc/protocol command, so every ` +
        `serialising transport would shadow it. Rename the reserved action, or give transport ` +
        `control its own frame kind, before this can ship.`,
    );
  }
}

// ── the wire-error projection ────────────────────────────────────────────────

/**
 * Classify a thrown value into the protocol's `RpcError`.
 *
 * This is the **other half of the envelope rule** documented at the top of this file, and it lives
 * here rather than in each transport because it must be identical in all of them. `in-process` does
 * not need it — nothing crosses a boundary, so it rethrows the `EngineError` itself and the caller
 * reads `.code` directly — but `stdio`, `socket` and `ws` all do, and three copies of this function
 * is the same "written per-transport instead of shared" drift `plan/05` lists as a risk for the
 * conformance suite.
 *
 * Three cases, in order:
 *
 *  1. **An `EngineError` keeps its code.** The registry throws `UNKNOWN_COMMAND` / `BAD_REQUEST`,
 *     and a handler may deliberately throw `CONFLICT` / `NOT_FOUND` / `UPSTREAM_FAILED`. Flattening
 *     those to `ENGINE_ERROR` would throw away the only thing that lets a client react differently
 *     per failure — which is the entire point of P1 work item 5.
 *  2. **Anything else is `ENGINE_ERROR`**, carrying the message.
 *  3. **A non-`Error` throw** (`throw "boom"`, `throw {code:1}`) is stringified. A handler doing this
 *     is a bug, but a transport that crashed on it would be a worse one.
 *
 * `details` is deliberately **not** populated with a stack: a stack trace names internal file paths
 * and is a disclosure risk the moment the peer is remote (`plan/05` work item 5). The message is
 * kept because it is the only diagnostic a remote client will ever have.
 */
export function toRpcError(err: unknown): RpcError {
  if (err instanceof EngineError) return err.toRpcError();
  return makeError("ENGINE_ERROR", err instanceof Error ? err.message : String(err));
}

/** Best-effort text for a remote error we cannot type. */
function describeRemoteError(raw: unknown): string {
  if (raw && typeof raw === "object" && "message" in raw) {
    return String((raw as { message: unknown }).message);
  }
  return "the engine refused a transport control frame";
}

/**
 * The **inverse** of `toRpcError`: rebuild an `EngineError` from a `RpcError` that arrived over the
 * wire.
 *
 * It lives here for the same reason `toRpcError` does — it must be identical in every serialising
 * transport. It used to be private to `stdio.ts`, and `ws` needed the same function verbatim; a
 * second copy is exactly the "written per-transport instead of shared" drift `plan/05` lists as a
 * risk for the conformance suite, one layer down.
 *
 * The code is **preserved, not flattened**. `toRpcError` goes to some trouble to keep a handler's own
 * `CONFLICT` / `NOT_FOUND` / `UPSTREAM_FAILED`, and throwing that away on the way back would undo it —
 * the client would see `ENGINE_ERROR` for everything and could not branch per failure, which is the
 * entire point of P1 work item 5. The conformance suite asserts this end to end ("preserves the code
 * of an EngineError a handler threw deliberately").
 *
 * Anything unrecognisable becomes `ENGINE_ERROR` carrying the best text available. A malformed error
 * must not crash a client that is otherwise healthy.
 */
export function remoteErrorToEngineError(raw: unknown): EngineError {
  if (raw && typeof raw === "object") {
    const e = raw as { code?: unknown; message?: unknown; details?: unknown };
    if (typeof e.code === "string" && typeof e.message === "string") {
      return new EngineError(e.code as ErrorCodeValue, e.message, e.details);
    }
  }
  return new EngineError("ENGINE_ERROR", describeRemoteError(raw));
}
