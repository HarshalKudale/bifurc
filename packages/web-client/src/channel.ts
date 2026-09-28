/**
 * A `ClientChannel` over the **native** `WebSocket` API — the browser's half of a Bifurc session.
 *
 * ## Why this is a channel and not a whole `Transport`
 *
 * `createClientSession()` (`@bifurc/protocol`) owns everything that is identical for every serialising
 * transport: correlation by frame id, the all-or-nothing subscription rule, refcounting by name, the
 * envelope rule, close semantics, and the guarantee that a request in flight when the peer dies
 * **rejects rather than hangs**. All a transport must supply is the six members of `ClientChannel`.
 * Writing a second session here would be the third copy that `session.ts` was extracted to prevent —
 * and the copy that drifts.
 *
 * ## Why this is its own package rather than part of `@bifurc/engine`
 *
 * `@bifurc/engine` already ships a `ws` client, but that file imports `node:crypto`, `node:http`,
 * `node:https`, `node:net` and the `ws` package, so a browser cannot load it. This module uses the
 * platform's own `WebSocket` and nothing else, and it deliberately does **not** depend on
 * `@bifurc/engine` — a Node package. A browser bundle that reached into it would be the opposite of
 * what P7 is for.
 *
 * ## The wire: this sends *text*, deliberately
 *
 * `ClientChannel.encode()` returns bytes, because `stdio` and `socket` need a length prefix in front
 * of one. A WebSocket frames natively, and the engine's server reads every frame as text
 * (`JSON.parse(raw.toString())`) — which is also what `@bifurc/engine`'s own `ws` client sends. So
 * `write()` sends the same bytes as a **text** frame: decoding our own encoder's output is lossless,
 * and it keeps the browser byte-identical to the Node client on the wire instead of exercising a
 * binary path the engine's server has never been tested against.
 *
 * ## Lazy, like every other transport
 *
 * The connection is opened by the first `ensureReady()`, never in the constructor: `Transport` is an
 * interface of plain methods, so a factory that returned a promise would make every caller's `try`
 * block asynchronous. A `subscribe()` issued before the socket opens is **queued** by the session's
 * outbox rather than dropped, because a subscription that is silently discarded is indistinguishable
 * from "nothing has happened yet".
 */
import {
  EngineError,
  createClientSession,
  type ClientChannel,
  type Transport,
} from "@bifurc/protocol";

/**
 * The DOM's `WebSocket.OPEN`.
 *
 * Spelled as a literal rather than read off the global because the constructor is injectable — a test
 * that supplies its own `WebSocket` has no reason to also supply the constant, and a `readyState` of 1
 * is fixed by the spec rather than by the implementation.
 */
const WS_OPEN = 1;

/**
 * How long to wait for TCP **and** the WebSocket handshake before giving up.
 *
 * The same number, and the same message, as `@bifurc/engine`'s `ws` channel — so the two clients
 * report a slow or half-dead engine identically rather than one of them hanging.
 *
 * **This is a channel concern and not a session one**, which is why it lives here rather than in
 * `createClientSession`: that layer knows nothing about how a peer is reached, so it cannot know what
 * "too long" means. The engine's channel makes the same split and says so.
 *
 * It is not decoration. A peer that **accepts the TCP connection and then never answers the upgrade**
 * produces neither an `open` nor a `close`, so without this timer `ensureReady()` never settles and
 * every request behind it waits forever — the "hangs rather than rejects" failure `plan/05` singles
 * out, arriving through the one door the session cannot see. Found by the suite on 2026-09-22: a
 * released ephemeral port was re-bound by another test's listener and one case timed out at 15 s.
 */
export const DEFAULT_CONNECT_TIMEOUT_MS = 10_000;

export interface BrowserTransportOptions {
  /** The engine's WebSocket endpoint, e.g. `ws://127.0.0.1:8080`. */
  url: string;
  /** Passed through to the `WebSocket` constructor — the standard subprotocol negotiation. */
  protocols?: string | string[];
  /**
   * Notified when the session dies for a reason the caller did not ask for — the peer closed, or the
   * connect failed.
   *
   * A caller-initiated `close()` deliberately does **not** call this: reporting a clean shutdown as a
   * fault would make every orderly exit look like a crash to whatever is watching. The mechanism is
   * that `dispose()` detaches the socket's handlers *before* closing it, so the `close` it triggers
   * never reaches the failure path.
   */
  onFatal?: (error: Error) => void;
  /**
   * The `WebSocket` constructor to use. Defaults to the platform global.
   *
   * Injectable so a test can drive this against a real server without depending on which WebSocket
   * implementation the host happens to provide — and so a non-browser host can supply one.
   */
  webSocketFactory?: (url: string, protocols?: string | string[]) => WebSocket;
  /** Overrides `DEFAULT_CONNECT_TIMEOUT_MS`. */
  connectTimeoutMs?: number;
}

export function createBrowserTransport(opts: BrowserTransportOptions): Transport {
  const factory = opts.webSocketFactory ?? ((url, protocols) => new WebSocket(url, protocols));

  let socket: WebSocket | undefined;
  let open = false;
  let connecting: Promise<void> | undefined;
  /**
   * The armed connect timeout.
   *
   * Held here rather than inside the promise so `releaseSocket()` can clear it: it is the one piece
   * of connect state that survives the handler detach, and one left armed after a teardown would
   * keep the event loop alive for up to ten seconds after every `close()`. Clearing it there is
   * **defensive rather than load-bearing** — `teardown` is idempotent, so a late fire reports
   * nothing — and is kept for the same reason the handler detach below is: the alternative is to
   * depend on the ordering of a file this one does not own.
   */
  let connectTimer: ReturnType<typeof setTimeout> | undefined;

  /** Disarm the connect timeout. Safe to call when none is armed. */
  function clearConnectTimer(): void {
    if (connectTimer === undefined) return;
    clearTimeout(connectTimer);
    connectTimer = undefined;
  }

  const encoder = new TextEncoder();
  const decoder = new TextDecoder();

  /**
   * Release the socket. Called from the session's single teardown path, so it must not throw and must
   * be safe to call when the socket never existed.
   */
  function releaseSocket(): void {
    const dead = socket;
    socket = undefined;
    open = false;
    clearConnectTimer();
    if (dead === undefined) return;

    // Detach **before** closing, so the `close` this triggers cannot re-enter the session.
    //
    // This is defensive rather than load-bearing, and the distinction is worth stating because the
    // obvious reading is the wrong one: `session.ts` sets its own `closed` flag *before* it calls
    // `dispose()`, so a `close` arriving with these handlers still attached would find the teardown
    // already done and report nothing anyway. It is kept because the alternative is for this file to
    // depend on that ordering — and because a `message` already queued when the socket closes should
    // not be handed to a session that has finished. A mutation that removes these lines is not
    // observable, which is a property of the design and not of the test.
    dead.onopen = null;
    dead.onmessage = null;
    dead.onerror = null;
    dead.onclose = null;

    try {
      dead.close();
    } catch {
      // Already gone. Teardown must never throw.
    }
  }

  function ensureOpen(): Promise<void> {
    if (socket !== undefined && open) return Promise.resolve();
    if (connecting !== undefined) return connecting;

    const timeoutMs = opts.connectTimeoutMs ?? DEFAULT_CONNECT_TIMEOUT_MS;
    const s = factory(opts.url, opts.protocols);
    // Assigned before `open` can fire so `releaseSocket()` can always reach the socket it must close.
    socket = s;
    /**
     * We only ever expect text, but a peer *may* send binary — and the default `binaryType` is
     * `"blob"`, whose contents can only be read asynchronously. Asking for an `ArrayBuffer` keeps the
     * binary case decodable synchronously in the handler below.
     */
    s.binaryType = "arraybuffer";

    connecting = new Promise<void>((resolve, reject) => {
      let settled = false;

      /** The one failure path, so `session.fail` and the rejection can never disagree. */
      const fail = (reason: Error): void => {
        session.fail(reason, true);
        if (settled) return;
        settled = true;
        reject(reason);
      };

      /**
       * The third way a connect can end, and the only one the socket itself will not report.
       *
       * A refused connection fires `close` and a failed handshake fires `error` — but a peer that
       * accepts the TCP connection and then never answers the upgrade fires **neither**. That is a
       * reachable state rather than a hypothetical: a port that was free when it was measured can be
       * bound by something else a moment later, which is exactly what this suite's dead-address case
       * does. Without this timer the promise never settles and the caller waits forever, which is the
       * "hangs rather than rejects" failure the transport contract forbids — arriving through the one
       * door `createClientSession` cannot see, because it does not know how a peer is reached.
       *
       * Reported through `fail()` like every other connect failure, so the session tears down, the
       * promise rejects and `onFatal` fires. One path, not three.
       */
      connectTimer = setTimeout(() => {
        if (settled) return;
        fail(
          new EngineError("ENGINE_ERROR", `Timed out after ${timeoutMs}ms connecting to ${opts.url}.`),
        );
      }, timeoutMs);

      s.onopen = () => {
        if (settled || session.closed) return;
        settled = true;
        clearConnectTimer();
        open = true;
        // Flush whatever the session queued before the socket came up — subscription control frames,
        // typically. `session.opened()` replays them verbatim; see the split in `session.ts`.
        session.opened();
        resolve();
      };

      s.onmessage = (event: MessageEvent) => {
        const raw = event.data;
        let parsed: unknown;
        try {
          parsed = JSON.parse(typeof raw === "string" ? raw : decoder.decode(raw as ArrayBuffer));
        } catch {
          // A frame that is not JSON cannot be interpreted, and guessing would mean acting on a
          // partially-understood message. The engine's own server takes the same posture and closes.
          fail(new EngineError("ENGINE_ERROR", "Received a frame that is not JSON."));
          return;
        }
        session.receive(parsed);
      };

      /**
       * **A failed connect is reported here — and on this platform it is reported *only* here.**
       *
       * Measured, not assumed (2026-09-22): Node's own `WebSocket` — undici's — fires `error` and then
       * **no `close` at all** when a connect fails. Three forms were probed and all three behaved
       * identically, with `readyState` still `CONNECTING` seconds later: a port that had been bound and
       * released, a listener that refused the upgrade by hand, and a listener that destroyed the socket
       * on arrival. `ws` — which the engine's own channel uses — *does* emit `close` after `error`, and
       * a spec-compliant browser does too. So this is a property of the **host**, not of the protocol,
       * and `ws.ts`'s handler cannot be copied verbatim here.
       *
       * That is why the guard is `open` rather than nothing, and both halves of it are load-bearing:
       *
       * - **Reporting unconditionally** is the defect this replaced. `error` also fires when an
       *   *established* connection drops, so every dropped session was announced as `Could not connect`
       *   — the message work item 2's UI reads as "the address was wrong" rather than "reconnect".
       * - **Silencing it entirely** is the tempting over-correction, and it is worse: on this host it
       *   leaves a wrong address with **no** report at all, so `ensureReady()` never settles and the caller
       *   hangs rather than fails. Only a socket that never opened belongs to this handler.
       *
       * `fail()` is idempotent and first-wins, so when `close` does follow `error` — a browser — this
       * message is the one that survives, and it is the right one: the socket never opened.
       */
      s.onerror = () => {
        if (open) return; // Established: `close` owns that message, and it says something different.
        fail(new EngineError("ENGINE_ERROR", `Could not connect to ${opts.url}.`));
      };

      /**
       * The other reporting path, distinguishing the two cases by whether the socket ever opened — the
       * only difference a caller can act on. "Could not connect" means retry from scratch; "the engine
       * closed the connection" means reconnect. A single message for both would make the UI lie.
       *
       * On this host the `!open` half is unreachable (see `onerror`): undici does not emit `close` for a
       * failed connect. It is kept because on a browser it is *the* path for that case, and dropping it
       * would make this file correct on the platform it is tested on and wrong on the platform it is
       * written for.
       */
      s.onclose = () => {
        clearConnectTimer();
        fail(
          new EngineError(
            "ENGINE_ERROR",
            open
              ? `Session closed: the engine closed the connection to ${opts.url}.`
              : `Could not connect to ${opts.url}.`,
          ),
        );
      };
    });

    // A rejected connect must not stay cached, or every later `request()` on a transport that failed
    // once would reject with the original error forever — including after a successful reconnect. The
    // `.catch` also marks the promise handled; the caller still sees the rejection.
    void connecting.catch(() => {
      connecting = undefined;
    });

    return connecting;
  }

  const channel: ClientChannel = {
    encode: (frame) => {
      const json = JSON.stringify(frame);
      // `JSON.stringify(undefined)` returns `undefined` rather than a string; encoding that would send
      // the literal text "undefined". Mirrors `ws.ts`'s own `encode()`.
      if (json === undefined) {
        throw new EngineError("BAD_REQUEST", "This payload cannot be represented as JSON.");
      }
      return encoder.encode(json);
    },

    write: (frame) => {
      const s = socket;
      if (s === undefined) throw new EngineError("ENGINE_ERROR", "The WebSocket is gone.");
      // See the header: the engine's server reads every frame as text, and its own `ws` client sends
      // text. This keeps both clients on one wire form rather than on two.
      s.send(decoder.decode(frame));
    },

    /**
     * False until `open` fires, so the session queues instead of writing.
     *
     * A browser *would* accept a `send()` on a `CONNECTING` socket and buffer it, but relying on that
     * would make this transport's correctness depend on an implementation detail of `WebSocket` — and
     * it would leave the session's outbox exercised by `stdio` and `socket` alone. Waiting costs
     * nothing and makes the guarantee explicit.
     */
    isReady: () => open && socket !== undefined && socket.readyState === WS_OPEN,

    ensureReady: () => ensureOpen(),
    dispose: () => releaseSocket(),
    onFatal: (error) => opts.onFatal?.(error),
    // No `onClose`: there is no shared pipe to leave alone and no child to end. The socket is released
    // by `dispose()`, which runs on every teardown, graceful or not.
  };

  const session = createClientSession({ kind: "ws", channel });

  return session.transport;
}
