/**
 * P4 work item 2, step 4 — the `socket` transport: a unix domain socket, or a Windows named pipe.
 *
 * This is the **last** of the four transports and, per D3, the one the desktop shell actually uses:
 * `plan/07`'s P6 supervisor spawns the engine and connects to `\\.\pipe\bifurc-<pid>` on Windows or
 * `$XDG_RUNTIME_DIR/bifurc-<pid>.sock` elsewhere. `plan/05`'s transport table gives the reason it was
 * chosen over an ephemeral TCP port: *"Preferred: no collisions, not network-reachable."*
 *
 * ## Why this file is short
 *
 * It is short because the three steps before it were done properly, and that is worth stating so the
 * brevity is not mistaken for incompleteness:
 *
 * | Concern | Where it lives |
 * |---|---|
 * | correlation, subscription refcounting, teardown, the envelope rule | `session.ts` |
 * | length-prefixed JSON | `framing.ts` |
 * | `hello`, the token check, scopes, `lastSeq` | `auth/authenticated.ts` |
 * | `seq` and replay | `eventLog.ts` |
 *
 * So the client below is one `ClientChannel` — six members — and the server is a decoder plus three
 * frame handlers. There is no third copy of the request state machine, which is the point of
 * `session.ts`: `plan/05`'s risks table lists "conformance suite written per-transport instead of
 * shared" as a risk, and the same argument applies one layer down.
 *
 * ## Framing: a socket is a byte stream, so this transport owns its message boundaries
 *
 * `ws` sends a bare JSON string because the WebSocket protocol already frames messages. A socket does
 * not — one `write()` can surface as two `data` events and two writes as one — so this transport uses
 * `framing.ts` (a `uint32` big-endian length prefix, then UTF-8 JSON) exactly as `stdio` does. The
 * wire contract is otherwise the same three `@bifurc/protocol` envelopes, so a client can move
 * between `stdio`, `socket` and `ws` with no translation layer.
 *
 * `MAX_FRAME_BYTES` is **not** overridable here, unlike `ws`'s `maxPayloadBytes`. `ws` needed an
 * option because WebSocket carries a *second*, independent limit that had to be aligned with the
 * codec's; a plain socket has only the codec's, and both ends read the same constant.
 *
 * ## Access control is the filesystem, and it is asserted rather than assumed
 *
 * `plan/05`'s auth table: *"`socket` / named pipe — Filesystem permissions (0600). Token additionally
 * recommended."* So on POSIX the socket file is `chmod 0600`ed **before `createSocketServer()`
 * resolves**, and the resulting mode is read back and verified; a socket that cannot be made private
 * is refused rather than served, because `plan/05`'s threat is an unauthenticated RPC surface and
 * `0600` is the whole of this transport's protection against one.
 *
 * `auth` is therefore **optional** here, exactly as on `ws` and for the same reason: a `0600` socket
 * is reachable by this user alone, which is the same reachability as loopback. Supplying it is
 * recommended — the plan says so — and costs one option.
 *
 * ## Refusing to bind over a live socket, and cleaning up a stale one
 *
 * A unix socket is a filesystem entry that outlives the process that created it, so a crashed engine
 * leaves one behind and `bind()` then fails with `EADDRINUSE` for as long as the file exists — the
 * classic "the app will not start and nothing says why". So before binding: if the path exists, check
 * it is really a socket, then **probe it**. A successful connect means another engine is live and
 * this call is refused loudly; a refused connect means the entry is a corpse and it is unlinked.
 *
 * Three details that are not decoration:
 *
 * - **`isSocket()` before `unlink()`.** Without it, a path that happens to be a user's regular file
 *   would be deleted by an engine starting up. That is the kind of bug that costs someone a document.
 * - **A named pipe has no filesystem entry**, so on Windows the check is skipped entirely and
 *   `EADDRINUSE` from `listen()` is projected instead.
 * - The probe is not perfectly race-free — two engines starting in the same millisecond could both
 *   conclude the path is stale — but the loser gets `EADDRINUSE` from `listen()` and reports it, so
 *   the outcome is a clear error rather than a silent split brain.
 *
 * ## One deliberate divergence from `ws`
 *
 * A frame the decoder rejects is reported through `onFatal` **and** closes that one connection. `ws`
 * closes without reporting, because there the framing lives inside the library and its only signal is
 * a close code. Here the framing is ours, so a real error object exists — and `oversized_frame` in
 * particular is a peer trying to make the engine reserve 4 GiB, which is worth recording rather than
 * silently disconnecting over. Everything else follows `ws`: one bad peer closes one connection, never
 * the process.
 *
 * ## What this deliberately does NOT do
 *
 * - **No TCP fallback.** D3 chose the socket *"with (a) as a fallback for environments where sockets
 *   are unavailable"*. That fallback is not implemented: no such environment is known to the
 *   programme (Windows has named pipes, POSIX has UDS), and adding a TCP listener would re-open work
 *   item 5's bind guards — the ones `ws.ts` carries — for a transport whose entire value is *"not
 *   network-reachable"*. If it is ever needed it is an additive `{host, port}` target behind an
 *   explicit opt-in, not a default.
 * - **No rate limiting, no concurrency cap, no per-command timeout.** Work item 5's remaining rows,
 *   unchanged by this transport existing.
 * - **No backpressure.** Work item 3's second half; `VERIFIABLE.backpressure` is still `false` in the
 *   conformance suite, so no runner may claim it.
 */
import { chmodSync, existsSync, lstatSync, statSync, unlinkSync } from "node:fs";
import { createConnection, createServer, type Server, type Socket } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  Capability,
  EngineError,
  makeError,
  RpcRequestSchema,
  SubscribeRequestSchema,
  UnsubscribeRequestSchema,
  type CapabilityValue,
} from "@bifurc/protocol";
import { bus as defaultBus, ENGINE_EVENT_NAMES, type EngineEventBus } from "../eventBus";
import type { CommandContext, CommandRegistry } from "../commands/registry";
import {
  createAuthenticatedTransport,
  IDENTITY_FAILURE_CODES,
  type AuthenticatedTransportOptions,
} from "./auth";
import type { EventLog } from "./eventLog";
import { createFrameDecoder, encodeFrame, frameFailure } from "./framing";
import { createEventPump } from "./eventPump";
import { createInProcessTransport } from "./inProcess";
import { createClientSession, describeError, type ClientChannel } from "./session";
import {
  assertBridgeIsTotal,
  BUS_NAME_BY_WIRE_NAME,
  toRpcError,
  type EngineEvent,
  type Transport,
} from "./types";

/**
 * Import-time validation, same as `inProcess.ts`, `stdio.ts` and `ws.ts`: this transport maps wire
 * names to bus names on the way out, so a wrong bridge produces a subscription that succeeds and
 * never fires. At module scope, so it fails when the engine loads rather than the first time a client
 * subscribes.
 */
assertBridgeIsTotal(ENGINE_EVENT_NAMES as readonly string[]);

const isWindows = process.platform === "win32";

/**
 * What `onFatal` reports as the peer for this transport.
 *
 * A unix socket and a named pipe are both local by construction and Node reports no `remoteAddress`
 * for either, so there is no address to record. Saying so explicitly is better than passing an empty
 * string or `undefined` — the audit hook (work item 4) will want a value, and *"local"* is the true
 * one. When the TCP fallback in D3 is ever built, this becomes a real address.
 */
const LOCAL_PEER = "local";

/**
 * The socket path `plan/07` names for a given engine pid — `\\.\pipe\bifurc-<pid>` on Windows,
 * `$XDG_RUNTIME_DIR/bifurc-<pid>.sock` (falling back to the temp directory) elsewhere.
 *
 * Here rather than in P6 because both ends need to agree on it and P6 is not the only caller: the CLI
 * (P8) probes the same path to decide whether to attach to a running engine instead of spawning one.
 * Two copies of this naming scheme is exactly how an attach-mode client ends up unable to find the
 * engine it just started.
 *
 * `XDG_RUNTIME_DIR` is preferred on POSIX because the spec requires it to be `0700` and per-user,
 * which makes the socket private even before the `chmod` below. `tmpdir()` is the documented fallback
 * for the environments that do not set it.
 */
export function defaultSocketPath(pid: number = process.pid): string {
  if (isWindows) return `\\\\.\\pipe\\bifurc-${pid}`;
  const runtime = process.env.XDG_RUNTIME_DIR;
  return join(runtime !== undefined && runtime.length > 0 ? runtime : tmpdir(), `bifurc-${pid}.sock`);
}

// ── the client ───────────────────────────────────────────────────────────────

export interface SocketTransportOptions {
  /**
   * Notified when the session dies for a reason the caller did not ask for — the engine closed the
   * connection, the connect failed, or the engine refused a control frame the client had already
   * validated.
   *
   * A caller-initiated `close()` deliberately does **not** call this: reporting a clean shutdown as a
   * fault would make every orderly exit look like a crash to whatever is watching.
   */
  onFatal?: (error: Error) => void;
}

/**
 * A `Transport` over a unix domain socket or a named pipe, connecting lazily.
 *
 * Lazy for the same reason `ws` is: `Transport` is an interface of plain methods, so a factory that
 * returned a promise would make every caller's `try` block asynchronous. The first `request()` awaits
 * the connection; a `subscribe()` before that **queues** its control frame in the session's outbox
 * rather than dropping it, because a subscription that is silently discarded is indistinguishable
 * from "nothing has happened yet" — the failure mode `plan/05` singles out.
 *
 * There is no connect timeout, unlike `ws`. A WebSocket connect can stall on DNS and a TCP handshake;
 * a local socket connect fails *immediately* with `ENOENT`, `EACCES` or `ECONNREFUSED`, which is the
 * whole reason this transport was chosen. A timeout here would be a timer that can only ever fire
 * when something else has already gone wrong. (A request that never returns is a different problem —
 * work item 5's per-command timeout.)
 */
export function createSocketTransport(
  path: string,
  opts: SocketTransportOptions = {},
): Transport {
  let socket: Socket | undefined;
  /** True once the socket has connected — see `ClientChannel.isReady`. */
  let connected = false;
  let connecting: Promise<void> | undefined;

  const decoder = createFrameDecoder({
    onFrame: (frame) => session.receive(frame),
    // A frame the codec rejects leaves the stream unusable: the boundaries are still intact but the
    // sender's intent is unknowable, and continuing would mean acting on a partially-understood
    // message. `frameFailure` keeps the `BAD_REQUEST`/`ENGINE_ERROR` split, so a caller can tell
    // "this stream is garbage" from "this command failed".
    onError: (err) => session.fail(frameFailure(err), true),
  });

  /**
   * Release the socket. Called from the session's single teardown path, so it must not throw and must
   * be safe to call when the socket never existed.
   */
  function releaseSocket(): void {
    const dead = socket;
    socket = undefined;
    connected = false;
    if (dead === undefined) return;

    dead.removeAllListeners();
    /**
     * An `error` listener has to survive `removeAllListeners()`.
     *
     * `destroySoon()` ends the stream, and ending a socket whose peer has already gone produces
     * `EPIPE`/`ECONNRESET`. An `error` event on an EventEmitter with **no** listener is *thrown*, so
     * without this line the teardown path could take the whole process down — while it is in the
     * middle of cleaning up, which is the worst possible moment.
     */
    dead.on("error", () => {});
    try {
      if (dead.destroyed) return;
      /**
       * `destroySoon()` rather than `destroy()`.
       *
       * `destroy()` discards anything still sitting in Node's writable buffer, and the session's
       * `close()` writes an `unsubscribe` control frame immediately before tearing down — that frame
       * is what stops the engine leaking one `EventLog` subscription per closed session. `destroySoon()`
       * flushes what is queued and *then* releases the descriptor, which is exactly those semantics.
       * It also ends the stream, so a half-open socket cannot keep the event loop alive.
       */
      dead.destroySoon();
    } catch {
      try {
        dead.destroy();
      } catch {
        // Already gone. Teardown must never throw.
      }
    }
  }

  function ensureConnected(): Promise<void> {
    if (socket !== undefined && connected) return Promise.resolve();
    if (connecting !== undefined) return connecting;

    const s = createConnection(path);
    // Assigned before `connect` fires so `releaseSocket()` can always reach the socket it must close.
    socket = s;

    connecting = new Promise<void>((resolve, reject) => {
      let settled = false;

      /** The one failure path, so `session.fail` and the rejection can never disagree. */
      const fail = (reason: Error): void => {
        session.fail(reason, true);
        if (settled) return;
        settled = true;
        reject(reason);
      };

      s.once("connect", () => {
        if (settled || session.closed) return;
        settled = true;
        connected = true;
        // Flush whatever the session queued before the socket came up — subscription control frames,
        // typically. `session.opened()` replays them verbatim; see the split in `session.ts`.
        session.opened();
        resolve();
      });

      s.on("data", (chunk: Buffer) => decoder.push(chunk));

      /**
       * The handler must exist even though it does nothing: an `error` event on an emitter with no
       * listener is thrown, which would take the process down instead of failing one session. The
       * teardown itself happens in `close`, which always follows `error` on a `net.Socket`.
       */
      s.on("error", () => {});

      s.on("close", () => {
        fail(
          new EngineError(
            "ENGINE_ERROR",
            `Session closed: the engine closed the connection to ${path}.`,
          ),
        );
      });
    });

    // A rejected connect must not stay cached, or every later `request()` on a transport that failed
    // once would reject with the original error forever — including after a successful reconnect.
    // The `.catch` also marks the promise handled; the caller still sees the rejection.
    void connecting.catch(() => {
      connecting = undefined;
    });

    return connecting;
  }

  const channel: ClientChannel = {
    encode: (frame) => encodeFrame(frame),

    write: (frame) => {
      const s = socket;
      if (s === undefined) throw new EngineError("ENGINE_ERROR", "The socket is gone.");
      s.write(frame);
    },

    /**
     * False until `connect` fires, so the session queues instead of writing.
     *
     * Node *would* accept a write on a connecting socket and buffer it internally, but relying on that
     * would mean this transport's correctness depends on an implementation detail of `net.Socket` —
     * and it would leave the session's outbox exercised by `ws` alone. Waiting for `connect` costs
     * nothing and makes the guarantee explicit.
     */
    isReady: () =>
      connected && socket !== undefined && !socket.destroyed && !socket.writableEnded,

    ensureReady: () => ensureConnected(),
    dispose: () => releaseSocket(),
    onFatal: (error) => opts.onFatal?.(error),
    // No `onClose`: unlike `stdio` there is no shared pipe to leave alone and no child to end. The
    // socket is released by `dispose()`, which runs on every teardown, graceful or not.
  };

  const session = createClientSession({ kind: "socket", channel });

  return session.transport;
}

// ── the server ───────────────────────────────────────────────────────────────

/**
 * Authentication for a `socket` server.
 *
 * Optional, and omitting it produces an **unauthenticated** server. That is legal because the socket
 * file is `0600` — reachable by this user alone, the same reachability as `ws`'s loopback bind — and
 * `plan/05`'s threat is an unauthenticated *remote* RPC surface. The plan nonetheless recommends a
 * token here as well, which is why the option exists.
 */
export interface SocketServerAuthOptions {
  /** The token `ensureEngineToken()` produced for this run. */
  token: string;
  /** Reported to the client in the `hello` response. Under D9 it tracks the protocol version. */
  engineVersion: string;
  /** Which scopes a session gets, by client. Defaults to `SHELL_SCOPES` (see `authenticated.ts`). */
  scopesFor?: AuthenticatedTransportOptions["scopesFor"];
  /** Notified when a session is torn down by an auth failure — the audit hook (work item 4). */
  onAuthFailure?: AuthenticatedTransportOptions["onAuthFailure"];
  /** Session-id factory, injectable so a test can assert on it rather than matching a UUID. */
  sessionId?: AuthenticatedTransportOptions["sessionId"];
  /** Extra capabilities to advertise. `socket` is added automatically. */
  capabilities?: readonly CapabilityValue[];
  /** Overrides the engine's protocol version, so a test can drive the major-mismatch path. */
  protocolVersion?: string;
}

export interface SocketServerOptions {
  /**
   * The socket path. **Required**, and deliberately not defaulted to `defaultSocketPath()`: the
   * caller owns the naming scheme, and a server that silently bound `bifurc-<pid>` would make a test
   * that forgot to pass a temp path collide with the developer's running engine.
   */
  path: string;
  registry: CommandRegistry;
  bus?: EngineEventBus;
  ctx?: CommandContext;
  /**
   * The engine's event log — **pass the engine's, not a new one**, for the same reason `inProcess.ts`
   * documents: `seq` is only meaningful across a reconnect if one authority issues it. Omitting it
   * gives each connection a private log, which is sound only for a standalone server and disables
   * replay (the decorator advertises `events.replay` off the back of this being present).
   */
  log?: EventLog;
  auth?: SocketServerAuthOptions;
  /** Notified when a connection dies for a reason the engine did not ask for. */
  onFatal?: (error: Error, peer: string) => void;
}

export interface SocketServer {
  /** The path actually bound — the same string the caller passed. */
  readonly path: string;
  close(): Promise<void>;
}

/**
 * Is something accepting connections on this path?
 *
 * A *stale* unix socket file is a file nobody is listening on, and the only way to tell it from a live
 * one is to try. `ECONNREFUSED` is the stale case; anything else (permissions, a path that vanished
 * between the check and the probe) is treated as "not live" too, because the alternative is refusing
 * to start with no way forward.
 *
 * The probe connects and immediately destroys, which the *other* engine sees as a session that opened
 * and closed. That is a real cost, but it is paid only on the "refuse to bind" path — the engine on
 * the other end is already running and about to be reported as the reason this one will not start.
 */
function isSocketLive(path: string): Promise<boolean> {
  return new Promise<boolean>((resolve) => {
    const probe = createConnection(path);
    let settled = false;
    const done = (live: boolean): void => {
      if (settled) return;
      settled = true;
      probe.removeAllListeners();
      // Survives `removeAllListeners()`, because `destroy()` below can still emit one and an
      // unhandled `error` event is thrown.
      probe.on("error", () => {});
      try {
        probe.destroy();
      } catch {
        // Nothing left to release.
      }
      resolve(live);
    };
    probe.once("connect", () => done(true));
    probe.once("error", () => done(false));
  });
}

/**
 * Serve the `CommandRegistry` over a unix socket or a named pipe — the engine half, and D3's decided
 * desktop transport.
 *
 * Asynchronous because it **binds** and, on POSIX, because it verifies the file's permissions before
 * returning: `createSocketServer()` resolving is the caller's guarantee that the socket is private
 * and accepting, with no window in which it is neither.
 *
 * One listener serves many sessions and each connection gets its **own** `Transport` — its own scopes,
 * its own `resumeFrom`, its own subscriptions. That is the difference from `companionServer.ts`, which
 * had one process-wide `ALLOWED_ACTIONS` set and therefore one authorisation decision for every client
 * that ever connected.
 */
export async function createSocketServer(opts: SocketServerOptions): Promise<SocketServer> {
  const { path } = opts;
  if (path.length === 0) {
    throw new EngineError("BAD_REQUEST", "createSocketServer needs a non-empty socket path.");
  }

  const eventBus = opts.bus ?? defaultBus;
  // No server-level `ctx` — `opts.ctx` is a template and each connection spreads it into its own
  // context. See `attachSession`. Same reason as `ws.ts`.

  // ── the stale-socket rule, before anything is bound ────────────────────────
  if (!isWindows && existsSync(path)) {
    let isSocket = false;
    try {
      // `lstat`, not `stat`: a symlink to a socket is not a socket, and refusing is the safe answer.
      isSocket = lstatSync(path).isSocket();
    } catch {
      isSocket = false;
    }
    if (!isSocket) {
      throw new EngineError(
        "ENGINE_ERROR",
        `Refusing to bind ${path}: something already exists there and it is not a socket. Remove it ` +
          `yourself if that is what you intended — the engine will not delete a file it did not create.`,
      );
    }
    if (await isSocketLive(path)) {
      throw new EngineError(
        "ENGINE_ERROR",
        `Refusing to bind ${path}: another engine is already listening there. Attach to it, or stop ` +
          `it first.`,
      );
    }
    try {
      // The file outlived the process that created it — a crashed engine. `bind()` would refuse with
      // `EADDRINUSE` forever, which is the "the app will not start and nothing says why" failure.
      unlinkSync(path);
    } catch (err) {
      throw new EngineError(
        "ENGINE_ERROR",
        `Could not remove the stale socket at ${path}: ${describeError(err)}`,
      );
    }
  }

  const connections = new Set<Socket>();
  let closed = false;

  const server: Server = createServer((socket) => {
    connections.add(socket);
    socket.on("close", () => connections.delete(socket));
    attachSession(socket);
  });

  await new Promise<void>((resolve, reject) => {
    const onError = (err: Error): void => {
      server.off("listening", onListening);
      reject(new EngineError("ENGINE_ERROR", `Could not bind ${path}: ${err.message}`));
    };
    const onListening = (): void => {
      server.off("error", onError);
      resolve();
    };
    server.once("error", onError);
    server.once("listening", onListening);
    server.listen(path);
  });

  /**
   * `0600` on POSIX, verified rather than assumed.
   *
   * The window between `bind()` creating the file and this `chmod` is real and is not closed here:
   * closing it properly needs the process umask, which is global mutable state a library must not
   * touch. What *is* guaranteed is the part the caller can rely on — `createSocketServer()` does not
   * resolve until the socket is private, so nothing built from this server can connect while it is
   * not. A caller that needs the window gone as well puts the socket in a `0700` directory, which is
   * why `defaultSocketPath()` prefers `$XDG_RUNTIME_DIR`.
   *
   * The read-back matters more than the `chmod`: a filesystem that ignores modes would otherwise leave
   * an unauthenticated RPC surface serving every local user, silently, which is precisely the outcome
   * work item 5 exists to prevent.
   */
  if (!isWindows) {
    let mode: number;
    try {
      chmodSync(path, 0o600);
      mode = statSync(path).mode & 0o077;
    } catch (err) {
      await closeServer();
      throw new EngineError(
        "ENGINE_ERROR",
        `Could not make ${path} private (0600): ${describeError(err)}. The socket transport relies on ` +
          `filesystem permissions for access control, so it refuses to serve one it cannot secure.`,
      );
    }
    if (mode !== 0) {
      await closeServer();
      throw new EngineError(
        "ENGINE_ERROR",
        `Refusing to serve ${path}: its permissions are still ${mode.toString(8)} after chmod 0600, so ` +
          `any local user could connect. Configure \`auth\`, or use a path on a filesystem that ` +
          `supports permissions.`,
      );
    }
  }

  /**
   * One session per connection — the inner transport is `in-process`, a direct call into the registry
   * with no `ipcMain` anywhere on the path. Auth, when configured, is a **wrapper** over it; this file
   * contains no auth logic, which is the payoff of item 4's decorator decision.
   *
   * The command context is per connection, not per server: `opts.ctx` is a template, and sharing one
   * object across sessions is how a mutation gets attributed to the wrong client. See `ws.ts`.
   */
  function attachSession(socket: Socket): void {
    const ctx: CommandContext = { ...(opts.ctx ?? { bus: eventBus }) };
    const inner = createInProcessTransport(opts.registry, { bus: eventBus, ctx, log: opts.log });
    const session: Transport =
      opts.auth === undefined
        ? inner
        : createAuthenticatedTransport(inner, {
            token: opts.auth.token,
            engineVersion: opts.auth.engineVersion,
            log: opts.log,
            scopesFor: opts.auth.scopesFor,
            onAuthFailure: opts.auth.onAuthFailure,
            sessionId: opts.auth.sessionId,
            protocolVersion: opts.auth.protocolVersion,
            /**
             * No `peer`, and that is deliberate rather than an oversight: a unix socket or a named pipe
             * has no peer *address*. The path names this engine's own endpoint, so recording it in a
             * field called `peer` would be a fiction — and a misleading one, since it would read like a
             * client identifier. Identity on this transport is the token plus the handshake, which is
             * exactly what `sessionId` and `clientName` carry.
             */
            onSession: (identity) => {
              ctx.session = identity;
            },
            // `socket` is advertised because this *is* the socket transport; `events.replay` is derived
            // by the decorator from `log` being present, so it is not listed here and must not be —
            // two sources for that flag is how they end up disagreeing.
            capabilities: [...(opts.auth.capabilities ?? []), Capability.SOCKET],
          });

    /**
     * Live subscriptions, keyed by wire name, with a count.
     *
     * The count is what makes a duplicate `subscribe` safe: a client that sent the same name twice
     * would otherwise have the second `off()` orphan the first, and events would stop arriving while
     * the client believed it was subscribed. The client refcounts too, but a server that trusted its
     * peer's bookkeeping would be one bug away from a silent stall.
     */
    const subs = new Map<string, { count: number; off: () => void }>();
    let connectionClosed = false;

    /**
     * The single teardown path for this connection, and the reason `close()` cannot be re-entered.
     *
     * `session.close()` is what detaches this session's `EventLog` subscriptions. Without it the engine
     * would hold one listener per disconnected client for the life of the process — the leak class
     * that bit the `stdio` server in P4.
     */
    function shutdown(): void {
      if (connectionClosed) return;
      connectionClosed = true;
      void session.close();
      // Best-effort by now, and knowingly so: `connectionClosed` above already makes `send` a no-op, so
      // the pump's flush cannot land. That is the acceptable half of the trade — this server is given
      // the engine's `EventLog`, so a client that reconnects and presents a `lastSeq` gets the entries
      // back by replay. What must not happen is the timer outliving the connection, which is why this
      // is here rather than nowhere.
      pump.close();
      try {
        socket.destroy();
      } catch {
        // Already gone.
      }
    }

    function send(message: unknown, onFlushed?: () => void): void {
      if (connectionClosed || closed || socket.destroyed || socket.writableEnded) return;
      try {
        const frame = encodeFrame(message);
        // The callback fires once the bytes have reached the kernel, which is the strongest ordering
        // guarantee available without a round trip — and it is what makes the identity-failure path
        // below deliver its reply before the socket disappears.
        if (onFlushed === undefined) socket.write(frame);
        else socket.write(frame, () => onFlushed());
      } catch (err) {
        // Never let this escape into a bus listener: a throw from there unwinds into machinery that
        // is not expecting it, and the resulting unhandled `error` event on a socket nobody is
        // watching is far harder to diagnose than a closed connection.
        opts.onFatal?.(err instanceof Error ? err : new Error(String(err)), LOCAL_PEER);
        shutdown();
      }
    }

    /**
     * The event pump — item 3's coalescing half.
     *
     * One per connection, for the reason `ws.ts` gives at the same seam: a pump's queue holds frames
     * for one channel, so sharing it across clients would send each of them the other's events.
     */
    const pump = createEventPump({ send });

    function handleSubscribe(id: string, payload: unknown): void {
      const parsed = SubscribeRequestSchema.safeParse(payload);
      if (!parsed.success) {
        send({
          id,
          ok: false,
          error: makeError("BAD_REQUEST", `Invalid subscribe payload: ${parsed.error.message}`),
        });
        return;
      }

      // Every name is resolved before any subscription is attached — the same all-or-nothing rule as
      // `in-process`, `stdio` and `ws`, and for the same reason: attaching as we go would leave the
      // earlier names live when a later one is refused, and a caller that caught the refusal and
      // retried would accumulate duplicates it could not see.
      for (const wire of parsed.data.events) {
        if (!BUS_NAME_BY_WIRE_NAME.has(wire)) {
          send({
            id,
            ok: false,
            error: makeError(
              "UNSUPPORTED",
              `This transport cannot deliver "${wire}". It is either not a known event or not yet ` +
                `on the wire; see BUS_EVENTS_NOT_ON_THE_WIRE in packages/engine/src/transport/types.ts.`,
            ),
          });
          return;
        }
      }

      try {
        for (const wire of parsed.data.events) {
          const existing = subs.get(wire);
          if (existing !== undefined) {
            existing.count += 1;
            continue;
          }
          /**
           * One `session.subscribe` per name, deliberately.
           *
           * The decorator drains the session's replay backlog **per call**, filtered to the names in
           * that call, so per-name attachment gives every name its backlog exactly once. Batching would
           * too — but then a per-name refcount could not tell which name a later `unsubscribe` was
           * retiring, and the count is the only thing standing between a duplicate subscribe and an
           * orphaned subscription.
           *
           * This cannot attach partially and then throw: the names are validated above, and the two
           * things the decorator can still refuse (`UNAUTHORIZED` before the handshake, `CONFLICT`
           * when the replay window moved) are both decided from state that cannot change inside this
           * synchronous loop, so either the first name fails or none do.
           */
          const off = session.subscribe([wire], (e: EngineEvent) => {
            // Through the pump, not straight to `send` — and this callback carries the decorator's
            // replay backlog as well as the live stream, so both are coalesced by one object.
            pump.push(e);
          });
          subs.set(wire, { count: 1, off });
        }
        send({ id, ok: true, data: { events: parsed.data.events } });
      } catch (err) {
        // `subscribe()` throws synchronously by contract. All three codes it can throw are
        // answerable, and none of them should kill the connection: `FORBIDDEN`-style refusals are
        // states to recover from, not disconnects, and a peer that is disconnected without being told
        // cannot tell a refusal from a network fault.
        send({ id, ok: false, error: toRpcError(err) });
      }
    }

    function handleUnsubscribe(id: string, payload: unknown): void {
      const parsed = UnsubscribeRequestSchema.safeParse(payload);
      if (!parsed.success) {
        send({
          id,
          ok: false,
          error: makeError("BAD_REQUEST", `Invalid unsubscribe payload: ${parsed.error.message}`),
        });
        return;
      }
      for (const wire of parsed.data.events) {
        const entry = subs.get(wire);
        if (entry === undefined) continue;
        entry.count -= 1;
        if (entry.count > 0) continue;
        subs.delete(wire);
        entry.off();
      }
      send({ id, ok: true, data: { events: parsed.data.events } });
    }

    async function handleCommand(id: string, action: string, payload: unknown): Promise<void> {
      try {
        // UNINSPECTED — the envelope rule. A handler that resolved has succeeded as far as the
        // envelope is concerned, whatever it resolved with.
        const data = await session.request(action, payload);
        send({ id, ok: true, data });
      } catch (err) {
        const rpc = toRpcError(err);
        if (IDENTITY_FAILURE_CODES.has(rpc.code)) {
          /**
           * `plan/05` work item 5: *"Auth failure returns `UNAUTHORIZED` and closes. Never fall back
           * to unauthenticated."*
           *
           * The reply goes out **first**, and the flush callback is the only way to order the two —
           * destroying the socket immediately would usually beat the write. The ordering matters more
           * here than it does on `ws`, not less: a WebSocket can at least carry a close code, whereas
           * a plain socket has no way to say *why* it went away. The reply is the only signal the peer
           * gets, and a peer disconnected without it cannot tell a rejected credential from a crash.
           */
          send({ id, ok: false, error: rpc }, () => shutdown());
          return;
        }
        send({ id, ok: false, error: rpc });
      }
    }

    function onFrame(frame: unknown): void {
      if (connectionClosed) return;
      /**
       * The protocol has no notification frame, so a frame without a string `id` cannot be answered
       * at all — there is nothing to correlate a reply to, and closing this one connection is the only
       * signal available. Unlike `stdio` it is not fatal to the *engine*: one malformed peer must not
       * take down the process serving everyone else.
       */
      const parsed = RpcRequestSchema.safeParse(frame);
      if (!parsed.success) {
        shutdown();
        return;
      }

      const { id, action, payload } = parsed.data;
      if (action === "subscribe") {
        handleSubscribe(id, payload);
        return;
      }
      if (action === "unsubscribe") {
        handleUnsubscribe(id, payload);
        return;
      }

      // Not awaited: a slow command must not block the frames behind it. Ordering is preserved where
      // it matters — `handleSubscribe` above is synchronous, so a `subscribe` frame is fully applied
      // before the next frame is looked at, which is what lets a client use a command round trip as a
      // subscription barrier.
      void handleCommand(id, action, payload);
    }

    const decoder = createFrameDecoder({
      onFrame,
      onError: (err) => {
        // Reported, then this connection alone is closed. See the header's "one deliberate divergence
        // from `ws`" — the framing is ours here, so there is a real error to report, and an
        // `oversized_frame` is a peer asking the engine to reserve 4 GiB.
        opts.onFatal?.(frameFailure(err), LOCAL_PEER);
        shutdown();
      },
    });

    socket.on("data", (chunk: Buffer) => decoder.push(chunk));
    socket.on("error", () => {
      // `close` always follows, and that is where the session is torn down.
    });
    socket.on("close", () => shutdown());
  }

  async function closeServer(): Promise<void> {
    if (closed) return;
    closed = true;

    /**
     * `destroy()` rather than `end()` on every connection.
     *
     * `server.close()` does not call back until every connection is gone, so a graceful shutdown would
     * have to wait on peers completing a close handshake they have no incentive to complete — which is
     * how a test suite, or an engine shutdown, hangs forever on one wedged client. A server being torn
     * down is not the moment to negotiate.
     */
    for (const socket of [...connections]) {
      try {
        socket.destroy();
      } catch {
        // Already gone.
      }
    }
    connections.clear();

    await new Promise<void>((resolve) => {
      server.close(() => resolve());
    });

    if (!isWindows) {
      // The listener is gone, so the entry is now a corpse. Leaving it behind would make the *next*
      // start pay for this one's cleanup, and the next start may be a different user who cannot
      // unlink it.
      try {
        unlinkSync(path);
      } catch {
        // Already removed, or never created because the bind failed.
      }
    }
  }

  return { path, close: closeServer };
}
