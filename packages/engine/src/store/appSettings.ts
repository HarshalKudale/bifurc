import * as fs from "fs";
import * as path from "path";
import { dataDir } from "./paths";

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

/**
 * The CA this **client** has installed into its own OS trust store (P3 work item 5).
 *
 * The engine half of the lifecycle reports an identity for the CA it holds
 * (`proxy/certManager.ts#CertIdentity`); this is the client half's record of *acting* on one, and
 * the two are compared to detect drift. It carries the **PEM** and not only the fingerprint because
 * un-trusting needs the actual certificate: once the engine regenerates, its copy is gone, and
 * `certutil` / `security` / `trust` will not remove a certificate you cannot hand them.
 *
 * **Client-owned; the engine never reads it.** It lives in this file because `AppSettings` is
 * already where shell-only state lives (`zoomLevel`, `themeId`, `hasSeenWelcome`), not because the
 * engine has any use for it. `loadConfig()` builds `AppConfig` from an explicit field list, so this
 * key cannot reach the renderer's `config:get` payload and `window.api` stays byte-identical
 * (`README.md` non-negotiable #3).
 *
 * The PEM is a **public** certificate — the private key never leaves the engine's data dir.
 */
export interface TrustedCa {
  /** `sha256:<64 lowercase hex>` over the DER, as reported by `tls.certStatus`. */
  fingerprint: string;
  /** SHA-1, uppercase, no separators — what `certutil -delstore` and `security -Z` want. */
  thumbprintSha1: string;
  /** The subject DN, for describing the record in a UI. Never parsed. */
  subject: string;
  /** The certificate itself, so it can be un-trusted after the engine has forgotten it. */
  pem: string;
  installedAt: number;
}

export interface AppSettings {
  port: number;
  webhookPort: number;
  companionPort: number;
  minimizeToTray: boolean;
  tlsEnabled: boolean;
  tlsCaCertPath: string | null;
  tlsCaKeyPath: string | null;
  /** See `TrustedCa`. Absent or `null` means this client has never installed the CA. */
  tlsTrustedCa?: TrustedCa | null;
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
    tlsTrustedCa: null,
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
  // Engine-side, Electron-free path (P2 work item 4): the Electron shell calls `setDataRoot()`
  // with its `userData` path at startup, so this resolves to the exact same directory Electron
  // would have returned — see `store/paths.ts`.
  return path.join(dataDir(), "app.json");
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
