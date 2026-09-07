import * as fs from "fs";
import * as path from "path";
import { app } from "electron";

export interface WorkspaceSyncConfig {
  remote: string;
  branch: string;
  autoSync: boolean;
}

export interface WorkspaceSyncMeta {
  lastPushedAt: number | null;
  lastPulledAt: number | null;
  lastSyncedCommit: string | null;
}

export interface WorkspaceMeta {
  id: string;
  name: string;
  activeEnvironmentId: string | null;
  syncConfig?: WorkspaceSyncConfig | null;
  syncMeta?: WorkspaceSyncMeta | null;
}

export interface AppSettings {
  port: number;
  webhookPort: number;
  companionPort: number;
  minimizeToTray: boolean;
  tlsEnabled: boolean;
  tlsCaCertPath: string | null;
  tlsCaKeyPath: string | null;
  workspaces: WorkspaceMeta[];
  activeWorkspaceId: string;
  /** False on first ever launch — renderer shows the welcome/login screen */
  hasSeenWelcome: boolean;
  /** Zoom level for the application window (0 = default, positive = zoomed in, negative = zoomed out) */
  zoomLevel: number;
  /** Whether the user has manually set the zoom level (disables auto-detection from display) */
  zoomLevelSetByUser: boolean;
  /** Selected UI theme id (see renderer/lib/themes.ts). Null = use the built-in default. */
  themeId: string | null;
}

function generateId(): string {
  return Date.now().toString(36) + Math.random().toString(36).slice(2, 7);
}

function makeDefaultSettings(): AppSettings {
  const id = generateId();
  return {
    port: 80,
    webhookPort: 9101,
    companionPort: 9271,
    minimizeToTray: true,
    tlsEnabled: false,
    tlsCaCertPath: null,
    tlsCaKeyPath: null,
    workspaces: [{ id, name: "Workspace 1", activeEnvironmentId: null }],
    activeWorkspaceId: id,
    hasSeenWelcome: false,
    zoomLevel: 0,
    zoomLevelSetByUser: false,
    themeId: null,
  };
}

let _settingsPathOverride: string | null = null;

export function setSettingsPathOverride(p: string | null): void {
  _settingsPathOverride = p;
}

function settingsPath(): string {
  if (_settingsPathOverride) return _settingsPathOverride;
  if (process.platform === "win32" && process.env.LOCALAPPDATA) {
    return path.join(process.env.LOCALAPPDATA, "Bifurc", "app.json");
  }
  return path.join(app.getPath("userData"), "app.json");
}

export function appDataDir(): string {
  return path.dirname(settingsPath());
}

export function loadSettings(): AppSettings {
  try {
    const raw = fs.readFileSync(settingsPath(), "utf-8");
    const parsed = JSON.parse(raw) as Partial<AppSettings>;
    const defaults = makeDefaultSettings();
    return { ...defaults, ...parsed };
  } catch {
    // First launch — generate and persist so the ID is stable across restarts
    const fresh = makeDefaultSettings();
    try { saveSettings(fresh); } catch { /* ignore write errors in non-Electron envs */ }
    return fresh;
  }
}

export function saveSettings(s: AppSettings): void {
  const p = settingsPath();
  fs.mkdirSync(path.dirname(p), { recursive: true });
  fs.writeFileSync(p, JSON.stringify(s, null, 2), "utf-8");
}
