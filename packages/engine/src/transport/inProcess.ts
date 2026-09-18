/**
 * P4 work item 2, step 1 — the `in-process` transport.
 *
 * This is the first and the most important adapter, for two reasons `plan/05` states plainly:
 *
 *  1. It is **not optional**. It is how the conformance suite runs fast (no ports, no serialisation,
 *     no flakiness), and it is the fallback if the shell migration ever resumes and MSIX turns out to
 *     be unable to spawn a child process.
 *  2. It forces the interface to be right **before I/O complicates it** — which is exactly why the
 *     plan orders it first.
 *
 * It calls `CommandRegistry.invoke()` directly. There is no `ipcMain` anywhere in this path, which is
 * the whole point: the registry was built in P2 specifically so that a transport could dispatch
 * through it with no Electron involved (`plan/03` work item 7).
 *
 * ## What this file deliberately does NOT do
 *
 * - **No auth.** `plan/05` work item 4 is a hard gate for `ws`, and it is meaningless here: an
 *   in-process caller is already inside the engine's address space, so there is nothing to
 *   authenticate against. `hello` is therefore not part of this transport — a session handshake
 *   protects a *boundary*, and there isn't one.
 * - **No coalescing and no flow control.** The other half of work item 3, and both are properties of
 *   a *wire*: there is nothing to batch and nothing to push back on when the caller is a direct
 *   function call.
 *
 *   But **not** no batch *shape*, and the distinction matters. `event.log.entry`'s payload is
 *   `LogEntryBatchEvent` (`{ entries: [...] }`) on every transport, because that shape belongs to
 *   `@bifurc/protocol` rather than to the batching: a subscriber that had to handle `{entries}` over a
 *   socket and a bare entry here would be reading two contracts for one event name. So a single entry
 *   is delivered as a batch of one, and what this transport lacks is the *coalescing* — many entries
 *   into one frame — not the envelope. `tests/transport/eventPump.test.ts`'s sibling case in the
 *   shared suite asserts the shape on every runner, this one included, so the two cannot drift.
 * - **No serialisation.** Values pass by reference. A `ws`/`stdio` transport must not do that, so
 *   the conformance suite is required to **avoid** asserting on reference identity — see the suite's
 *   header for what that costs.
 *
 * ## `seq` comes from the `EventLog`, not from here
 *
 * It used to be a counter local to this factory, which made it per-*transport*. That is the wrong
 * authority, and the reason is replay: `plan/05`'s `hello` sketch presents a `lastSeq` with no session
 * id to disambiguate it, so the number is only meaningful if it belongs to the **engine** rather than
 * to the session that happened to observe it. A per-transport counter would give the same number two
 * different meanings across a reconnect, which is precisely the "works locally, broken remotely" class
 * of bug item 3 exists to remove.
 *
 * So `subscribe()` delegates to `EventLog.subscribe()`, which assigns the `seq` and owns retention.
 * `inProcess.ts` no longer knows what a sequence number is.
 */
import { EngineError, type CommandAction } from "@bifurc/protocol";
import { bus, ENGINE_EVENT_NAMES } from "../eventBus";
import type { CommandContext, CommandRegistry } from "../commands/registry";
import { EventLog } from "./eventLog";
import { toClientEvent } from "./eventPump";
import {
  assertBridgeIsTotal,
  BUS_NAME_BY_WIRE_NAME,
  type EngineEvent,
  type Transport,
} from "./types";

/**
 * Import-time validation of the bus-name ↔ wire-name bridge.
 *
 * Deliberately at module scope rather than inside the factory: a wrong mapping is a *programming*
 * error in the protocol/engine pair, not a per-instance condition, and it should surface the moment
 * anything imports this module rather than the first time a client subscribes.
 */
assertBridgeIsTotal(ENGINE_EVENT_NAMES as readonly string[]);

export interface InProcessTransportOptions {
  /**
   * Overrides the event bus. Defaults to the process-wide singleton.
   *
   * Injectable because the singleton is shared by every test file in a Vitest worker: without this,
   * a leaked subscription in one test would be observed by another, and the symptom (a callback
   * firing with an event nobody in *this* test emitted) reads like an engine bug.
   */
  bus?: typeof bus;
  /** Overrides the command context. Defaults to `{ bus }`, matching what the shell's adapter passes. */
  ctx?: CommandContext;
  /**
   * The engine's event log, when there is one.
   *
   * **Pass the engine's log, not a new one, for anything that can reconnect.** `createEngine()` owns a
   * single log for the process and hands the same one to every transport, which is what makes `seq`
   * mean the same thing across sessions. A transport given a log does **not** close it — the log is
   * shared, and closing it would detach the engine's retention listeners for every other session.
   *
   * Omitting it is supported and is the right call for a transport that is genuinely standalone (a
   * unit test's, the conformance suite's before it wires one in): the factory creates a private log
   * over `bus`, **owns** it, and closes it in `close()`. What that costs is the sharing — two
   * transports with private logs each count from 1 — so it is only sound where no session can outlive
   * its transport, which for `in-process` is always: the caller holds the object.
   */
  log?: EventLog;
}

/**
 * Wrap a `CommandRegistry` in the `Transport` interface.
 *
 * One instance == one session. `seq` is **not** per instance any more — see the header — but the
 * event log's retention is per *engine*, so two transports sharing one log see one order.
 */
export function createInProcessTransport(
  registry: CommandRegistry,
  opts: InProcessTransportOptions = {},
): Transport {
  const eventBus = opts.bus ?? bus;
  const ctx: CommandContext = opts.ctx ?? { bus: eventBus };

  /**
   * Own a log only when we created it. A log passed in belongs to whoever passed it — normally
   * `createEngine()`, whose log must outlive this session by definition, since a log that died with
   * the session it was replaying for could not replay anything.
   */
  const ownsLog = opts.log === undefined;
  const log = opts.log ?? new EventLog({ bus: eventBus });

  let closed = false;

  /** Every live listener, so `close()` can detach them all without the caller's cooperation. */
  const detachers = new Set<() => void>();

  function assertOpen(what: string): void {
    if (closed) {
      throw new EngineError("ENGINE_ERROR", `Cannot ${what}: this transport has been closed.`);
    }
  }

  return {
    kind: "in-process",

    async request(cmd: string, payload: unknown): Promise<unknown> {
      assertOpen("request()");

      // Checked explicitly rather than letting `invoke()` throw, so that a client-supplied string
      // is classified by the registry's *registration* state instead of by matching the text of an
      // error message. `invoke()`'s own `UNKNOWN_COMMAND` remains the backstop.
      if (!registry.isRegistered(cmd)) {
        throw new EngineError("UNKNOWN_COMMAND", `Unknown command "${cmd}".`);
      }

      try {
        // `invoke()` is synchronous by design (several handlers are, and `tests/ipc/handlers.test.ts`
        // asserts on their return value without awaiting). `await` here does not change that: it
        // adopts a returned promise and passes a plain value through unchanged. The transport is a
        // *new* surface, so it is allowed to be async everywhere — which is what every real
        // transport has to be.
        return await registry.invoke(cmd as CommandAction, payload, ctx);
      } catch (err) {
        // Already typed (the registry's own `UNKNOWN_COMMAND` / `BAD_REQUEST`, or an `EngineError`
        // a handler threw deliberately) — pass it through untouched so the most specific code wins.
        if (err instanceof EngineError) throw err;
        // Anything else is an engine-side fault. The message is preserved because it is the only
        // diagnostic a remote client will ever see; `details` is not populated with a stack, since
        // a stack trace crosses the trust boundary and is a disclosure risk on `ws` (work item 5).
        throw new EngineError("ENGINE_ERROR", err instanceof Error ? err.message : String(err));
      }
    },

    subscribe(events: string[], cb: (e: EngineEvent) => void): () => void {
      assertOpen("subscribe()");
      if (events.length === 0) return () => {};

      // Resolve every name *before* attaching any listener, and against this transport's own error
      // code. The log validates too and refuses the same names, but it throws a plain `Error`: the
      // `UNSUPPORTED` code is part of the *transport* contract, so the transport is where it has to
      // come from. The log's check stays as the last line of defence for a caller using it directly.
      for (const wire of events) {
        if (!BUS_NAME_BY_WIRE_NAME.has(wire)) {
          throw new EngineError(
            "UNSUPPORTED",
            `This transport cannot deliver "${wire}". It is either not a known event or not yet on ` +
              `the wire; see BUS_EVENTS_NOT_ON_THE_WIRE in packages/engine/src/transport/types.ts.`,
          );
        }
      }

      // Re-read `closed` per delivery, in a wrapper rather than in the caller's callback. `close()`
      // detaches, but a callback can call `close()` from *inside* an emit — and the emitter (and the
      // log) iterate a snapshot, so the remaining callbacks of a transport that has just been closed
      // would still be invoked. Without this they would be, and the caller would see events from a
      // session it had already ended.
      const detach = log.subscribe(events, (e) => {
        if (closed) return;
        // The batch shape is the protocol's, not the wire's — see the header. Coalescing is what this
        // transport does not do; the envelope is not optional. `toClientEvent` is idempotent, which
        // matters because this transport is also the inner half of the `ws` and `socket` servers: their
        // pump sits downstream of this line and must be able to re-apply the shape harmlessly.
        cb(toClientEvent(e));
      });
      let detached = false;
      const once = (): void => {
        if (detached) return;
        detached = true;
        detach();
        detachers.delete(once);
      };
      detachers.add(once);
      return once;
    },

    async close(): Promise<void> {
      if (closed) return;
      closed = true;
      // Copy first: `once()` removes itself from `detachers` while we iterate.
      for (const detach of [...detachers]) detach();
      detachers.clear();
      // Only a log we created. A shared one belongs to `createEngine()`, and closing it here would
      // silently stop event numbering for every other session in the process — a bug whose symptom
      // (a client that stops receiving events when an unrelated session disconnects) points nowhere
      // near its cause.
      if (ownsLog) log.close();
    },
  };
}
