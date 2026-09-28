/**
 * P4 work item 2, step 2 — the `stdio` transport.
 *
 * `plan/05` orders this second because it is "simpler than WebSocket (no ports, no auth)" and because
 * it "validates the framing". Both halves of that are real: there is no port to collide on and no
 * token to rotate, so if something is broken here it is the framing or the correlation — and the
 * frame codec is already proven by `framing.ts`'s own checks.
 *
 * ## Why this file contains two factories
 *
 * A pipe has two ends, and they are not the same object:
 *
 * | Factory | Runs in | `request()` means |
 * |---|---|---|
 * | `createStdioTransport` | the **client** — the CLI (P8), the shell | "ask the peer to invoke this" |
 * | `createStdioServer` | the **engine** — driven over its own stdin/stdout | "receive a request, invoke it, reply" |
 *
 * They live in one file on purpose: the wire contract below is the thing they must agree on, and two
 * files is how a request shape ends up changing on one side only.
 *
 * **The server is deliberately not a `Transport`.** `Transport.request()` means "invoke this command";
 * on the server side that would mean "invoke it on behalf of a peer", which is not what the server
 * does — it *receives* requests and *emits* events. Forcing the interface on it would produce a
 * `request()` method nobody calls, and a reader would reasonably assume something calls it.
 *
 * ## The wire contract
 *
 * Frames are length-prefixed JSON (`framing.ts`). A frame is one of three shapes, discriminated by
 * which key is present — `event` for an event, `id` for a response, `id` **and** `action` for a
 * request. Both directions carry all three, which is what makes the pipe full-duplex rather than
 * request/response:
 *
 * ```ts
 * client → engine   { id, action, payload }                  // RpcRequest
 * engine → client   { id, ok: true,  data }                  // RpcResponse — success
 * engine → client   { id, ok: false, error: RpcError }        // RpcResponse — failure
 * engine → client   { event, seq, payload }                   // EventEnvelope
 * ```
 *
 * These are the **frozen** `@bifurc/protocol` shapes, not a stdio-specific invention. That matters
 * more than it looks: it means the `ws` transport (step 3) carries the same envelopes over a
 * different channel, and a client can move between them without a translation layer.
 *
 * ## The envelope rule, seen from a serialising transport
 *
 * `types.ts` states it: a handler that **resolves** — with any shape, including `{ok:false}` — is an
 * envelope-level *success*. `stdio` is the first transport where this can be got wrong in a new way,
 * because now there are two `ok` flags in play and they are not the same flag:
 *
 *  - `{ id, ok: true, data: { ok: false, error: "..." } }` — the **outer** `ok` is the transport's:
 *    the handler ran. The **inner** `ok:false` is the handler's own report and rides through
 *    untouched. The client resolves with it, verbatim.
 *  - `{ id, ok: false, error: {...} }` — the handler **threw**. The client rejects with an
 *    `EngineError` built from `error`.
 *
 * Conflating them breaks `window.api` for every legacy handler that reports failure as a resolved
 * value, which is most of them. `run-stdio.test.ts` asserts both directions.
 *
 * ## What this deliberately does NOT do
 *
 * - **No `hello`, no auth, no scope.** `plan/05` work item 4's table says `stdio` needs none: "the
 *   pipe is inherited and process-scoped". There is no boundary to authenticate across, and a token
 *   here would be security theatre. `subscribe`/`unsubscribe` are reserved action names rather than
 *   commands, and no session exists to attach a scope to.
 * - **No replay buffer, no coalescing, no flow control.** Work item 3. Events are written as they
 *   arrive and Node's stream buffers them; there is no `resyncRequired` and no `lastSeq`, so a client
 *   that reconnects gets nothing it missed. `plan/05`'s risks table rates "reconnect semantics get
 *   half-implemented" as **high** — this is not half of it, it is none of it.
 * - **No timeout on a request.** A command that never returns leaves `request()` pending until the
 *   stream dies, at which point it rejects. Per-command timeouts are work item 5's "Timeouts" row.
 */
import { randomUUID } from "node:crypto";
import { spawn, type ChildProcess } from "node:child_process";
import type { Readable, Writable } from "node:stream";
import {
  EngineError,
  EVENT_NAMES,
  makeError,
  SubscribeRequestSchema,
  UnsubscribeRequestSchema,
  type CommandAction,
  type EventName,
  type RpcResponse,
} from "@bifurc/protocol";
import { bus as defaultBus, ENGINE_EVENT_NAMES, type EngineEvents } from "../eventBus";
import type { CommandContext, CommandRegistry } from "../commands/registry";
import { createFrameDecoder, encodeFrame, frameFailure, type FrameDecoder } from "./framing";
import { createEventPump } from "./eventPump";
import {
  assertBridgeIsTotal,
  BUS_NAME_BY_WIRE_NAME,
  remoteErrorToEngineError,
  toRpcError,
  type EngineEvent,
  type Transport,
} from "./types";

/**
 * Import-time validation, same as `inProcess.ts`: the server half maps wire names to bus names, so a
 * wrong bridge is a programming error in the protocol/engine pair and should fail the moment anything
 * imports this module.
 */
assertBridgeIsTotal(ENGINE_EVENT_NAMES as readonly string[]);

/**
 * Action names that mean *transport control* rather than *invoke a command*.
 *
 * Defined in `./types.ts` and re-exported here: `ws` uses the same two names, and the import-time
 * guard against the protocol ever growing a command with one of them belongs somewhere both
 * transports load. Re-exported rather than moved outright so this module's public surface — which
 * `transport/index.ts` and `tests/transport/stdio.test.ts` both use — is unchanged.
 */
export { RESERVED_ACTIONS } from "./types";

/** Is this a wire event name this transport can route? */
function isWireEventName(name: string): name is EventName {
  return (EVENT_NAMES as readonly string[]).includes(name);
}

/** Narrow a decoded frame. The decoder guarantees a non-null, non-array object. */
function asRecord(frame: unknown): Record<string, unknown> {
  return frame as Record<string, unknown>;
}

/** An `EngineError` for a frame-level failure, so callers only ever see one error type. */
// `frameFailure` lives in `framing.ts` — it is a statement about that codec's error type, and
// `socket` needs the identical projection. See the note there.

// ── the client ───────────────────────────────────────────────────────────────

export interface StdioTransportOptions {
  /** Where frames are written — the engine's stdin. */
  input: Writable;
  /** Where frames are read — the engine's stdout. */
  output: Readable;
  /**
   * The child process, when this transport spawned it.
   *
   * Its presence is what decides whether `close()` **ends the pipe**: ending a pipe we created is how
   * the engine learns to exit, but ending one we merely attached to would break whatever else is
   * using it. So: end what you own.
   */
  child?: ChildProcess;
  /**
   * Called once when the session dies — a frame error, a stream error, an unexpected end, or a
   * protocol violation. Not called for a caller-initiated `close()`.
   */
  onFatal?(error: Error): void;
}

/**
 * A `Transport` over a pipe.
 *
 * One instance == one session == one `seq` counter, exactly as `in-process`.
 */
export function createStdioTransport(opts: StdioTransportOptions): Transport {
  /** Requests awaiting a response, keyed by the `id` we generated. */
  const pending = new Map<string, { resolve: (v: unknown) => void; reject: (e: Error) => void }>();
  /**
   * `id`s of control frames (`subscribe` / `unsubscribe`) we sent. Their responses carry no value, so
   * they are tracked only to be *recognised* — an unrecognised response id is a different condition.
   */
  const control = new Set<string>();
  /**
   * Live subscriptions, keyed by wire name.
   *
   * The value is a set of **tokens**, one per `subscribe()` call, rather than a set of callbacks.
   * With a set of callbacks, subscribing the same function to the same event twice would collapse
   * into one entry and the second `unsubscribe()` would silently cancel the first — which is not what
   * `in-process` does, and the whole point of the shared conformance suite is that the transports
   * cannot disagree about things like this.
   */
  const listeners = new Map<string, Set<{ cb: (e: EngineEvent) => void }>>();

  let seq = 0;
  let closed = false;
  let fatal: Error | undefined;

  /**
   * The single teardown path.
   *
   * `pending` is snapshotted and cleared **before** anything is rejected: a rejection handler is
   * arbitrary caller code and could re-enter here, and it must find nothing to act on. This is also
   * the guard that makes a hung CLI impossible — a request in flight when the engine dies rejects
   * instead of leaving the caller awaiting a promise nobody will ever settle, which is the classic
   * stdio-transport bug and is invisible until the engine crashes.
   */
  function teardown(reason: Error, notify: boolean): void {
    if (closed) return;
    closed = true;
    fatal = reason;

    const waiting = [...pending.values()];
    pending.clear();
    control.clear();
    for (const entry of waiting) entry.reject(reason);

    for (const set of listeners.values()) set.clear();
    listeners.clear();

    opts.output.off("data", onData);
    opts.output.off("end", onEnd);
    opts.output.off("close", onEnd);
    opts.output.off("error", onStreamError);

    if (notify) opts.onFatal?.(reason);
  }

  /**
   * Write one frame. Returns false if the write itself failed, in which case the session is already
   * being torn down.
   *
   * `encodeFrame` throws for a payload JSON cannot represent (a cycle, a `BigInt`) and for one over
   * `MAX_FRAME_BYTES`. That is a **caller** error, not a stream error, so it is reported by throwing
   * and the caller decides — it must not kill a healthy session.
   */
  function write(message: unknown): void {
    opts.input.write(encodeFrame(message));
  }

  function sendControl(action: string, payload: unknown): void {
    const id = randomUUID();
    control.add(id);
    write({ id, action, payload });
  }

  function deliver(frame: Record<string, unknown>): void {
    const wire = frame.event;
    if (typeof wire !== "string") return;
    const set = listeners.get(wire);
    if (!set || set.size === 0) return;
    seq += 1;
    const event: EngineEvent = { event: wire as EventName, seq, payload: frame.payload };
    // Iterate a copy: a callback is allowed to unsubscribe itself, which mutates `set`.
    for (const token of [...set]) token.cb(event);
  }

  function handleResponse(frame: Record<string, unknown>): void {
    const id = frame.id;
    if (typeof id !== "string") {
      // No `event` and no `id` — nothing to correlate and nothing to deliver. The peer is broken in a
      // way we cannot report to anyone, so the session ends rather than being left half-usable.
      teardown(
        new EngineError("ENGINE_ERROR", "Received a frame that is neither a response nor an event."),
        true,
      );
      return;
    }

    if (control.delete(id)) {
      // A control acknowledgement. `ok:true` needs nothing further; `ok:false` means the server
      // refused a subscription it should have accepted (the client already validated the names
      // against the same frozen `EVENT_NAMES` list), so it is a protocol violation and is surfaced
      // loudly rather than swallowed — a silently-missing subscription is indistinguishable from
      // "nothing has happened yet" and would be debugged as a broken engine.
      if (frame.ok === false) {
        teardown(
          new EngineError(
            "ENGINE_ERROR",
            `The engine refused a transport control frame: ${remoteErrorToEngineError(frame.error).message}`,
          ),
          true,
        );
      }
      return;
    }

    const entry = pending.get(id);
    if (!entry) {
      // A response to an id we are not waiting on: a duplicate, or a late answer to a request whose
      // session has moved on. There is no caller to tell, and killing a working session over it would
      // be worse than ignoring it.
      return;
    }
    pending.delete(id);

    if (frame.ok === true) {
      // UNINSPECTED. See the header: an inner `{ok:false}` is the handler's own report and must reach
      // the caller as data, not as a rejection.
      entry.resolve(frame.data);
      return;
    }
    if (frame.ok === false) {
      entry.reject(remoteErrorToEngineError(frame.error));
      return;
    }
    entry.reject(
      new EngineError("ENGINE_ERROR", "Response frame has no boolean `ok` field."),
    );
  }

  function onData(chunk: Buffer): void {
    decoder.push(chunk);
  }

  function onEnd(): void {
    // `exitCode` is read here rather than in a separate `child.on("exit")` handler so that the
    // message is right whichever of the two fires first — and they race.
    const code = opts.child?.exitCode;
    const why =
      code === null || code === undefined
        ? "the engine's output stream ended"
        : `the engine exited with code ${code}`;
    teardown(new EngineError("ENGINE_ERROR", `Session closed: ${why}.`), true);
  }

  function onStreamError(err: Error): void {
    teardown(new EngineError("ENGINE_ERROR", `Output stream error: ${err.message}`), true);
  }

  const decoder: FrameDecoder = createFrameDecoder({
    onFrame: (frame) => {
      if (closed) return;
      const record = asRecord(frame);
      if (typeof record.event === "string") deliver(record);
      else handleResponse(record);
    },
    onError: (err) => teardown(frameFailure(err), true),
  });

  opts.output.on("data", onData);
  // `end` is the graceful path (the peer closed its stdout); `close` also covers a destroy without an
  // end, which is what a killed child looks like. Both land in the same idempotent teardown.
  opts.output.on("end", onEnd);
  opts.output.on("close", onEnd);
  opts.output.on("error", onStreamError);

  function assertOpen(what: string): void {
    if (closed) {
      throw fatal
        ? new EngineError("ENGINE_ERROR", `Cannot ${what}: this session ended. ${fatal.message}`)
        : new EngineError("ENGINE_ERROR", `Cannot ${what}: this transport has been closed.`);
    }
  }

  return {
    kind: "stdio",

    async request(cmd: string, payload: unknown): Promise<unknown> {
      assertOpen("request()");
      const id = randomUUID();
      const promise = new Promise<unknown>((resolve, reject) => {
        pending.set(id, { resolve, reject });
      });
      try {
        write({ id, action: cmd, payload });
      } catch (err) {
        // The payload could not be framed. Drop the pending entry so it cannot leak, and report the
        // caller's error — the session is still healthy.
        pending.delete(id);
        throw new EngineError("BAD_REQUEST", err instanceof Error ? err.message : String(err));
      }
      return promise;
    },

    subscribe(events: string[], cb: (e: EngineEvent) => void): () => void {
      assertOpen("subscribe()");
      if (events.length === 0) return () => {};

      // All-or-nothing, and synchronously — `subscribe()` is not async, so a bad name has to be
      // caught here rather than awaited. Validating against the protocol's own `EVENT_NAMES` is what
      // makes the server's answer predictable: both ends read the same frozen list, so a refusal
      // would mean the server is out of step with the protocol, not that the caller was wrong.
      const fresh: string[] = [];
      for (const wire of events) {
        if (!isWireEventName(wire)) {
          throw new EngineError(
            "UNSUPPORTED",
            `This transport cannot deliver "${wire}". It is either not a known event or not yet on ` +
              `the wire; see BUS_EVENTS_NOT_ON_THE_WIRE in packages/engine/src/transport/types.ts.`,
          );
        }
        if (!listeners.has(wire)) {
          listeners.set(wire, new Set());
          fresh.push(wire);
        }
      }

      const token = { cb };
      for (const wire of events) listeners.get(wire)!.add(token);

      // Only ask the server for names it is not already sending us — refcounted by *name*, so two
      // callers subscribing to `event.log.entry` share one wire subscription.
      if (fresh.length > 0) sendControl("subscribe", { events: fresh });

      let detached = false;
      return () => {
        if (detached) return;
        detached = true;
        const dropped: string[] = [];
        for (const wire of events) {
          const set = listeners.get(wire);
          if (!set) continue;
          set.delete(token);
          if (set.size === 0) {
            listeners.delete(wire);
            dropped.push(wire);
          }
        }
        // Nothing to send if the session is already gone, and nothing that *can* be sent.
        if (dropped.length > 0 && !closed) sendControl("unsubscribe", { events: dropped });
      };
    },

    async close(): Promise<void> {
      if (closed) return;
      /**
       * Say goodbye before tearing down.
       *
       * Without this, closing an **attached** transport (no `child`, so we do not own the pipe and
       * must not end it) leaves the engine's bus listeners attached with nobody to receive them —
       * and in attached mode the pipe may outlive us by hours. The engine would accumulate a
       * listener per closed session until Node's 11-listener warning fired and the leak was blamed
       * on whatever subscribed last.
       *
       * Only on the graceful path. A fatal teardown must not try to write: the stream is already
       * gone, and `sendControl` would just fail again. Best-effort throughout, because `close()`
       * must not throw — a caller cleaning up in a `finally` block has nothing to do with an error
       * from the goodbye.
       */
      const live = [...listeners.keys()];
      if (live.length > 0) {
        try {
          sendControl("unsubscribe", { events: live });
        } catch {
          // The pipe is already gone; there is nobody left to tell.
        }
      }
      // Not `notify: true` — a caller-initiated close is not a fault, and reporting it as one would
      // make every clean shutdown look like a crash to whatever is watching `onFatal`.
      teardown(new EngineError("ENGINE_ERROR", "This transport has been closed."), false);
      try {
        // Ending the engine's stdin is how a spawned engine learns there will be no more requests.
        // Only done when we own the child: an attached transport must not close a pipe it shares.
        if (opts.child) opts.input.end();
      } catch {
        // Already closed by the peer — nothing to do, and nothing worth reporting.
      }
      if (opts.child && opts.child.exitCode === null) opts.child.kill();
    },
  };
}

// ── spawn-on-demand ──────────────────────────────────────────────────────────

export interface StdioSpawnOptions {
  /** Executable to run. Prefer an absolute path to `node` plus the engine entry, not an `npm` shim. */
  command: string;
  args?: string[];
  cwd?: string;
  env?: NodeJS.ProcessEnv;
}

/**
 * Spawn an engine and return a `Transport` to it — `plan/05`'s "spawn-on-demand mode".
 *
 * The engine **must not write anything but frames to stdout**. Its logging has to go to stderr, which
 * is why `stderr` is `"inherit"` here: the engine's diagnostics land in the user's terminal, where a
 * CLI user can see them, instead of being silently swallowed. A single stray `console.log` in the
 * engine corrupts the stream from that point on, and the symptom — every later request hanging — looks
 * nothing like its cause.
 *
 * **Windows:** spawning an `npm`-installed CLI by its `.cmd` shim needs `shell: true`, because Node
 * refuses to spawn `.cmd`/`.bat` without it (the CVE-2024-27980 fix). `shell: true` also means the
 * arguments go through a shell, so this deliberately does not set it — pass `process.execPath` and the
 * script path instead. That is the form the CLI (P8) will use.
 */
export function spawnStdioEngine(opts: StdioSpawnOptions): Transport {
  const child = spawn(opts.command, opts.args ?? [], {
    stdio: ["pipe", "pipe", "inherit"],
    cwd: opts.cwd,
    env: opts.env ?? process.env,
    windowsHide: true,
  });

  const { stdin, stdout } = child;
  if (!stdin || !stdout) {
    child.kill();
    throw new EngineError(
      "ENGINE_ERROR",
      "Failed to open pipes to the engine process; it was started without stdio pipes.",
    );
  }

  return createStdioTransport({ input: stdin, output: stdout, child });
}

// ── the engine side ──────────────────────────────────────────────────────────

export interface StdioServerOptions {
  /** Frames arrive here — the engine's stdin. */
  input: Readable;
  /** Frames are written here — the engine's stdout. */
  output: Writable;
  registry: CommandRegistry;
  /** Overrides the event bus. Defaults to the process-wide singleton. */
  bus?: typeof defaultBus;
  /** Overrides the command context. Defaults to `{ bus }`. */
  ctx?: CommandContext;
  /** Called once on a frame-level failure or an unexpected stream end. */
  onFatal?(error: Error): void;
}

export interface StdioServer {
  readonly kind: "stdio";
  readonly closed: boolean;
  /** Why the session ended, when it ended by failure rather than by `close()`. */
  readonly error?: Error;
  close(): Promise<void>;
}

/**
 * Serve the `CommandRegistry` over a pipe — `plan/05`'s "attached mode", and the engine half of both.
 *
 * Deliberately thin. It does exactly three things: decode frames, dispatch requests into
 * `CommandRegistry.invoke()`, and forward subscribed bus events. Everything else a real server needs
 * — `hello`, auth, scopes, replay, coalescing, flow control — is a later work item, and the reserved
 * action names above are where those will attach.
 *
 * The two exit paths differ deliberately, and the difference is the whole contract:
 *
 *  - **`close()`** (graceful, caller-initiated) does **not** end the output stream. The output is
 *    usually `process.stdout`, and closing it is not this object's decision to make — the process is
 *    going to exit anyway, and a server that closed its own stdout would take the shell's console
 *    with it.
 *  - **A fatal frame error** *does* end it. The session is unrecoverable, and a peer that is not told
 *    waits forever: every request in flight on the other end hangs with no error and no timeout. So a
 *    caller that passes `process.stdout` is accepting that an unrecoverable session error closes it —
 *    which, for a single-session stdio engine, is exactly the intended "this process is done".
 */
export function createStdioServer(opts: StdioServerOptions): StdioServer {
  const eventBus = opts.bus ?? defaultBus;
  const ctx: CommandContext = opts.ctx ?? { bus: eventBus };

  /** Wire names currently subscribed, mapped to the listener attached for them. */
  const attached = new Map<string, (payload: unknown) => void>();
  let seq = 0;
  let closed = false;
  let fatal: Error | undefined;

  /**
   * The event pump — item 3's coalescing half.
   *
   * One per server, which here is one per session: a pipe has exactly one peer by construction, so
   * there is no second client whose events could be batched together. Created eagerly (it arms no
   * timer until something is pushed) so that `close()` has an object to close on every path.
   */
  const pump = createEventPump({ send });

  function send(message: unknown): boolean {
    if (closed) return false;
    try {
      opts.output.write(encodeFrame(message));
      return true;
    } catch (err) {
      // Never let this escape into a bus listener or the stream's `data` handler: a throw from either
      // unwinds into machinery that is not expecting it, and the resulting unhandled `error` event on
      // a pipe nobody is watching is far harder to diagnose than a clean teardown.
      fail(frameFailure(err));
      return false;
    }
  }

  function fail(error: Error): void {
    if (closed) return;
    closed = true;
    fatal = error;
    detachAll();
    // After `closed = true`, so the pump's best-effort flush is a no-op: the output is about to be
    // ended and `send` refuses once `closed`. A fatal path has no tail to deliver.
    pump.close();
    opts.input.off("data", onData);
    opts.input.off("end", onEnd);
    opts.input.off("error", onEnd);

    /**
     * Close the session's output — the difference between `fail()` and `close()`.
     *
     * A graceful `close()` deliberately leaves the output alone (see the factory's header), because
     * the caller may be about to use the process for something else. A **fatal** failure is the
     * opposite case: the session is unrecoverable, and a peer that cannot be told will wait forever.
     * Every request in flight on the other end of this pipe would hang with no error and no timeout,
     * which is the worst failure mode a CLI can have. `plan/05` work item 5 is explicit that a failed
     * session closes rather than degrading ("Auth failure returns `UNAUTHORIZED` and closes. Never
     * fall back to unauthenticated"); the same applies to every other unrecoverable frame error.
     *
     * Ending the output is also the only signal available here: the request that caused this may have
     * had no `id` to answer, which is precisely why it was fatal.
     */
    try {
      opts.output.end();
    } catch {
      // Already ended or errored. Nothing left to close, and `fail()` must not throw.
    }

    opts.onFatal?.(error);
  }

  function detachAll(): void {
    for (const [wire, listener] of attached) {
      const busName = BUS_NAME_BY_WIRE_NAME.get(wire);
      if (busName) {
        eventBus.offTyped(busName as keyof EngineEvents, listener);
      }
    }
    attached.clear();
  }

  function attach(wire: string): void {
    const busName = BUS_NAME_BY_WIRE_NAME.get(wire);
    if (!busName) return; // unreachable: the caller resolved every name first
    const listener = (payload: unknown): void => {
      if (closed) return;
      seq += 1;
      // Through the pump rather than straight to `send`, so `log.entry` is coalesced and every other
      // name keeps its immediate path. `seq` is still this server's own counter — a pipe has one peer
      // and no `EventLog`, so there is nothing to share it with.
      pump.push({ event: wire as EventName, seq, payload });
    };
    eventBus.onTyped(busName as keyof EngineEvents, listener);
    attached.set(wire, listener);
  }

  function reply(response: RpcResponse): void {
    send(response);
  }

  function handleSubscribe(id: string, payload: unknown): void {
    const parsed = SubscribeRequestSchema.safeParse(payload);
    if (!parsed.success) {
      reply({ id, ok: false, error: makeError("BAD_REQUEST", `Invalid subscribe payload: ${parsed.error.message}`) });
      return;
    }
    // Resolve every name before attaching any — same all-or-nothing rule as `in-process`, and for the
    // same reason: attaching as we go would leave the earlier names live when a later one fails.
    const fresh: string[] = [];
    for (const wire of parsed.data.events) {
      if (!BUS_NAME_BY_WIRE_NAME.has(wire)) {
        reply({
          id,
          ok: false,
          error: makeError(
            "UNSUPPORTED",
            `This transport cannot deliver "${wire}". It is either not a known event or not yet on ` +
              `the wire; see BUS_EVENTS_NOT_ON_THE_WIRE in packages/engine/src/transport/types.ts.`,
          ),
        });
        return;
      }
      if (!attached.has(wire)) fresh.push(wire);
    }
    for (const wire of fresh) attach(wire);
    reply({ id, ok: true, data: { events: parsed.data.events } });
  }

  function handleUnsubscribe(id: string, payload: unknown): void {
    const parsed = UnsubscribeRequestSchema.safeParse(payload);
    if (!parsed.success) {
      reply({ id, ok: false, error: makeError("BAD_REQUEST", `Invalid unsubscribe payload: ${parsed.error.message}`) });
      return;
    }
    for (const wire of parsed.data.events) {
      const listener = attached.get(wire);
      if (!listener) continue;
      const busName = BUS_NAME_BY_WIRE_NAME.get(wire);
      if (busName) eventBus.offTyped(busName as keyof EngineEvents, listener);
      attached.delete(wire);
    }
    reply({ id, ok: true, data: { events: parsed.data.events } });
  }

  async function handleCommand(id: string, action: string, payload: unknown): Promise<void> {
    let response: RpcResponse;
    try {
      // `invoke()` is synchronous by design (see `registry.ts`); `await` adopts a handler that
      // returned a promise and passes a plain value through unchanged.
      const data = await opts.registry.invoke(action as CommandAction, payload, ctx);
      // UNINSPECTED — the envelope rule. A handler that resolved has succeeded as far as the envelope
      // is concerned, whatever it resolved with.
      response = { id, ok: true, data };
    } catch (err) {
      response = { id, ok: false, error: toRpcError(err) };
    }
    reply(response);
  }

  function onFrame(frame: unknown): void {
    if (closed) return;
    const record = asRecord(frame);
    const id = record.id;
    const action = record.action;

    if (typeof id !== "string" || typeof action !== "string") {
      // A request we cannot correlate and therefore cannot answer. The protocol has no notification
      // frame (`RpcRequestSchema` requires `id`), so this is a peer bug rather than a supported shape.
      fail(new EngineError("BAD_REQUEST", "Received a frame with no string `id` and `action`."));
      return;
    }

    if (action === "subscribe") {
      handleSubscribe(id, record.payload);
      return;
    }
    if (action === "unsubscribe") {
      handleUnsubscribe(id, record.payload);
      return;
    }

    // Not awaited: a slow command must not block the frames behind it. Ordering per id is preserved
    // by the client correlating on `id`, and a `slow` command overtaking a `fast` one is correct
    // behaviour, not a bug — commands are independent.
    void handleCommand(id, action, record.payload);
  }

  function onData(chunk: Buffer): void {
    decoder.push(chunk);
  }

  function onEnd(): void {
    fail(new EngineError("ENGINE_ERROR", "The client closed the connection."));
  }

  const decoder: FrameDecoder = createFrameDecoder({
    onFrame,
    onError: (err) => fail(frameFailure(err)),
  });

  opts.input.on("data", onData);
  opts.input.on("end", onEnd);
  opts.input.on("error", onEnd);

  return {
    kind: "stdio",
    get closed(): boolean {
      return closed;
    },
    get error(): Error | undefined {
      return fatal;
    },
    async close(): Promise<void> {
      if (closed) return;
      // Before `closed = true`, and that ordering is the whole point: a graceful close deliberately
      // leaves the output usable (see the factory's header), so the pump's flush still has somewhere
      // to write. Discarding instead would be **unrecoverable here** — unlike `ws`/`socket`, this
      // server has no `EventLog`, so an entry dropped at close cannot be replayed to anyone.
      pump.close();
      closed = true;
      detachAll();
      opts.input.off("data", onData);
      opts.input.off("end", onEnd);
      opts.input.off("error", onEnd);
    },
  };
}
