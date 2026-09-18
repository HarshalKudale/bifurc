/**
 * P4 work item 4 — the authenticated transport decorator.
 *
 * Wraps any `Transport` and puts the `hello` handshake and per-session authorisation in front of it.
 * It is a **decorator** rather than a feature of each transport because the mechanism is identical for
 * every transport that has a boundary, and `plan/05`'s whole design premise is that the four
 * transports are the same interface. `in-process` and `stdio` are simply never wrapped: `plan/05`
 * work item 4 says `stdio` needs no authentication ("the pipe is inherited and process-scoped"), and
 * `in-process` has no boundary at all. A token there would be security theatre.
 *
 * ## Why this can exist before any socket does
 *
 * `ws` is gated on this work item ("an unauthenticated remote RPC surface is a remote code execution
 * hole"), and the gate would be unsatisfiable if the mechanism could only be tested through `ws`. A
 * decorator is testable over `in-process` today, so the auth path is built and proven before the
 * thing it protects exists — and when `ws` lands, auth is a wrapper, not a redesign.
 *
 * ## The three failure modes, and why they do not share one code
 *
 * | Situation | Code | Session |
 * |---|---|---|
 * | first request is not `hello`, or the token is wrong | `UNAUTHORIZED` | **closed** |
 * | the client's protocol **major** differs | `UNSUPPORTED` | **closed** |
 * | the command needs a scope the session lacks | `FORBIDDEN` | stays open |
 *
 * The first two are *identity* failures — the peer has not established who it is, so the session ends
 * and `plan/05` work item 5's rule applies: "Auth failure returns `UNAUTHORIZED` and closes. Never
 * fall back to unauthenticated."
 *
 * The third is not. The peer proved who it is and asked for something its scope does not cover;
 * closing would turn a scope error into a disconnect and make a mis-scoped client look like a network
 * fault. It is answered and the session continues.
 *
 * ## Version compatibility is not `!==`
 *
 * A naive equality check would refuse the companion extension, which `plan/05` calls the *expected*
 * state for weeks after every release: it is an external client on its own release cycle. So the
 * check is `checkVersionCompatibility()` from `@bifurc/protocol`, which refuses only on a **major**
 * mismatch and lets a minor mismatch connect and degrade.
 *
 * ## Replay rides on the same handshake (work item 3)
 *
 * `hello` may carry `lastSeq`, and this is where it is answered — see the section above
 * `AuthenticatedTransportOptions.log` for why the decision is made here and the *delivery* is not.
 */
import { randomUUID } from "node:crypto";
import {
  Capability,
  checkVersionCompatibility,
  COMMANDS,
  EngineError,
  HelloRequestSchema,
  HelloResponseSchema,
  PROTOCOL_VERSION,
  type CapabilityValue,
  type HelloRequest,
  type RpcError,
} from "@bifurc/protocol";
import type { ReplayOutcome } from "../eventLog";
import type { SessionIdentity } from "../../commands/registry";
// The client-facing payload shape, applied to the replay backlog this decorator synthesises itself —
// see the loop in `subscribe()` for the two-shapes-for-one-event-name defect that made this necessary.
import { toClientEvent } from "../eventPump";
import type { EngineEvent, Transport } from "../types";
import { commandIsAllowed, requiredScopeFor, SHELL_SCOPES, type Scope } from "./scopes";
import { verifyToken } from "./token";

/**
 * The bare action name of the handshake.
 *
 * Bare — no `.` — like `stdio`'s `subscribe`/`unsubscribe`, because every real command is namespaced.
 * It is **not** a registry command: `createAuthenticatedTransport` intercepts it, and a `hello`
 * command in the protocol would be shadowed rather than served. The guard below makes that a
 * load-time failure instead of a silent one, which is the same treatment `stdio.ts` gives its own
 * reserved names.
 */
export const HELLO_ACTION = "hello";

if (HELLO_ACTION in COMMANDS) {
  throw new Error(
    `@bifurc/protocol has a command named "${HELLO_ACTION}", which collides with the P4 handshake. ` +
      `The handshake is intercepted by createAuthenticatedTransport and would shadow it, so the ` +
      `command could never be invoked. Rename one of them.`,
  );
}

/**
 * The `RpcError` codes that mean *the peer has not established who it is* — and therefore the ones a
 * transport must **close** the session for.
 *
 * The two identity rows of the table above, as data. `FORBIDDEN` is deliberately absent: the peer
 * proved who it is and asked for something out of scope, which is a state to recover from rather
 * than a disconnect. `BAD_REQUEST` is absent for the same reason — a malformed `hello` is answered
 * and the session continues, so a client can fix its request and retry.
 *
 * Exported rather than re-declared per transport because it is the **consequence** of this module's
 * own decision table, and two copies of it would be two places for that table to drift. `ws.ts` and
 * `socket.ts` each apply it with a different mechanism — a WebSocket close code there, a flushed
 * reply followed by a destroyed socket here — but they must agree on *which* codes trigger it.
 */
export const IDENTITY_FAILURE_CODES: ReadonlySet<string> = new Set(["UNAUTHORIZED", "UNSUPPORTED"]);

export interface AuthenticatedTransportOptions {
  /** The token `ensureEngineToken()` produced for this run. */
  token: string;
  /**
   * The protocol version this engine speaks. Defaults to `PROTOCOL_VERSION`; injectable so a test can
   * exercise the major-mismatch path without inventing a fake protocol.
   */
  protocolVersion?: string;
  /** The engine's own version, reported to the client. Under D9 it tracks the protocol version. */
  engineVersion: string;
  /** Advertised in the `hello` response. Defaults to none — this decorator adds no capability. */
  capabilities?: readonly CapabilityValue[];
  /**
   * Scopes granted to a session that authenticates successfully.
   *
   * A function rather than a set because scopes are a property of **who** connected: the shell's own
   * session is the user and gets `SHELL_SCOPES`, while the companion extension is an external client
   * and gets `COMPANION_SCOPES`. Defaults to the shell's.
   */
  scopesFor?: (hello: HelloRequest) => ReadonlySet<Scope>;
  /** Session-id factory. Injectable so a test can assert on it rather than matching a UUID. */
  sessionId?: () => string;
  /**
   * The engine's event log, for `lastSeq` replay. Omit it and a client that presents a `lastSeq` is
   * told `resyncRequired` — honestly, because an engine with no log cannot serve one.
   *
   * ## Why the decision is made here and the delivery is not
   *
   * `hello` is the only place `lastSeq` arrives, so the *answer* (`replayedFrom` / `resyncRequired`)
   * has to be produced here. The *events* cannot be: at handshake time the engine has no idea which
   * names the client will ask for, and replaying everything would push events the client never
   * subscribed to. So the handshake records the resume point and the session's `subscribe()` calls
   * drain it — atomically with attaching the live path, so nothing emitted in between can be lost.
   *
   * The handshake therefore answers a question about the buffer **as it is at handshake time**. If the
   * window moves before the client subscribes, the promise cannot be kept, and `subscribe()` says so
   * rather than replaying a partial history — see the `CONFLICT` branch there.
   *
   * Structural type rather than `EventLog` so a test can supply a stub; `EventLog` satisfies it.
   */
  log?: { replayFrom(lastSeq: number, names?: readonly string[]): ReplayOutcome };
  /** Notified when the session is torn down by an auth failure — for the audit trail (work item 4). */
  onAuthFailure?: (error: RpcError, hello: Partial<HelloRequest>) => void;
  /**
   * Notified once the handshake succeeds, so the caller can record **who** this session is.
   *
   * The counterpart of `onAuthFailure`, and deliberately an observer rather than something the
   * decorator does itself: the decorator is the only thing that knows the identity, but *where* it
   * should be recorded is the caller's business — the command context, a log, both, or neither.
   *
   * Fires **exactly once**, and **never on a rejected handshake**. That is the property worth
   * asserting: firing on the way out of a failure would put a client that never authenticated into the
   * audit trail, which is worse than no trail at all.
   */
  onSession?: (identity: SessionIdentity) => void;
}

/**
 * Put the handshake and scope enforcement in front of `inner`.
 *
 * One instance == one session, matching every other transport: `sessionId` and `scopes` are the
 * instance's state, and the instance is closed once.
 */
export function createAuthenticatedTransport(
  inner: Transport,
  opts: AuthenticatedTransportOptions,
): Transport {
  const protocolVersion = opts.protocolVersion ?? PROTOCOL_VERSION;
  /**
   * `events.replay` is advertised exactly when there is something to replay from.
   *
   * Derived rather than left to the caller's list, because the two must not disagree: a client that
   * sees no `events.replay` capability will not send `lastSeq` at all, so an engine with a log and
   * without the capability would silently never replay anything. That failure looks like "replay is
   * broken" and would be debugged in the wrong file.
   */
  const capabilities: readonly CapabilityValue[] =
    opts.log === undefined
      ? (opts.capabilities ?? [])
      : [...new Set([...(opts.capabilities ?? []), Capability.EVENTS_REPLAY])];
  const makeSessionId = opts.sessionId ?? (() => randomUUID());

  let sessionId: string | undefined;
  let scopes: ReadonlySet<Scope> | undefined;
  let closed = false;
  /**
   * The seq this session must be caught up from, set by a `hello` that carried a `lastSeq` the engine
   * can serve.
   *
   * **Kept for the session, not consumed by the first `subscribe()`.** A client that subscribes in
   * two calls — the shape a UI has when a second panel opens — would otherwise get the backlog for
   * whichever call came first and nothing for the other, and would then be permanently stale for
   * those events while looking perfectly connected. That is the "UI that is permanently wrong in a
   * way nobody can reproduce" `plan/05` names as the high risk, so the resume point survives until
   * the session ends.
   */
  let resumeFrom: number | undefined;

  /**
   * End the session because the peer failed to establish who it is.
   *
   * `closed` is set **synchronously**, before the `await`: otherwise a second request arriving while
   * the inner transport is closing would find a session that is neither open nor marked closed, and
   * would be evaluated against `sessionId` — which is exactly the race an attacker would aim for.
   */
  async function reject(error: EngineError, hello: Partial<HelloRequest> = {}): Promise<never> {
    closed = true;
    const closing = inner.close();
    opts.onAuthFailure?.(error.toRpcError(), hello);
    await closing;
    throw error;
  }

  async function handshake(payload: unknown): Promise<unknown> {
    const parsed = HelloRequestSchema.safeParse(payload);
    if (!parsed.success) {
      // A malformed hello is not an identity failure — nothing was claimed. Answered without closing
      // so a client with a serialisation bug gets a usable error rather than a silent disconnect.
      throw new EngineError("BAD_REQUEST", `invalid ${HELLO_ACTION} payload: ${parsed.error.message}`);
    }
    const hello = parsed.data;

    // Token first, then version. Order matters: an unauthenticated peer should learn nothing beyond
    // "your credential was wrong", and a legitimate client with a stale protocol version but a valid
    // token still gets the actionable `UNSUPPORTED` rather than a misleading `UNAUTHORIZED`.
    if (!verifyToken(opts.token, hello.token)) {
      return reject(
        new EngineError(
          "UNAUTHORIZED",
          `the ${HELLO_ACTION} token is missing or does not match this engine's token`,
        ),
        hello,
      );
    }

    const compatibility = checkVersionCompatibility(hello.protocolVersion, protocolVersion);
    if (compatibility === "incompatible") {
      return reject(
        new EngineError(
          "UNSUPPORTED",
          `protocol version "${hello.protocolVersion}" is not compatible with this engine's ` +
            `"${protocolVersion}" (major version mismatch)`,
          { client: hello.protocolVersion, engine: protocolVersion },
        ),
        hello,
      );
    }

    sessionId = makeSessionId();
    scopes = opts.scopesFor?.(hello) ?? SHELL_SCOPES;

    /**
     * Identity is now established — the token and the version were both verified above — so tell the
     * caller who this is.
     *
     * After `scopes`, not before: `scopesFor` is caller-supplied and may throw, and a recorded session
     * that never actually opened is a worse artefact than a missing one. That ordering is also what
     * lets `scopes` ride along in the same payload: the authority is known by the time we report, and
     * a handler downstream needs it to answer `callerHasScope()`.
     *
     * `clientName`/`clientVersion` are required by `HelloRequestSchema`, so they are `string` here
     * rather than `string | undefined` — the `Partial<HelloRequest>` in `onAuthFailure` is the failure
     * path, where the hello may not have parsed at all.
     */
    opts.onSession?.({
      sessionId,
      clientName: hello.clientName,
      clientVersion: hello.clientVersion,
      scopes,
    });

    /**
     * The replay decision — the one thing in this handshake that is about *state* rather than identity.
     *
     * Three outcomes, and the absence of a `lastSeq` is a fourth state rather than a fourth field:
     *
     * | `lastSeq` | log | response |
     * |---|---|---|
     * | absent | any | neither field — a first connection, nothing was missed |
     * | present, servable | present | `replayedFrom: lastSeq` |
     * | present, not servable | present | `resyncRequired: true` |
     * | present | absent | `resyncRequired: true` |
     *
     * The last row is the one worth stating: a transport wrapped without a log has no history, so a
     * client asking for one is told to re-fetch rather than being left to wait for events that were
     * never buffered. Answering `replayedFrom` there would be the silent-staleness bug with extra
     * steps.
     */
    let replayedFrom: number | undefined;
    let resyncRequired: boolean | undefined;
    if (hello.lastSeq !== undefined) {
      const outcome = opts.log?.replayFrom(hello.lastSeq);
      if (outcome?.kind === "replay") {
        replayedFrom = hello.lastSeq;
        resumeFrom = hello.lastSeq;
      } else {
        resyncRequired = true;
      }
    }

    // Validated rather than hand-built: the response is a protocol type, and a shape that drifts from
    // `HelloResponseSchema` would be a wire contract nobody checked.
    return HelloResponseSchema.parse({
      protocolVersion,
      engineVersion: opts.engineVersion,
      capabilities: [...capabilities],
      sessionId,
      // Spread rather than assigned, because both are optional and `undefined` is not the same as
      // absent: `exactOptionalPropertyTypes` would reject the assignment, and a client that tested
      // `"replayedFrom" in response` would get a different answer from one testing `!== undefined`.
      ...(replayedFrom === undefined ? {} : { replayedFrom }),
      ...(resyncRequired === undefined ? {} : { resyncRequired }),
    });
  }

  async function request(cmd: string, payload: unknown): Promise<unknown> {
    // Closed: delegate. The inner transport owns the canonical "closed" error, and inventing a second
    // one here would give the same state two different codes depending on which layer noticed first.
    if (closed) return inner.request(cmd, payload);

    if (cmd === HELLO_ACTION) {
      if (sessionId !== undefined) {
        // A second `hello` is refused rather than re-evaluated. Re-running it would let a peer swap
        // its scopes mid-session, which turns the handshake from a boundary into a suggestion.
        throw new EngineError(
          "CONFLICT",
          `this session is already authenticated; "${HELLO_ACTION}" is only valid once`,
        );
      }
      return handshake(payload);
    }

    if (sessionId === undefined) {
      return reject(
        new EngineError(
          "UNAUTHORIZED",
          `the first request must be "${HELLO_ACTION}"; refusing "${cmd}" on an unauthenticated session`,
        ),
      );
    }

    const granted = scopes ?? SHELL_SCOPES;
    if (!commandIsAllowed(granted, cmd)) {
      const required = requiredScopeFor(cmd);
      if (required === undefined) {
        // Not in the scope table. `assertScopesAreTotal()` guarantees every protocol command has a
        // scope, so this is not a protocol command — delegating lets the registry answer
        // `UNKNOWN_COMMAND`, which is a better error than `FORBIDDEN` (that would imply it exists).
        //
        // Not fail-open: an unknown command executes nothing, because the registry has no handler for
        // it either. The fail-closed default in `commandIsAllowed()` still guards every caller that
        // uses it as a plain predicate.
        return inner.request(cmd, payload);
      }
      throw new EngineError(
        "FORBIDDEN",
        `this session's scopes do not include "${required}", which "${cmd}" requires`,
        { required, granted: [...granted] },
      );
    }

    return inner.request(cmd, payload);
  }

  function subscribe(events: string[], cb: (e: EngineEvent) => void): () => void {
    if (closed) return inner.subscribe(events, cb);

    if (sessionId === undefined) {
      // Throws rather than returning a no-op unsubscribe: `subscribe` is synchronous by contract, and
      // a subscription that succeeds and never fires is the failure mode `plan/05` singles out.
      throw new EngineError(
        "UNAUTHORIZED",
        `cannot subscribe before the "${HELLO_ACTION}" handshake`,
      );
    }

    /**
     * The backlog, computed **before** anything is attached, so the `CONFLICT` below is thrown with
     * no subscription left behind. A caller cannot clean up what it was never given, so throwing
     * after `inner.subscribe()` would leak a live subscription on every refusal.
     *
     * The three steps are then in the only order that is both gap-free and duplicate-free:
     *
     *  1. read the buffer (here),
     *  2. attach the live path,
     *  3. deliver the backlog.
     *
     * Nothing can be emitted between 1 and 2 — this whole function is one synchronous frame, and a bus
     * emit comes from another stack — so no event can slip between the snapshot and the subscription.
     * And an event emitted *during* 3 is delivered by the live path but is not in the snapshot, so it
     * arrives exactly once. Reversing 2 and 3 would make step 3 a duplicate-delivery bug; reversing 1
     * and 2 would open the gap.
     */
    let backlog: readonly { seq: number; event: EngineEvent["event"]; payload: unknown }[] = [];
    if (resumeFrom !== undefined) {
      const outcome = opts.log?.replayFrom(resumeFrom, events);
      if (outcome !== undefined && outcome.kind === "resync") {
        // The handshake promised this session could be caught up from `resumeFrom`, and the window has
        // since moved past it — which takes 1,000 events or 5 minutes between `hello` and here, so it
        // is rare but not impossible.
        //
        // Thrown rather than answered, and `CONFLICT` rather than `ENGINE_ERROR`: the request conflicts
        // with the session's state, it is not retryable as-is, and the client's recovery is exactly the
        // one `resyncRequired` prescribes — reconnect without a `lastSeq`, then re-fetch state with
        // ordinary commands. A throw is used because it is the only signal that cannot be missed: the
        // alternative considered was delivering `event.server.error`, and that only reaches a client
        // which happened to subscribe to that name. A client that did not would be left believing it
        // was current, which is the failure this whole work item exists to prevent.
        throw new EngineError(
          "CONFLICT",
          `this session's resume point (seq ${resumeFrom}) can no longer be served: ${outcome.reason}. ` +
            `Reconnect without "lastSeq" and re-fetch state instead of waiting for events.`,
          { lastSeq: resumeFrom, reason: outcome.reason },
        );
      }
      if (outcome !== undefined) backlog = outcome.events;
    }

    const off = inner.subscribe(events, cb);

    try {
      /**
       * `toClientEvent` because this loop **synthesises** the event rather than receiving it from the
       * inner transport, so it is the one path that would otherwise skip whatever shape the inner
       * transport applies on its live path.
       *
       * The concrete symptom, found on 2026-09-18: a reconnecting client replaying `log.entry` over
       * `in-process + auth` got the engine's raw `RequestLogEntry` while its *live* entries arrived as
       * the protocol's `LogEntryBatchEvent` — two shapes for one event name, differing by which path an
       * event took. The three transports with a wire were unaffected, because their pump normalises, so
       * this only ever showed up on the composition the conformance suite runs.
       */
      for (const e of backlog) cb(toClientEvent({ event: e.event, seq: e.seq, payload: e.payload }));
    } catch (err) {
      // The callback is arbitrary caller code. If it throws mid-backlog, the live subscription must
      // not survive the call that failed — the caller has no way to detach it, because it never got
      // the unsubscribe function.
      off();
      throw err;
    }

    return off;
  }

  async function close(): Promise<void> {
    if (closed) return;
    closed = true;
    await inner.close();
  }

  return { kind: inner.kind, request, subscribe, close };
}
