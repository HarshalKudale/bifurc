import { contextBridge, ipcRenderer } from "electron";

import { createClient } from "@bifurc/client";

import { createIpcTransport } from "./ipcTransport";

/**
 * P6 work item 1 — the shell's RPC client, and now the **whole** `window.api`.
 *
 * ## What this file is
 *
 * It was a hand-written table of 136 `ipcRenderer.invoke` calls. It is now a one-line spread of the
 * client, plus thirteen explicit overrides. The table is gone because `renderer/types/window.ts` was
 * never the contract — `src/preload.ts` was — and `@bifurc/client` was built in P5 to satisfy that
 * contract exactly, positional arguments included. So `{ ...client }` *is* the surface, and the
 * surface test in `packages/client/tests/surface.test.ts` asserts it against the real object.
 *
 * ## Why eight keys are still `ipcRenderer.invoke`, and why that is not sloppiness
 *
 * Step 3b-1 routes everything that can safely be routed. The remaining eight cannot, for two
 * different reasons, and the distinction matters because they need two different fixes:
 *
 * **Four have no registry implementation at all** (`plan/07` §5b). `checkUpdate`, `listAudit`,
 * `saveRunnerConfig`, `loadRunnerConfig` are pinned in the ratchet as `SPLIT` / `NARROWED` /
 * `BLOCKED` / `BLOCKED` respectively. Routing them would turn a working method into an
 * `UNKNOWN_COMMAND`, so they stay on the channels that serve them until the protocol moves. Note
 * this is *not* a backlog: `app.checkUpdate` is resolved by P12, and the two `runner.*Config` are
 * blocked by a **passing test** that saves a config the frozen schema rejects.
 *
 * **Four are artifact egress that would regress if routed.** The two `ClientLocal` hooks they need
 * (`writeArtifact` / `readArtifactFile`) now exist — see `clientHandlers.ts` — and they unlocked five
 * of this group (`exportAudit`, `tlsExportCert`, `shareCaptureJson`, `tlsImportCert`, `tlsImportKey`).
 * The remaining four each have a specific defect in the *client's* egress path, listed where they are
 * overridden below: `exportData` would corrupt the binary `workspace-zip`, `exportRunnerReport` would
 * drop the HTML choice, and the two imports would open a second dialog.
 *
 * So the count is **136 routed, 8 held back**, and every held-back key says why.
 *
 * ## The two things that made routing safe, both checked rather than assumed
 *
 * 1. **`config.save` no longer calls `updateTrayMenu()`** — the registry handler emits
 *    `settings.changed` on the bus instead. That is only equivalent because `src/main.ts:299`
 *    subscribes (`bus.onTyped("settings.changed", () => updateTrayMenu())`). Had that subscription
 *    not existed, routing `saveConfig` would have silently stopped updating the tray.
 * 2. **`event.log.entry` arrives as a batch, and the renderer expects one entry.** `eventPump.ts`
 *    coalesces under the same wire name, so a pass-through would hand the capture panel an object
 *    shaped `{entries}` exactly when there was most traffic to show. `onLogEntry` is the one
 *    subscription the client does not pass through — it unwraps the batch and calls the callback
 *    once per entry, which is what the legacy `log:entry` channel did.
 *
 * ## Why `local` is built here rather than being another `ipcRenderer` table
 *
 * `createClient` requires it: `ClientLocal` is the keys that act on **the machine the client runs
 * on** — native pickers, the OS trust store, theme, zoom, launch state. They are not RPC and must
 * never be: `tlsInstallCA` mutates the *user's* trust store, so routing it would let an engine
 * install a root certificate on the client. The client takes them as a dependency rather than
 * guessing, which is also what makes it impossible to route one by accident. `platform` is the one
 * that never crossed IPC at all — it is `process.platform`, read here.
 */
const client = createClient(createIpcTransport(), {
  local: {
    openExternal: (url: string) => ipcRenderer.invoke("shell:openExternal", url),
    setTitleBarOverlay: (color: string, symbolColor: string) =>
      ipcRenderer.invoke("shell:setTitleBarOverlay", color, symbolColor),
    getTheme: () => ipcRenderer.invoke("theme:get"),
    setTheme: (themeId: string) => ipcRenderer.invoke("theme:set", themeId),
    tlsInstallCA: () => ipcRenderer.invoke("tls:installCA"),
    openFileDialog: () => ipcRenderer.invoke("dialog:openFile"),
    pickFilePath: (title: string, filters?: unknown) => ipcRenderer.invoke("dialog:pickFilePath", title, filters),
    pickFolderPath: (title: string) => ipcRenderer.invoke("dialog:pickFolderPath", title),
    platform: process.platform,
    isFirstLaunch: () => ipcRenderer.invoke("app:isFirstLaunch"),
    completeFirstLaunch: () => ipcRenderer.invoke("app:completeFirstLaunch"),
    getZoomLevel: () => ipcRenderer.invoke("zoom:get"),
    setZoomLevel: (level: number) => ipcRenderer.invoke("zoom:set", level),

    // ── The two filesystem hooks that unlock the artifact-egress methods ────
    //
    // `writeArtifact` is a save dialog and a write — the client half of every egress. The engine
    // produces the bytes; only the user chooses where they go, and only this process can put them
    // there. `readArtifactFile` is the mirror for imports, and `dialog:openFile` already returns
    // exactly `LocalFileContent` (or `{error}` over its 1 MB limit, or `null` on cancel), so it is a
    // pass-through rather than a new channel.
    writeArtifact: (content: string, suggestedName: string, mimeType: string) =>
      ipcRenderer.invoke("client:writeArtifact", content, suggestedName, mimeType),
    readArtifactFile: () => ipcRenderer.invoke("dialog:openFile"),
  },
});

contextBridge.exposeInMainWorld("api", {
  ...client,

  // ── Four commands with no registry implementation — see the header ────────
  //
  // Each of these would answer `UNKNOWN_COMMAND` through the bridge. They are pinned in the
  // "registry coverage" ratchet in `tests/ipc/handlers.test.ts`, which is what stops them from being
  // quietly routed here later without the protocol changing first.
  checkUpdate: () => ipcRenderer.invoke("app:checkUpdate"),
  listAudit: (opts?: unknown) => ipcRenderer.invoke("audit:list", opts),
  saveRunnerConfig: (wsId: string, folderId: string, config: unknown) =>
    ipcRenderer.invoke("runner:saveConfig", wsId, folderId, config),
  loadRunnerConfig: (wsId: string, folderId: string) => ipcRenderer.invoke("runner:loadConfig", wsId, folderId),

  // ── Four artifact-egress methods held back, each for a specific reason ────
  //
  // The two `ClientLocal` hooks above now exist, so most of this group routes. These four do not,
  // and **each would be a user-visible regression** rather than a missing feature — which is why they
  // are pinned here with their reason instead of being quietly routed:
  //
  // - `exportData` — `workspace-zip` is a **binary** format, and `ClientLocal.writeArtifact` takes a
  //   decoded *string*. The client base64-decodes into a utf-8 string before handing it over, so a
  //   ZIP routed through the bridge would be corrupted on write. This is a real gap in the client's
  //   egress contract, not a shell problem: `writeArtifact` needs a binary variant (or to take
  //   base64) before `exportData` can move.
  // - `exportRunnerReport` — the client hardcodes `format: "json"`, but the shell's dialog offers
  //   **HTML or JSON** (`runnerHandlers.ts` derives the format from the chosen extension). Routing it
  //   would silently remove HTML export.
  // - `preflightImport` / `importData` — the client opens its **own** file dialog and ignores any path
  //   in `req`. That is right for a blob-based client, but it is a second dialog unless the renderer
  //   never supplied a path in the first place. Unverified, so it stays on the channel that is known
  //   to work; confirm against the e2e import spec before flipping.
  exportData: (req: unknown) => ipcRenderer.invoke("importExport:export", req),
  preflightImport: (req: unknown) => ipcRenderer.invoke("importExport:preflight", req),
  importData: (req: unknown) => ipcRenderer.invoke("importExport:import", req),
  exportRunnerReport: (report: unknown) => ipcRenderer.invoke("runner:exportReport", report),
});
