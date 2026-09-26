/**
 * The connection form's state, and how it becomes a URL.
 *
 * Kept apart from the form component on purpose: everything here is pure, so it can be tested in the
 * **node** environment without a DOM — the same split `connect.ts` uses, and for the same reason. The
 * component below it is the part that can only be exercised by loading a page, which is work item 6's
 * Playwright job and the owner's to run.
 */

export interface HostSettings {
  /** A hostname or IP, e.g. `127.0.0.1` or `engine.internal`. */
  host: string;
  /**
   * A string, not a number, because that is what an `<input>` holds. Converting here would mean the
   * form cannot represent "the user is halfway through typing", which is a real state.
   */
  port: string;
}

/**
 * ## There was a third field, `token`, and it went on 2026-09-25
 *
 * It held the engine's per-start secret, typed into the form and remembered alongside the address. It is
 * gone with the token itself: the engine binds loopback and refuses every other address, so there is no
 * boundary for a credential to guard, and a browser is by construction on the machine the engine is on.
 * See `packages/engine/src/transport/ws.ts`'s header for the bind rule and
 * `packages/engine/src/lib/net.ts#isAllowedOrigin` for the check that replaced it.
 *
 * **A stored-shape change, and it degrades rather than breaks.** `loadSettings()` below validates field by
 * field and ignores what it does not know, so a `localStorage` value written by the previous build still
 * loads — the stale `token` key is simply never read, and `saveSettings()` rewrites the object without it
 * on the next connect.
 */

/** `plan/14` D16's loopback port for a host's RPC endpoint. */
export const DEFAULT_HOST = "127.0.0.1";
export const DEFAULT_PORT = "8080";

export const DEFAULT_SETTINGS: HostSettings = {
  host: DEFAULT_HOST,
  port: DEFAULT_PORT,
};

/** Where the last-used settings are remembered. */
const STORAGE_KEY = "bifurc.host-settings";

/**
 * `localStorage`, or `undefined` where there is none.
 *
 * Reached defensively because this module is imported by the **node** suite, where there is no
 * `localStorage` at all — and a module-level `localStorage.getItem` would be a `ReferenceError` at
 * import time rather than a missing feature. Same reasoning as `@bifurc/web-client`'s `storage()`.
 */
function defaultStorage(): Storage | undefined {
  try {
    return typeof localStorage === "undefined" ? undefined : localStorage;
  } catch {
    return undefined;
  }
}

/** The minimal storage surface this module needs, so a test can pass a plain object. */
export interface SettingsStore {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
}

/**
 * The remembered settings, falling back to the defaults.
 *
 * Every field is validated rather than trusted: this is a value a user can edit by hand in devtools, and
 * a `host` that is a number or a `port` that is an object would otherwise reach `new URL` and produce an
 * exception from inside the connection attempt — where it would read as a network failure.
 */
export function loadSettings(store: SettingsStore | undefined = defaultStorage()): HostSettings {
  if (store === undefined) return DEFAULT_SETTINGS;

  let raw: string | null;
  try {
    raw = store.getItem(STORAGE_KEY);
  } catch {
    return DEFAULT_SETTINGS;
  }
  if (raw === null) return DEFAULT_SETTINGS;

  try {
    const parsed: unknown = JSON.parse(raw);
    if (typeof parsed !== "object" || parsed === null) return DEFAULT_SETTINGS;
    const record = parsed as Record<string, unknown>;
    return {
      host: typeof record.host === "string" && record.host !== "" ? record.host : DEFAULT_HOST,
      port: typeof record.port === "string" && record.port !== "" ? record.port : DEFAULT_PORT,
    };
  } catch {
    // A corrupt value is not worth reporting: the defaults are correct and the form is right there.
    return DEFAULT_SETTINGS;
  }
}

/**
 * Remember the settings for next time.
 *
 * Nothing here is secret any more. The address was always harmless, and the token that used to ride along
 * with it is gone — see the note on `HostSettings`. `JSON.stringify(settings)` therefore writes exactly the
 * two fields above, which is also how a value left by the previous build is tidied up.
 */
export function saveSettings(
  settings: HostSettings,
  store: SettingsStore | undefined = defaultStorage(),
): void {
  if (store === undefined) return;
  try {
    store.setItem(STORAGE_KEY, JSON.stringify(settings));
  } catch {
    // A full or disabled store is not a reason to fail a connection that already succeeded.
  }
}

/** `{host, port}` → `ws://host:port`. */
export function toUrl(settings: Pick<HostSettings, "host" | "port">): string {
  return `ws://${settings.host}:${settings.port}`;
}

/**
 * `ws://host:port` → `{host, port}`. Returns `undefined` when the string is not a ws URL.
 *
 * Needed because `?engine=` is a *prefill* for the form: the endpoint arrives as one string and the form
 * holds it as two fields, so the conversion has to be able to fail rather than produce a half-filled form
 * from a typo.
 */
export function fromUrl(url: string): Pick<HostSettings, "host" | "port"> | undefined {
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    return undefined;
  }
  if (parsed.protocol !== "ws:" && parsed.protocol !== "wss:") return undefined;
  if (parsed.hostname === "") return undefined;

  return {
    host: parsed.hostname,
    port: parsed.port === "" ? (parsed.protocol === "wss:" ? "443" : "80") : parsed.port,
  };
}
