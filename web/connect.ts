/**
 * Connect a browser page to a workspace host — **handshake included**.
 *
 * ## The defect this file fixes
 *
 * The browser used to go straight from `createBrowserTransport` to `createClient` and call
 * `config.get`. Against a headless host that is refused, because the host wraps its socket in
 * `createAuthenticatedTransport`, which requires `hello` as the **first** request and — per `plan/05`
 * work item 5, *"auth failure returns `UNAUTHORIZED` and closes. Never fall back to unauthenticated"* —
 * **closes the session** when it is not. So the page saw a connection that died immediately, and the
 * engine logged `the first request must be "hello"` once per attempt.
 *
 * Electron never hit this: its transport is `in-process`, which has no boundary and is therefore never
 * wrapped. The browser is the first client that has to authenticate, which is why the gap survived P6.
 *
 * ## Why the handshake is here and not inside `createClient`
 *
 * `plan/06` is explicit — *"the handshake is a plain `request("hello", …)` on the raw transport before
 * `createClient()` — so the client never learns it is remote, which is what P6 needs."* The renderer must
 * not be able to tell a local engine from a remote one, and a client that performed the handshake itself
 * would know. So the composition layer owns it, exactly as
 * `packages/client/tests/transports.test.ts` already does for the `ws` transport.
 *
 * ## Why the token is conditional
 *
 * `HelloRequest.token` is optional **in the schema** because `in-process` and `stdio` have no boundary
 * and a token there would be security theatre — one schema serves both. So a token present means "this
 * endpoint has a boundary, authenticate"; a token absent means "this one does not". That is also what
 * keeps the browser usable against a plain, unwrapped `ws` server, where a `hello` would be an
 * `UNKNOWN_COMMAND` — `hello` is deliberately not a registry command.
 *
 * The cost is one real failure mode with confusing copy: connecting *without* a token to an endpoint
 * that requires one. It is handled explicitly in `classify()` rather than left to the engine's message.
 */
import { createClient, type BifurcClient } from "@bifurc/client";
import { EngineError, HELLO_ACTION, HelloResponseSchema, PROTOCOL_VERSION } from "@bifurc/protocol";
import { createBrowserLocal, createBrowserTransport } from "@bifurc/web-client";

/** The engine's log records this, so it names the kind of client rather than a build number. */
export const WEB_CLIENT_NAME = "web-ui" as const;

/** Used when the host has no version to report — a dev server, or a test. */
export const UNKNOWN_CLIENT_VERSION = "0.0.0-unknown";

/**
 * How a connection attempt failed, in terms a **form** can act on.
 *
 * `plan/08`'s acceptance criterion is that *"handshake failure, auth failure and version mismatch all
 * produce actionable UI, not generic errors"* — which needs the distinction to survive as data rather
 * than being flattened into one string. The engine's own message is kept in `cause` for the log.
 */
export type ConnectFailure =
  /** Nothing answered: refused, no route, or the connect timeout. */
  | "unreachable"
  /** The endpoint requires a token and none was supplied, or the one supplied is wrong. */
  | "unauthorized"
  /** The engine answered and refuses this client's protocol major. */
  | "version"
  /** The engine answered and refused for some other reason. */
  | "refused";

export class ConnectError extends Error {
  constructor(
    readonly failure: ConnectFailure,
    message: string,
    override readonly cause?: unknown,
  ) {
    super(message);
    this.name = "ConnectError";
  }
}

export interface ConnectOptions {
  /** The engine's RPC endpoint, e.g. `ws://127.0.0.1:8080`. */
  url: string;
  /**
   * The host's per-start secret, read from `<dataDir>/engine.token` on the machine running the host.
   *
   * A browser cannot read a file, so this is supplied by the user through the connection form. Absent
   * means "this endpoint has no auth boundary" — see the note at the top.
   */
  token?: string;
  /** Reported in the engine's log. Defaults to `UNKNOWN_CLIENT_VERSION`. */
  clientVersion?: string;
  /**
   * The highest event `seq` already seen, for a **reconnect** — `plan/05` work item 3.
   *
   * Absent means "send me only live events", which is what a *first* connection wants. A reconnect always
   * has a value, even `0`. Getting this wrong is silent in both directions: omit it and a brief network
   * blip loses log entries and leaves the UI stale forever; send a value the engine cannot serve and it
   * answers `resyncRequired` rather than refusing.
   */
  lastSeq?: number;
  /**
   * Notified when the session dies for a reason the caller did not ask for. A page needs this more than
   * the desktop does: there is no process boundary to notice the engine is gone, so without it a dropped
   * engine leaves the UI showing a stale screen forever — the "silently freezes" risk work item 2's
   * acceptance criterion names.
   */
  onFatal?: (error: Error) => void;
}

/**
 * What the engine answered in `hello`, in the three fields a host acts on.
 *
 * The whole response is parsed for validity (see `handshake`), but only these survive — `capabilities`
 * is a list nothing consults yet, and forwarding it would invite a caller to depend on a shape no test
 * pins.
 */
export interface HelloOutcome {
  sessionId: string;
  /** The engine's build, for the footer. `plan/08`'s "Connected" row asks for engine identity. */
  engineVersion: string;
  /**
   * Present **only** when this was a reconnect whose `lastSeq` the engine could serve: it has retained
   * every event after that point and will deliver them once the client subscribes.
   */
  replayedFrom?: number;
  /**
   * The reconnect asked to be caught up from a `lastSeq` the engine **cannot** serve — the events in
   * between were dropped. `plan/05`: *"`resyncRequired` **must** be handled by every client. Silently
   * ignoring it produces a UI that is permanently wrong in a way nobody can reproduce."* So the host has
   * to re-fetch authoritative state with ordinary commands rather than wait for events that will never
   * arrive.
   */
  resyncRequired: boolean;
}

export interface Connection {
  /** The typed client. `close()` on it is the whole teardown — it closes the transport too. */
  client: BifurcClient;
  /**
   * What the engine said in `hello` — identity, and whether replay happened.
   *
   * Optional because an endpoint with **no auth boundary** has no handshake at all: `hello` is not a
   * registry command, so sending it to an unwrapped server is an `UNKNOWN_COMMAND`. Fabricating a
   * `sessionId` for that case would be a lie the caller could not detect.
   */
  hello?: HelloOutcome;
  /** A real round trip, so "connected" means the engine answered rather than the socket opened. */
  probe(): Promise<unknown>;
  /** Tear the session down. The UI's surface deliberately does not carry this — see `web/api.ts`. */
  close(): Promise<void>;
  /**
   * The highest event `seq` this connection has seen — what the **next** reconnect sends as `lastSeq`.
   *
   * Read from the client rather than tracked here, so there is one high-water mark rather than two that
   * can drift. Non-enumerable on the client, which is why it is reachable here and not from the UI.
   */
  lastSeq(): number;
}

/**
 * Connect, and return a client that is already authenticated.
 *
 * **Async, and that is load-bearing.** The handshake must complete before the first real request, so a
 * host cannot install `window.api` and then connect — it has to connect, and only then render the UI
 * that uses it. That is the whole reason `web/` is a container rather than a mount point.
 */
export async function connectToEngine(opts: ConnectOptions): Promise<Connection> {
  const transport = createBrowserTransport({ url: opts.url, onFatal: opts.onFatal });

  try {
    // Before `createClient()`, per `plan/06` — and only when the endpoint has an auth boundary. An
    // unwrapped endpoint has no handshake at all, which is why the outcome is optional rather than
    // invented: a fabricated `sessionId` would be a lie a caller could not detect.
    const hello =
      opts.token !== undefined && opts.token !== "" ? await handshake(transport, opts) : undefined;

    const client = createClient(transport, { local: createBrowserLocal() });
    const connection: Connection = {
      client,
      ...(hello === undefined ? {} : { hello }),
      /**
       * `config.get` rather than a second handshake: now that the session is authenticated this is an
       * ordinary command, and going through `client` rather than the transport is deliberate — it
       * exercises the client's own correlation and retry layer, not just the wire.
       */
      probe: () => client.getConfig(),
      close: () => client.close(),
      lastSeq: () => client.lastSeq(),
    };

    /**
     * **Prove the session works before handing it back.**
     *
     * This is the round trip that makes "connected" mean something. Without it, connecting with *no*
     * token to an endpoint that requires one succeeds here — the handshake is skipped by design, so
     * nothing has failed yet — and the container would mount the whole UI before the first real request
     * came back `UNAUTHORIZED`. That is precisely the failure that prompted this work, and it is why
     * `classify()`'s `"unauthorized"` arm has to fire on a probe as well as on a handshake.
     */
    await connection.probe();
    return connection;
  } catch (error) {
    /*
     * Close before rethrowing.
     *
     * A refused handshake has already been closed **by the engine** — `UNAUTHORIZED` and `UNSUPPORTED`
     * are identity failures, and `plan/05` work item 5 says the session ends. But the *client's* end is
     * ours, and a transport nobody holds is a socket nobody can close: the form can retry, so every
     * failed attempt would otherwise leave another one behind. `close()` is documented idempotent, so
     * this cannot double-close a transport the failure path already tore down.
     */
    await transport.close();
    throw error instanceof ConnectError ? error : classify(error, opts);
  }
}

/**
 * `hello` on the raw transport, before `createClient()` exists.
 *
 * The response is **parsed**, not cast. `HelloResponseSchema` is the wire contract, and an engine that
 * drifted from it would otherwise surface later as a confusing field-level `undefined` rather than here
 * as a shape mismatch — the same reasoning `createAuthenticatedTransport` gives for validating its own
 * reply.
 */
async function handshake(
  transport: ReturnType<typeof createBrowserTransport>,
  opts: ConnectOptions,
): Promise<HelloOutcome> {
  // No `try`/`catch`: `connectToEngine` owns the single place a failure becomes a `ConnectError` and the
  // single place the transport is closed. Classifying here too would be a second copy of both, and the
  // two would drift the moment one of them learned about a new code.
  const raw = await transport.request(HELLO_ACTION, {
    protocolVersion: PROTOCOL_VERSION,
    clientName: WEB_CLIENT_NAME,
    clientVersion: opts.clientVersion ?? UNKNOWN_CLIENT_VERSION,
    // Spread rather than assigned: both are optional, and `undefined` is not the same as absent —
    // `exactOptionalPropertyTypes` rejects the assignment, and the engine branches on whether `lastSeq`
    // is *present* when deciding whether this is a reconnect or a first connection.
    ...(opts.token === undefined ? {} : { token: opts.token }),
    ...(opts.lastSeq === undefined ? {} : { lastSeq: opts.lastSeq }),
  });

  const parsed = HelloResponseSchema.parse(raw);

  /**
   * Three fields kept, out of a response that is otherwise validated and dropped.
   *
   * `sessionId` and `engineVersion` are identity — `plan/08`'s "Connected" row asks the footer to show
   * engine identity, and it was being thrown away here. `replayedFrom` and `resyncRequired` are the
   * *outcome* of a reconnect, and `plan/05` is explicit that the second must be handled by every client:
   * *"silently ignoring it produces a UI that is permanently wrong in a way nobody can reproduce."*
   *
   * `capabilities` is deliberately not forwarded. Nothing consults it yet, and a caller that started to
   * would be depending on a shape no test pins.
   */
  return {
    sessionId: parsed.sessionId,
    engineVersion: parsed.engineVersion,
    ...(parsed.replayedFrom === undefined ? {} : { replayedFrom: parsed.replayedFrom }),
    resyncRequired: parsed.resyncRequired ?? false,
  };
}

/**
 * Turn a transport failure into something a form can say.
 *
 * The distinction that matters most is `unauthorized` versus `unreachable`. The engine's message for a
 * refused request reads *"the first request must be hello"* — which describes the engine's rule rather
 * than the user's problem, and would send them to check an address that is correct.
 */
function classify(error: unknown, opts: ConnectOptions): ConnectError {
  if (error instanceof EngineError) {
    if (error.code === "UNAUTHORIZED") {
      return new ConnectError(
        "unauthorized",
        opts.token === undefined
          ? "This engine requires a token. Read it from <data-dir>/engine.token on the machine running " +
            "the host, and enter it above."
          : "The engine rejected this token. It is regenerated every time the host starts, so a token " +
            "from a previous run will not work — read the current one from <data-dir>/engine.token.",
        error,
      );
    }
    if (error.code === "UNSUPPORTED") {
      return new ConnectError(
        "version",
        `This engine speaks a different protocol major. ${error.message}`,
        error,
      );
    }
    return new ConnectError("refused", error.message, error);
  }

  return new ConnectError(
    "unreachable",
    error instanceof Error ? error.message : String(error),
    error,
  );
}
