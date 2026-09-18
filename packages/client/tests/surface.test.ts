/**
 * The surface contract — P5 work items 1 and 4, and the acceptance criterion
 * "a script asserting `Object.keys(oldApi)` vs `Object.keys(newApi)` reports zero differences".
 *
 * ## Why this imports the real preload instead of reading the type
 *
 * `renderer/types/window.ts` under-declares the runtime surface by four keys
 * (`isFirstLaunch`, `completeFirstLaunch`, `getZoomLevel`, `setZoomLevel` — see
 * `src/surface.ts`'s header). A key-diff driven by the *type* would therefore report zero
 * differences while the client was missing four methods the renderer can call today. So this
 * test mocks `electron`, imports `src/preload.ts` for real, and captures the object that
 * `contextBridge.exposeInMainWorld("api", …)` was handed. **That object is the authority.**
 *
 * It also means the diff cannot go stale: adding a key to the preload fails this test until the
 * key is classified in `src/surface.ts`.
 */

import { describe, expect, it, vi } from "vitest";
import { SURFACE, SURFACE_KEYS, assertSurfaceIsTotal } from "../src/surface";

const captured = vi.hoisted(() => ({ api: undefined as Record<string, unknown> | undefined }));

vi.mock("electron", () => ({
  contextBridge: {
    exposeInMainWorld: (_key: string, value: Record<string, unknown>) => {
      captured.api = value;
    },
  },
  ipcRenderer: {
    invoke: () => Promise.resolve(undefined),
    on: () => {},
    off: () => {},
  },
}));

/** Import the real preload for its side effect: populating `captured.api`. */
async function realPreloadKeys(): Promise<string[]> {
  if (!captured.api) await import("../../../src/preload");
  if (!captured.api) throw new Error("preload did not call exposeInMainWorld");
  return Object.keys(captured.api);
}

describe("the window.api surface", () => {
  it("is exactly what src/preload.ts exposes — both directions", async () => {
    const keys = await realPreloadKeys();

    expect(keys.length).toBeGreaterThan(100);
    expect(assertSurfaceIsTotal(keys)).toBeUndefined();

    // Stated as two sets rather than a length, so a swap (one key added, one removed) fails.
    expect([...SURFACE_KEYS].sort()).toEqual([...keys].sort());
  });

  it("exposes 144 keys, which is what the preload actually has", async () => {
    /**
     * Pinned deliberately. The plan says 136 and `renderer/types/window.ts` declares 140; both
     * are stale, and a test that merely said "> 100" would let either number drift back in.
     */
    const keys = await realPreloadKeys();
    expect(keys).toHaveLength(144);
    expect(SURFACE_KEYS).toHaveLength(144);
  });

  it("classifies every key it exposes, with no key classified twice", async () => {
    const keys = await realPreloadKeys();
    const classified = Object.keys(SURFACE);

    expect(new Set(classified).size).toBe(classified.length); // no duplicates possible in an object
    expect(classified.filter((k) => !keys.includes(k))).toEqual([]);
    expect(keys.filter((k) => !classified.includes(k))).toEqual([]);
  });

  it("names the four keys the declared type is missing", async () => {
    /**
     * These four are exposed by the preload and implemented in `clientHandlers.ts`, but absent
     * from `BifurcApi`. They are classified `local` — client-side window/launch concerns, never
     * transport calls — and recorded here so the discrepancy cannot be mistaken for an oversight
     * in either direction.
     */
    const keys = await realPreloadKeys();
    for (const k of ["isFirstLaunch", "completeFirstLaunch", "getZoomLevel", "setZoomLevel"]) {
      expect(keys).toContain(k);
      expect(SURFACE[k].kind).toBe("local");
    }
  });

  it("routes the seven subscriptions to real wire events", () => {
    const subs = Object.entries(SURFACE).filter(([, e]) => e.kind === "subscribe");
    expect(subs).toHaveLength(7);
    for (const [name, entry] of subs) {
      if (entry.kind !== "subscribe") throw new Error("unreachable");
      // Every subscription must name a wire event — an empty name would subscribe to nothing.
      expect(entry.event, name).toMatch(/^event\./);
    }
  });

  it("keeps dialogs, zoom, theme and trust-store operations off the transport", () => {
    /**
     * The acceptance criterion "client-local methods are not routed over the transport". These
     * are operations on the *client's own machine*; on a remote engine they are meaningless at
     * best and a security hole at worst (`tlsInstallCA` mutates the OS trust store).
     */
    const mustBeLocal = [
      "openFileDialog",
      "pickFilePath",
      "pickFolderPath",
      "setTitleBarOverlay",
      "getZoomLevel",
      "setZoomLevel",
      "getTheme",
      "setTheme",
      "tlsInstallCA",
      "openExternal",
      "platform",
      "isFirstLaunch",
      "completeFirstLaunch",
    ];
    for (const key of mustBeLocal) {
      expect(SURFACE[key].kind, key).toBe("local");
    }
  });

  it("reports the classification counts so the shape is visible", async () => {
    const keys = await realPreloadKeys();
    const byKind = (kind: string) => keys.filter((k) => SURFACE[k].kind === kind).length;

    // 73 transport + 48 entity + 7 subscribe + 3 shim + 13 local = 144
    expect(byKind("transport")).toBe(73);
    expect(byKind("entity")).toBe(48);
    expect(byKind("subscribe")).toBe(7);
    expect(byKind("shim")).toBe(3);
    expect(byKind("local")).toBe(13);
  });
});
