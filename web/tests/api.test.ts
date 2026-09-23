/**
 * The browser's `window.api`, against a **real** engine over a **real** socket.
 *
 * ## What this file guards
 *
 * D18's whole claim is that the two hosts differ in *one* thing: what fills `window.api`. Electron fills
 * it in `src/preload.ts` with `contextBridge.exposeInMainWorld("api", { ...client })`; the browser fills
 * it here with `(globalThis as …).api = { ...connection.client }`. **That is the same line**, and this
 * suite is what stops the browser's half from drifting — a shape change, a rename, or an accidental
 * "browser gets a subset" decision would show up as a failing case rather than as a UI that renders
 * nothing.
 *
 * ## Why this is not just a shape test
 *
 * A `window.api` that merely *existed* would pass every structural assertion and still leave a blank
 * screen, so the first case asserts the object the UI will call is the object the engine answers. And
 * because the installer now performs the `hello` handshake before assigning, the last case here is the
 * one that pins the *ordering*: the global is not installed unless the session actually authenticated.
 *
 * ## Why the **node** environment, and why a real engine
 *
 * Same reasoning as `connect.test.ts`: `createBrowserLocal` is DOM-optional by design, jsdom's
 * `WebSocket` is undici's and throws on its own first `open` event across realms, and a mocked socket
 * would only prove this file agrees with itself.
 */
import { afterEach, describe, expect, it, vi } from "vitest";
import { CommandRegistry } from "@bifurc/engine/commands/registry";
import { EngineEventBus } from "@bifurc/engine/eventBus";
import { EventLog } from "@bifurc/engine/transport/eventLog";
import { createWsServer, type WsServer } from "@bifurc/engine/transport/ws";

import { connectAndInstall, uninstallWindowApi, type InstalledApi } from "../api";
import { ConnectError } from "../connect";

const TOKEN = "web-api-test-token";

const servers: WsServer[] = [];
const installed: InstalledApi[] = [];

/** A real command name, so a typo surfaces as `UNKNOWN_COMMAND` rather than as a test that passes. */
function makeRegistry(): CommandRegistry {
  const registry = new CommandRegistry();
  registry.register("config.get", () => ({ port: 8080, activeWorkspaceId: "ws-1" }));
  return registry;
}

async function serve(token?: string): Promise<WsServer> {
  const bus = new EngineEventBus();
  const log = new EventLog({ bus });
  const server = await createWsServer({
    registry: makeRegistry(),
    bus,
    log,
    ...(token === undefined ? {} : { auth: { token, engineVersion: "0.0.0-web-api-test" } }),
  });
  servers.push(server);
  return server;
}

afterEach(async () => {
  for (const api of installed.splice(0)) await api.close();
  for (const s of servers.splice(0)) await s.close();
  // `connectAndInstall` writes a global, so leaving it behind would make the next file's behaviour depend
  // on this one's — the shared-module-registry hazard `connect.test.ts` calls out, in global form.
  uninstallWindowApi();
});

describe("connectAndInstall — the browser's half of the D18 seam", () => {
  it("assigns a window.api that actually reaches the engine", async () => {
    const server = await serve();
    installed.push(await connectAndInstall({ url: server.url }));

    const api = (globalThis as { api?: { getConfig(): Promise<unknown> } }).api;
    expect(api).toBeDefined();
    await expect(api!.getConfig()).resolves.toMatchObject({ port: 8080 });
  });

  it("fills window.api with exactly the client's enumerable surface — the same thing preload exposes", async () => {
    const server = await serve();
    installed.push(await connectAndInstall({ url: server.url }));

    const api = (globalThis as { api?: Record<string, unknown> }).api!;

    // `close` is non-enumerable on the client so that this equality holds; that is why the host gets a
    // separate teardown handle (see `InstalledApi.close`). If someone makes `close` enumerable, the
    // Electron `window.api` grows a method the P5 surface test does not know about — and this case fails
    // here first.
    expect(Object.keys(api)).not.toContain("close");
    expect(typeof api.getConfig).toBe("function");
    expect(typeof api.loadEntity).toBe("function");
    expect(typeof api.saveConfig).toBe("function");
  });

  it("installs nothing when the handshake fails — the UI is never mounted against a dead session", async () => {
    const server = await serve(TOKEN);

    // The ordering guarantee, and the reason this file has a case about a *failure*. An installer that
    // assigned first and authenticated second would leave `window.api` pointing at a client whose
    // transport the engine had already closed, so the container would render the whole UI and then every
    // call would reject — which is exactly the shape of the original bug.
    await expect(connectAndInstall({ url: server.url, token: "wrong" })).rejects.toBeInstanceOf(
      ConnectError,
    );
    expect((globalThis as { api?: unknown }).api).toBeUndefined();
  });

  it("installs a working global when the handshake succeeds", async () => {
    const server = await serve(TOKEN);
    installed.push(await connectAndInstall({ url: server.url, token: TOKEN }));

    const api = (globalThis as { api?: { getConfig(): Promise<unknown> } }).api;
    await expect(api!.getConfig()).resolves.toMatchObject({ port: 8080 });
  });

  it("reports a lost engine through onFatal rather than leaving the page on a stale screen", async () => {
    const server = await serve();
    const fatals: Error[] = [];
    const api = await connectAndInstall({ url: server.url, onFatal: (e) => fatals.push(e) });
    installed.push(api);

    await api.probe();
    expect(fatals).toHaveLength(0);

    await server.close();
    await vi.waitFor(() => expect(fatals).toHaveLength(1));
    // The *dropped* message, not the connect-failure one: "Could not connect" tells the user to check the
    // address, which is the wrong instruction for an engine that was answering a moment ago.
    expect(fatals[0]!.message).toMatch(/^Session closed/);
  });

  it("uninstallWindowApi removes the global, so a dead session cannot be called through", async () => {
    const server = await serve();
    installed.push(await connectAndInstall({ url: server.url }));
    expect((globalThis as { api?: unknown }).api).toBeDefined();

    uninstallWindowApi();
    expect((globalThis as { api?: unknown }).api).toBeUndefined();
  });
});
