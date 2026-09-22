/**
 * The **browser** answer to `ClientLocal` — the sixteen operations that must never cross the
 * transport, implemented for a page instead of for Electron.
 *
 * ## Why this file decides whether P7 can start at all
 *
 * `createClient(transport, { local })` **requires** `local`, and `RequiredClientLocal` is the whole
 * interface minus the three filesystem hooks — whose docblock already says a browser "omits" them.
 * So until these sixteen exist, a browser cannot call `createClient` and nothing else in P7 can be
 * built. That is what makes this the increment after the transport rather than the web target.
 *
 * ## The governing rule: fail loudly, never silently no-op
 *
 * Four of these operations have **no browser equivalent at all**, and the temptation is to return
 * `{ ok: true }` and move on. `local.ts`'s own header names why that is the worst option: "a default
 * that silently did nothing would look like a working install". A page cannot install a root CA into
 * the OS trust store, cannot name a filesystem path, and has no release channel to check. Those
 * return a failure carrying the reason, so the UI can say *why* rather than showing a button that
 * appears to work.
 *
 * The one that is genuinely dangerous rather than merely impossible is `tlsInstallCA`: on the desktop
 * it mutates **the client's** trust store, which is exactly why `local.ts` keeps it off the wire.
 * In a browser the request is not merely unsupported, it is one a page must never be able to satisfy.
 *
 * ## `openExternal` is a security boundary here, not a convenience
 *
 * On the desktop this opens the OS browser for a URL the *engine* supplied. In a browser it would be
 * `window.open(url)` — so a remote engine could hand the client an arbitrary URL to open, including
 * a `javascript:` one. That is a phishing and script-injection vector that the desktop's version does
 * not have, because the OS browser is a separate trust domain. So this implementation **allowlists
 * `http:`/`https:`** and refuses everything else, which is a real, testable property rather than a
 * comment.
 */
import type { RequiredClientLocal } from "@bifurc/client";

/**
 * The cap `dialog:openFile` enforces on the desktop, and the message it returns when exceeded.
 *
 * Both are duplicated here rather than imported: the counterpart is `src/ipc/handlers/clientHandlers.ts`,
 * which lives in the app rather than in a package, so there is nothing to import from. It is the same
 * two-homes-drift risk `types.ts` records for `RESERVED_ACTIONS` — recorded rather than hidden, because
 * a browser and a desktop that disagree about this cap would accept different files for the same UI.
 */
const OPEN_FILE_MAX_BYTES = 1024 * 1024;
const OPEN_FILE_TOO_LARGE = "File exceeds 1 MB limit";

/** Storage keys, namespaced so a page hosting something else does not collide with us. */
const THEME_KEY = "bifurc.theme";
const WELCOME_KEY = "bifurc.welcomeSeen";
const ZOOM_KEY = "bifurc.zoom";

export interface BrowserLocalOptions {
  /**
   * The client's own version, reported by `checkUpdate`.
   *
   * Injected rather than imported: this is a property of the thing the user is running, and a package
   * that hardcoded it would be wrong for every build but one.
   */
  version?: string;
  /** Overrides the detected platform string. Injectable so a test need not fake a user agent. */
  platform?: string;
}

/** Base64 of some bytes, without assuming Node — the client's own egress path has the same rule. */
function toBase64(bytes: Uint8Array): string {
  let binary = "";
  // Chunked: `String.fromCharCode(...bytes)` blows the argument limit somewhere around 100 KB, and a
  // file up to the 1 MB cap above would hit it.
  const CHUNK = 0x8000;
  for (let i = 0; i < bytes.length; i += CHUNK) {
    binary += String.fromCharCode(...bytes.subarray(i, i + CHUNK));
  }
  return btoa(binary);
}

/**
 * The storage area, or `undefined` when the page has none.
 *
 * Reached through `globalThis` and wrapped, because `localStorage` **throws** rather than returning
 * null in a page whose storage is blocked (a sandboxed iframe, or a privacy mode that denies it).
 * A throw here would take down every caller; `undefined` degrades to "no persistence".
 */
function storage(): Storage | undefined {
  try {
    return globalThis.localStorage ?? undefined;
  } catch {
    return undefined;
  }
}

function readStored(key: string): string | null {
  try {
    return storage()?.getItem(key) ?? null;
  } catch {
    return null;
  }
}

function writeStored(key: string, value: string): void {
  try {
    storage()?.setItem(key, value);
  } catch {
    // A full or denied store is not a failure of the operation the user asked for. Nothing here is
    // load-bearing enough to justify throwing at a settings toggle.
  }
}

/**
 * The platform string, for the places the UI names the machine the client is running on.
 *
 * `userAgentData.platform` first (the modern, un-faked one), then the deprecated `platform`, then
 * `"unknown"` — never a guess. The engine's own platform is a different question and must not be
 * substituted: under D9 they are only accidentally equal, which is the argument `local.ts` makes for
 * `checkUpdate` living here.
 */
function detectPlatform(): string {
  const nav = globalThis.navigator as
    | (Navigator & { userAgentData?: { platform?: string } })
    | undefined;
  return nav?.userAgentData?.platform ?? nav?.platform ?? "unknown";
}

/** Apply a zoom level the way Electron does, so the same number means the same size in both UIs. */
function applyZoom(level: number): void {
  const factor = 1.2 ** level;
  try {
    globalThis.document?.documentElement?.style.setProperty("zoom", String(factor));
  } catch {
    // No document (a worker, a test without a DOM). The level is still stored, so it applies on the
    // next load rather than being lost.
  }
}

/**
 * A `ClientLocal` for a browser page.
 *
 * The three optional filesystem hooks (`writeArtifact`, `pickSavePath`, `readArtifactFile`) are
 * deliberately **absent**, which is the case `local.ts` designs for: the egress methods then fail
 * with a clear error instead of silently resolving `{ ok: false }`. Work item 3 replaces them with
 * downloads and file inputs; until then, omitting them is the honest state rather than a stub that
 * claims to write a file.
 */
export function createBrowserLocal(opts: BrowserLocalOptions = {}): RequiredClientLocal {
  const version = opts.version ?? "0.0.0-unknown";

  return {
    // ── identity ───────────────────────────────────────────────────────────
    platform: opts.platform ?? detectPlatform(),

    // ── theme ──────────────────────────────────────────────────────────────
    /**
     * `null` means "follow the platform", which is the same contract the desktop uses — so a page
     * with no stored choice honours `prefers-color-scheme` rather than being pinned to a default.
     */
    getTheme: async () => readStored(THEME_KEY),

    setTheme: async (themeId: string) => {
      writeStored(THEME_KEY, themeId);
      try {
        // On the document element rather than `<body>`, so a stylesheet can key off it before the body
        // exists and so the value survives a re-render.
        globalThis.document?.documentElement?.setAttribute("data-theme", themeId);
      } catch {
        // No document. Stored anyway; see `applyZoom` for the same reasoning.
      }
      return { ok: true };
    },

    // ── the window chrome Electron owns and a page does not ────────────────
    /**
     * A browser has no native title bar to overlay — its chrome is the browser's, and a page cannot
     * reach it. Reported as a failure rather than `{ ok: true }`: the caller is drawing an Electron
     * window's controls, and a success here would tell it the controls had been themed when there are
     * none.
     */
    setTitleBarOverlay: async () => ({ ok: false }),

    // ── the trust store ────────────────────────────────────────────────────
    /**
     * **Must fail.** Installing a root CA into the browser's trust store is not something a page can
     * do, and `needsManualInstall` is the field for exactly this: the user has to install it in the OS
     * (or the browser's) store themselves. So this is a *supported outcome* with instructions, not an
     * error — the distinction matters because one is actionable and the other is not.
     */
    tlsInstallCA: async () => ({
      ok: false,
      needsManualInstall: true,
      instructions:
        "A browser cannot install a certificate authority. Install the engine's CA in your operating " +
        "system's trust store yourself, then reload this page.",
    }),

    /**
     * The local half is a no-op in a browser — there is no trust store entry this client wrote — and
     * the engine half is unreachable from here.
     *
     * That second point is a real limitation rather than an oversight: `local.ts` records this key as
     * **two-sided**, calling `tls.removeCert` *through the transport* and then un-trusting locally. But
     * `ClientLocal` is injected without the transport, so a client that is not the Electron shell has
     * no way to reach the engine's half. Recorded in `TRAPS.md`; a fix is a change to the injection
     * point, not to this file.
     */
    tlsRemoveCert: async () => ({
      ok: false,
      error:
        "This client has no local trust store to remove the certificate from, and cannot reach the " +
        "engine's half of this operation.",
    }),

    /**
     * The client's own certificate view: a page has none, so every field is truthfully empty.
     *
     * Not an error — a browser client genuinely has no certificate of its own, and the UI is right to
     * show nothing. The *engine's* certificate is a different question, and answering it from here
     * would be the same mistake `tlsCertStatus`'s reclassification fixed on the desktop.
     */
    tlsCertStatus: async () => ({ generated: false, certPath: null, keyPath: null }),

    // ── file pickers ───────────────────────────────────────────────────────
    /**
     * A file input, capped and shaped like the desktop's `dialog:openFile`.
     *
     * **The cancel case is the interesting one.** The desktop gets `{ canceled }` from
     * `dialog.showOpenDialog`; a page gets nothing at all if the user dismisses the picker, so a naive
     * `onchange`-only implementation returns a promise that **never settles** — the "request in flight
     * that hangs" failure this codebase treats as a bug class of its own. The `cancel` event is the
     * fix; it is supported in current Chrome, Firefox and Safari, and where it is absent the promise
     * waits rather than resolving to a wrong answer.
     */
    openFileDialog: async () => {
      const file = await pickOneFile();
      if (file === null) return null;
      if (file.size > OPEN_FILE_MAX_BYTES) return { error: OPEN_FILE_TOO_LARGE };

      const bytes = new Uint8Array(await file.arrayBuffer());
      return {
        name: file.name,
        size: file.size,
        base64: toBase64(bytes),
        /**
         * The platform's own guess, not a second copy of the desktop's extension table.
         *
         * The desktop derives this from the extension (`clientHandlers.ts`'s `mimeMap`); a browser has
         * already done that and put the answer on `File.type`. Re-deriving it here would be the
         * two-homes drift `RESERVED_ACTIONS` warns about, for a value the platform computes for free.
         * The fallback matches the desktop's.
         */
        mimeType: file.type || "application/octet-stream",
      };
    },

    /**
     * **Throws, and that is the honest answer.** A browser file input yields a `File`, not a path —
     * `File.name` is a bare name with no directory, and there is no path to thread back into a second
     * call. The desktop's contract is a *reusable path*, and returning a synthetic one would be a lie
     * that fails at the next step instead of this one. The browser's egress path is `pickSavePath`
     * (a download), which is work item 3.
     */
    pickFilePath: async () => {
      throw new Error(
        "A browser client has no filesystem paths to return. Use the download path instead.",
      );
    },

    pickFolderPath: async () => {
      throw new Error(
        "A browser client has no filesystem paths to return. Use the download path instead.",
      );
    },

    // ── external links ─────────────────────────────────────────────────────
    /**
     * Open an `http(s)` URL in a new tab, and refuse anything else.
     *
     * The allowlist is the point. The URL comes from the **engine**, which may be remote, so without it
     * an engine could ask this client to open `javascript:` — and `window.open` would run it in a
     * context the user believes belongs to the app. `noopener,noreferrer` is the other half: without
     * it the opened page gets a `window.opener` handle back into this one.
     */
    openExternal: async (url: string) => {
      let scheme: string;
      try {
        scheme = new URL(url).protocol;
      } catch {
        throw new Error(`Refusing to open a URL that cannot be parsed: ${url}`);
      }
      if (scheme !== "http:" && scheme !== "https:") {
        throw new Error(
          `Refusing to open a "${scheme}" URL. A browser client opens http and https links only, ` +
            `because the URL comes from the engine and a page must not run a scheme it was handed.`,
        );
      }
      globalThis.open?.(url, "_blank", "noopener,noreferrer");
    },

    // ── first run ──────────────────────────────────────────────────────────
    /** Mirrors `clientHandlers.ts`'s `!settings.hasSeenWelcome`, with the page's storage as the store. */
    isFirstLaunch: async () => readStored(WELCOME_KEY) === null,

    completeFirstLaunch: async () => {
      writeStored(WELCOME_KEY, "1");
      return { ok: true };
    },

    // ── zoom ───────────────────────────────────────────────────────────────
    getZoomLevel: async () => {
      const raw = readStored(ZOOM_KEY);
      const parsed = raw === null ? Number.NaN : Number(raw);
      return Number.isFinite(parsed) ? parsed : 0;
    },

    /**
     * Stored and applied as `zoom: 1.2 ** level`, which is Electron's own mapping — so a level the
     * user chose in the desktop UI produces the same size here. The clamp matches
     * `clientHandlers.ts`'s `[-5, 9]`.
     */
    setZoomLevel: async (level: number) => {
      const clamped = Math.max(-5, Math.min(9, level));
      writeStored(ZOOM_KEY, String(clamped));
      applyZoom(clamped);
      return { ok: true };
    },

    // ── updates ────────────────────────────────────────────────────────────
    /**
     * A page is not installed and has no release channel, so there is nothing to compare against.
     *
     * `ok: false` with the version still reported: the caller gets a truthful "I cannot answer this"
     * rather than a fabricated "you are up to date", which would be indistinguishable from a real
     * answer and would hide the fact that this client never checks.
     */
    checkUpdate: async () => ({
      ok: false,
      hasUpdate: false,
      currentVersion: version,
      latestVersion: version,
      downloadUrl: "",
      releaseUrl: "",
      error: "This client is a web UI: it is served, not installed, so it has no release to check.",
    }),
  } satisfies RequiredClientLocal;
}

/**
 * Show a file input and resolve with the chosen file, or `null` if the user dismissed it.
 *
 * Extracted so the cancel handling above has one home. The input is never appended to the document:
 * a detached `<input type="file">` still opens its picker in every current browser, and attaching it
 * would need a style, a removal step, and a place in the layout for something the user must never see.
 */
async function pickOneFile(): Promise<File | null> {
  const doc = globalThis.document;
  if (doc === undefined) return null;

  return new Promise<File | null>((resolve) => {
    const input = doc.createElement("input");
    input.type = "file";

    let settled = false;
    const finish = (file: File | null): void => {
      if (settled) return;
      settled = true;
      resolve(file);
    };

    input.addEventListener("change", () => finish(input.files?.[0] ?? null));
    // The dismissal signal. Without it this promise never settles and the caller waits forever.
    input.addEventListener("cancel", () => finish(null));

    input.click();
  });
}
