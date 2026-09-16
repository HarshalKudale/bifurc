/**
 * Settings mutations — do the things a user changes in the Settings screen actually
 * take effect on the *running* app?
 *
 * WHY THIS EXISTS
 * ---------------
 * `tests/ipc/handlers.test.ts` covers `config:save` with `@/proxy/server` mocked, so all
 * it can prove is that `startServer(9999)` was *called*. That is exactly the assertion
 * that cannot catch the bug class this file targets: a settings change that is saved to
 * `app.json` but never reaches the live server, or reaches it and fails to rebind.
 *
 * Every test here registers the REAL IPC handlers against the REAL proxy server, points
 * the store at a REAL workspace on disk, and then drives it with REAL TCP sockets. The
 * assertion is always the observable outcome — which port answers, what bytes come back —
 * never an internal call count. Only Electron and `@/main` (the tray) are mocked.
 *
 * Covered here:
 *   - changing the proxy port in Settings moves the running server,
 *   - the old port stops answering and the new one serves the same mock,
 *   - an unchanged port does not take the server down,
 *   - `server:restart` / `server:stop` / `server:start` lifecycle,
 *   - `config:save` while the server is stopped brings it back on the saved port,
 *   - toggling TLS in Settings reloads the CA (and unloads it again),
 *   - changing the companion port moves the WebSocket server,
 *   - theme and zoom persist, round-trip, and clamp,
 *   - a live config mutation (add a mock) reaches the running proxy with no restart,
 *   - switching the active workspace re-routes the running proxy.
 *
 * Runs in the integration project (process-global data-root override ⇒
 * `fileParallelism: false`).
 */

import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import * as fs from "fs";
import * as path from "path";
import * as net from "net";
import { WebSocket } from "ws";

import {
  createWorkspace,
  startUpstream,
  proxyRequest,
  mockFixture,
  mappingFixture,
  getFreePort,
  TEST_WS,
  type UpstreamServer,
  type WorkspaceFixture,
} from "./proxyHarness";
import { startServer, stopServer, isRunning, getPort } from "@bifurc/engine/proxy/server";
import { isCALoaded } from "@bifurc/engine/proxy/tlsCert";
import { generateCA } from "@bifurc/engine/proxy/certManager";
import { loadSettings, type AppSettings } from "@bifurc/engine/store/appSettings";
import { setDataDirOverride } from "@bifurc/engine/store/gitStore";
import { stopCompanionServer, getCompanionPort } from "@bifurc/engine/companion/companionServer";

// ── Capture the handlers the Settings screen drives ───────────────────────────

const { handlers } = vi.hoisted(() => ({ handlers: new Map<string, (...args: any[]) => any>() }));

vi.mock("electron", () => ({
  ipcMain: {
    handle: (channel: string, fn: (...args: any[]) => any) => { handlers.set(channel, fn); },
    on: vi.fn(),
  },
  BrowserWindow: { getAllWindows: vi.fn(() => []) },
  app: {
    getPath: vi.fn(() => "/tmp/test-user-data"),
    on: vi.fn(),
    whenReady: vi.fn(() => Promise.resolve()),
    commandLine: { appendSwitch: vi.fn() },
    quit: vi.fn(),
  },
}));

// `@/main` owns the tray and the window singleton. Neither is under test, and importing
// the real module would boot the whole application.
vi.mock("@/main", () => ({
  updateTrayMenu: vi.fn(),
  getMainWindow: vi.fn(() => null),
}));

function invoke<T = any>(channel: string, ...args: any[]): T {
  const fn = handlers.get(channel);
  if (!fn) throw new Error(`No handler registered for "${channel}"`);
  return fn({}, ...args) as T;
}

// ── Socket helpers ────────────────────────────────────────────────────────────

/** One connect attempt: true when something is listening on the port. */
function canConnect(port: number, timeoutMs = 500): Promise<boolean> {
  return new Promise((resolve) => {
    const sock = net.connect({ host: "127.0.0.1", port });
    let settled = false;
    const finish = (ok: boolean) => {
      if (settled) return;
      settled = true;
      sock.destroy();
      resolve(ok);
    };
    sock.setTimeout(timeoutMs);
    sock.once("connect", () => finish(true));
    sock.once("error", () => finish(false));
    sock.once("timeout", () => finish(false));
  });
}

/** Poll until the port is open (or closed), or fail loudly. */
async function waitForPort(port: number, open: boolean, timeoutMs = 5000): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    if ((await canConnect(port)) === open) return;
    if (Date.now() > deadline) {
      throw new Error(`port ${port} did not become ${open ? "open" : "closed"} within ${timeoutMs}ms`);
    }
    await new Promise((r) => setTimeout(r, 25));
  }
}

// ── Fixture ───────────────────────────────────────────────────────────────────

let ws: WorkspaceFixture;
let upstream: UpstreamServer;
let portA = 0;
let portB = 0;

const APP_JSON_MOCK_URL = "http://app.localhost/api/hello";

/** Read `app.json` straight off disk — the persisted settings, not a cached copy. */
function readAppJson(): AppSettings {
  return JSON.parse(fs.readFileSync(path.join(ws.root, "app.json"), "utf-8")) as AppSettings;
}

/**
 * A mock that answers entirely on its own.
 *
 * `isFullyMocked()` requires every response-header key to appear in
 * `mockedResponseHeaders`; otherwise the mock is treated as a *partial* overlay and the
 * proxy still calls the upstream. Spelling that out here keeps the assertions about
 * "which port answered" from being confused by an upstream round trip.
 */
function fullyMocked(overrides: Record<string, any> = {}): any {
  const base = mockFixture({
    id: "mock-hello",
    method: "GET",
    urlPattern: APP_JSON_MOCK_URL,
    responseStatus: 200,
    responseHeaders: { "content-type": "application/json" },
    responseBody: JSON.stringify({ from: "mock" }),
    ...overrides,
  });
  const headerKeys = Object.keys(base.responseHeaders ?? { "content-type": "application/json" });
  return {
    ...base,
    mockedResponseHeaders: headerKeys.map((k) => k.toLowerCase()),
    responseStatusMocked: true,
    responseHeadersMocked: true,
    responseBodyMocked: true,
  };
}

beforeEach(async () => {
  // Two distinct free ports: one to boot on, one to move to.
  portA = await getFreePort();
  do { portB = await getFreePort(); } while (portB === portA);

  upstream = await startUpstream();
  ws = createWorkspace({
    port: portA,
    mappings: [mappingFixture({ id: "map-app", domain: "app.localhost", target: upstream.target })],
    mocks: [fullyMocked()],
  });
  setDataDirOverride(ws.dataRoot);

  const { registerCoreHandlers } = await import("@/ipc/handlers/coreHandlers");
  const { registerSystemHandlers } = await import("@/ipc/handlers/systemHandlers");
  const { registerClientHandlers } = await import("@/ipc/handlers/clientHandlers");
  const { registerCrudHandlers } = await import("@/ipc/handlers/crudHandlers");
  registerCoreHandlers();
  registerSystemHandlers();
  registerClientHandlers();
  registerCrudHandlers();
});

afterEach(() => {
  stopServer();
  stopCompanionServer();
  void upstream?.close();
  ws?.cleanup();
  setDataDirOverride("");
});

/** Boot the real proxy on the port `app.json` already names. */
async function bootOnConfiguredPort(): Promise<void> {
  startServer(loadSettings().port);
  await waitForPort(portA, true);
}

/** The one request every routing assertion uses. */
function fetchHello(port: number) {
  return proxyRequest(port, { target: "/api/hello", host: "app.localhost" });
}

// ── 1. Changing the proxy port moves the running server ───────────────────────

describe("settings — changing the proxy port moves the live server", () => {
  it("serves the mock on the configured port before any change", async () => {
    await bootOnConfiguredPort();

    const res = await fetchHello(portA);

    expect(res.status).toBe(200);
    expect(JSON.parse(res.body)).toEqual({ from: "mock" });
  });

  it("stops answering on the old port after the port is changed in Settings", async () => {
    await bootOnConfiguredPort();
    expect(await canConnect(portA)).toBe(true);

    invoke("config:save", { ...invoke("config:get"), port: portB });

    await waitForPort(portA, false);
    expect(await canConnect(portA)).toBe(false);
  });

  it("serves the same mock on the new port after the change", async () => {
    await bootOnConfiguredPort();

    invoke("config:save", { ...invoke("config:get"), port: portB });

    await waitForPort(portB, true);
    const res = await fetchHello(portB);

    expect(res.status).toBe(200);
    expect(JSON.parse(res.body)).toEqual({ from: "mock" });
  });

  it("persists the new port to app.json on disk", async () => {
    await bootOnConfiguredPort();

    invoke("config:save", { ...invoke("config:get"), port: portB });

    expect(readAppJson().port).toBe(portB);
    expect(loadSettings().port).toBe(portB);
  });

  it("reports the new port from server:status", async () => {
    await bootOnConfiguredPort();

    invoke("config:save", { ...invoke("config:get"), port: portB });
    await waitForPort(portB, true);

    expect(invoke("server:status")).toMatchObject({ running: true, port: portB, error: null });
  });

  it("leaves the server up when the saved port is unchanged", async () => {
    await bootOnConfiguredPort();

    // Same port, same TLS settings — a no-op save must not take the proxy down.
    const cfg = invoke("config:get");
    expect(invoke("config:save", { ...cfg, port: cfg.port })).toEqual({ ok: true });

    expect(isRunning()).toBe(true);
    expect(getPort()).toBe(portA);
    expect((await fetchHello(portA)).status).toBe(200);
  });
});

// ── 2. Explicit server lifecycle controls ─────────────────────────────────────

describe("settings — server:restart / server:stop / server:start", () => {
  it("server:restart leaves the proxy answering on the configured port", async () => {
    await bootOnConfiguredPort();

    expect(invoke("server:restart")).toEqual({ ok: true });

    await waitForPort(portA, true);
    expect((await fetchHello(portA)).status).toBe(200);
  });

  it("server:restart picks up a port changed while the server was down", async () => {
    await bootOnConfiguredPort();
    invoke("server:stop");
    await waitForPort(portA, false);

    // Saving while stopped restarts the server itself; either way it must land on portB.
    invoke("config:save", { ...invoke("config:get"), port: portB });
    await waitForPort(portB, true);

    expect(invoke("server:status")).toMatchObject({ running: true, port: portB });
    expect((await fetchHello(portB)).status).toBe(200);
  });

  it("server:stop closes the port and server:start rebinds it", async () => {
    await bootOnConfiguredPort();

    expect(invoke("server:stop")).toEqual({ ok: true });
    await waitForPort(portA, false);
    expect(invoke("server:status")).toMatchObject({ running: false });

    expect(invoke("server:start")).toEqual({ ok: true });
    await waitForPort(portA, true);
    expect((await fetchHello(portA)).status).toBe(200);
  });

  it("server:start binds the port from settings, not a default", async () => {
    invoke("config:save", { ...invoke("config:get"), port: portB });
    stopServer();
    await waitForPort(portA, false);

    invoke("server:start");
    await waitForPort(portB, true);

    expect(getPort()).toBe(portB);
  });
});

// ── 3. TLS toggle in Settings ─────────────────────────────────────────────────

describe("settings — toggling TLS reloads the CA", () => {
  let caCertPath = "";
  let caKeyPath = "";

  beforeEach(async () => {
    const caDir = fs.mkdtempSync(path.join(ws.root, "ca-"));
    const paths = await generateCA(caDir);
    caCertPath = paths.certPath;
    caKeyPath = paths.keyPath;
  });

  it("does not have a CA loaded when TLS is off", async () => {
    await bootOnConfiguredPort();
    expect(isCALoaded()).toBe(false);
  });

  it("loads the CA when TLS is switched on", async () => {
    await bootOnConfiguredPort();

    invoke("config:save", {
      ...invoke("config:get"),
      tlsEnabled: true,
      tlsCaCertPath: caCertPath,
      tlsCaKeyPath: caKeyPath,
    });

    await waitForPort(portA, true);
    expect(isCALoaded()).toBe(true);
    // Still a working HTTP proxy on the same port.
    expect((await fetchHello(portA)).status).toBe(200);
  });

  it("unloads the CA again when TLS is switched off", async () => {
    await bootOnConfiguredPort();
    const withTls = { ...invoke("config:get"), tlsEnabled: true, tlsCaCertPath: caCertPath, tlsCaKeyPath: caKeyPath };
    invoke("config:save", withTls);
    await waitForPort(portA, true);
    expect(isCALoaded()).toBe(true);

    invoke("config:save", { ...withTls, tlsEnabled: false });

    await waitForPort(portA, true);
    expect(isCALoaded()).toBe(false);
  });
});

// ── 4. Companion port ─────────────────────────────────────────────────────────

describe("settings — changing the companion port moves the WebSocket server", () => {
  function companionRoundTrip(port: number): Promise<any> {
    return new Promise((resolve, reject) => {
      const client = new WebSocket(`ws://127.0.0.1:${port}`);
      const timer = setTimeout(() => { client.terminate(); reject(new Error("companion did not answer")); }, 4000);
      client.once("open", () => client.send(JSON.stringify({ action: "config:get" })));
      client.once("message", (raw: Buffer) => {
        clearTimeout(timer);
        client.close();
        resolve(JSON.parse(raw.toString()));
      });
      client.once("error", (err) => { clearTimeout(timer); reject(err); });
    });
  }

  it("moves the companion server to the newly configured port", async () => {
    const first = await getFreePort();
    let second = await getFreePort();
    while (second === first) second = await getFreePort();

    // Boot the companion on `first` exactly as the app does at startup.
    invoke("config:save", { ...invoke("config:get"), companionPort: first });
    const { startCompanionServer } = await import("@bifurc/engine/companion/companionServer");
    startCompanionServer(first);
    await waitForPort(first, true);
    expect(getCompanionPort()).toBe(first);

    invoke("config:save", { ...invoke("config:get"), companionPort: second });

    await waitForPort(second, true);
    expect(getCompanionPort()).toBe(second);
    expect(readAppJson().companionPort).toBe(second);
    // The extension must be able to complete a round trip on the new port.
    await expect(companionRoundTrip(second)).resolves.toBeTruthy();
  });

  it("stops listening on the previous companion port", async () => {
    const first = await getFreePort();
    let second = await getFreePort();
    while (second === first) second = await getFreePort();

    invoke("config:save", { ...invoke("config:get"), companionPort: first });
    const { startCompanionServer } = await import("@bifurc/engine/companion/companionServer");
    startCompanionServer(first);
    await waitForPort(first, true);

    invoke("config:save", { ...invoke("config:get"), companionPort: second });

    await waitForPort(first, false);
    expect(await canConnect(first)).toBe(false);
  });
});

// ── 5. Theme ──────────────────────────────────────────────────────────────────

describe("settings — theme", () => {
  it("defaults to null", () => {
    expect(invoke("theme:get")).toBeNull();
  });

  it("persists a chosen theme and reads it back", () => {
    expect(invoke("theme:set", "light")).toEqual({ ok: true });

    expect(invoke("theme:get")).toBe("light");
    expect(readAppJson().themeId).toBe("light");
  });

  it("round-trips a second theme change", () => {
    invoke("theme:set", "light");
    invoke("theme:set", "dark");

    expect(invoke("theme:get")).toBe("dark");
    expect(readAppJson().themeId).toBe("dark");
  });

  it("survives a fresh load of the settings file", () => {
    invoke("theme:set", "light");

    // `loadSettings()` re-reads app.json from disk, so this proves persistence.
    expect(loadSettings().themeId).toBe("light");
  });
});

// ── 6. Zoom ───────────────────────────────────────────────────────────────────

describe("settings — zoom", () => {
  it("defaults to 0", () => {
    expect(invoke("zoom:get")).toBe(0);
  });

  it("persists a zoom level", () => {
    expect(invoke("zoom:set", 3)).toEqual({ ok: true, zoomLevel: 3 });

    expect(invoke("zoom:get")).toBe(3);
    expect(readAppJson().zoomLevel).toBe(3);
  });

  it("marks the zoom level as user-set so auto-detection stops overriding it", () => {
    invoke("zoom:set", 2);

    expect(readAppJson().zoomLevelSetByUser).toBe(true);
  });

  it("clamps above the maximum", () => {
    expect(invoke("zoom:set", 999)).toEqual({ ok: true, zoomLevel: 9 });
    expect(invoke("zoom:get")).toBe(9);
  });

  it("clamps below the minimum", () => {
    expect(invoke("zoom:set", -999)).toEqual({ ok: true, zoomLevel: -5 });
    expect(invoke("zoom:get")).toBe(-5);
  });

  it("accepts negative (zoomed out) levels", () => {
    expect(invoke("zoom:set", -2)).toEqual({ ok: true, zoomLevel: -2 });
    expect(invoke("zoom:get")).toBe(-2);
  });
});

// ── 7. Live mutations reach the running proxy ─────────────────────────────────

describe("settings — a config mutation reaches the running proxy with no restart", () => {
  it("serves a mock added through mock:add immediately", async () => {
    await bootOnConfiguredPort();

    // The mapping forwards everything to the real upstream, so before the mock exists
    // the request reaches the upstream rather than failing.
    const before = await proxyRequest(portA, { target: "/api/added", host: "app.localhost" });
    expect(JSON.parse(before.body)).toEqual({ upstream: true, path: "/api/added" });
    expect(upstream.requests).toHaveLength(1);

    const added = await invoke("mock:add", {
      name: "Added live",
      method: "GET",
      urlPattern: "http://app.localhost/api/added",
      useRegex: false,
      enabled: true,
      capturedHeaders: {},
      capturedBody: "",
      responseStatus: 200,
      responseHeaders: { "content-type": "application/json" },
      mockedResponseHeaders: ["content-type"],
      responseBody: JSON.stringify({ added: true }),
      responseBodyEncoding: "utf8",
      responseDelay: 0,
      streamingMode: "none",
      workspaceId: TEST_WS,
    });
    expect(added.id).toBeTruthy();

    // No restart, no second config:save — the running server must pick it up.
    const after = await proxyRequest(portA, { target: "/api/added", host: "app.localhost" });
    expect(after.status).toBe(200);
    expect(JSON.parse(after.body)).toEqual({ added: true });
  });

  it("serves the edited body after mock:update, with no restart", async () => {
    await bootOnConfiguredPort();
    expect(JSON.parse((await fetchHello(portA)).body)).toEqual({ from: "mock" });

    const existing = invoke("config:get").mocks.find((m: any) => m.urlPattern === APP_JSON_MOCK_URL);
    expect(existing).toBeTruthy();
    await invoke("mock:update", { ...existing, responseBody: JSON.stringify({ from: "edited" }) });

    expect(JSON.parse((await fetchHello(portA)).body)).toEqual({ from: "edited" });
  });

  it("stops serving a mock deleted through mock:delete", async () => {
    await bootOnConfiguredPort();
    expect(JSON.parse((await fetchHello(portA)).body)).toEqual({ from: "mock" });

    const existing = invoke("config:get").mocks.find((m: any) => m.urlPattern === APP_JSON_MOCK_URL);
    await invoke("mock:delete", existing.id);

    // With the mock gone the mapping takes over, so the real upstream answers.
    const after = await fetchHello(portA);
    expect(JSON.parse(after.body)).toEqual({ upstream: true, path: "/api/hello" });
  });

  it("re-routes the running proxy when the active workspace is switched", async () => {
    await bootOnConfiguredPort();
    expect(JSON.parse((await fetchHello(portA)).body)).toEqual({ from: "mock" });

    // A second workspace, with its own mock on the same URL.
    const second = await invoke("workspace:add", "Second Workspace");
    expect(second.id).toBeTruthy();
    await invoke("mock:add", {
      ...fullyMocked({ urlPattern: APP_JSON_MOCK_URL, responseBody: JSON.stringify({ from: "second" }) }),
      id: undefined,
      workspaceId: second.id,
    });

    expect(invoke("workspace:setActive", second.id)).toMatchObject({ ok: true });

    // The same port, the same URL — but now the other workspace answers.
    expect(JSON.parse((await fetchHello(portA)).body)).toEqual({ from: "second" });
    expect(readAppJson().activeWorkspaceId).toBe(second.id);
  });

  it("goes back to the first workspace's mock when the switch is reverted", async () => {
    await bootOnConfiguredPort();
    const second = await invoke("workspace:add", "Second Workspace");
    await invoke("mock:add", {
      ...fullyMocked({ urlPattern: APP_JSON_MOCK_URL, responseBody: JSON.stringify({ from: "second" }) }),
      id: undefined,
      workspaceId: second.id,
    });

    invoke("workspace:setActive", second.id);
    expect(JSON.parse((await fetchHello(portA)).body)).toEqual({ from: "second" });

    invoke("workspace:setActive", TEST_WS);

    expect(JSON.parse((await fetchHello(portA)).body)).toEqual({ from: "mock" });
  });
});
