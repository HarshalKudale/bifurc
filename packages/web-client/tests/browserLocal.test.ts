/**
 * The browser `ClientLocal`, in a real DOM.
 *
 * `jsdom` rather than an injected fake, for the same reason the transport suite uses the platform's
 * own `WebSocket`: these implementations *are* the DOM calls, so a fake would only assert that the
 * file agrees with itself. The two exceptions are `platform` and the file picker, where the DOM
 * cannot express what a real browser does — an override and a captured input are used there, and both
 * are named at the point of use.
 *
 * @vitest-environment jsdom
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createBrowserLocal } from "../src/local";

beforeEach(() => {
  localStorage.clear();
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe("browser local — theme and first run", () => {
  it("reports no stored theme as null, meaning 'follow the platform'", async () => {
    // `null` is a value here, not a missing answer: it is what lets the page honour
    // `prefers-color-scheme` instead of being pinned to a default the user never chose.
    const local = createBrowserLocal();
    await expect(local.getTheme()).resolves.toBeNull();
  });

  it("stores a theme and applies it to the document element", async () => {
    const local = createBrowserLocal();
    await expect(local.setTheme("midnight")).resolves.toEqual({ ok: true });

    await expect(local.getTheme()).resolves.toBe("midnight");
    // On `<html>`, so a stylesheet can key off it before `<body>` exists.
    expect(document.documentElement.getAttribute("data-theme")).toBe("midnight");
  });

  it("treats a fresh page as a first launch, and remembers after it is completed", async () => {
    const local = createBrowserLocal();
    await expect(local.isFirstLaunch()).resolves.toBe(true);
    await expect(local.completeFirstLaunch()).resolves.toEqual({ ok: true });
    await expect(local.isFirstLaunch()).resolves.toBe(false);
  });
});

describe("browser local — zoom", () => {
  it("defaults to level 0 and applies Electron's own 1.2**level mapping", async () => {
    const local = createBrowserLocal();
    await expect(local.getZoomLevel()).resolves.toBe(0);

    await local.setZoomLevel(1);
    // The same number must mean the same size in both UIs, so the mapping is Electron's, not invented.
    expect(document.documentElement.style.getPropertyValue("zoom")).toBe(String(1.2));
  });

  it("clamps to [-5, 9], matching the shell's handler", async () => {
    const local = createBrowserLocal();
    await local.setZoomLevel(99);
    await expect(local.getZoomLevel()).resolves.toBe(9);
    await local.setZoomLevel(-99);
    await expect(local.getZoomLevel()).resolves.toBe(-5);
  });
});

describe("browser local — openExternal is a security boundary", () => {
  it("refuses a javascript: URL the engine supplied", async () => {
    // The reason this is not a convenience wrapper: the URL comes from the engine, which may be
    // remote, and `window.open("javascript:...")` would run script in a context the user believes
    // belongs to the app. The desktop's version cannot have this bug, because the OS browser is a
    // separate trust domain.
    const open = vi.spyOn(window, "open").mockReturnValue(null);
    const local = createBrowserLocal();

    await expect(local.openExternal("javascript:alert(1)")).rejects.toThrow(/Refusing to open/);
    expect(open).not.toHaveBeenCalled();
  });

  it("refuses an unparseable URL rather than guessing", async () => {
    const open = vi.spyOn(window, "open").mockReturnValue(null);
    const local = createBrowserLocal();

    await expect(local.openExternal("not a url")).rejects.toThrow(/cannot be parsed/);
    expect(open).not.toHaveBeenCalled();
  });

  it("opens an https URL with noopener,noreferrer", async () => {
    const open = vi.spyOn(window, "open").mockReturnValue(null);
    const local = createBrowserLocal();

    await local.openExternal("https://example.com/docs");

    expect(open).toHaveBeenCalledWith(
      "https://example.com/docs",
      "_blank",
      // Without this the opened page gets a `window.opener` handle back into this one.
      "noopener,noreferrer",
    );
  });
});

describe("browser local — the four operations a page cannot do", () => {
  it("fails tlsInstallCA but marks it actionable, which is a different thing", async () => {
    // A supported outcome with instructions, not a dead end: the user *can* install the CA in their
    // OS store themselves. The distinction is the whole reason `needsManualInstall` exists.
    const result = await createBrowserLocal().tlsInstallCA();
    expect(result.ok).toBe(false);
    expect(result.needsManualInstall).toBe(true);
    expect(result.instructions).toMatch(/trust store/i);
  });

  it("refuses pickFilePath instead of inventing a path", async () => {
    // A browser yields a `File`, not a path — `File.name` is a bare name. Returning a synthetic path
    // would fail at the *next* call instead of this one, which is the worse place to find out.
    const local = createBrowserLocal();
    await expect(local.pickFilePath("Import")).rejects.toThrow(/no filesystem paths/);
    await expect(local.pickFolderPath("Choose")).rejects.toThrow(/no filesystem paths/);
  });

  it("reports a truthful empty certificate view rather than an error", async () => {
    // Not a failure: a browser client genuinely has no certificate of its own, and the UI is right to
    // show nothing.
    await expect(createBrowserLocal().tlsCertStatus()).resolves.toEqual({
      generated: false,
      certPath: null,
      keyPath: null,
    });
  });

  it("does not fabricate 'up to date' for a client that cannot check", async () => {
    // `ok:false` with the version still reported. Returning `{ok:true, hasUpdate:false}` would be
    // indistinguishable from a real answer and would hide that this client never checks at all.
    const result = await createBrowserLocal({ version: "9.9.9" }).checkUpdate();
    expect(result.ok).toBe(false);
    expect(result.hasUpdate).toBe(false);
    expect(result.currentVersion).toBe("9.9.9");
    expect(result.error).toMatch(/served, not installed/);
  });

  it("reports no native title bar to overlay", async () => {
    await expect(createBrowserLocal().setTitleBarOverlay("#fff", "#000")).resolves.toEqual({
      ok: false,
    });
  });
});

describe("browser local — the file picker", () => {
  /**
   * Capture the `<input type="file">` the picker creates, so a test can drive it.
   *
   * The element is deliberately never attached to the document, so there is nothing to query for —
   * intercepting `createElement` is the only way to reach it without changing the implementation to
   * suit the test.
   */
  function captureInput(): { last: () => HTMLInputElement } {
    const created: HTMLInputElement[] = [];
    const real = document.createElement.bind(document);
    vi.spyOn(document, "createElement").mockImplementation((tag: string) => {
      const el = real(tag);
      if (tag === "input") created.push(el as HTMLInputElement);
      return el;
    });
    return {
      last: () => {
        const el = created[created.length - 1];
        if (el === undefined) throw new Error("no input was created");
        return el;
      },
    };
  }

  function withFiles(input: HTMLInputElement, ...files: File[]): void {
    // jsdom's `files` is read-only; the implementation only indexes it, so a list-shaped object is
    // enough and avoids depending on jsdom internals.
    Object.defineProperty(input, "files", { value: Object.assign(files, { length: files.length }) });
  }

  it("returns null when the user dismisses the picker, rather than hanging", async () => {
    // The case worth having. The desktop gets `{canceled}` from the OS dialog; a page gets nothing at
    // all on dismissal, so an `onchange`-only implementation returns a promise that **never settles**
    // — the hang this codebase treats as a bug class of its own.
    const captured = captureInput();
    const local = createBrowserLocal();

    const pending = local.openFileDialog();
    captured.last().dispatchEvent(new Event("cancel"));

    await expect(pending).resolves.toBeNull();
  });

  it("reads a chosen file as base64 with its name and size", async () => {
    const captured = captureInput();
    const local = createBrowserLocal();
    const file = new File([new Uint8Array([104, 105])], "hi.txt", { type: "text/plain" });

    const pending = local.openFileDialog();
    withFiles(captured.last(), file);
    captured.last().dispatchEvent(new Event("change"));

    await expect(pending).resolves.toEqual({
      name: "hi.txt",
      size: 2,
      base64: "aGk=", // "hi"
      mimeType: "text/plain",
    });
  });

  it("enforces the same 1 MB cap, with the same message, as the shell's handler", async () => {
    // Two clients that disagreed about this cap would accept different files for the same UI control.
    const captured = captureInput();
    const local = createBrowserLocal();
    const big = new File([new Uint8Array(1024 * 1024 + 1)], "big.bin");

    const pending = local.openFileDialog();
    withFiles(captured.last(), big);
    captured.last().dispatchEvent(new Event("change"));

    await expect(pending).resolves.toEqual({ error: "File exceeds 1 MB limit" });
  });
});
