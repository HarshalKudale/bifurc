/**
 * P4 work item 2, step 3 — the `ws` transport, and work item 1's generalisation of the
 * companion server.
 *
 * `plan/05` orders the work "define `Transport` and implement `in-process` first, then `stdio`, then
 * **WebSocket + auth**", and adds the hard gate: *"Do not ship a `ws` transport before item 4 is
 * done, even to test."* Item 4 is done, so this is that step — and it is deliberately a **runner
 * plus an adapter**, not a redesign: the conformance suite already runs against four transports, and
 * the delivery barrier and the auth cases were both built with this one in mind.
 *
 * ## This is the companion server, generalised — not a second server
 *
 * `plan/05` work item 1 says it plainly: *"Do not write a new server."* The companion server already
 * speaks a WebSocket JSON envelope on loopback with a shipping external consumer (the companion
 * extension, now in its own repository). What it lacked is everything this file supplies, and the
 * plan lists the five changes: enrich `error: string` into a `RpcError`, add the mandatory `hello`,
 * replace the static allowlist with per-session scopes, add event delivery with `seq`, and move it
 * out of `src/companion/` — *"the directory name `companion` will make no sense once the browser
 * extension is just another client."* That last change is now done: it lives at
 * `transport/legacyCompanion.ts` and `src/companion/` no longer exists.
 *
 * ## The four colon-named actions did **not** become aliases — that is a finding, not an omission
 *
 * The obvious way to finish item 1 is to map `config:get` → `config.get`, `mock:add`/`request:add` →
 * `entity.create`, `folder:add` → `folder.add`, and delete the hand-written bodies. Measured on
 * 2026-09-18, that is **not a migration — it is four behaviour changes**, one of which would make a
 * lower-trust caller's "add" destructive. The full table is in `legacyCompanion.ts`'s header; the
 * load-bearing one is that the registry's mock create runs `onAddConflict`, which **disables an
 * existing enabled mock** with the same `method|urlPattern|capturedBody`, while the extension's
 * allowlist is documented and tested as **additive-only**. So those four stay served by
 * `legacyCompanion.ts` — byte-compatible, on port 9271 — until the extension moves to v2 and the
 * mapping is *decided* rather than assumed. `plan/05` item 1 carries the same table.
 *
 * ## Why auth is a wrapper here and not a feature
 *
 * Each connection builds its own session: `createInProcessTransport(registry)` wrapped in
 * `createAuthenticatedTransport(...)`. That is the whole payoff of item 4's decorator decision — the
 * handshake, the constant-time token check and the scope gate were built and proven over
 * `in-process` before this socket existed, so this file contains **no auth logic at all**. It
 * forwards frames into a `Transport` and writes the answers back out.
 *
 * The one thing it does own is the *consequence* of an auth failure. `plan/05` work item 5:
 * *"Auth failure returns `UNAUTHORIZED` and closes. Never fall back to unauthenticated."* So an
 * `UNAUTHORIZED` or `UNSUPPORTED` reply is flushed and **then** the socket is closed — the reply goes
 * first because a peer that is disconnected without being told sees a network fault instead of a
 * rejected credential, which is the one diagnostic a remote client has.
 *
 * ## The wire contract is the protocol's, not this file's
 *
 * Frames carry the frozen `RpcRequest {id, action, payload}` / `RpcResponse {id, ok, data|error}` /
 * `EventEnvelope {event, seq, payload}` envelopes, discriminated by which key is present — identical
 * to `stdio`, minus the length prefix, because WebSocket already frames messages. A `ws`-specific
 * envelope would mean a translation layer between the two transports and a second place for the
 * envelope rule to be got wrong, and the rule is the thing that keeps `window.api` byte-identical.
 *
 * That rule has teeth over a serialising transport, because there are **two `ok` flags that are not
 * the same flag**:
 *
 * ```ts
 * { id, ok: true,  data: { ok: false, error: "..." } }   // the handler ran; inner ok:false rides through as DATA
 * { id, ok: false, error: { code, message, ... } }        // the handler THREW; this rejects
 * ```
 *
 * `request()` resolves with the handler's value **uninspected**; its only failure path is the
 * registry throwing. Conflating the two would break every legacy handler that reports failure as a
 * resolved value, which is most of them.
 *
 * ## Bind address and TLS — work item 5's two rules, encoded rather than documented
 *
 * | `host` | `allowNonLoopback` | `tls` | result |
 * |---|---|---|---|
 * | loopback (default) | any | any | binds |
 * | non-loopback | absent/false | any | **refused** — remote mode is opt-in and explicit |
 * | non-loopback | `true` | absent | **refused** — "TLS on the channel, not optional" |
 * | non-loopback | `true` | present | binds, `wss://` |
 *
 * `plan/05`: *"Never default to `0.0.0.0`. Remote mode is opt-in and explicit."* and, for the TLS
 * row, *"Required for `ws` beyond loopback."* Two guards rather than one because they are two
 * different mistakes: binding wide by accident, and binding wide in the clear on purpose. Encoding
 * them means the engine cannot be talked into an unauthenticated, unencrypted, remotely-reachable RPC
 * surface by a config typo — the process refuses to start rather than exposing one.
 *
 * The practical consequence, stated so nobody is surprised: **remote mode is not reachable yet**.
 * Nothing in the programme sets `allowNonLoopback`, and the TLS path is a pass-through to
 * `node:https` that has no test behind it. That is the honest position — the guard is what makes it
 * safe to leave that work to P9/P12.
 */
import { createServer as createHttpServer } from "node:http";
import { createServer as createHttpsServer } from "node:https";
import type { AddressInfo } from "node:net";
import { WebSocket, WebSocketServer, type RawData } from "ws";
import {
  Capability,
  EngineError,
  makeError,
  RpcRequestSchema,
  SubscribeRequestSchema,
  UnsubscribeRequestSchema,
  createClientSession,
  type CapabilityValue,
  type ClientChannel,
  type CommandAction,
  type RpcResponse,
} from "@bifurc/protocol";
import {
  bus as defaultBus,
  ENGINE_EVENT_NAMES,
  type EngineEventBus,
  type EngineEvents,
} from "../eventBus";
import type { CommandContext, CommandRegistry } from "../commands/registry";
import {
  createAuthenticatedTransport,
  IDENTITY_FAILURE_CODES,
  type AuthenticatedTransportOptions,
} from "./auth";
import { createEventPump } from "./eventPump";
import { createInProcessTransport } from "./inProcess";
import type { EventLog } from "./eventLog";
import {
  assertBridgeIsTotal,
  BUS_NAME_BY_WIRE_NAME,
  toRpcError,
  type Transport,
} from "./types";

/**
 * Import-time validation, same as `inProcess.ts` and `stdio.ts`: this transport maps wire names to
 * bus names on the way out, so a wrong bridge produces a subscription that succeeds and never fires.
 * At module scope, so it fails when the engine loads rather than the first time a client subscribes.
 */
assertBridgeIsTotal(ENGINE_EVENT_NAMES as readonly string[]);

/** The engine binds loopback unless a caller says otherwise, twice. */
export const DEFAULT_WS_HOST = "127.0.0.1";

/** Hosts treated as loopback for the bind guard. `localhost` is included because it resolves to one. */
export const LOOPBACK_HOSTS: readonly string[] = ["127.0.0.1", "::1", "localhost"];

/**
 * Frame-size ceiling — work item 5's "Request size: ... at the frame level".
 *
 * `ws` defaults to 100 MiB, which is a large allocation to hand a peer that controls the header. This
 * is a **chosen** bound rather than a measured one, and it is an option because the blob work
 * (P7/P9) may need it raised: a base64 blob chunk is the largest legitimate payload this transport
 * will ever carry, and it is not yet known what size those are at this layer.
 */
export const DEFAULT_MAX_PAYLOAD_BYTES = 16 * 1024 * 1024;

/** How long a client waits for TCP + the WebSocket handshake before giving up. */
export const DEFAULT_CONNECT_TIMEOUT_MS = 10_000;

/**
 * The close code used for an identity failure and for a malformed frame.
 *
 * `1008` is the standard "policy violation" — semantically right for both, and inside the range
 * `ws` accepts from a server (`1000`, or `3000`–`4999`, excluding the reserved `1004`–`1006`).
 */
export const CLOSE_CODE_POLICY = 1008;

/**
 * Codes that mean the peer has not established *who it is*.
 *
 * The set itself lives in `auth/authenticated.ts`, next to the decision table it encodes, because
 * `socket.ts` must apply the same set with a different mechanism. The consequence is what is local:
 * these close the session, and `FORBIDDEN` / `BAD_REQUEST` do not. A `FORBIDDEN` peer proved its
 * identity and asked for something out of scope; closing would turn a scope error into a disconnect
 * and make a mis-scoped client look like a network fault.
 */

function isLoopbackHost(host: string): boolean {
  return LOOPBACK_HOSTS.includes(host.toLowerCase());
}

/** `ws://host:port`, bracketing a bare IPv6 literal. */
function urlFor(host: string, port: number, secure: boolean): string {
  const bracketed = host.includes(":") ? `[${host}]` : host;
  return `${secure ? "wss" : "ws"}://${bracketed}:${port}`;
}

/** Serialise a frame, reporting a payload JSON cannot represent as a caller error. */
function encode(message: unknown): string {
  const json = JSON.stringify(message);
  if (json === undefined) {
    throw new EngineError("BAD_REQUEST", "This payload cannot be represented as JSON.");
  }
  return json;
}

// ── the client ───────────────────────────────────────────────────────────────

export interface WsTransportOptions {
  /** Passed through to the `WebSocket` constructor — the standard subprotocol negotiation. */
  protocols?: string | string[];
  /** Extra handshake headers. The place a remote deployment would put a bearer credential. */
  headers?: Record<string, string>;
  /**
   * Notified when the session dies for a reason the caller did not ask for — the peer closed, the
   * connect failed, or the engine refused a control frame the client had already validated.
   *
   * A caller-initiated `close()` deliberately does **not** call this: reporting a clean shutdown as a
   * fault would make every orderly exit look like a crash to whatever is watching.
   */
  onFatal?: (error: Error) => void;
  /** Overrides `DEFAULT_CONNECT_TIMEOUT_MS`. */
  connectTimeoutMs?: number;
}

/**
 * A `Transport` over a WebSocket, connecting lazily.
 *
 * Lazy rather than eager because the constructor must stay synchronous — `Transport` is an interface
 * of plain methods and a factory that returned a promise would make every caller's `try` block
 * asynchronous. The first `request()` awaits the connection; a `subscribe()` before that **queues**
 * its control frame rather than dropping it, because a subscription that is silently discarded is
 * indistinguishable from "nothing has happened yet" — the failure mode `plan/05` singles out.
 *
 * **The state machine is not in this file.** Correlation by `id`, the control-frame set, subscriber
 * refcounting by wire name, the outbox, the envelope rule, and the guarantee that a request in flight
 * when the peer dies **rejects rather than hangs** all live in `createClientSession`
 * (`@bifurc/protocol`'s `wire/session.ts`). This file supplies only the six `ClientChannel` members.
 *
 * It used to carry its own copy — the **third** one, after `stdio` and `socket` — and the copies had
 * already drifted: this one read `seq` off the envelope while `stdio`'s recounted locally. That is
 * precisely the "similar becomes drifted" failure `session.ts` was extracted to prevent, and it was
 * invisible until the extraction forced the question *"who else implements this?"*. The `seq` rule
 * (`seq` is engine-scoped, so a session subscribed to a subset sees deliberate gaps) is documented
 * there, where all three transports now share one implementation of it.
 */
export function createWsTransport(url: string, opts: WsTransportOptions = {}): Transport {
  const encoder = new TextEncoder();
  const decoder = new TextDecoder();

  let socket: WebSocket | undefined;
  let open = false;
  let connecting: Promise<void> | undefined;

  /**
   * The single release path, called from **any** teardown via `ClientChannel.dispose()`.
   *
   * Listeners are detached **before** the socket is closed: the `close` we are about to trigger must
   * not re-enter `session.fail()`, and a `message` already queued must not be delivered into state
   * that has just been cleared. Must not throw — it runs inside the teardown path, where there is
   * nobody left to report a failure to.
   */
  function release(): void {
    const dead = socket;
    socket = undefined;
    open = false;
    if (dead === undefined) return;
    dead.removeAllListeners();
    /**
     * The one listener that must **outlive** the detach.
     *
     * `ws`'s `close()` on a socket that never finished connecting aborts the handshake and then emits
     * `error` on the **next tick** — "WebSocket was closed before the connection was established". An
     * `error` event with no listener **throws**, and because it is emitted on a later tick the `try`
     * below cannot catch it, so it escapes as an uncaught exception and takes the process down rather
     * than failing one session. That is the same hazard the connect handler's `error` no-op documents.
     *
     * Reachable whenever a connect times out, because the socket is still `CONNECTING` — and it was
     * live in the pre-refactor copy too, which detached and closed in the same order. Nothing had ever
     * exercised the timeout, so nothing had ever emitted it.
     */
    dead.on("error", () => {});
    try {
      dead.close();
    } catch {
      // Already gone. `close()` must never throw.
    }
  }

  function ensureConnected(): Promise<void> {
    if (socket !== undefined && open) return Promise.resolve();
    if (connecting !== undefined) return connecting;

    const timeoutMs = opts.connectTimeoutMs ?? DEFAULT_CONNECT_TIMEOUT_MS;
    const ws = new WebSocket(url, opts.protocols, { headers: opts.headers });
    // Assigned before `open` so `release()` can always reach the socket it must destroy.
    socket = ws;

    connecting = new Promise<void>((resolve, reject) => {
      let settled = false;

      /**
       * The connect timeout is a **channel** concern, not a session one: `createClientSession` knows
       * nothing about how a peer is reached, so it cannot know what "too long" means.
       */
      const timer = setTimeout(() => {
        if (settled) return;
        settled = true;
        const err = new EngineError(
          "ENGINE_ERROR",
          `Timed out after ${timeoutMs}ms connecting to ${url}.`,
        );
        session.fail(err, true);
        reject(err);
      }, timeoutMs);

      ws.on("open", () => {
        if (settled || session.closed) return;
        settled = true;
        clearTimeout(timer);
        open = true;
        // Drains the session's outbox: subscriptions queued before the socket came up are replayed
        // verbatim, which is what makes a `subscribe()` before `request()` safe.
        session.opened();
        resolve();
      });

      ws.on("message", (raw) => {
        // Decoding is a channel concern — `stdio` and `socket` hand over length-prefixed bytes, so
        // the session is handed an already-parsed frame and never sees `RawData`.
        let parsed: unknown;
        try {
          parsed = JSON.parse(raw.toString());
        } catch {
          session.fail(
            new EngineError("ENGINE_ERROR", "The engine sent a frame that is not JSON."),
            true,
          );
          return;
        }
        session.receive(parsed);
      });

      /**
       * `ws` always emits `close` after `error`, so the teardown and the rejection both happen there.
       * Handling `error` as well would double-report, and the handler must exist regardless: an
       * `error` event on an EventEmitter with no listener **throws**, which would take the process
       * down instead of failing one session.
       */
      ws.on("error", () => {});

      ws.on("close", (code) => {
        const reason = new EngineError(
          "ENGINE_ERROR",
          `Session closed: the engine closed the connection (code ${code}).`,
        );
        if (!settled) {
          settled = true;
          clearTimeout(timer);
          session.fail(reason, true);
          reject(reason);
          return;
        }
        session.fail(reason, true);
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
    /**
     * `encode()` returns **bytes**, not a string, because the other two channels frame a byte stream
     * and the session must be able to hand either an opaque `Uint8Array`. The WebSocket protocol
     * frames messages itself, so the bytes here are simply the UTF-8 of the JSON and `write()`
     * decodes them straight back — the round trip is the price of one shared session.
     */
    encode: (frame) => encoder.encode(encode(frame)),

    write: (frame) => {
      const live = socket;
      if (live === undefined) return;
      // Text, not bytes: the engine's `ws` server reads frames as text (`raw.toString()`) and answers
      // in kind, so sending binary here would be a protocol the other end does not speak.
      live.send(decoder.decode(frame));
    },

    isReady: () => socket !== undefined && open,
    ensureReady: () => ensureConnected(),
    dispose: () => release(),
    /**
     * The caller's `onFatal` is fired by the session, which owns the decision about whether a death
     * was a fault — a caller-initiated `close()` is deliberately not one, because reporting a clean
     * shutdown as a fault would make every orderly exit look like a crash to whatever is watching.
     */
    onFatal: (error) => opts.onFatal?.(error),
  };

  const session = createClientSession({ kind: "ws", channel });
  return session.transport;
}

// ── the server ───────────────────────────────────────────────────────────────

/**
 * Authentication for a `ws` server.
 *
 * Optional, and omitting it produces an **unauthenticated** server. That is legal only because the
 * bind guard makes an unauthenticated server reachable from loopback alone: `plan/05`'s threat is an
 * unauthenticated *remote* RPC surface, and the `127.0.0.1` bind is what keeps this one out of it.
 * (The same reasoning `companionServer.ts` used, kept deliberately and now enforced by the guard
 * rather than by a comment.)
 */
export interface WsServerAuthOptions {
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
  /** Extra capabilities to advertise. `ws` is added automatically. */
  capabilities?: readonly CapabilityValue[];
  /** Overrides the engine's protocol version, so a test can drive the major-mismatch path. */
  protocolVersion?: string;
}

export interface WsServerOptions {
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
  auth?: WsServerAuthOptions;
  /** Defaults to `DEFAULT_WS_HOST`. Anything else requires `allowNonLoopback` **and** `tls`. */
  host?: string;
  /** Defaults to `0` — an ephemeral port, so two engines on one machine cannot collide. */
  port?: number;
  /** Turns the listener into `wss://`. Mandatory for any non-loopback bind. */
  tls?: { cert: string | Buffer; key: string | Buffer };
  /** Explicit acknowledgement that a non-loopback bind is intended. Does not substitute for `tls`. */
  allowNonLoopback?: boolean;
  /** Overrides `DEFAULT_MAX_PAYLOAD_BYTES`. */
  maxPayloadBytes?: number;
  /** Notified when a connection dies for a reason the engine did not ask for. */
  onFatal?: (error: Error, remoteAddress: string) => void;
}

export interface WsServer {
  readonly host: string;
  /** The **resolved** port, which is what matters when the caller asked for `0`. */
  readonly port: number;
  readonly url: string;
  readonly secure: boolean;
  close(): Promise<void>;
}

/**
 * Serve the `CommandRegistry` over WebSocket — the engine half, and `plan/05` work item 1's
 * generalisation of `companionServer.ts`.
 *
 * Asynchronous because it **binds**: the port is not known until the OS assigns one, and a factory
 * that pretended otherwise would have to guess. That is why the conformance suite's `make()` may now
 * return a promise.
 *
 * One `WebSocketServer` serves many sessions, and each connection gets its **own** `Transport` — its
 * own scopes, its own `resumeFrom`, its own subscriptions. That is the difference from
 * `companionServer.ts`, which had one process-wide `ALLOWED_ACTIONS` set and therefore one
 * authorisation decision for every client that ever connected.
 */
export async function createWsServer(opts: WsServerOptions): Promise<WsServer> {
  const host = opts.host ?? DEFAULT_WS_HOST;
  const loopback = isLoopbackHost(host);

  // Two guards, not one, because they catch two different mistakes — see the header's table.
  if (!loopback && opts.allowNonLoopback !== true) {
    throw new EngineError(
      "ENGINE_ERROR",
      `Refusing to bind ${host}: the engine binds loopback by default and a non-loopback address is ` +
        `opt-in and explicit (plan/05 work item 5: "Never default to 0.0.0.0"). Pass ` +
        `allowNonLoopback: true to accept the exposure.`,
    );
  }
  if (!loopback && opts.tls === undefined) {
    throw new EngineError(
      "ENGINE_ERROR",
      `Refusing to serve plaintext WebSocket on ${host}: plan/05 work item 5 requires TLS on the ` +
        `channel beyond loopback, not as an option. Supply \`tls\` (cert + key) or bind loopback.`,
    );
  }

  const eventBus = opts.bus ?? defaultBus;
  // No server-level `ctx` here on purpose — `opts.ctx` is a *template*, and each connection spreads it
  // into its own context. See the connection handler.
  const secure = opts.tls !== undefined;

  const server =
    opts.tls === undefined ? createHttpServer() : createHttpsServer({ cert: opts.tls.cert, key: opts.tls.key });

  // A non-upgrade request gets an answer rather than a hang. This is not decoration: a health check
  // or a browser hitting the port must not be left waiting for a response that never comes.
  server.on("request", (_req, res) => {
    res.writeHead(426, { "content-type": "text/plain; charset=utf-8", upgrade: "websocket" });
    res.end("This port speaks WebSocket, not HTTP.\n");
  });

  const wss = new WebSocketServer({
    server,
    maxPayload: opts.maxPayloadBytes ?? DEFAULT_MAX_PAYLOAD_BYTES,
  });

  let closed = false;

  wss.on("connection", (socket, request) => {
    const remoteAddress = request.socket.remoteAddress ?? "unknown";

    /**
     * One command context per connection — deliberately not the server-level `opts.ctx`.
     *
     * `opts.ctx` is shared by every session, so spreading it keeps a caller's overrides while giving
     * this session its own `session` slot. Two sessions sharing one context object is exactly how a
     * mutation ends up attributed to the wrong client, and it would be invisible: each session looks
     * correct in isolation.
     */
    const ctx: CommandContext = { ...(opts.ctx ?? { bus: eventBus }) };

    /**
     * One session per connection. The inner transport is `in-process` — a direct call into the
     * registry with no `ipcMain` anywhere on the path, which is what the registry was built in P2 to
     * make possible. Auth, when configured, is a **wrapper** over it; this file contains no auth
     * logic, which is the payoff of item 4's decorator decision.
     *
     * `ctx` is handed to both the inner transport and the decorator, and they must be the **same
     * object**: the decorator learns the session id during the handshake, after the inner transport
     * has already captured its context, so the identity reaches handlers only by mutation of this one
     * object.
     */
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
            // `peer` is added here rather than inside the decorator: only the server sees the
            // connection, so only the server knows the address. A session that never authenticates
            // never fires this, so `ctx.session` stays absent — which is the honest state for a
            // client that was never identified.
            onSession: (identity) => {
              ctx.session = { ...identity, peer: remoteAddress };
            },
            // `ws` is advertised because this *is* the ws transport; `events.replay` is derived by the
            // decorator from `log` being present, so it is not listed here and must not be — two
            // sources for that flag is how they end up disagreeing.
            capabilities: [...(opts.auth.capabilities ?? []), Capability.WS],
          });

    /**
     * Live subscriptions, keyed by wire name, with a count.
     *
     * The count is what makes a duplicate `subscribe` safe: a client that sent the same name twice
     * would otherwise have the second `off()` orphan the first, and the events would stop arriving
     * while the client believed it was subscribed. The client refcounts too, but a server that
     * trusted its peer's bookkeeping would be one bug away from a silent stall.
     */
    const subs = new Map<string, { count: number; off: () => void }>();

    function send(message: unknown, onFlushed?: () => void): void {
      if (closed || socket.readyState !== WebSocket.OPEN) return;
      try {
        const json = encode(message);
        if (onFlushed === undefined) socket.send(json);
        else socket.send(json, () => onFlushed());
      } catch (err) {
        // Never let this escape into a bus listener: a throw from there unwinds into machinery that
        // is not expecting it, and the resulting unhandled `error` event on a socket nobody is
        // watching is far harder to diagnose than a closed connection.
        opts.onFatal?.(err instanceof Error ? err : new Error(String(err)), remoteAddress);
        socket.terminate();
      }
    }

    function closeSocket(code: number, reason: string): void {
      try {
        socket.close(code, reason);
      } catch {
        socket.terminate();
      }
    }

    /**
     * The event pump — item 3's coalescing half.
     *
     * **One per connection, not one per server**, and the difference is why this cannot live beside
     * `wss`: a pump's queue holds frames destined for one channel, so a shared pump would hand each
     * client the other's events. The `EventLog` is the object that is deliberately shared; this is the
     * one that deliberately is not.
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
      // `in-process` and `stdio`, and for the same reason: attaching as we go would leave the earlier
      // names live when a later one is refused, and a caller that caught the refusal and retried
      // would accumulate duplicates it could not see.
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
           * that call, so per-name attachment gives every name its backlog exactly once. Batching
           * would too — but then a per-name refcount could not tell which name a later `unsubscribe`
           * was retiring, and the count is the only thing standing between a duplicate subscribe and
           * an orphaned subscription.
           *
           * This cannot attach partially and then throw: the names are validated above, and the two
           * things the decorator can still refuse (`UNAUTHORIZED` before the handshake, `CONFLICT`
           * when the replay window moved) are both decided from state that cannot change inside this
           * synchronous loop, so either the first name fails or none do.
           */
          const off = session.subscribe([wire], (e) => {
            // Through the pump, not straight to `send`. The decorator above delivers the replay
            // backlog through this same callback, so both halves of a session's event stream are
            // coalesced by the same object — a batch shape that appeared on the live path and not on
            // the replay path would be the worst of both.
            pump.push(e);
          });
          subs.set(wire, { count: 1, off });
        }
        send({ id, ok: true, data: { events: parsed.data.events } });
      } catch (err) {
        // `subscribe()` throws synchronously by contract. All three codes it can throw are
        // answerable, and none of them should kill the connection: `FORBIDDEN`-style refusals are
        // states to recover from, not disconnects, and a peer that is disconnected without being
        // told cannot tell a refusal from a network fault.
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
           * The reply goes out **first** and the close waits for it to flush, which is the whole
           * point: a peer disconnected without being told sees a network fault instead of a rejected
           * credential, and the credential is the one diagnostic it can act on. `send`'s flush
           * callback is the only way to order those two — `socket.close()` called first would race
           * the reply and usually win.
           */
          send({ id, ok: false, error: rpc }, () => closeSocket(CLOSE_CODE_POLICY, rpc.message));
          return;
        }
        send({ id, ok: false, error: rpc });
      }
    }

    socket.on("message", (raw) => {
      if (closed) return;
      // The handler must not throw: it runs inside `ws`'s emitter, where a throw becomes an
      // unhandled `error` event on a socket nobody is watching.
      try {
        let parsed: unknown;
        try {
          parsed = JSON.parse(raw.toString());
        } catch {
          closeSocket(CLOSE_CODE_POLICY, "frames must be JSON");
          return;
        }

        const request_ = RpcRequestSchema.safeParse(parsed);
        if (!request_.success) {
          // The protocol has no notification frame, so a request without a string `id` cannot be
          // answered at all — there is nothing to correlate a reply to. Closing this one connection
          // is the only signal available, and unlike `stdio` it is not fatal to the *engine*: one
          // malformed peer must not take down the process serving everyone else.
          closeSocket(CLOSE_CODE_POLICY, "frames must be RpcRequest envelopes");
          return;
        }

        const { id, action, payload } = request_.data;
        if (action === "subscribe") {
          handleSubscribe(id, payload);
          return;
        }
        if (action === "unsubscribe") {
          handleUnsubscribe(id, payload);
          return;
        }

        // Not awaited: a slow command must not block the frames behind it. Ordering is preserved
        // where it matters — `handleSubscribe` above is synchronous, so a `subscribe` frame is fully
        // applied before the next frame is looked at, which is what lets a client use a command round
        // trip as a subscription barrier.
        void handleCommand(id, action, payload);
      } catch (err) {
        opts.onFatal?.(err instanceof Error ? err : new Error(String(err)), remoteAddress);
        closeSocket(CLOSE_CODE_POLICY, "internal error");
      }
    });

    socket.on("error", () => {
      // `close` always follows, and that is where the session is torn down.
    });

    socket.on("close", () => {
      // The session owns the log subscriptions; closing it detaches them. Without this the engine
      // would hold one listener per disconnected client for the life of the process.
      void session.close();
      // And the pump owns a timer. Its flush is best-effort by now — `send` refuses once the socket is
      // no longer OPEN — but leaving the timer armed would hold a handle for up to the batch window
      // after every disconnect, which is the kind of thing a long-running engine accumulates.
      pump.close();
    });
  });

  await new Promise<void>((resolve, reject) => {
    const onError = (err: Error): void => {
      server.off("listening", onListening);
      reject(
        new EngineError(
          "ENGINE_ERROR",
          `Could not bind ${host}:${opts.port ?? 0}: ${err.message}`,
        ),
      );
    };
    const onListening = (): void => {
      server.off("error", onError);
      resolve();
    };
    server.once("error", onError);
    server.once("listening", onListening);
    server.listen(opts.port ?? 0, host);
  });

  const address = server.address() as AddressInfo | null;
  if (address === null) {
    throw new EngineError("ENGINE_ERROR", `The listener for ${host} has no address after binding.`);
  }

  return {
    host,
    port: address.port,
    url: urlFor(host, address.port, secure),
    secure,

    async close(): Promise<void> {
      if (closed) return;
      closed = true;

      /**
       * `terminate()` rather than `close()`.
       *
       * `WebSocketServer.close()` does not call back until **every** client is gone, so a graceful
       * shutdown has to wait on peers completing a close handshake they have no incentive to
       * complete. That is how a test suite — or an engine shutdown — hangs forever on one wedged
       * client. `terminate()` destroys the socket immediately; a server being torn down is not the
       * moment to negotiate.
       */
      for (const client of wss.clients) client.terminate();

      await new Promise<void>((resolve) => {
        wss.close(() => resolve());
      });

      // `wss.close()` does **not** close a server it was given rather than created, so the listener
      // is ours to release. Any connection still tracked is a keep-alive socket from an upgrade that
      // never happened; `closeAllConnections` is what stops `server.close()` waiting on those.
      server.closeAllConnections?.();
      await new Promise<void>((resolve) => {
        server.close(() => resolve());
      });
    },
  };
}
