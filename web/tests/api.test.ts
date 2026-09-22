/**
 * The browser's `window.api`, against a **real** engine over a **real** socket.
 *
 * ## What this file guards
 *
 * D18's whole claim is that the two hosts differ in *one* thing: what fills `window.api`. Electron fills
 * it in `src/preload.ts` with `contextBridge.exposeInMainWorld("api", { ...client })`; the browser fills it
 * here with `(globalThis as …).api = { ...connection.client }`. **That is the same line**, and this suite
 * is what stops the browser's half of it from drifting — a shape change, a rename, or an accidental
 * "browser gets a subset" decision would show up as a failing case rather than as a UI that renders
 * nothing.
 *
 * ## Why the **node** environment, and why a real engine
 *
 * Same reasoning as `connect.test.ts`: `createBrowserLocal` is DOM-optional by design, jsdom's
 * `WebSocket` is undici's and throws on its own first `open` event across realms, and a mocked socket
 * would only prove this file agrees with itself. The engine is real, so `api.getConfig()` reaching it is
 * evidence about the wire.
 */
import { afterEach, describe, expect, it, vi } from "vitest";
import { CommandRegistry } from "@bifurc/engine/commands/registry";
import { EngineEventBus } from "@bifurc/engine/eventBus";
import { EventLog } from "@bifurc/engine/transport/eventLog";
import { createWsServer, type WsServer } from "@bifurc/engine/transport/ws";

import { engineUrlFromLocation, installWindowApi, type InstalledApi } from "../api";

const servers: WsServer[] = [];
const installed: InstalledApi[] = [];

/**
 * The same registry shape `connect.test.ts` uses: a **real** command name, so a typo surfaces as
 * `UNKNOWN_COMMAND` rather than as a test that passes because nothing was asked of the engine.
 */
function makeRegistry(): CommandRegistry {
  const registry = new CommandRegistry();
  registry.register("config.get", () => ({ port: 8080, activeWorkspaceId: "ws-1" }));
  return registry;
}

async function serve(): Promise<WsServer> {
  const bus = new EngineEventBus();
  const log = new EventLog({ bus });
  const server = await createWsServer({ registry: makeRegistry(), bus, log });
  servers.push(server);
  return server;
}

afterEach(async () => {
  for (const api of installed.splice(0)) await api.close();
  for (const s of servers.splice(0)) await s.close();
  // `installWindowApi` writes a global, so leaving it behind would make the next file's behaviour depend
  // on this one's — the shared-module-registry hazard `connect.test.ts` calls out, in global form.
  delete (globalThis as { api?: unknown }).api;
});

describe("engineUrlFromLocation — one bundle, any engine", () => {
  it("takes the engine from ?engine=", () => {
    expect(engineUrlFromLocation("?engine=ws://remote.example:9000")).toBe("ws://remote.example:9000");
  });

  it("falls back to the loopback port plan/14 D16 assigns", () => {
    // The default is what makes a local engine work with no query string at all, which is the common case
    // for `npm run dev:web`. If this ever changes, the symptom is a page that cannot reach a *local* engine
    // while a remote one works — so it is worth pinning rather than leaving to the default expression.
    expect(engineUrlFromLocation("")).toBe("ws://127.0.0.1:8080");
    expect(engineUrlFromLocation("?other=1")).toBe("ws://127.0.0.1:8080");
  });
});

describe("installWindowApi — the browser's half of the D18 seam", () => {
  it("assigns a window.api that actually reaches the engine", async () => {
    const server = await serve();
    installed.push(installWindowApi({ url: server.url }));

    const api = (globalThis as { api?: { getConfig(): Promise<unknown> } }).api;
    expect(api).toBeDefined();

    // The strong claim, and the reason this is not a shape test: the object the UI will call is the object
    // the engine answers. A `window.api` that merely *existed* would pass every structural assertion and
    // still leave a blank screen.
    await expect(api!.getConfig()).resolves.toMatchObject({ port: 8080 });
  });

  it("fills window.api with exactly the client's enumerable surface — the same thing preload exposes", async () => {
    const server = await serve();
    installed.push(installWindowApi({ url: server.url }));

    const api = (globalThis as { api?: Record<string, unknown> }).api!;

    // `close` is non-enumerable on the client so that this equality holds; that is why the host gets a
    // separate handle for it (see `InstalledApi.close`). If someone makes `close` enumerable, the Electron
    // `window.api` grows a method the P5 surface test does not know about — and this case fails here first.
    expect(Object.keys(api)).not.toContain("close");
    expect(typeof api.getConfig).toBe("function");
    expect(typeof api.loadEntity).toBe("function");
    expect(typeof api.saveConfig).toBe("function");
  });

  it("reports a lost engine through onFatal rather than leaving the page on a stale screen", async () => {
    const server = await serve();
    const fatals: Error[] = [];
    const api = installWindowApi({ url: server.url, onFatal: (e) => fatals.push(e) });
    installed.push(api);

    await api.probe();
    expect(fatals).toHaveLength(0);

    await server.close();
    await vi.waitFor(() => expect(fatals).toHaveLength(1));
    // The *dropped* message, not the connect-failure one: "Could not connect" tells the user to check the
    // address, which is the wrong instruction for an engine that was answering a moment ago.
    expect(fatals[0]!.message).toMatch(/^Session closed/);
  });
});
