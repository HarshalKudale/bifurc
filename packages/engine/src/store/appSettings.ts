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
  /**
   * The **reverse proxy** — the edge router. Host-level, ONE listener for every workspace, and the port every
   * existing `*.localhost` mapping already points at. Changeable without a restart.
   */
  port: number;
  /** The webhook receiver. Host-level since 2026-09-25; changeable without a restart. */
  webhookPort: number;
  /** The forward proxy. Host-level since 2026-09-25 — the port a browser's proxy setting names. */
  forwardProxyPort: number;
  /**
   * The **companion** endpoint — the engine's RPC over WebSocket, which the browser extension connects to.
   * **The only port whose change needs a restart**, and the restart is manual: the UI shows the new value as
   * *"applicable on next restart"* rather than acting on it.
   */
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
  /** Selected UI theme id (see packages/gui/src/lib/themes.ts). Null = use the built-in default. */
  themeId: string | null;
}

// ── The port model's defaults, and the one place they are stated ──────────────
//
// **Every port is engine-level; none is per workspace** (2026-09-25). A workspace is a *project*, so switching
// workspaces changes the **context** — which rules and mocks are in force — while the ports and the
// infrastructure stay put. The rationale for each number, and the finders that use them, are in
// `runtime/portAllocator.ts`, whose header carries the whole table.

/** The reverse proxy's default. `80`, so no existing `*.localhost` mapping has to change. */
export const DEFAULT_REVERSE_PROXY_PORT = 80;

/** The forward proxy's default. The browser's proxy setting names this one. */
export const DEFAULT_FORWARD_PROXY_PORT = 8080;

/**
 * The companion endpoint's default — **deliberately non-standard**, because it is the one port a client (the
 * browser extension) has to be pointed at, so it must not be a number another app is likely to want.
 *
 * It is also the **base the port scan starts from** (`plan/17` §1): the search walks upward from here until it
 * finds a free port. The base is therefore a **published protocol fact** — the extension scans from the same
 * number — not an implementation detail.
 */
export const DEFAULT_COMPANION_PORT = 4747;

/**
 * The band a fresh install's webhook port is drawn from.
 *
 * **Deliberately not a fixed default.** `plan/17` §1: the webhook is a *feature*, off by default, and *"the
 * default is deliberately **random** rather than a fixed number, because a fixed default would collide with
 * whatever else on the machine already wanted it."* A fixed `9101` was the old behaviour, and it is the one
 * shape of default that is guaranteed to be wrong on a machine that already runs one Bifurc.
 *
 * The band is **10000–19999**: above the privileged range, and below the IANA ephemeral range
 * (49152–65535), which the OS hands out to *outbound* connections — binding a listener there invites a
 * collision that looks like a bug in someone else's program. The band is uncommon rather than reserved,
 * which is all a default can be; the port is still checked for real when the user starts the receiver.
 */
export const WEBHOOK_PORT_BAND_MIN = 10000;
export const WEBHOOK_PORT_BAND_MAX = 19999;

/** A port from the band above, for a fresh install. See `WEBHOOK_PORT_BAND_MIN`. */
export function randomWebhookPort(): number {
  const span = WEBHOOK_PORT_BAND_MAX - WEBHOOK_PORT_BAND_MIN + 1;
  return WEBHOOK_PORT_BAND_MIN + Math.floor(Math.random() * span);
}

/**
 * `BIFURC_PROXY_BASE_PORT`, resolved.
 *
 * ## The name is a leftover and the behaviour is new — deliberately
 *
 * It used to move a **scan base**: D16's forward proxy counted up from it (`8080`, then `8081`, …). `plan/17`
 * §1 removed the scan for this port entirely — the forward proxy **binds `8080` and fails if occupied**,
 * because a browser's proxy setting names the port and a silently moved one breaks the browser — so there is
 * no base left for this variable to move. It now sets the port **outright**.
 *
 * The variable is **not renamed**, and that is a decision rather than an oversight: it is documented in
 * `Dockerfile`, `docker-compose.yml` and `DOCKER.md` as the way a container pins the published proxy port,
 * and that is still exactly what it does. Renaming it would break every deployment that already sets it in
 * exchange for a shorter name. `plan/17` §6 asks only that the repurposing not be **silent** — so the three
 * documents are updated, and this comment is the fourth place it is stated.
 *
 * Returns `null` when the variable is unset or empty, so a caller can tell "no override" from "override to
 * the default". A malformed value **throws**: a container that set the variable meant it, and falling back
 * would publish a port the operator did not ask for.
 */
export function forwardProxyPortFromEnv(): number | null {
  const raw = process.env.BIFURC_PROXY_BASE_PORT;
  if (raw === undefined || raw === "") return null;
  const parsed = Number.parseInt(raw, 10);
  if (!Number.isInteger(parsed) || parsed < 0 || parsed > 65535) {
    throw new Error(
      `BIFURC_PROXY_BASE_PORT must be a port number (0-65535), got ${JSON.stringify(raw)}. ` +
        `It sets the forward proxy's port outright (plan/17 §1 — the forward proxy no longer scans).`,
    );
  }
  return parsed;
}

function generateId(): string {
  return Date.now().toString(36) + Math.random().toString(36).slice(2, 7);
}

function makeDefaultSettings(): AppSettings {
  const id = generateId();
  return {
    port: DEFAULT_REVERSE_PROXY_PORT,
    webhookPort: randomWebhookPort(),
    forwardProxyPort: DEFAULT_FORWARD_PROXY_PORT,
    companionPort: DEFAULT_COMPANION_PORT,
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
  // Resolved **before** the read, so a malformed value fails on every path rather than only the one that
  // finds a file. `forwardProxyPortFromEnv()` throws rather than falling back; see its own comment.
  const envForwardProxyPort = forwardProxyPortFromEnv();
  try {
    const raw = fs.readFileSync(settingsPath(), "utf-8");
    const parsed = JSON.parse(raw) as Partial<AppSettings>;
    const defaults = makeDefaultSettings();
    return applyEnvOverrides({ ...defaults, ...parsed }, envForwardProxyPort);
  } catch {
    // First launch — generate and persist so the ID is stable across restarts
    const fresh = applyEnvOverrides(makeDefaultSettings(), envForwardProxyPort);
    try { saveSettings(fresh); } catch { /* ignore write errors in non-Electron envs */ }
    return fresh;
  }
}

/**
 * Lay the environment over the stored settings. The environment wins, and that direction is the point: the
 * variable is a **deployment** knob, so a container's `-e` must beat a value an earlier run left in the
 * volume — otherwise the override would work exactly once, on the first boot of an empty volume, which is
 * the boot where nothing needs overriding.
 *
 * Only `forwardProxyPort` is overlaid today, and there is no generic "env for every field" loop on purpose:
 * each of the other three ports has a *different* rule about whether it may move (`plan/17` §1), and a
 * uniform mechanism would quietly offer a restart-only port a runtime override it cannot honour.
 */
function applyEnvOverrides(settings: AppSettings, forwardProxyPort: number | null): AppSettings {
  return forwardProxyPort === null ? settings : { ...settings, forwardProxyPort };
}

export function saveSettings(s: AppSettings): void {
  const p = settingsPath();
  fs.mkdirSync(path.dirname(p), { recursive: true });
  fs.writeFileSync(p, JSON.stringify(s, null, 2), "utf-8");
}
