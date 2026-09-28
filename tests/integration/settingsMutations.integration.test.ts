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
 * Every test here registers the REAL IPC handlers against the REAL engine, points the store at a REAL
 * workspace on disk, and then drives it with REAL TCP sockets. The assertion is always the observable
 * outcome — which port answers, what bytes come back — never an internal call count. Only Electron and
 * `@/main` (the tray) are mocked.
 *
 * ## Two listeners, and the split is why — `plan/15` work item 3, revised by `plan/17` §1
 *
 * This suite boots **both** the host-level **edge** router and the host-level **forward proxy**, exactly as
 * `packages/engine/src/index.ts` does. Before the split there was one listener and `settings.port` was its
 * port, so every assertion below said "the proxy" and meant the same socket. After the split
 * `settings.port` is the **edge's** port and the two halves answer different questions:
 *
 *   - the **edge** serves the **active** workspace's mappings and re-resolves it *per request*;
 *   - the **proxy** is one host-level listener too, and since `plan/17` §1 it **follows the same pointer** —
 *     it caches its config at `start()` and swaps it on `reloadConfig()`, so a switch re-points it rather
 *     than leaving it serving the workspace it happened to start on.
 *
 * `fetchHello()` is therefore the edge seam, and `forwardToMapped()` the forward one. §7's P14 round trip is
 * the case that reads both, and it is the one that would have been unfalsifiable if the suite had kept
 * reading one port.
 *
 * Covered here:
 *   - `settings.port` names the **edge** and `settings.forwardProxyPort` the **proxy** — different listeners,
 *     different settings,
 *   - **a save re-binds the listener whose port changed, and only that one** (`plan/17` §5 work item 7): the
 *     edge moves when `settings.port` changes, the proxy moves when `settings.forwardProxyPort` changes, and
 *     each leaves the other alone — with the vacated port asserted **dark**, which is the half that makes the
 *     claim falsifiable,
 *   - a port change **persists**, so the next boot agrees with the running process,
 *   - `server.status` reports the proxy's **own** port, not the edge's newly saved one,
 *   - an unchanged port does not take the server down,
 *   - `server.restart` / `server.stop` / `server.start` lifecycle on the proxy's port, and `server.stop`
 *     leaving the edge serving (the asymmetric-teardown negative),
 *   - toggling TLS in Settings reloads the CA (and unloads it again),
 *   - theme and zoom persist, round-trip, and clamp,
 *   - a live config mutation (add a mock) reaches the running server with no restart,
 *   - switching the active workspace re-routes the **edge** and re-points the **proxy**.
 *
 * **Removed, not migrated:** the two companion-port cases. The companion was retired outright in the
 * companion removal (`plan/15` §6.1 superseded by §6.4) — it is not a surface that moved ports, it is a
 * surface that no longer exists. `ws.ts`, which replaced it, has its own suites.
 *
 * Runs in the integration project (process-global data-root override ⇒
 * `fileParallelism: false`).
 */

import { describe, it, expect, beforeAll, beforeEach, afterEach, vi } from "vitest";
import * as fs from "fs";
import * as path from "path";
import * as net from "net";
import { WebSocket } from "ws";

import {
  createWorkspace,
  startUpstream,
  proxyRequest,
  edgeRequest,
  mockFixture,
  mappingFixture,
  getFreePort,
  TEST_WS,
  type UpstreamServer,
  type WorkspaceFixture,
} from "./proxyHarness";
import { startServer, stopServer, isRunning, getPort } from "@bifurc/engine/proxy/server";
import { bindEdgeInstance, createEdgeRouter, type EdgeRouterInstance } from "@bifurc/engine/proxy/edge";
import { isCALoaded } from "@bifurc/engine/proxy/tlsCert";
import { generateCA } from "@bifurc/engine/proxy/certManager";
import { loadSettings, type AppSettings } from "@bifurc/engine/store/appSettings";
import { loadConfig } from "@bifurc/engine/store/config";
import { setDataDirOverride } from "@bifurc/engine/store/gitStore";
import { commandRegistry } from "@bifurc/engine/kernel/registry";
import { bus } from "@bifurc/engine/kernel/eventBus";

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

/**
 * The three `server.*` lifecycle channels, driven directly — the reduction's **fifth** group
 * (`plan/07` §5b-sedecies).
 *
 * `server:restart` / `server:stop` / `server:start` were deleted with the four dead channels in
 * `src/ipc/handlers/systemHandlers.ts` on 2026-09-21. Unlike the earlier groups these bodies were
 * **not** `commandRegistry.invoke()` delegates — they called `startServer` / `stopServer` directly —
 * so the criterion was re-applied per channel against `proxy/serverCommands.ts` rather than
 * inherited. It found exact equivalents, and the commands take no payload, so the translation is
 * positional-argument-free: what the channel did was call the command.
 *
 * A local table rather than the shared `deletedChannelHarness`: this suite drives eleven channels and
 * only these three died, so a harness would have had to model channels the reduction has not reached.
 * Same reasoning as `foldersAndWorkspaces.integration.test.ts`'s folder table.
 */
const DELETED_SERVER_CHANNELS: Record<string, (...args: any[]) => unknown> = {
  "server:restart": () => commandRegistry.invoke("server.restart", {}, { bus }),
  "server:stop": () => commandRegistry.invoke("server.stop", {}, { bus }),
  "server:start": () => commandRegistry.invoke("server.start", {}, { bus }),
  // `server:status` joined this table in the reduction's **sixth** group (`plan/07`
  // §5b-septendecies, 2026-09-21). It was deliberately *not* here after the fifth group, because that
  // group left it live — and that is the whole point of the entry: the two groups took three channels
  // and then a fourth, and a table that had been written as "the three server channels" would have had
  // to be re-derived rather than extended. It takes no payload, so unlike `server.*`'s siblings the
  // positional signature is empty.
  "server:status": () => commandRegistry.invoke("server.status", {}, { bus }),
};

/**
 * The three `coreHandlers.ts` channels this suite drives — the P6 reduction's **sixth** group
 * (`plan/07` §5b-septendecies, 2026-09-21).
 *
 * The mapping is forced by the call style rather than chosen: every call site uses the shell's
 * positional signature (`invoke("workspace:setActive", id)`, `invoke("workspace:add", id, name)`), so
 * this table translates positionals into the commands' **named** payloads. Written out one entry at a
 * time rather than spread generically, because a spread is silently wrong for `workspace:add`:
 * `(id, name)` cannot become `{0: id, 1: name}` and the command would reject it.
 *
 * **Two entries this table deliberately does *not* have, both of which a first draft added wrongly:**
 *
 *   - `config:save` — eleven call sites read `invoke("config:save", …)`, but that channel has not
 *     existed since §5c moved its body into `configCommands.ts`; the call sites resolve through the
 *     `handlers.get()` fallback below because `config.save` is a plain command. Nothing to translate.
 *   - `zoom:*` / `theme:*` — five and three call sites respectively, and they are **live channels**,
 *     registered by `registerClientHandlers()` in this suite's `beforeEach`
 *     (`src/ipc/handlers/clientHandlers.ts:37-71`). They are shell-local behaviour (they touch
 *     `webContents` and the title-bar overlay), not commands, so the reduction has no claim on them —
 *     `plan/07` §5b-decies records `clientHandlers.ts` as **17 live / 0 dead → untouchable**. Mapping
 *     them to invented `theme.get` / `zoom.set` commands fails as `No handler registered for command
 *     "theme.set"`, which is the signal that the entry is wrong: **a routed-surface error naming a
 *     command you just wrote is the registry telling you the command was your idea, not the code's.**
 */
const DELETED_CORE_CHANNELS: Record<string, (args: any[]) => { command: string; payload: any }> = {
  "config:get": () => ({ command: "config.get", payload: {} }),
  "workspace:add": ([name]) => ({ command: "workspace.add", payload: { name } }),
  "workspace:setActive": ([id]) => ({ command: "workspace.setActive", payload: { id } }),
  // `config:save` reaches this table even though its channel died in §5c rather than in the reduction,
  // and the reason is a **shape** mismatch that outlived the name: the call sites pass the config object
  // itself (`invoke("config:save", { ...invoke("config:get"), port: portB })`), while `config.save`
  // takes `{ config }` (`ConfigSaveParams`). So it is the case where a name that predates the reduction
  // still needs a translation line added *by* it. Without one the call falls through to
  // `handlers.get()` and reports `No handler registered for "config:save"`.
  "config:save": ([config]) => ({ command: "config.save", payload: { config } }),
};

function invoke<T = any>(channel: string, ...args: any[]): T {
  const deleted = DELETED_SERVER_CHANNELS[channel];
  if (deleted) return deleted(...args) as T;
  const core = DELETED_CORE_CHANNELS[channel];
  if (core) {
    const { command, payload } = core(args);
    return commandRegistry.invoke(command as never, payload, { bus }) as T;
  }
  const fn = handlers.get(channel);
  if (!fn) throw new Error(`No handler registered for "${channel}"`);
  return fn({}, ...args) as T;
}

/**
 * The collapsed `entity.*` commands, driven directly — the per-kind `mock:*` channels are the step-3
 * reduction's target (`plan/07` §5b-ter) and were only ever one-line `commandRegistry.invoke()`
 * adapters, so this is what the channel did minus the channel.
 *
 * `create` unwraps `{id, entity}` because the channel did (`entityCrudFactory.ts:72`) and the
 * assertions below read the created mock's own fields; without it they would compare against the
 * envelope and pass on `id` alone. Same defect as `plan/07` §5b-quater, one layer up.
 */
const createMock = async <T = any>(entity: any): Promise<T> => {
  const r: any = await commandRegistry.invoke(
    "entity.create",
    { kind: "mocks", entity, workspaceId: entity.workspaceId },
    { bus },
  );
  return ("entity" in r ? r.entity : r) as T;
};
const updateMock = (entity: any) =>
  commandRegistry.invoke("entity.update", { kind: "mocks", entity }, { bus });
const deleteMock = (id: string) =>
  commandRegistry.invoke("entity.delete", { kind: "mocks", id }, { bus });

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
/**
 * The **forward proxy's** port — the one `startServer()` resolved, and the one the Settings screen's
 * "proxy port" no longer names.
 *
 * `portA`/`portB` are the *settings* ports, i.e. the **edge's** after `plan/15` work item 3. This variable
 * is the second listener, and a case that means "the proxy's port" must read it rather than guessing from
 * the settings value; see `bootOnConfiguredPort()`.
 */
let proxyPort = 0;

/** The suite's own edge router — the reverse half, on `settings.port`. Mirrors `runtime/host.ts`. */
let edge: EdgeRouterInstance | null = null;

/**
 * Stop the suite's edge router and forget it. Idempotent, like `stopServer()`.
 *
 * **It unbinds as well as stops, and the unbind is not tidiness** (`plan/17` §5 work item 7). The binding is
 * process-global module state, and this suite is not the only thing in its worker that drives `config.save` —
 * a binding left behind would have a *later* suite's `config.save` re-bind **this** file's dead edge, which
 * fails as a mystery `EADDRINUSE` in a file that never mentions an edge. `facadeBinding.integration.test.ts`'s
 * teardown records the same rule for the proxy and the webhook.
 */
function stopEdgeRouter(): void {
  edge?.stop();
  bindEdgeInstance(null);
  edge = null;
}

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
  /**
   * **Three** distinct free ports now, not two.
   *
   * `plan/17` §1 gives the forward proxy its own setting, so the edge's port, the proxy's port, and the port a
   * case moves a listener *to* are three different numbers — and a case that moved one listener onto another's
   * port would fail as an `EADDRINUSE` that reads like a busy machine.
   *
   * `proxyPort` is chosen **here** rather than inside `bootOnConfiguredPort()` because it is now written into
   * `app.json`, and the fixture is built in this hook. That is what lets the boot helper start the proxy on
   * `settings.forwardProxyPort` — the way the engine does — instead of on a port the setting disagrees with.
   */
  portA = await getFreePort();
  do { portB = await getFreePort(); } while (portB === portA);
  proxyPort = await getFreePort();
  while (proxyPort === portA || proxyPort === portB) proxyPort = await getFreePort();

  upstream = await startUpstream();
  ws = createWorkspace({
    port: portA,
    forwardProxyPort: proxyPort,
    mappings: [mappingFixture({ id: "map-app", domain: "app.localhost", target: upstream.target })],
    mocks: [fullyMocked()],
  });
  setDataDirOverride(ws.dataRoot);

  const { registerCoreKinds } = await import("@bifurc/engine/store/kinds/coreKinds");
  const { registerClientHandlers } = await import("@/ipc/handlers/clientHandlers");
  const { registerCrudKinds } = await import("@bifurc/engine/store/kinds/crudKinds");
  registerCoreKinds();
  registerClientHandlers();
  registerCrudKinds();
  // `registerSystemHandlers()` used to be called here, for the three `server:*` channels. They are
  // gone (P6 reduction, fifth group) and the one channel it still registers — `app:checkUpdate` — is
  // not driven by this suite, so the call was removed with them.
  //
  // `registerCoreHandlers()` became `registerCoreKinds()` on 2026-09-21: the reduction's sixth group
  // deleted its seventeen channels and the file now registers the `environments` kind alone
  // (`plan/07` §5b-septendecies). The call stays because that kind registration is still needed by any
  // suite that drives `entity.*` by hand.
});

// P6 §5c: `config.get` and `workspace.setActive` moved out of `coreHandlers`' module scope into the
// engine, so a suite that registers handler groups by hand has to wire the engine half too — otherwise
// the channel exists with nothing behind it.
//
// **`beforeAll`, not `beforeEach`, and that is not a style choice.** The channel layer is per-test and
// re-registering it is harmless (the electron mock's `handlers` map is keyed by channel, so a second
// `ipcMain.handle` for the same channel just overwrites). The *registry* is the opposite: it is a
// module-level singleton with no `clear()`, and `register()` deliberately throws on a duplicate to
// catch double-registration. Called from `beforeEach` this passes the first test and fails every one
// after it with `Command "config.get" is already registered.` — which is exactly how it was found:
// 29 failures in this file, all from one throw inside the second test's `beforeEach`.
beforeAll(async () => {
  const { commandRegistry } = await import("@bifurc/engine/kernel/registry");
  const { registerCoreCommands } = await import("@bifurc/engine/store/coreCommands");
  registerCoreCommands(commandRegistry);
  // P6 §5c slice 7: the four collapsed `entity.*` commands left `entityCrudFactory.ts`'s module scope.
  // This goes in the `beforeAll` above, **not** next to `registerCrudKinds()` in the `beforeEach` —
  // see the note above for why a registry registration in `beforeEach` kills every test after the
  // first. The channels below are the ones `beforeEach` re-registers harmlessly.
  const { registerEntityCrudCommands } = await import("@bifurc/engine/store/entityCrudCommands");
  registerEntityCrudCommands(commandRegistry);
  // P6 reduction, fifth group: the three `server:*` channels are gone, so `DELETED_SERVER_CHANNELS`
  // invokes the commands directly and this is the only registration they need. Same shape as the
  // `registerFolderCommands()` call the third group added.
  const { registerServerCommands } = await import("@bifurc/engine/proxy/serverCommands");
  registerServerCommands(commandRegistry);
  // P6 reduction, sixth group: **the largest registration repair any group has forced on a suite.**
  // Eight `config.save` call sites read `invoke("config:save", …)`, and that channel has not existed
  // since §5c moved its body into `configCommands.ts` — so they resolve through the `handlers.get()`
  // fallback to a plain command that this suite had never registered. The failure is
  // `No handler registered for "config:save"`: the message names the *old* surface and points the reader
  // at a file that no longer contains it, which is why the rule is to compare this suite's
  // `register*Commands()` set against `src/ipc/handlers.ts` rather than against its own history.
  const { registerConfigCommands } = await import("@bifurc/engine/store/configCommands");
  registerConfigCommands(commandRegistry);
});

afterEach(() => {
  stopServer();
  stopEdgeRouter();
  void upstream?.close();
  ws?.cleanup();
  setDataDirOverride("");
});

/**
 * Boot **both** listeners the way the engine does, on the ports `app.json` already names.
 *
 * ## Why this is two calls and not one — `plan/15` work item 3
 *
 * Before the split this helper was `startServer(loadSettings().port)`, and it was correct: there was one
 * listener and `settings.port` was its port. After the split `settings.port` is the **edge's** port, and
 * `startServer(settings.port)` would be twofold wrong — the edge is not bound at all (so every
 * `app.localhost` case gets the forward proxy's deliberate **400**), and the call names a port the engine
 * itself would hold, which is the `EADDRINUSE`-that-looks-like-a-busy-machine failure `index.ts:265` records.
 *
 * So this mirrors `packages/engine/src/index.ts`'s sequence, which is load-bearing in both directions:
 *
 *   1. the **edge** takes `settings.port` — the migration §6.2 promises, and the reason no mapping is edited;
 *   2. the forward proxy is then bound on a port the edge does **not** hold.
 *
 * **Step 2 uses `getFreePort()`, not `findFreePortFrom(proxyBasePort())`.** The engine scans upward from
 * D16's base because that is where a *browser proxy setting* expects to find it; a test has no such client,
 * and a scan from a shared base is exactly the kind of machine-state dependency that produces the
 * "passed alone, failed in the suite" flake. One free port, asked for once.
 *
 * `resolveConfig` reads `loadConfig()` lazily, per request — the same shape as the engine's and the reason
 * a config change needs no invalidation step the edge has to be told about.
 *
 * Idempotent for the edge: a helper called twice in one case (none does today, but `bootOnConfiguredPort()`
 * is called once per case and `afterEach` is the only teardown) would otherwise leak a listener per call,
 * and the second `edge.start()` would fail with `EADDRINUSE` against the first.
 */
async function bootOnConfiguredPort(): Promise<void> {
  const settings = loadSettings();

  stopEdgeRouter();
  edge = createEdgeRouter({ resolveConfig: () => loadConfig() });
  /**
   * **Bound as well as started — `plan/17` §5 work item 7.** `config.save` re-binds the edge through the
   * module-level binding, and `restartEdge()` is a **no-op** when nothing is bound: `proxy/edge.ts` has no lazy
   * default, unlike the proxy and the webhook. So a suite that created an edge without binding it would leave
   * the entire rebind path unexercised while every case still passed — the "assertion that cannot fail" shape
   * `TRAPS.md` warns about. Mirrors `runtime/host.ts`, which binds its edge the same way.
   */
  bindEdgeInstance(edge);
  edge.start(settings.port);
  await waitForPort(settings.port, true);

  /**
   * **The proxy boots on `settings.forwardProxyPort`, and that is the change `plan/17` §1 makes.**
   *
   * This line used to be `proxyPort = await getFreePort()` under a comment reading *"the proxy's port is
   * **resolved**, not configured"* — D16's scan, which `plan/17` removed. There is no scan now, so a helper
   * that picked its own port would boot the suite into a state the engine **cannot produce**: a proxy listening
   * somewhere `app.json` does not name. `beforeEach` writes `proxyPort` into the fixture instead, and this
   * reads it back, so the suite's proxy is on the port the settings name — as the engine's is.
   *
   * The `EADDRINUSE`-that-looks-like-a-busy-machine failure the old comment guarded against is still guarded
   * against, one level up: `beforeEach` makes `proxyPort`, `portA` and `portB` three distinct free ports.
   */
  startServer(settings.forwardProxyPort);
  await waitForPort(settings.forwardProxyPort, true);
}

/**
 * The one request every routing assertion uses — sent to the **edge**.
 *
 * `app.localhost` is a mapped host, i.e. reverse-half traffic, and the reverse half is `proxy/edge.ts` after
 * work item 3. Sending this to the forward proxy is what produced this suite's `400 to be 200` mass: the
 * forward proxy answers every relative request with a deliberate 400 whose body names the edge. Kept as a
 * one-line seam so a case that *means* the forward half reaches for `proxyRequest()` directly through
 * `forwardToMapped()` below. Exactly one thing here does — §7's P14 case, which asks what the *bound* proxy
 * is serving rather than which workspace the edge would route to, and the two are different questions after
 * work item 3.
 */
function fetchHello(port: number) {
  return edgeRequest(port, { target: "/api/hello", host: "app.localhost" });
}

/**
 * The same URL, sent through the **forward** seam — an absolute-form target naming the mapped host.
 *
 * That is what a browser sends when it has been configured to use this port as its proxy, and it is why the
 * forward half can serve a mapped URL's *mock* without owning the mapping: the mock is matched on the
 * absolute URL, and the mapping is only needed when the mock misses. §7's P14 case needs exactly that
 * distinction — it asks what the bound proxy is serving, not which workspace the edge would route to.
 */
function forwardToMapped(port: number, path: string) {
  return proxyRequest(port, { target: `http://app.localhost${path}`, host: "app.localhost" });
}

// ── 1. `settings.port` names the EDGE, and a save does not move the proxy ─────

/**
 * ## This block has now asserted three different rules, and the third is `plan/17` §5 work item 7
 *
 * **Its first subject** was *"changing the proxy port moves the running server"*: `config:save({port: portB})`
 * took the single listener down off `portA` and rebound it on `portB`. That was correct while there was one
 * listener and `settings.port` was its port.
 *
 * **Its second subject** — `plan/15` work item 3 — was the opposite. `settings.port` had become the **edge's**,
 * so a save that rebound *the proxy* there would either fight the edge for the port or silently move the address
 * every `*.localhost` mapping points at. So the save kept the proxy on its resolved port and the edge did not
 * move. That was correct too, and its cost was stated rather than hidden: *"changing the port in Settings no
 * longer moves this listener."*
 *
 * **Its third subject** is the one below, and it removes that cost. `plan/17` §1 splits the ports by whether a
 * change needs a restart, and only the **companion** does — the edge, the forward proxy and the webhook all
 * re-bind at runtime. So a save now moves **the listener the changed port actually belongs to**:
 *
 *   1. `settings.port` is the edge's, and changing it **moves the edge**. The old port goes dark, which is the
 *      assertion the second rule could not make and this one exists to make;
 *   2. `settings.forwardProxyPort` is the proxy's, and changing *that* moves the proxy — asserted separately,
 *      because one save moving one listener must not be confused with a save moving all of them;
 *   3. a port change **persists**, so the next boot agrees with the running process instead of contradicting it.
 *
 * **What the second rule's block asserted and this one drops:** *"the running proxy is not disturbed by a port
 * save"* survives in spirit — a save that changes the *edge's* port still leaves the proxy alone — but the old
 * blanket phrasing is gone, because there is now a port whose change **does** disturb it.
 */
describe("settings — a save re-binds the listener whose port changed (plan/17 §1)", () => {
  it("serves the mock on the configured port before any change", async () => {
    await bootOnConfiguredPort();

    const res = await fetchHello(portA);

    expect(res.status).toBe(200);
    expect(JSON.parse(res.body)).toEqual({ from: "mock" });
  });

  it("names the edge's port from settings, and the proxy's from getPort()", async () => {
    await bootOnConfiguredPort();

    // The two ports are different listeners, and the whole point of the block is that they are.
    expect(await canConnect(portA)).toBe(true);
    expect(await canConnect(proxyPort)).toBe(true);
    expect(proxyPort).not.toBe(portA);
    expect(getPort()).toBe(proxyPort);
  });

  it("moves the EDGE to the new port, and leaves the forward proxy where it was", async () => {
    await bootOnConfiguredPort();

    invoke("config:save", { ...invoke("config:get"), port: portB });

    /**
     * **`await waitForPort()` before reading anything — and it is not a workaround.**
     *
     * Every `start()` in this engine is fire-and-forget: it hands off to an async `listen()` and returns, which
     * `index.ts`'s own boot records at length. So for one tick after the save the listener is neither up on the
     * new port nor reported as running, and a case that skipped this would see `{running: false}` for a rebind
     * that is about to succeed.
     */
    await waitForPort(portB, true);

    // The save took, and the **edge** moved onto it.
    expect(loadSettings().port).toBe(portB);
    expect((await fetchHello(portB)).status).toBe(200);
    expect(JSON.parse((await fetchHello(portB)).body)).toEqual({ from: "mock" });

    /**
     * **The port it left is dark, and this is the assertion the previous version of this block could not make.**
     *
     * Under `plan/15` the edge deliberately did not move on a save, and the case that followed asserted *that*.
     * `plan/17` §1 reverses the rule, so the discriminating half is now the opposite one — and it is written as
     * `waitForPort(portA, false)` rather than a bare `canConnect()` because a rebind that left the old listener
     * up would otherwise be a race the case could win by accident.
     */
    await waitForPort(portA, false);

    // …and the forward proxy is untouched, because `portB` is not *its* port. Its own port is
    // `settings.forwardProxyPort`, which this save never named.
    expect(getPort()).toBe(proxyPort);
    expect(isRunning()).toBe(true);
  });

  it("moves the forward proxy when forwardProxyPort changes, and leaves the edge alone", async () => {
    await bootOnConfiguredPort();

    let movedProxyPort = await getFreePort();
    while (movedProxyPort === portA || movedProxyPort === proxyPort) movedProxyPort = await getFreePort();

    invoke("config:save", { ...invoke("config:get"), forwardProxyPort: movedProxyPort });
    await waitForPort(movedProxyPort, true);

    /**
     * **The mirror of the case above, and the reason both exist.** A `config.save` that re-bound *everything*
     * on any change would pass the edge case and fail this one; a save that bound nothing would pass neither.
     * One save moving one listener must not be confused with a save moving all of them.
     */
    expect(getPort()).toBe(movedProxyPort);
    expect(loadSettings().forwardProxyPort).toBe(movedProxyPort);
    await waitForPort(proxyPort, false);

    // The edge is on its own setting and did not follow the proxy.
    expect((await fetchHello(portA)).status).toBe(200);
  });

  it("persists the new port to app.json on disk", async () => {
    await bootOnConfiguredPort();

    invoke("config:save", { ...invoke("config:get"), port: portB });

    expect(readAppJson().port).toBe(portB);
    expect(loadSettings().port).toBe(portB);
  });

  it("reports the proxy's own port from server:status, not the edge's newly saved one", async () => {
    await bootOnConfiguredPort();

    invoke("config:save", { ...invoke("config:get"), port: portB });

    /**
     * `port` here is `proxy.getPort()` — the **forward proxy's** port — and the assertion is deliberately
     * `proxyPort` rather than `portB`.
     *
     * `server.status` answers *"where is the proxy listening"*, and after `plan/17` §1 that is not the same
     * question as *"what does `app.json` say"*: the two are separate settings now, and this save changed the
     * **edge's**. A regression that re-pointed this field at `settings.port` would be a UI bug of the worst kind
     * — the Settings screen showing a port that nothing is bound to — and it is exactly what a reader would
     * "fix" this assertion into.
     *
     * **The missing `waitForPort()` is part of the assertion.** Under `plan/15` this case needed one, because
     * the save restarted the proxy on the same port and the restart was asynchronous. It no longer restarts the
     * proxy at all, so the port must be readable **immediately** — adding a wait back would paper over a rebind
     * that should not be happening.
     */
    expect(invoke("server:status")).toMatchObject({ running: true, port: proxyPort, error: null });
  });

  it("leaves the server up when the saved port is unchanged", async () => {
    await bootOnConfiguredPort();

    // Same port, same TLS settings — a no-op save must not take the proxy down.
    const cfg = invoke("config:get");
    expect(invoke("config:save", { ...cfg, port: cfg.port })).toEqual({ ok: true });

    expect(isRunning()).toBe(true);
    expect(getPort()).toBe(proxyPort);
    expect((await fetchHello(portA)).status).toBe(200);
  });
});

// ── 2. Explicit server lifecycle controls ─────────────────────────────────────

/**
 * These drive the **proxy's** port, so they read `proxyPort` — and the distinction is the whole reason the
 * block had to change. Every one of them used to read `portA`, because before the split `portA` *was* the
 * proxy's port. It is the edge's now, so an unchanged case would be asserting that `server.stop()` closes
 * the edge, which it does not and must not (the edge is host-level and outlives any one proxy).
 *
 * The edge is deliberately **not** stopped or restarted by these commands; there is no `edge.stop`. That
 * asymmetry is asserted once, in the last case, as the negative it is.
 */
describe("settings — server.restart / server.stop / server.start (deleted channels)", () => {
  it("server:restart leaves the proxy answering on its resolved port", async () => {
    await bootOnConfiguredPort();

    expect(invoke("server:restart")).toEqual({ ok: true });

    await waitForPort(proxyPort, true);
    expect((await fetchHello(portA)).status).toBe(200);
  });

  it("server:stop closes the proxy's port and leaves the edge serving", async () => {
    await bootOnConfiguredPort();

    expect(invoke("server:stop")).toEqual({ ok: true });
    await waitForPort(proxyPort, false);
    expect(invoke("server:status")).toMatchObject({ running: false });

    /**
     * **The negative this block exists to pin.** `server.stop` is a *proxy* lifecycle command; the edge is
     * the host's and answering mappings has nothing to do with whether one workspace's proxy is up. A
     * regression that reached for a shared teardown — the tempting shape, since both are `http`/`net`
     * listeners owned by the engine — would take the mappings down with the proxy, and the user-visible
     * symptom would be "stopping the proxy broke all my `*.localhost` domains".
     *
     * It also fails loudly in the right way if the *boot helper* were ever changed to bind one listener:
     * `portA` would be dead here and the case says which one it expected to survive.
     */
    expect((await fetchHello(portA)).status).toBe(200);

    expect(invoke("server:start")).toEqual({ ok: true });
    await waitForPort(proxyPort, true);
    expect((await fetchHello(portA)).status).toBe(200);
  });

  it("server:restart picks up a port changed while the server was down", async () => {
    await bootOnConfiguredPort();
    invoke("server:stop");
    await waitForPort(proxyPort, false);

    /**
     * **The proxy's port does not come from `settings.port`, and that is the whole of this case.**
     *
     * Pre-split, a save while stopped was the *only* path that moved a listener: stop, save a new port, start
     * again, and `start` read `incoming.port`. Under `plan/15` that read would have bound the proxy onto the
     * **edge's** future port — an `EADDRINUSE` the moment the engine booted the edge there — so the body was
     * corrected to `getPort() || incoming.port`. `plan/17` §1 removes the ambiguity at the source: the proxy
     * has its own setting, so this save's `port: portB` is the *edge's*, and the proxy comes back on
     * `settings.forwardProxyPort` — the port it was on, and the port `app.json` names for it.
     *
     * **The `!isRunning()` arm is still why it comes back at all**, and it is worth keeping: a save is the
     * natural moment to retry a listener that is down, and the recovery path for a port occupied at boot is
     * exactly this one.
     *
     * The edge moved in the same save — see the block above — so the surviving-mapping assertion reads `portB`.
     */
    invoke("config:save", { ...invoke("config:get"), port: portB });
    await waitForPort(proxyPort, true);
    await waitForPort(portB, true);

    expect(invoke("server:status")).toMatchObject({ running: true, port: proxyPort });
    expect(loadSettings().port).toBe(portB);
    expect((await fetchHello(portB)).status).toBe(200);
  });

  it("server:stop closes the port and server:start rebinds it", async () => {
    await bootOnConfiguredPort();

    expect(invoke("server:stop")).toEqual({ ok: true });
    await waitForPort(proxyPort, false);
    expect(invoke("server:status")).toMatchObject({ running: false });

    expect(invoke("server:start")).toEqual({ ok: true });
    await waitForPort(proxyPort, true);
    expect((await fetchHello(portA)).status).toBe(200);
  });

  it("server:start rebinds the PROXY's port, not the edge's newly saved one", async () => {
    invoke("config:save", { ...invoke("config:get"), port: portB });
    stopServer();

    invoke("server:start");
    await waitForPort(proxyPort, true);

    /**
     * **`portB` is the edge's, so the proxy must not come back on it.**
     *
     * `serverCommands.ts`'s `proxyPortInForce()` is `getPort() || loadConfig().forwardProxyPort`, so **both**
     * arms name the proxy's port and `settings.port` is never consulted — which is the property this case
     * pins, and the one `plan/17` §1 made true by giving the forward proxy its own setting rather than by
     * ordering two reads of the same one.
     *
     * The `getPort()` arm leads, and its retention is asserted in `tests/proxy/server.test.ts`: a proxy that
     * has been started once reports the port it really bound, which is a stronger answer than a setting. The
     * fallback arm covers a process that asks this command before any proxy has started — a path this case
     * does **not** take, since it never calls `bootOnConfiguredPort()`. Both arms are the proxy's port, and
     * that is what `not.toBe(portB)` asserts.
     */
    expect(getPort()).toBe(proxyPort);
    expect(getPort()).not.toBe(portB);
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

    await waitForPort(proxyPort, true);
    expect(isCALoaded()).toBe(true);
    // Still a working HTTP proxy on the same port — the request goes to the edge (a mapped host), and the
    // CA is *process-global* state the proxy owns, so its being loaded is what this asserts.
    expect((await fetchHello(portA)).status).toBe(200);
  });

  it("unloads the CA again when TLS is switched off", async () => {
    await bootOnConfiguredPort();
    const withTls = { ...invoke("config:get"), tlsEnabled: true, tlsCaCertPath: caCertPath, tlsCaKeyPath: caKeyPath };
    invoke("config:save", withTls);
    await waitForPort(proxyPort, true);
    expect(isCALoaded()).toBe(true);

    invoke("config:save", { ...withTls, tlsEnabled: false });

    await waitForPort(proxyPort, true);
    expect(isCALoaded()).toBe(false);
  });
});

// ── 4. (removed) The companion port ───────────────────────────────────────────
//
// The two cases that lived here drove `transport/legacyCompanion.ts`, which was **retired outright** in the
// companion removal (`plan/15` §6.1 superseded by §6.4). They are deleted rather than migrated: the
// companion is not a surface that moved ports, it is a surface that no longer exists, and `ws.ts` — the
// generalised server that replaced it — has its own suites.

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
  it("serves a mock added through entity.create immediately", async () => {
    await bootOnConfiguredPort();

    // The mapping forwards everything to the real upstream, so before the mock exists the request reaches
    // the upstream rather than failing. It is the **edge** that forwards it — `app.localhost` is a mapped
    // host — and the edge reads its config per request, which is what makes the mutation below visible with
    // no restart.
    const before = await edgeRequest(portA, { target: "/api/added", host: "app.localhost" });
    expect(JSON.parse(before.body)).toEqual({ upstream: true, path: "/api/added" });
    expect(upstream.requests).toHaveLength(1);

    const added = await createMock({
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
    const after = await edgeRequest(portA, { target: "/api/added", host: "app.localhost" });
    expect(after.status).toBe(200);
    expect(JSON.parse(after.body)).toEqual({ added: true });
  });

  it("serves the edited body after an update, with no restart", async () => {
    await bootOnConfiguredPort();
    expect(JSON.parse((await fetchHello(portA)).body)).toEqual({ from: "mock" });

    const existing = invoke("config:get").mocks.find((m: any) => m.urlPattern === APP_JSON_MOCK_URL);
    expect(existing).toBeTruthy();
    await updateMock({ ...existing, responseBody: JSON.stringify({ from: "edited" }) });

    expect(JSON.parse((await fetchHello(portA)).body)).toEqual({ from: "edited" });
  });

  it("stops serving a mock after a delete", async () => {
    await bootOnConfiguredPort();
    expect(JSON.parse((await fetchHello(portA)).body)).toEqual({ from: "mock" });

    const existing = invoke("config:get").mocks.find((m: any) => m.urlPattern === APP_JSON_MOCK_URL);
    await deleteMock(existing.id);

    // With the mock gone the mapping takes over, so the real upstream answers.
    const after = await fetchHello(portA);
    expect(JSON.parse(after.body)).toEqual({ upstream: true, path: "/api/hello" });
  });

  /**
   * `plan/17` §1 (2026-09-25) — **the forward proxy now follows the pointer, and this case used to assert the
   * opposite.**
   *
   * ## What changed, and why the old rule was not wrong
   *
   * P14 WI 4 (decided 2026-09-21) dropped `reloadConfig()` from `workspace.setActive`, so the pointer and the
   * proxies were independent. That was correct for its model: there were **N** proxies, one per workspace,
   * each bound to its own — switching the selection moved nothing, because the port belonged to the workspace
   * rather than to the selection. This case asserted exactly that, through the forward seam.
   *
   * `plan/17` §1 deletes the N. There is **one** host-level forward proxy and its whole job is *"which
   * workspace am I looking at?"* — it loads the active workspace's rules and mocks **at start** and **swaps
   * its in-memory config when the active workspace changes**, rather than resolving them per request
   * (`plan/17` §5 work item 3, verbatim: *"Changing workspace changes the point to workspace."*). So
   * `reloadConfig()` is back in `workspace.setActive` (`store/coreCommands.ts`) and this case is **inverted**
   * rather than deleted: it is now the guard for the swap.
   *
   * **The failure that produced this rewrite is the reason it is worth reading.** The old assertion read
   * `{from: "second"}` where it expected `{from: "mock"}` — i.e. the proxy *did* follow the pointer, because
   * the swap had been restored. Neither answer is a bug on its own; which one is right depends entirely on
   * how many listeners there are, which is what §1 decides.
   *
   * ## What the pair of cases now says
   *
   * Both halves follow the active workspace, and the surviving difference is **how** — which is the part
   * worth pinning:
   *
   *   - the **forward proxy** follows by an **explicit swap**: `workspace.setActive` calls `reloadConfig()`,
   *     which re-reads the config this instance dispatches with. It is deliberately *not* per-request.
   *   - the **edge** follows by **re-resolving per request** (`resolveConfig`), which is why it needs no
   *     invalidation step at all.
   *
   * A change that made the proxy resolve per request would not fail this case; a change that **dropped the
   * swap** would, reading `{from: "mock"}`. That is the mutation this case is aimed at.
   */
  it("follows the pointer on the PROXY too — one listener serves the active workspace (plan/17 §1)", async () => {
    await bootOnConfiguredPort();

    // A second workspace, with its own mock on the same URL.
    const second = await invoke("workspace:add", "Second Workspace");
    expect(second.id).toBeTruthy();
    await createMock({
      ...fullyMocked({ urlPattern: APP_JSON_MOCK_URL, responseBody: JSON.stringify({ from: "second" }) }),
      id: undefined,
      workspaceId: second.id,
    });

    expect(invoke("workspace:setActive", second.id)).toMatchObject({ ok: true });

    // The pointer moved…
    expect(readAppJson().activeWorkspaceId).toBe(second.id);

    /**
     * …and the **proxy** moved with it, because `workspace.setActive` reloaded the one host-level proxy's
     * config. Asserted through the **forward** seam, which is the one that answers "what is this listener
     * serving".
     *
     * The request is an absolute-form target for the mapped host, which is what a browser configured to use
     * this port as its proxy sends — the shape the forward half exists for.
     */
    const res = await forwardToMapped(proxyPort, "/api/hello");
    expect(res.status).toBe(200);
    expect(JSON.parse(res.body)).toEqual({ from: "second" });
  });

  it("lets the EDGE follow the active workspace, because that is how it needs no invalidation step", async () => {
    await bootOnConfiguredPort();
    expect(JSON.parse((await fetchHello(portA)).body)).toEqual({ from: "mock" });

    const second = await invoke("workspace:add", "Second Workspace");
    await createMock({
      ...fullyMocked({ urlPattern: APP_JSON_MOCK_URL, responseBody: JSON.stringify({ from: "second" }) }),
      id: undefined,
      workspaceId: second.id,
    });

    invoke("workspace:setActive", second.id);

    /**
     * **`{from: "second"}` is the correct answer here**, and asserting it is the point of the case.
     *
     * §3.3: the edge serves the **active** workspace's mappings, and `resolveConfig` is called per request —
     * which is what `edge.ts` records as the reason `workspace.setActive` needs no invalidation step. A
     * cached active id would keep serving the previous workspace's domains until something remembered to
     * tell it, and there is nothing that would.
     *
     * So this case fails if a future change makes the edge *cache* the active workspace. **The pair used to
     * fail if the two halves *agreed*; since `plan/17` §1 they agree on the outcome and differ only in
     * mechanism** — an explicit `reloadConfig()` swap for the proxy (the case above), per-request resolution
     * for the edge (this one). So what the pair pins is the mechanism, and neither is visible through a
     * single seam. That is why there are two cases and not one.
     */
    const res = await fetchHello(portA);
    expect(res.status).toBe(200);
    expect(JSON.parse(res.body)).toEqual({ from: "second" });
  });

  it("returns the edge to the first workspace when the pointer is switched back", async () => {
    await bootOnConfiguredPort();
    const second = await invoke("workspace:add", "Second Workspace");
    await createMock({
      ...fullyMocked({ urlPattern: APP_JSON_MOCK_URL, responseBody: JSON.stringify({ from: "second" }) }),
      id: undefined,
      workspaceId: second.id,
    });

    invoke("workspace:setActive", second.id);
    // The edge follows the pointer out…
    expect(JSON.parse((await fetchHello(portA)).body)).toEqual({ from: "second" });

    invoke("workspace:setActive", TEST_WS);

    // …and back again, on the same port and with no restart. The pair is the round trip; the single-hop
    // case above is the direction. If the edge cached anything, exactly one of the two would fail.
    expect(JSON.parse((await fetchHello(portA)).body)).toEqual({ from: "mock" });
    expect(readAppJson().activeWorkspaceId).toBe(TEST_WS);
  });
});
