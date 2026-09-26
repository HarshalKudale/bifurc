/**
 * The connection form's pure half.
 *
 * Split from the form for the same reason `connect.ts` is split from `main.tsx`: everything here is
 * pure, so it runs in the **node** environment with no DOM and no `localStorage` — the component above it
 * is the part that can only be exercised by loading a page, which is work item 6's Playwright job.
 *
 * The cases that matter are the ones about **untrusted input**. These settings come from `localStorage`,
 * which a user can edit by hand in devtools, and from `?engine=`, which anyone can put anything in. A
 * `host` that arrives as a number or a `port` that arrives as an object must become the default rather
 * than reach `new URL()` and throw from inside a connection attempt, where it would read as a network
 * failure and send someone debugging the wrong thing.
 *
 * **There is no `token` field any more** (2026-09-25) — see `settings.ts`'s header. The stored shape is
 * `{ host, port }`, and a record written by an older build is *degraded* rather than rejected: the extra
 * key is ignored on read and gone on the next write. That is pinned below, because it is the difference
 * between a user's existing settings surviving the upgrade and their form resetting to defaults.
 */
import { describe, expect, it } from "vitest";

import {
  DEFAULT_HOST,
  DEFAULT_PORT,
  DEFAULT_SETTINGS,
  fromUrl,
  loadSettings,
  saveSettings,
  toUrl,
  type SettingsStore,
} from "../settings";

/** A `SettingsStore` that is just a plain object, so no test needs a DOM. */
function store(initial: Record<string, string> = {}): SettingsStore & { data: Record<string, string> } {
  const data: Record<string, string> = { ...initial };
  return {
    data,
    getItem: (key) => (key in data ? data[key]! : null),
    setItem: (key, value) => {
      data[key] = value;
    },
  };
}

describe("toUrl / fromUrl", () => {
  it("round-trips host and port", () => {
    const url = toUrl({ host: "engine.internal", port: "9100" });
    expect(url).toBe("ws://engine.internal:9100");
    expect(fromUrl(url)).toEqual({ host: "engine.internal", port: "9100" });
  });

  it("rejects anything that is not a ws URL", () => {
    // `?engine=` is user input, so a typo has to produce a half-empty form rather than an exception.
    expect(fromUrl("http://127.0.0.1:8080")).toBeUndefined();
    expect(fromUrl("127.0.0.1:8080")).toBeUndefined();
    expect(fromUrl("not a url")).toBeUndefined();
  });

  it("fills in the default port for a scheme that omits it", () => {
    // `new URL("ws://host")` has an empty `port`, and an empty port field would build `ws://host:`.
    expect(fromUrl("ws://127.0.0.1")).toEqual({ host: "127.0.0.1", port: "80" });
    expect(fromUrl("wss://engine.internal")).toEqual({ host: "engine.internal", port: "443" });
  });
});

describe("loadSettings", () => {
  it("falls back to the loopback endpoint plan/14 D16 assigns", () => {
    // This is the default that used to live in `web/api.ts` as `engineUrlFromLocation()`. It moved here
    // when the form became the place an endpoint is chosen, and it is worth pinning: if it ever changed,
    // the symptom would be a form that cannot reach a *local* host while a remote one works.
    expect(loadSettings(store())).toEqual(DEFAULT_SETTINGS);
    expect(DEFAULT_SETTINGS).toEqual({ host: DEFAULT_HOST, port: DEFAULT_PORT });
    expect(toUrl(DEFAULT_SETTINGS)).toBe("ws://127.0.0.1:8080");
  });

  it("round-trips what saveSettings wrote", () => {
    const s = store();
    const settings = { host: "10.0.0.5", port: "9000" };
    saveSettings(settings, s);
    expect(loadSettings(s)).toEqual(settings);
  });

  it("falls back to the defaults on corrupt JSON", () => {
    // Not worth reporting: the defaults are correct and the form is right there.
    expect(loadSettings(store({ "bifurc.host-settings": "{not json" }))).toEqual(DEFAULT_SETTINGS);
  });

  it("replaces a field of the wrong type rather than trusting it", () => {
    const s = store({
      "bifurc.host-settings": JSON.stringify({ host: 42, port: { a: 1 } }),
    });
    expect(loadSettings(s)).toEqual(DEFAULT_SETTINGS);
  });

  it("keeps a valid field while defaulting a broken one", () => {
    const s = store({ "bifurc.host-settings": JSON.stringify({ host: "10.0.0.5", port: "" }) });
    expect(loadSettings(s)).toEqual({ host: "10.0.0.5", port: DEFAULT_PORT });
  });

  it("ignores a stored `token`, and drops it on the next save", () => {
    // A user who ran a build before 2026-09-25 still has this key in `localStorage`, and nothing migrates
    // it. Both halves matter: the read must not treat it as a corrupt *record* (which would reset a valid
    // host/port to the defaults), and the write must not carry it forward — otherwise a secret that no
    // longer guards anything would sit in the browser's storage indefinitely.
    const s = store({
      "bifurc.host-settings": JSON.stringify({ host: "10.0.0.5", port: "9000", token: "stale-secret" }),
    });
    const loaded = loadSettings(s);
    expect(loaded).toEqual({ host: "10.0.0.5", port: "9000" });

    saveSettings(loaded, s);
    expect(JSON.parse(s.data["bifurc.host-settings"]!)).toEqual({ host: "10.0.0.5", port: "9000" });
  });

  it("does not throw when there is no storage at all", () => {
    // The node environment is exactly this case, and it is why `defaultStorage()` is defensive: a
    // module-level `localStorage.getItem` would be a `ReferenceError` at import time.
    expect(loadSettings(undefined)).toEqual(DEFAULT_SETTINGS);
    expect(() => saveSettings(DEFAULT_SETTINGS, undefined)).not.toThrow();
  });
});
