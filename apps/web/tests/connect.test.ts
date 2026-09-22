/**
 * The composition, against a **real** engine over a **real** socket.
 *
 * This is the first test that can exercise the whole web path — the browser transport, the sixteen
 * `RequiredClientLocal` members, and the client's own correlation and retry layer on top of both —
 * because `createClient` requires a `local` and there is no Electron shell here to supply one. A
 * mocked `WebSocket` could assert none of it; it would only prove this file agrees with itself.
 *
 * ## Why the **node** environment, not jsdom
 *
 * `createBrowserLocal` is written to be **DOM-optional**, and its own comments say so: `storage()`
 * returns `undefined` rather than throwing when there is no `localStorage`, `detectPlatform()` falls
 * back to `"unknown"`, `applyZoom()` swallows a missing `document`, and `pickOneFile()` returns `null`.
 * So nothing here needs a document — and reaching for jsdom would actively break the half that matters.
 *
 * Measured, not assumed: under the jsdom environment all three cases fail with
 *
 *     TypeError: The "event" argument must be an instance of Event. Received an instance of Event
 *     ❯ WebSocket.dispatchEvent  node:internal/event_target
 *     ❯ fireEvent  node_modules/jsdom/node_modules/undici/lib/web/websocket/util.js
 *
 * jsdom's `WebSocket` is undici's, and it builds its `Event` objects from a different realm than the
 * `EventTarget` it dispatches them to, so the first `open` event throws. The node environment gives
 * Node's own global `WebSocket`, which is the **production default** on any host that provides one —
 * so this suite drives the real code path rather than an injected stand-in, exactly as
 * `browserTransport.test.ts` does, and `webSocketFactory` stays unused.
 *
 * What it still cannot cover is a real `WebSocket` in a real page on a real browser: that is work item
 * 6's Playwright job, and the owner runs it. This file pins the *wiring*, not the browser.
 */
import { afterEach, describe, expect, it, vi } from "vitest";
import { CommandRegistry } from "@bifurc/engine/commands/registry";
import { EngineEventBus } from "@bifurc/engine/eventBus";
import { EventLog } from "@bifurc/engine/transport/eventLog";
import { createWsServer, type WsServer } from "@bifurc/engine/transport/ws";
import { connectToEngine, type Connection } from "../src/connect";

const servers: WsServer[] = [];
const connections: Connection[] = [];

/** A registry with a real command name, so a typo is `UNKNOWN_COMMAND` rather than a passing test. */
function makeRegistry(): CommandRegistry {
  const registry = new CommandRegistry();
  registry.register("config.get", () => ({ port: 8080, activeWorkspaceId: "ws-1" }));
  return registry;
}

/**
 * Bind a real engine and connect a page to it.
 *
 * An `EventLog` per pair, for the reason the conformance suite gives: Vitest shares a module registry,
 * so asserting against the process-wide singleton would make one test's leak look like the next test's
 * bug.
 */
async function connect(onFatal?: (error: Error) => void) {
  const bus = new EngineEventBus();
  const log = new EventLog({ bus });
  const server = await createWsServer({ registry: makeRegistry(), bus, log });
  servers.push(server);

  const connection = connectToEngine({ url: server.url, onFatal });
  connections.push(connection);
  return { server, connection };
}

afterEach(async () => {
  // `client.close()` is the whole teardown — it closes the hub and the transport — and it is
  // non-enumerable on the client so that `Object.keys(client)` matches `window.api`. It is still
  // reachable by name, which is what the shell (and this suite) relies on.
  for (const c of connections.splice(0)) await c.client.close();
  for (const s of servers.splice(0)) await s.close();
});

describe("the web target's one job — connect a page to an engine", () => {
  it("opens a real socket, issues a real command, and returns the engine's answer", async () => {
    const { connection } = await connect();

    // The engine's own value, through the entire path: native WebSocket -> ClientChannel ->
    // createClientSession -> the typed surface method a screen would call.
    await expect(connection.probe()).resolves.toMatchObject({ port: 8080 });
  });

  it("proves the client-local half is wired, not merely present", async () => {
    const { connection } = await connect();

    // `createBrowserLocal` is passed to `createClient` as `local`. If it were absent or incomplete the
    // construction would throw before any of this ran — so reaching the engine is itself the evidence
    // that the sixteen members satisfy `RequiredClientLocal`. This case asserts the stronger thing:
    // that a *client-side* operation also works in a page, over the same instance.
    await connection.probe();
    const level = await connection.client.setZoomLevel(3);
    expect(level).toEqual({ ok: true });
  });

  it("reports a lost engine through onFatal rather than leaving the page waiting", async () => {
    const fatals: Error[] = [];
    const { server, connection } = await connect((error) => fatals.push(error));

    // Connected first, so this is about a session dying rather than one that never opened.
    await connection.probe();
    expect(fatals).toHaveLength(0);

    await server.close();
    await vi.waitFor(() => expect(fatals).toHaveLength(1));
    // Work item 2 gives "lost the engine" and "could not connect" different UI copy, so the message has
    // to be about the connection rather than a generic failure.
    expect(fatals[0]!.message).toMatch(/closed|connection/i);
  });
});
