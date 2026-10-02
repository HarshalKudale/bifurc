import { app, BrowserWindow, Tray, Menu, nativeImage, dialog, screen } from "electron";
import * as path from "path";
import { registerIpcHandlers } from "@/ipc/handlers";
import { registerClientHandlers } from "@/ipc/handlers/clientHandlers";
import { loadConfig } from "@bifurc/engine/workspace/config";
import { loadSettings, saveSettings } from "@bifurc/engine/workspace/appSettings";
import { checkGitInstalled } from "@bifurc/engine/workspace/gitStore";
import { bus } from "@bifurc/engine/kernel/eventBus";
import { commandRegistry } from "@bifurc/engine/kernel/registry";
import { setDataRoot, dataDir, platformDefaultDataDir } from "@bifurc/engine/workspace/paths";
import { preflight } from "@bifurc/engine/lifecycle/startup";
import { shutdownEngine } from "@bifurc/engine/lifecycle/shutdown";
import { createWorkspaceHost, type WorkspaceHost } from "@bifurc/engine/runtime/host";


app.commandLine.appendSwitch("disable-gpu-shader-disk-cache");
app.commandLine.appendSwitch("disable-gpu");

let mainWindow: BrowserWindow | null = null;
let tray: Tray | null = null;
let quitting = false;
/**
 * The engine host — the same composition the standalone engine runs (`runtime/host.ts`): forward proxy, edge,
 * blob sweeper, workspace bootstrap and the data directory's socket endpoint. Held so `before-quit` can stop it.
 */
let engineHost: WorkspaceHost | null = null;

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
      // A settings patch through the engine, like any client. The menu rebuilds on `settings.changed`.
      click: (item) => {
        void commandRegistry.invoke("settings.update", { minimizeToTray: item.checked }, { bus });
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
    // On Windows the app's data has always been in %LOCALAPPDATA%\Bifurc (the engine used to hardcode it,
    // ignoring the root it was given). The root is now that directory itself, which is also the default a
    // standalone engine and the CLI use, so all three agree. Elsewhere `userData` was already the root.
    setDataRoot(process.platform === "win32" ? platformDefaultDataDir() : app.getPath("userData"));
    const hasGit = await checkGitInstalled();
    if (!hasGit) {
      dialog.showErrorBox(
        "Git required",
        "Bifurc requires Git to be installed for config versioning and sync.\n\nPlease install Git from https://git-scm.com and restart.",
      );
      app.quit();
      return;
    }

    const settings = loadSettings();

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
      ],
    }).then((checks) => {
      for (const c of checks) {
        if (!c.ok) console.warn(`[preflight] ${c.code}: ${c.message}`);
      }
    });

    // The engine itself — one composition shared with the standalone host (`createWorkspaceHost`): it registers
    // the commands, bootstraps the workspaces, starts the blob sweeper, the forward proxy and the edge, and opens
    // the data directory's socket so the CLI joins this process instead of starting a second engine. Electron's
    // own IPC (`registerIpcHandlers`) is the shell's door to the same registry.
    try {
      engineHost = createWorkspaceHost({
        dataDir: dataDir(),
        engineVersion: app.getVersion(),
        onNotice: (message) => console.log(`[engine] ${message}`),
        onShutdownRequested: () => { quitting = true; app.quit(); },
      });
      await engineHost.start();
    } catch (err) {
      dialog.showErrorBox("Bifurc engine could not start", String(err instanceof Error ? err.message : err));
      app.quit();
      return;
    }

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
  });

  app.on("before-quit", () => {
    quitting = true;
    void engineHost?.stop();
    engineHost = null;
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
