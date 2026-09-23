/**
 * The composition, against a **real** engine over a **real** socket.
 *
 * This is the first suite that can exercise the whole web path — the browser transport, the sixteen
 * `RequiredClientLocal` members, the `hello` handshake, and the client's own correlation and retry layer
 * on top of all three — because `createClient` requires a `local` and there is no Electron shell here to
 * supply one. A mocked `WebSocket` could assert none of it; it would only prove this file agrees with
 * itself.
 *
 * ## Why the handshake cases are the important ones
 *
 * The browser shipped without a handshake, and the symptom was a connection that **died immediately**:
 * the engine's `createAuthenticatedTransport` requires `hello` as the first request, answers
 * `UNAUTHORIZED` and closes the session, so the page's first `config.get` came back on a socket that was
 * already gone. Electron never hit it — its transport is `in-process` and is never wrapped — which is why
 * the gap survived P6 and had to be found by running the thing.
 *
 * So the cases below are written to fail if the handshake is removed, not merely to pass while it is
 * present. `connectToEngine` with the right token must succeed against an auth-wrapped server; with no
 * token and with a wrong token it must fail as `"unauthorized"`; and a major mismatch must fail as
 * `"version"`. The third of those is also the one that proves the *classification* works, which is what
 * `plan/08` means by "actionable UI, not generic errors".
 *
 * ## Why the **node** environment, not jsdom
 *
 * `createBrowserLocal` is written to be **DOM-optional**, and its own comments say so: `storage()`
 * returns `undefined` rather than throwing when there is no `localStorage`, `detectPlatform()` falls
 * back to `"unknown"`, `applyZoom()` swallows a missing `document`, and `pickOneFile()` returns `null`.
 * So nothing here needs a document — and reaching for jsdom would actively break the half that matters.
 *
 * Measured, not assumed: under the jsdom environment the connection cases fail with
 *
 *     TypeError: The "event" argument must be an instance of Event. Received an instance of Event
 *     ❯ WebSocket.dispatchEvent  node:internal/event_target
 *     ❯ fireEvent  node_modules/jsdom/node_modules/undici/lib/web/websocket/util.js
 *
 * jsdom's `WebSocket` is undici's, and it builds its `Event` objects from a different realm than the
 * `EventTarget` it dispatches them to, so the first `open` event throws. The node environment gives
 * Node's own global `WebSocket`, which is the **production default** on any host that provides one — so
 * this suite drives the real code path rather than an injected stand-in, exactly as
 * `browserTransport.test.ts` does, and `webSocketFactory` stays unused.
 *
 * What it still cannot cover is a real `WebSocket` in a real page on a real browser: that is work item
 * 6's Playwright job. This file pins the *wiring*, not the browser.
 *
 * **That job is not the owner's alone, and saying so was wrong for two days.** The project's own Playwright
 * is installed and Chromium is already in the cache — at revision 1243, while Playwright 1.60 expects 1223,
 * so one `executablePath` is the whole difference. A headless browser runs from the agent shell without an
 * interactive session, and `plan/08` work item 6 records the recipe. Do not repeat "the owner runs it"
 * without re-measuring: a constraint written into a comment outlives the constraint.
 */
import { afterEach, describe, expect, it, vi } from "vitest";
import { CommandRegistry } from "@bifurc/engine/commands/registry";
import { EngineEventBus } from "@bifurc/engine/eventBus";
import { EventLog } from "@bifurc/engine/transport/eventLog";
import { createWsServer, type WsServer } from "@bifurc/engine/transport/ws";

import { ConnectError, connectToEngine, type Connection } from "../connect";

const TOKEN = "web-connect-test-token";

const servers: WsServer[] = [];
const connections: Connection[] = [];

/** A registry with a real command name, so a typo is `UNKNOWN_COMMAND` rather than a passing test. */
function makeRegistry(): CommandRegistry {
  const registry = new CommandRegistry();
  registry.register("config.get", () => ({ port: 8080, activeWorkspaceId: "ws-1" }));
  return registry;
}

interface ServeOptions {
  /** Omit for an **unauthenticated** server — the shape a bare `ws` endpoint has. */
  token?: string;
  /** Drive the major-mismatch path: the server claims a different protocol major. */
  protocolVersion?: string;
}

/**
 * Bind a real engine.
 *
 * An `EventLog` per server, for the reason the conformance suite gives: Vitest shares a module registry,
 * so asserting against the process-wide singleton would make one test's leak look like the next test's
 * bug.
 */
async function serve(opts: ServeOptions = {}): Promise<WsServer> {
  const bus = new EngineEventBus();
  const log = new EventLog({ bus });
  const server = await createWsServer({
    registry: makeRegistry(),
    bus,
    log,
    ...(opts.token === undefined
      ? {}
      : {
          auth: {
            token: opts.token,
            engineVersion: "0.0.0-web-test",
            ...(opts.protocolVersion === undefined ? {} : { protocolVersion: opts.protocolVersion }),
          },
        }),
  });
  servers.push(server);
  return server;
}

async function connect(opts: {
  url: string;
  token?: string;
  lastSeq?: number;
  onFatal?: (e: Error) => void;
}) {
  const connection = await connectToEngine(opts);
  connections.push(connection);
  return connection;
}

afterEach(async () => {
  // `close()` is the whole teardown — it closes the hub and the transport — and it is non-enumerable on
  // the client so that `Object.keys(client)` matches `window.api`. `web/connect.ts` exposes it for
  // exactly this reason.
  for (const c of connections.splice(0)) await c.close();
  for (const s of servers.splice(0)) await s.close();
});

describe("the web target's one job — connect a page to an engine", () => {
  it("opens a real socket, issues a real command, and returns the engine's answer", async () => {
    const server = await serve();
    const connection = await connect({ url: server.url });

    // The engine's own value, through the entire path: native WebSocket -> ClientChannel ->
    // createClientSession -> the typed surface method a screen would call.
    await expect(connection.probe()).resolves.toMatchObject({ port: 8080 });
  });

  it("proves the client-local half is wired, not merely present", async () => {
    const server = await serve();
    const connection = await connect({ url: server.url });

    // `createBrowserLocal` is passed to `createClient` as `local`. If it were absent or incomplete the
    // construction would throw before any of this ran — so reaching the engine is itself the evidence
    // that the sixteen members satisfy `RequiredClientLocal`. This case asserts the stronger thing: that
    // a *client-side* operation also works in a page, over the same instance.
    await connection.probe();
    const level = await connection.client.setZoomLevel(3);
    expect(level).toEqual({ ok: true });
  });

  it("reports a lost engine through onFatal rather than leaving the page waiting", async () => {
    const fatals: Error[] = [];
    const server = await serve();
    const connection = await connect({ url: server.url, onFatal: (error) => fatals.push(error) });

    // Connected first, so this is about a session dying rather than one that never opened.
    await connection.probe();
    expect(fatals).toHaveLength(0);

    await server.close();
    await vi.waitFor(() => expect(fatals).toHaveLength(1));
    // Work item 2 gives "lost the engine" and "could not connect" different UI copy, so this has to be
    // the *dropped* message and not the connect-failure one: `Could not connect` tells the user to check
    // the address, which is the wrong instruction for an engine that was answering a moment ago.
    expect(fatals[0]!.message).toMatch(/^Session closed/);
    expect(fatals[0]!.message).not.toMatch(/Could not connect/);
    // That is precision, not coverage — measured rather than assumed. Deleting the `open` guard in
    // `channel.ts` (mutation M10) leaves this case **passing**, because an engine's `close()` never
    // reaches the `error` path on this host. The guard itself is pinned in `browserTransport.test.ts`;
    // see TRAPS.md, "A case can be sound and still guard nothing about the line you changed".
  });
});

describe("the hello handshake — the defect that made the browser's connection die on arrival", () => {
  it("authenticates before the first real request, so the session survives", async () => {
    const server = await serve({ token: TOKEN });

    // Without the handshake this is the case that fails: `connectToEngine` probes `config.get`, the
    // decorator refuses it, and the engine closes the session.
    const connection = await connect({ url: server.url, token: TOKEN });

    // Still usable afterwards — a handshake that authenticated and then dropped the session would pass
    // the connect above and fail here.
    await expect(connection.probe()).resolves.toMatchObject({ port: 8080 });
  });

  it("classifies a missing token as `unauthorized`, not as an unreachable host", async () => {
    const server = await serve({ token: TOKEN });

    // The user-visible bug in one case: the address is *correct*, and the engine's own message
    // ("the first request must be hello") describes the engine's rule rather than the user's problem.
    await expect(connect({ url: server.url })).rejects.toMatchObject({ failure: "unauthorized" });
  });

  it("classifies a wrong token as `unauthorized`, and says the token is rotated", async () => {
    const server = await serve({ token: TOKEN });

    const error = await connect({ url: server.url, token: "not-the-token" }).catch((e: unknown) => e);
    expect(error).toBeInstanceOf(ConnectError);
    expect((error as ConnectError).failure).toBe("unauthorized");
    // The actionable half: `ensureEngineToken` generates a token per start, so a token from a previous
    // run is the single most likely cause of this failure and the message has to say so.
    expect((error as ConnectError).message).toMatch(/regenerated/);
  });

  it("classifies a protocol major mismatch as `version`", async () => {
    // The server claims a different major, which is the one thing `checkVersionCompatibility` refuses.
    const server = await serve({ token: TOKEN, protocolVersion: "99.0.0" });

    const error = await connect({ url: server.url, token: TOKEN }).catch((e: unknown) => e);
    expect(error).toBeInstanceOf(ConnectError);
    expect((error as ConnectError).failure).toBe("version");
  });

  it("skips the handshake when no token is given, so a bare ws endpoint still works", async () => {
    // `hello` is deliberately not a registry command, so sending it to an unwrapped server would be an
    // `UNKNOWN_COMMAND` — which is why the handshake is conditional rather than unconditional.
    const server = await serve();
    const connection = await connect({ url: server.url });
    await expect(connection.probe()).resolves.toMatchObject({ port: 8080 });
  });
});

/**
 * Replay — what a reconnect tells the engine, and what comes back.
 *
 * `plan/05` work item 3 exists because *"a brief network blip silently loses log entries and leaves the UI
 * showing stale state forever"*, which the plan calls the most likely source of "works locally, broken
 * remotely" bugs. The decision lives in `eventLog.replayFrom()`, and these cases pin the **client's half**:
 * that `connectToEngine` sends `lastSeq` at all, and that it keeps the answer instead of discarding it.
 *
 * The engine's own suite covers the buffer arithmetic. What was never executed before this — and is the
 * reason these exist — is the *branching* on this side: `lastSeq` was being parsed for validity and thrown
 * away, so `replayedFrom` and `resyncRequired` could not have been observed by any client.
 */
describe("replay — the outcome of a reconnect", () => {
  it("sends no lastSeq on a first connection, so nothing is replayed", async () => {
    const server = await serve({ token: TOKEN });
    const connection = await connect({ url: server.url, token: TOKEN });

    // Absence means "send me only live events", which is what a first connection wants — the schema says
    // so, and the engine branches on `lastSeq` being *present* rather than on its value.
    expect(connection.hello?.replayedFrom).toBeUndefined();
    expect(connection.hello?.resyncRequired).toBe(false);
  });

  it("replays from a lastSeq the engine can serve", async () => {
    const server = await serve({ token: TOKEN });
    // A fresh engine has numbered nothing, so `0` — the honest "I have seen nothing" — is exactly in range
    // (`eventLog.replayFrom`: `oldest === undefined` and `lastSeq === this.seq`).
    const connection = await connect({ url: server.url, token: TOKEN, lastSeq: 0 });

    expect(connection.hello?.replayedFrom).toBe(0);
    expect(connection.hello?.resyncRequired).toBe(false);
  });

  it("asks for a resync when the lastSeq is ahead of anything the engine ever sent", async () => {
    const server = await serve({ token: TOKEN });
    const connection = await connect({ url: server.url, token: TOKEN, lastSeq: 9999 });

    // The engine's rule: a client claiming to have seen events it never sent must **not** get an empty
    // replay, because that would leave it believing it is up to date — a silent staleness bug rather than a
    // visible one. So it is a resync, and `plan/05` requires every client to honour it: *"silently ignoring
    // it produces a UI that is permanently wrong in a way nobody can reproduce."*
    expect(connection.hello?.resyncRequired).toBe(true);
    expect(connection.hello?.replayedFrom).toBeUndefined();
  });

  it("delivers the events it replayed, not just the promise of them", async () => {
    const bus = new EngineEventBus();
    const log = new EventLog({ bus });
    const server = await createWsServer({
      registry: makeRegistry(),
      bus,
      log,
      auth: { token: TOKEN, engineVersion: "0.0.0-web-test" },
    });
    servers.push(server);

    /**
     * A first connection that subscribes and then leaves — because **retention attaches on the first
     * subscription, not on the first emit.** `eventLog.ts`: *"a bus listener for an event is attached the
     * first time anything subscribes to that event and is held until `close()`"*. So an emit before anybody
     * has ever subscribed is neither numbered nor buffered, and a version of this case that emitted first
     * was asserting against a log that had recorded nothing. That is a property of the engine, not a bug:
     * an event nobody could have wanted does not need retaining.
     */
    const first = await connect({ url: server.url, token: TOKEN });
    first.client.onServerError(() => {});
    await first.probe();
    const lastSeq = first.lastSeq();
    await first.close();

    // The client is away and the engine keeps going. This is the event replay exists for.
    bus.emitTyped("server.error", "missed-while-away");

    const second = await connect({ url: server.url, token: TOKEN, lastSeq });
    expect(second.hello?.replayedFrom).toBe(lastSeq);

    // The half a shape assertion cannot reach: **the event actually arrives**, on a subscription
    // established *after* the handshake — which is what a real client does, since the UI subscribes when it
    // mounts, long after `hello`. A replay that only reached a subscription opened before the handshake
    // would be no replay at all.
    const seen: unknown[] = [];
    second.client.onServerError((err) => seen.push(err));
    await vi.waitFor(() => expect(seen.length).toBeGreaterThan(0), { timeout: 5000 });

    expect(seen).toContain("missed-while-away");
  });
});
