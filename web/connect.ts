/**
 * The whole of the first web target: connect a page to an engine.
 *
 * Deliberately one function with **no DOM in it**, so the composition — `createClient` over
 * `createBrowserTransport`, with `createBrowserLocal` supplying the client-local half — can be
 * exercised by a Node-side suite against a **real** engine over a **real** socket. That is the closest
 * such a suite can get to "it works in a browser", and it is the part worth pinning; everything else
 * about a web UI is UI, and the DOM lives in `main.ts` precisely so it cannot leak in here.
 *
 * `createClient` **requires** a `local`, which is why this file could not be written until the sixteen
 * `RequiredClientLocal` members existed. There is no Electron shell here to supply them — that is the
 * whole point of the exercise.
 */
import { createClient, type BifurcClient } from "@bifurc/client";
import { createBrowserLocal, createBrowserTransport } from "@bifurc/web-client";

export interface ConnectOptions {
  /** The engine's RPC endpoint, e.g. `ws://127.0.0.1:8080`. */
  url: string;
  /**
   * Notified when the session dies for a reason the caller did not ask for. A page needs this more
   * than the desktop does: there is no process boundary to notice the engine is gone, so without it a
   * dropped engine leaves the UI showing a stale screen forever — the "silently freezes" risk work
   * item 2's acceptance criterion names.
   */
  onFatal?: (error: Error) => void;
}

export interface Connection {
  /** The typed client. `close()` on it is the whole teardown — it closes the transport too. */
  client: BifurcClient;
  /** A real round trip, so "connected" means the engine answered rather than the socket opened. */
  probe(): Promise<unknown>;
}

export function connectToEngine(opts: ConnectOptions): Connection {
  const transport = createBrowserTransport({ url: opts.url, onFatal: opts.onFatal });
  const client = createClient(transport, { local: createBrowserLocal() });

  return {
    client,
    /**
     * `config.get` rather than a `hello` handshake.
     *
     * The handshake belongs to the auth layer and is not wired into a browser client yet (work item 2),
     * and a command that needs no token is enough to prove the round trip end to end: the socket
     * opened, the frame was framed, the engine answered, and the envelope came back through exactly the
     * path a real screen would use. Going through `client` rather than the transport is deliberate —
     * it exercises the client's own correlation and retry layer, not just the wire.
     */
    probe: () => client.getConfig(),
  };
}
