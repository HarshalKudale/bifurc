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
 * wrapped. The browser is the first client that has to run the handshake, which is why the gap survived P6.
 *
 * ## Why the handshake is here and not inside `createClient`
 *
 * `plan/06` is explicit — *"the handshake is a plain `request("hello", …)` on the raw transport before
 * `createClient()` — so the client never learns it is remote, which is what P6 needs."* The renderer must
 * not be able to tell a local engine from a remote one, and a client that performed the handshake itself
 * would know. So the composition layer owns it, exactly as
 * `packages/client/tests/transports.test.ts` already does for the `ws` transport.
 *
 * ## The handshake became unconditional on 2026-09-25, and that is the simplification
 *
 * It used to be conditional on a **token**, and the token *was* the discriminator: `HelloRequest.token` was
 * optional in the schema because `in-process` and `stdio` have no boundary, so a token present meant "this
 * endpoint has a boundary, authenticate" and a token absent meant "this one does not". The token is gone —
 * the engine binds loopback and refuses every other address — so the discriminator went with it, and the
 * conclusion it was drawing is now simply true of every endpoint this client can reach: **there is a
 * handshake**.
 *
 * The alternatives were considered and are all worse. A `?token=`-style query flag would be a second, weaker
 * spelling of the same question. Probing for `hello` support behind a `try`/`catch` would make an
 * `UNKNOWN_COMMAND` indistinguishable from a genuine failure, and would hide the one case worth reporting —
 * an endpoint that is not a Bifurc engine at all. Defaulting to *no* handshake would restore the bug at the
 * top of this file against every host that has one.
 *
 * `Connection.hello` is therefore **required** rather than optional, which retires the "fabricated
 * `sessionId`" caveat it used to carry: a caller that gets a `Connection` back can read the engine's
 * identity directly.
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
  /**
   * The engine answered `UNAUTHORIZED` — it refused the session before establishing it.
   *
   * This used to mean "a token is required and none was supplied, or the one supplied is wrong". There is no
   * token now and this client always sends the handshake first, so reaching this means something else is
   * speaking to the endpoint — see `classify()`'s copy.
   */
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
   * **Required since the handshake became unconditional** (2026-09-25). It used to be optional because an
   * endpoint with no handshake would answer `hello` with `UNKNOWN_COMMAND` — `hello` is deliberately not a
   * registry command — so there was a real case with no response to report. This client no longer talks to
   * such an endpoint: it always sends the handshake, and one that is not understood is a failure rather than
   * a mode.
   */
  hello: HelloOutcome;
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
 * Connect, and hand back a client whose session is already established.
 *
 * **Async, and that is load-bearing.** The handshake must complete before the first real request, so a
 * host cannot install `window.api` and then connect — it has to connect, and only then render the UI
 * that uses it. That is the whole reason `web/` is a container rather than a mount point.
 */
export async function connectToEngine(opts: ConnectOptions): Promise<Connection> {
  const transport = createBrowserTransport({ url: opts.url, onFatal: opts.onFatal });

  try {
    // Before `createClient()`, per `plan/06`, and unconditionally — see the note at the top. A refusal here
    // is reported rather than routed around: an `UNKNOWN_COMMAND` means the endpoint is not a Bifurc engine.
    const hello = await handshake(transport, opts);

    const client = createClient(transport, { local: createBrowserLocal() });
    const connection: Connection = {
      client,
      hello,
      /**
       * `config.get` rather than a second handshake: the session is established, so this is an ordinary
       * command — and going through `client` rather than the transport is deliberate, because it exercises
       * the client's own correlation and retry layer rather than just the wire.
       */
      probe: () => client.getConfig(),
      close: () => client.close(),
      lastSeq: () => client.lastSeq(),
    };

    /**
     * **Prove the session works before handing it back.**
     *
     * This is the round trip that makes "connected" mean something. A socket that opens and a handshake that
     * parses are both things a half-broken endpoint can do, and without this the container would mount the
     * whole UI before the first real request failed — which is precisely the failure that prompted this work,
     * and the reason `classify()`'s arms have to fire on a probe as well as on the handshake.
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
    // Spread rather than assigned, because it is optional and `undefined` is not the same as absent:
    // `exactOptionalPropertyTypes` rejects the assignment, and the engine branches on whether `lastSeq`
    // is *present* when deciding whether this is a reconnect or a first connection.
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
        "The engine refused this session. This client always sends the handshake as its first request, " +
          "so reaching this means something else is using the endpoint — another client on the same " +
          "socket, or a proxy that rewrote the first frame.",
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
