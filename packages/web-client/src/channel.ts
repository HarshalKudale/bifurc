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
}

export function createBrowserTransport(opts: BrowserTransportOptions): Transport {
  const factory = opts.webSocketFactory ?? ((url, protocols) => new WebSocket(url, protocols));

  let socket: WebSocket | undefined;
  let open = false;
  let connecting: Promise<void> | undefined;

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

      s.onopen = () => {
        if (settled || session.closed) return;
        settled = true;
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
       * The browser gives no detail here, by design: exposing why a cross-origin connect failed would
       * leak information about the other origin. So the message names only what we asked for.
       */
      s.onerror = () => {
        fail(new EngineError("ENGINE_ERROR", `Could not connect to ${opts.url}.`));
      };

      // `close` always follows `error`, so this is the ordinary path for a peer that goes away.
      s.onclose = () => {
        fail(
          new EngineError(
            "ENGINE_ERROR",
            `Session closed: the engine closed the connection to ${opts.url}.`,
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
