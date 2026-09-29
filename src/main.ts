import { app, BrowserWindow, Tray, Menu, nativeImage, dialog, screen } from "electron";
import * as path from "path";
import { registerIpcHandlers } from "@/ipc/handlers";
import { registerClientHandlers } from "@/ipc/handlers/clientHandlers";
import { loadConfig } from "@bifurc/engine/workspace/config";
import { loadSettings, saveSettings } from "@bifurc/engine/workspace/appSettings";
import { startServer, bindProxyInstance, createProxyServer } from "@bifurc/engine/proxy/server";
import { bindEdgeInstance, createEdgeRouter, type EdgeRouterInstance } from "@bifurc/engine/reverseproxy/edge";
import { bindWebhookInstance, createWebhookServer } from "@bifurc/engine/webhooks/webhookServer";
import { checkGitInstalled } from "@bifurc/engine/workspace/gitStore";
import { bus } from "@bifurc/engine/kernel/eventBus";
import { setDataRoot, dataDir } from "@bifurc/engine/workspace/paths";
import { preflight, bootstrapWorkspaces } from "@bifurc/engine/lifecycle/startup";
import { shutdownEngine } from "@bifurc/engine/lifecycle/shutdown";


app.commandLine.appendSwitch("disable-gpu-shader-disk-cache");
app.commandLine.appendSwitch("disable-gpu");

let mainWindow: BrowserWindow | null = null;
let tray: Tray | null = null;
let quitting = false;
/**
 * The edge router, held so `before-quit` can close it (`plan/15` work item 3).
 *
 * Module-level for the same reason `tray` is: it is created inside `whenReady()` and released from a
 * different handler. Without this the edge's listening socket would outlive the shutdown the rest of the
 * engine performs — and an open listening handle is exactly the kind of thing that keeps a process alive.
 */
let edgeRouter: EdgeRouterInstance | null = null;

function getTitleBarOverlayTheme(themeId: string | null | undefined): { color: string; symbolColor: string } {
  return themeId === "light"
    ? { color: "#eff2f6", symbolColor: "#151b21" }
    : { color: "#090e12", symbolColor: "#eef2f7" };
}

/**
 * Resolve a bundled icon file.
 *
 * Packaged builds ship the icons as extra resources (see `extraResources` in
 * package.json), so they live under `process.resourcesPath`. In dev they are
 * read from the build-resources folder.
 */
function iconPath(file: string): string {
  return app.isPackaged
    ? path.join(process.resourcesPath, file)
    : path.join(__dirname, "..", "build", file);
}

/**
 * Load an icon, falling back to the executable's embedded icon. This guarantees
 * the tray is never created from an empty NativeImage, which renders as a
 * blank / invisible tray slot.
 */
function loadIcon(file: string): Electron.NativeImage {
  const image = nativeImage.createFromPath(iconPath(file));
  if (!image.isEmpty()) return image;
  return nativeImage.createFromPath(process.execPath);
}

function getAppIcon(): Electron.NativeImage {
  return loadIcon("icon.png");
}

function getTrayIcon(): Electron.NativeImage {
  return loadIcon("tray-icon.png");
}

function createTray(): void {
  tray = new Tray(getTrayIcon());
  tray.setToolTip("Bifurc");
  updateTrayMenu();

  tray.on("double-click", () => {
    if (mainWindow) {
      mainWindow.show();
      mainWindow.focus();
    }
  });
}

export function getMainWindow(): BrowserWindow | null {
  return mainWindow;
}

export function updateTrayMenu(): void {
  if (!tray) return;
  const cfg = loadConfig();
  const menu = Menu.buildFromTemplate([
    {
      label: "Open Bifurc",
      click: () => {
        if (mainWindow) { mainWindow.show(); mainWindow.focus(); }
        else createWindow();
      },
    },
    { type: "separator" },
    {
      label: "Minimize to Tray on Close",
      type: "checkbox",
      checked: cfg.minimizeToTray,
      click: (item) => {
        const current = loadConfig();
        const { saveConfig } = require("@bifurc/engine/workspace/config");
        saveConfig({ ...current, minimizeToTray: item.checked });
        updateTrayMenu();
      },
    },
    { type: "separator" },
    {
      label: "Quit",
      click: () => {
        quitting = true;
        app.quit();
      },
    },
  ]);
  tray.setContextMenu(menu);
}

/** Base titlebar overlay height at zoom level 0 */
const BASE_TITLEBAR_HEIGHT = 35;

/** Compute the overlay height adjusted for the current zoom level */
function titleBarHeightForZoom(zoomLevel: number): number {
  return Math.round(BASE_TITLEBAR_HEIGHT * Math.pow(1.2, zoomLevel));
}

/** Update the titlebar overlay height to match the current zoom level */
function syncTitleBarOverlay(win: BrowserWindow, zoomLevel: number, themeId: string | null | undefined): void {
  if (win.isDestroyed()) return;
  try {
    const { color, symbolColor } = getTitleBarOverlayTheme(themeId);
    win.setTitleBarOverlay({
      color,
      symbolColor,
      height: titleBarHeightForZoom(zoomLevel),
    });
  } catch { /* setTitleBarOverlay not supported on all platforms */ }
}

/**
 * Compute a default zoom level based on the primary display's logical resolution.
 * High-DPI scaling is already handled by the OS (scaleFactor), so this targets
 * the logical work-area size to keep UI elements comfortably sized.
 */
function computeDefaultZoomForDisplay(): number {
  const display = screen.getPrimaryDisplay();
  const { width, height } = display.workAreaSize;
  const scaleFactor = display.scaleFactor;

  // The effective physical resolution
  const physicalWidth = width * scaleFactor;

  // On standard 1080p (1920x1080 @ 100%) → zoom 0
  // On 1440p (2560x1440 @ 100%) → slight zoom up
  // On 4K (3840x2160 @ 100%) → larger zoom up
  // If OS DPI scaling is applied (e.g., 4K @ 150%), logical res is smaller → zoom stays low
  if (physicalWidth >= 3840 && scaleFactor <= 1.0) return 2;
  if (physicalWidth >= 3840 && scaleFactor <= 1.25) return 1;
  if (physicalWidth >= 2560 && scaleFactor <= 1.0) return 1;
  if (physicalWidth >= 2560 && scaleFactor <= 1.25) return 0.5;
  return 0;
}

function createWindow(): void {
  const settings = loadSettings();
  const initialOverlayTheme = getTitleBarOverlayTheme(settings.themeId);

  mainWindow = new BrowserWindow({
    width: 1400,
    height: 850,
    minWidth: 1400,
    minHeight: 850,
    title: "Bifurc",
    icon: getAppIcon(),
    backgroundColor: initialOverlayTheme.color,
    webPreferences: {
      preload: path.join(__dirname, "preload.js"),
      contextIsolation: true,
      nodeIntegration: false,
    },
    show: false,
    titleBarStyle: "hidden",
    titleBarOverlay: {
      color: initialOverlayTheme.color,
      symbolColor: initialOverlayTheme.symbolColor,
      height: BASE_TITLEBAR_HEIGHT,
    },
  });

  mainWindow.loadFile(path.join(__dirname, "renderer/index.html"));
  mainWindow.once("ready-to-show", () => {
    const currentSettings = loadSettings();
    // Use persisted zoom if user has set one, otherwise compute from display
    let zoom = currentSettings.zoomLevel ?? 0;
    if (zoom === 0 && !currentSettings.zoomLevelSetByUser) {
      zoom = computeDefaultZoomForDisplay();
      // Persist the computed default so it's consistent across restarts
      saveSettings({ ...currentSettings, zoomLevel: zoom });
    }
    mainWindow!.webContents.setZoomLevel(zoom);
    syncTitleBarOverlay(mainWindow!, zoom, loadSettings().themeId);
    mainWindow!.show();
  });

  // ── Zoom shortcuts (Ctrl+=/Ctrl+-/Ctrl+0) ──────────────────────────────────
  mainWindow.webContents.on("before-input-event", (_event, input) => {
    if (input.type !== "keyDown") return;
    const ctrl = input.control && !input.alt && !input.meta;
    if (!ctrl) return;

    let newLevel: number | null = null;
    const current = mainWindow!.webContents.getZoomLevel();

    if (input.key === "=" || input.key === "+") {
      // Zoom in
      newLevel = Math.min(current + 0.5, 9);
    } else if (input.key === "-") {
      // Zoom out
      newLevel = Math.max(current - 0.5, -5);
    } else if (input.key === "0") {
      // Reset zoom
      newLevel = 0;
    }

    if (newLevel !== null && newLevel !== current) {
      mainWindow!.webContents.setZoomLevel(newLevel);
      syncTitleBarOverlay(mainWindow!, newLevel, loadSettings().themeId);
      const s = loadSettings();
      saveSettings({ ...s, zoomLevel: newLevel, zoomLevelSetByUser: true });
    }
  });

  mainWindow.on("close", (e) => {
    if (!quitting && loadConfig().minimizeToTray) {
      e.preventDefault();
      mainWindow!.hide();
    }
  });

  mainWindow.on("closed", () => { mainWindow = null; });
}

const gotTheLock = app.requestSingleInstanceLock();

if (!gotTheLock) {
  app.quit();
} else {
  app.on("second-instance", () => {
    if (mainWindow && !mainWindow.isDestroyed()) {
      if (mainWindow.isMinimized()) mainWindow.restore();
      mainWindow.show();
      mainWindow.focus();
    } else {
      createWindow();
    }
  });

  app.whenReady().then(async () => {
    // P2 work item 4: the engine-side store modules (appSettings.ts, workspaceFs.ts) are now
    // Electron-free and resolve their paths via `dataDir()`. This is the one place the shell
    // hands them Electron's own `userData` path, preserving current behaviour exactly.
    setDataRoot(app.getPath("userData"));

    const hasGit = await checkGitInstalled();
    if (!hasGit) {
      dialog.showErrorBox(
        "Git required",
        "Bifurc requires Git to be installed for config versioning and sync.\n\nPlease install Git from https://git-scm.com and restart.",
      );
      app.quit();
      return;
    }

    let settings = loadSettings();

    // Non-fatal headless-startup diagnostics (data dir writable, ports free, mkcert usable) —
    // see packages/engine/src/startup.ts (P2 work item 6). Logged only: the shell has always tolerated port
    // conflicts (the server reports its own bind failure via bus.emit("server.error", ...)), so
    // this adds visibility without changing existing behaviour.
    preflight({
      dataDir: dataDir(),
      ports: [
        // `settings.port` is the **edge**'s port now (`plan/15` work item 3), which is why the name
        // changed rather than the number. The forward proxy's port is deliberately absent: it is resolved
        // after this check, from D16's base, and reporting a value that has not been chosen yet would be a
        // preflight result about a different port than the one the proxy will use.
        { name: "edge", port: settings.port },
        { name: "webhook", port: settings.webhookPort ?? 9101 },
      ],
    }).then((checks) => {
      for (const c of checks) {
        if (!c.ok) console.warn(`[preflight] ${c.code}: ${c.message}`);
      }
    });

    // P2 work item 6: the workspace bootstrap (create each workspace's dirs + git repo, start
    // auto-sync where configured, then repair or create the active workspace) now lives in the
    // engine — see `bootstrapWorkspaces()` in `packages/engine/src/startup.ts` — so the CLI and Docker entrypoints
    // run the identical sequence. It returns the effective settings rather than mutating ours.
    settings = (await bootstrapWorkspaces(settings)).settings;

    registerIpcHandlers();
    registerClientHandlers();

    // Bridge the engine bus's settings-change notification to the shell's own tray update.
    // It lives here, beside the tray it updates, rather than in `@/ipc/handlers` — that would be an
    // `@/main` <-> `@/ipc/handlers` import cycle. (This used to cite `eventBridge.ts` as the
    // alternative home; step 3c deleted that file, and the cycle argument is the one that stands.)
    //
    // Load-bearing for `config.save`: the registry handler no longer calls `updateTrayMenu()` itself,
    // so this subscription is the only thing that keeps the tray in step with the settings.
    bus.onTyped("settings.changed", () => updateTrayMenu());

    createWindow();
    createTray();

    // `plan/17` §1 — the desktop composes the **same two host-level listeners** the deployable host does,
    // and binds the module facades to them. Before this the desktop built a `WorkspaceRuntime` and a host
    // built N of them; that abstraction is deleted, so the two compositions are now the same composition at
    // different scopes rather than two different shapes.
    //
    // The facades are **bound** rather than replaced, and that distinction still matters for the reason P14
    // WI 7 recorded: 23 engine modules import `reloadConfig()` from `proxy/server.ts`, so removing the facade
    // would leave them updating a lazily created *phantom* instance whose config the running proxy never
    // sees. Binding keeps the facade meaning "the server, wherever it lives" and changes nothing for those
    // callers.
    //
    // `workspaceId: null` is now right for the **opposite** reason it was before. P14 chose it because
    // `workspace.setActive` had stopped calling `reloadConfig()`, so a facade resolving by `activeWorkspaceId`
    // would have looked up a workspace the proxy was not serving. `plan/17` §1 restores that call — the
    // forward proxy **follows** the active workspace, which is what a host-level listener means — so `null`
    // is right because following the pointer is now the intended behaviour rather than a hazard.
    const cfg = loadConfig();

    /**
     * ## The proxy split — `plan/15` work item 3 — and what `plan/17` changed about the port
     *
     * `cfg.port` used to be the one socket that served both halves. It now belongs to the **edge**, and
     * that direction is the migration rather than a preference: `settings.port` is where every existing
     * `*.localhost` mapping already points, so handing it to the edge is what keeps those URLs working
     * (`plan/15` §6.2) with no edit to a single mapping. The **forward** proxy moves instead — its port is
     * one line in a browser's proxy settings, against one edit per mapping, which the user cannot make.
     *
     * `cfg.port` comes from `app.json`, which is **host-local** (`store/appSettings.ts`), so this is not
     * the git-tracked config carrying one machine's port to another.
     *
     * **`plan/17` §1 replaced the scan with a setting.** This used to call `findFreePortFrom(proxyBasePort(),
     * …)` — D16's base-and-scan — which meant the port a browser had to be pointed at could move on its own.
     * It is now `settings.forwardProxyPort` (`8080` by default, `BIFURC_PROXY_BASE_PORT` to pin it in a
     * container), bound exactly as configured: *"It binds `8080` and **fails if occupied**, naming the port,
     * so the user enters a working one instead of the app silently incrementing."* A busy port is reported by
     * `server.status`'s `error` field and by the proxy's own `[server:<port>]` log line — the engine's oldest
     * bind contract, and the same one the edge uses.
     */
    const edgePort = cfg.port;

    const forwardProxy = createProxyServer({ workspaceId: null });
    bindProxyInstance(forwardProxy);
    /**
     * The webhook receiver is created but **not started** — `plan/17` §1 makes it off by default, and the
     * desktop has never opened that port at boot (`WebhooksPanel` → `window.api.startWebhookServer()` is what
     * binds it). Creating it here is what gives the facade one authority to point at rather than a lazily
     * created default that would become a second one.
     */
    bindWebhookInstance(createWebhookServer({ workspaceId: null }));

    /**
     * The edge, on the port the mappings use.
     *
     * `resolveConfig` reads the **bound** proxy's dispatch config rather than loading the config again.
     * The edge routes the same workspace's mappings, and a second reader of the same files is how the two
     * halves drift — `store/serverUtils.ts#workspaceCfg()` is the record of what that cost. Read per
     * request, so a `workspace.setActive` needs no invalidation step on the edge's side.
     *
     * No `bindHost`: the desktop's edge is loopback, exactly as its proxy has always been. That is no longer
     * a *choice this module is declining* — since 2026-09-25 loopback is the only value the edge's guard
     * accepts, for the shell and for the deployable host alike, and `--edge-bind-host` is a fatal flag. See
     * `reverseproxy/edge.ts`.
     */
    edgeRouter = createEdgeRouter({ resolveConfig: () => forwardProxy.dispatchConfig() });
    /**
     * ## `plan/17` §5 work item 7 — and this is the desktop's half of it
     *
     * `createEngine()` binds its own edge, but **this module is the desktop's composition and does not call
     * `createEngine()`** — it builds the proxy, the webhook and the edge itself, which is exactly why it
     * already calls `bindProxyInstance()` and `bindWebhookInstance()` a few lines above. The edge had no
     * equivalent call, so `config.save` could not reach it *here*: `restartEdge()` would have been a no-op, and
     * *"changing the port in Settings"* would have kept its pre-`plan/17` meaning — persisted immediately, in
     * force only at the next launch.
     *
     * **Bound before `start()`**, matching `index.ts` and `runtime/host.ts`: a save arriving in the boot window
     * should find the instance rather than silently skip the re-bind. `before-quit` unbinds it, because this is
     * process-global module state and a leaked binding outlives the listener it names.
     */
    bindEdgeInstance(edgeRouter);
    edgeRouter.start(edgePort);

    // Delegating to the bound instance above — `startServer()` acts on whatever `bindProxyInstance()` was
    // given, so this is the same call it always was.
    startServer(cfg.forwardProxyPort);
  });

  app.on("before-quit", () => {
    quitting = true;
    // The edge first, and synchronously: it is this module's own listener, and a request arriving during
    // teardown should find a closed edge rather than a live edge in front of the proxies `shutdownEngine()`
    // is about to stop. `EdgeRouterInstance.stop()` owns no async handle, so there is nothing to await.
    edgeRouter?.stop();
    // The binding goes with it — process-global module state, so it is released rather than left pointing at a
    // stopped listener. The same rule `bindProxyInstance()` / `bindWebhookInstance()` follow; see
    // `bindEdgeInstance()` for why a leak here is paid for by whatever runs next in the process, not by this one.
    bindEdgeInstance(null);
    // Idempotent by design (P2 work item 3): the shell quitting, a supervisor restarting a crashed
    // engine and a SIGTERM handler may all ask for this, in any order. See `packages/engine/src/shutdown.ts`.
    void shutdownEngine();
  });

  app.on("window-all-closed", () => {
    // Don't quit on window close — tray keeps app alive
  });

  app.on("activate", () => {
    if (mainWindow === null) createWindow();
    else { mainWindow.show(); mainWindow.focus(); }
  });
}
