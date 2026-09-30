/**
 * The surface contract — P5 work items 1 and 4, and the acceptance criterion
 * "a script asserting `Object.keys(oldApi)` vs `Object.keys(newApi)` reports zero differences".
 *
 * ## Why this imports the real preload instead of reading the type
 *
 * `packages/gui/src/types/window.ts` under-declares the runtime surface by four keys
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

  it("exposes 136 keys, which is what the preload actually has", async () => {
    /**
     * Pinned deliberately. The plan says 136 and `packages/gui/src/types/window.ts` declares 128; both
     * are stale, and a test that merely said "> 100" would let either number drift back in.
     *
     * **144 → 132 on 2026-09-25** (`plan/17` work item 8, mocks became REST-only): the nine mock CRUD
     * methods (`graphqlMocks` / `grpcMocks` / `soapMocks` × add/update/delete) and the three
     * `grpc*MockServer` transport methods left the surface. The *declared* type lost the same twelve,
     * so the four-key gap this file's header describes is still four — which is what makes this a
     * recorded contract change rather than a number that was edited to match reality.
     *
     * **132 → 136 on 2026-09-29**: the four `*Saved` execute methods
     * (`replayRequestSaved` / `graphqlExecuteSaved` / `soapExecuteSaved` / `grpcExecuteSaved`). This is
     * the **first time the surface has grown** rather than shrunk, and it is why the plan's stale "136"
     * in the paragraph above is now the runtime's real number — a coincidence worth not mistaking for
     * agreement. The *declared* type gained the same four (`128 → 132`), so the gap is still exactly
     * four, in the same direction: **the runtime is the authority and the type is the secondary check**,
     * which is the property this file's header exists to state.
     */
    const keys = await realPreloadKeys();
    expect(keys).toHaveLength(154);
    expect(SURFACE_KEYS).toHaveLength(154);
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

  it("routes the eleven subscriptions to real wire events", () => {
    const subs = Object.entries(SURFACE).filter(([, e]) => e.kind === "subscribe");
    expect(subs).toHaveLength(11);
    for (const [name, entry] of subs) {
      if (entry.kind !== "subscribe") throw new Error("unreachable");
      // Every subscription must name a wire event — an empty name would subscribe to nothing.
      expect(entry.event, name).toMatch(/^event\./);
    }
  });

  it("keeps dialogs, zoom, theme, trust-store and the update check off the transport", () => {
    /**
     * The acceptance criterion "client-local methods are not routed over the transport". These
     * are operations on the *client's own machine*; on a remote engine they are meaningless at
     * best and a security hole at worst (`tlsInstallCA` mutates the OS trust store).
     *
     * `checkUpdate` joined this list on 2026-09-20, when it stopped being the surface's only `SPLIT`.
     * It belongs here for a different reason than the rest — not "it would be a security hole" but
     * "no engine can answer it correctly": `currentVersion` is the version of the thing the *user*
     * runs, and `downloadUrl` must be the asset for the *user's* platform. An engine matching its own
     * `process.platform` hands a Windows user the AppImage.
     *
     * `tlsRemoveCert` and `tlsCertStatus` joined on the same day, for a third reason again: each is
     * **two-sided** — an engine half plus an operation on the host's OS trust store — so routing either
     * one delivers half the answer and silently drops the other. `tlsRemoveCert` was the live defect
     * (`packages/gui/src/panels/settings/TlsSettingsSection.tsx` calls it, and the OS store kept trusting the
     * CA); `tlsCertStatus` lost the three drift fields. They are listed here so the reclassification
     * cannot be quietly undone — this is the assertion that would have caught the original routing.
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
      "tlsRemoveCert",
      "tlsCertStatus",
      "openExternal",
      "platform",
      "isFirstLaunch",
      "completeFirstLaunch",
      "checkUpdate",
    ];
    for (const key of mustBeLocal) {
      expect(SURFACE[key].kind, key).toBe("local");
    }
  });

  it("reports the classification counts so the shape is visible", async () => {
    const keys = await realPreloadKeys();
    const byKind = (kind: string) => keys.filter((k) => SURFACE[k].kind === kind).length;

    // 80 transport + 39 entity + 11 subscribe + 8 shim + 16 local = 154
    //
    // The total is asserted because it is what proves nothing was dropped in a move — a key that
    // vanished would keep this test green only if some other count happened to absorb it, which the
    // sum rules out.
    //
    //   `transport` 73 → 72, `local` 13 → 14    `checkUpdate` — the surface's only `SPLIT`
    //   `transport` 72 → 70, `local` 14 → 16    `tlsRemoveCert` + `tlsCertStatus` — both two-sided
    //   `transport` 70 → 67, `shim` 3 → 6       `tlsGenerate` / `tlsImportCert` / `tlsImportKey`
    //   `transport` 67 → 65, `shim` 6 → 8       `loadEntity` + `setEntityEnabled` (2026-09-21)
    //   `transport` 65 → 62, `entity` 48 → 39   mocks became REST-only (`plan/17` WI 8, 2026-09-25)
    //   `transport` 62 → 66                     the four `*Saved` execute methods (2026-09-29)
    //   `transport` 66 → 69, `subscribe` 7 → 9  the run's own commands and events (`plan/19` WI 4)
    //
    // The sixth is the first *addition* in this list, and it moves exactly one count. A change that
    // raised one count while lowering another would be a reclassification wearing an addition's
    // clothes — the failure this line is here to catch.
    //
    // **The seventh moves two counts, and both are additions of one feature.** `plan/19` WI 4 gives the
    // collection runner three commands (`runner.start` / `runner.cancel` / `runner.status`) and two events
    // (`event.runner.progress` / `event.runner.finished`), because a run the *engine* owns has to be
    // startable, stoppable, askable-about, and watchable. Three transport + two subscribe is the whole of
    // it — no key changed kind, which is what "both are additions" means here.
    //
    // The third and fourth are corrections of a *mis*classification rather than new decisions. Those
    // five keys were filed as 1:1 mappings while the shell channels they replaced each performed a step
    // the engine does not; the 1:1 filing is exactly what let a field be dropped when they were routed,
    // and `shim` is the kind that cannot express that mistake (`plan/07` §5b-undecies, §5b-septies).
    //
    // The fourth pair is the only one of the five whose mis-filing was *live* rather than latent:
    // `loadEntity`'s channel translated the engine-internal kinds `"rules"` / `"sockets"` to the
    // protocol's `"proxyRules"` / `"wsConnections"`, and two panels pass the engine's names — so their
    // loads threw `BAD_REQUEST` into a `.catch(() => {})` and silently returned nothing.
    //
    // The fifth is the only *removal* of the five, and the only one where both counts fall together:
    // the three `grpc*MockServer` transport methods and the nine mock CRUD methods (three kinds ×
    // add/update/delete) went with their entity kinds. Two counts moving by the exact size of the
    // deletion is the signal that nothing was reclassified on the way out — a key filed under a
    // different kind would show as one count rising while the other fell by less.
    // The tenth moves two counts, and both are the same feature: `plan/19` WI 5 item 2's
    // `captureBuildRequest` / `captureBuildMock` — capture → entity construction, the renderer's last
    // domain logic in the capture panel.
    //
    // **The eleventh is `plan/19` WI 6, and it is the largest step this table has taken: +7 transport and
    // +2 subscribe.** Six `ws.*` commands plus `env.getMerged`, and the socket's two events. It is a
    // *growth* rather than a reclassification, and the reason is worth stating because every earlier row
    // here was a rename or a move: `ws.*` has **no legacy channel** to rename. The renderer's socket spoke
    // to the browser's `WebSocket` constructor rather than to the shell, so there was never a
    // `window.api` method for these to replace. `env.getMerged` is the one that *does* replace something —
    // `App.tsx`'s `mergeEnvVars` — but it replaced a renderer function, not a surface key, which is why it
    // lands as an addition here too.
    //
    // Both counts move, and both by exactly the size of the feature. A key filed under a different kind
    // would show as one count rising while another fell, which is the failure this table exists to catch.
    expect(byKind("transport")).toBe(80);
    expect(byKind("entity")).toBe(39);
    expect(byKind("subscribe")).toBe(11);
    expect(byKind("shim")).toBe(8);
    expect(byKind("local")).toBe(16);
  });
});
