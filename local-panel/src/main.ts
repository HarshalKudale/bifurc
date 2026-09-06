import { app, BrowserWindow, Tray, Menu, nativeImage, dialog, ipcMain, screen } from "electron";
import * as path from "path";
import { processSpawner } from "@/applications/processSpawner";
import { registerIpcHandlers } from "@/ipc/handlers";
import { loadConfig, generateId } from "@/store/config";
import { loadSettings, saveSettings } from "@/store/appSettings";
import { initWorkspaceDir, dataRoot, wsDir } from "@/store/workspaceFs";
import { startServer, stopServer } from "@/proxy/server";
import { startCompanionServer, stopCompanionServer } from "@/companion/companionServer";
import { checkGitInstalled, initWorkspaceRepo } from "@/store/gitStore";
import { startAutoSync, stopAllAutoSync, setAutoSyncReloadFn } from "@/sync/autoSync";
import { getSyncConfig } from "@/sync/syncManager";
import * as fs from "fs";


app.commandLine.appendSwitch("disable-gpu-shader-disk-cache");
app.commandLine.appendSwitch("disable-gpu");

let mainWindow: BrowserWindow | null = null;
let tray: Tray | null = null;
let quitting = false;

function getTitleBarOverlayTheme(themeId: string | null | undefined): { color: string; symbolColor: string } {
  return themeId === "light"
    ? { color: "#eff2f6", symbolColor: "#151b21" }
    : { color: "#090e12", symbolColor: "#eef2f7" };
}

function getAppIcon(): Electron.NativeImage {
  return nativeImage.createFromPath(path.join(__dirname, "..", "icon.png"));
}

function createTray(): void {
  tray = new Tray(getAppIcon());
  tray.setToolTip("Local Panel");
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
      label: "Open Local Panel",
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
        const { saveConfig } = require("@/store/config");
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
    height: 780,
    minWidth: 1400,
    minHeight: 800,
    title: "Local Panel",
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

  processSpawner.setMainWindow(mainWindow);

  mainWindow.on("close", (e) => {
    if (!quitting && loadConfig().minimizeToTray) {
      e.preventDefault();
      mainWindow!.hide();
    }
  });

  mainWindow.on("closed", () => { mainWindow = null; });
}

app.whenReady().then(async () => {
  const hasGit = await checkGitInstalled();
  if (!hasGit) {
    dialog.showErrorBox(
      "Git required",
      "Local Panel requires Git to be installed for config versioning and sync.\n\nPlease install Git from https://git-scm.com and restart.",
    );
    app.quit();
    return;
  }

  let settings = loadSettings();

  // Init dirs/repos for all known workspaces first
  for (const ws of settings.workspaces) {
    initWorkspaceDir(ws.id, ws.name);
    await initWorkspaceRepo(ws.id);
    const syncCfg = getSyncConfig(ws.id);
    if (syncCfg?.autoSync) startAutoSync(ws.id);
  }

  // Validate active workspace — fallback or create new if its dir is missing
  const wsDirExists = (id: string) => {
    try { return fs.statSync(wsDir(id)).isDirectory(); } catch { return false; }
  };
  if (!wsDirExists(settings.activeWorkspaceId)) {
    const valid = settings.workspaces.find((w) => wsDirExists(w.id));
    if (valid) {
      settings.activeWorkspaceId = valid.id;
      saveSettings(settings);
    } else {
      // No valid workspace on disk — create a fresh empty one
      const id = generateId();
      const name = "Workspace 1";
      settings.workspaces = [{ id, name, activeEnvironmentId: null }];
      settings.activeWorkspaceId = id;
      saveSettings(settings);
      initWorkspaceDir(id, name);
      await initWorkspaceRepo(id);
    }
  }

  const { reloadConfig } = require("@/proxy/server");
  setAutoSyncReloadFn(reloadConfig);

  registerIpcHandlers();

  // First-launch IPC
  ipcMain.handle("app:isFirstLaunch", () => {
    const s = loadSettings();
    return !s.hasSeenWelcome;
  });
  ipcMain.handle("app:completeFirstLaunch", () => {
    const s = loadSettings();
    saveSettings({ ...s, hasSeenWelcome: true });
    return { ok: true };
  });

  // Handle second instance on Windows
  app.on("second-instance", () => {
    if (mainWindow) { mainWindow.show(); mainWindow.focus(); }
  });

  createWindow();
  createTray();

  const cfg = loadConfig();
  startServer(cfg.port);
  startCompanionServer(cfg.companionPort ?? 9271);
});

app.on("before-quit", () => {
  quitting = true;
  processSpawner.stopAll();
  stopAllAutoSync();
  stopCompanionServer();
  stopServer();
});


app.on("window-all-closed", () => {
  // Don't quit on window close — tray keeps app alive
});

app.on("activate", () => {
  if (mainWindow === null) createWindow();
  else { mainWindow.show(); mainWindow.focus(); }
});
