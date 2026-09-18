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
 * ## Why thirteen keys are still `ipcRenderer.invoke`, and why that is not sloppiness
 *
 * Step 3b-1 routes everything that can be routed. The remaining thirteen cannot, for two different
 * reasons, and the distinction matters because they need two different fixes:
 *
 * **Four have no registry implementation at all** (`plan/07` §5b). `checkUpdate`, `listAudit`,
 * `saveRunnerConfig`, `loadRunnerConfig` are pinned in the ratchet as `SPLIT` / `NARROWED` /
 * `BLOCKED` / `BLOCKED` respectively. Routing them would turn a working method into an
 * `UNKNOWN_COMMAND`, so they stay on the channels that serve them until the protocol moves. Note
 * this is *not* a backlog: `app.checkUpdate` is resolved by P12, and the two `runner.*Config` are
 * blocked by a **passing test** that saves a config the frozen schema rejects.
 *
 * **Nine are artifact egress** and need two `ClientLocal` hooks this shell does not provide yet
 * (`writeArtifact` / `readArtifactFile`). The engine side is ready — `capture.shareJson`,
 * `audit.export`, `runner.exportReport`, `tls.exportCert` and the two `tls.import*` are all
 * registered commands — but the *client* half of an egress is a save dialog and a file write, and
 * that has to live in the shell. Wiring those two hooks is a separate, self-contained change; doing
 * it here would mix a new channel into the flip and make the flip's own failures unreadable.
 *
 * So the count is **131 routed, 13 held back**, and every held-back key says why.
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

  // ── Nine artifact egress methods — need `writeArtifact` / `readArtifactFile` ──
  //
  // The engine half of every one of these is a registered command already; what is missing is the
  // client half (a save dialog and a file write, or an open dialog and a read). Until those two
  // `ClientLocal` hooks exist in this shell, the client's egress methods would resolve
  // `{ ok: false, error: "this client cannot write files" }` — which is a worse failure than the
  // working legacy channel, because it looks like the export failed rather than the bridge.
  exportData: (req: unknown) => ipcRenderer.invoke("importExport:export", req),
  preflightImport: (req: unknown) => ipcRenderer.invoke("importExport:preflight", req),
  importData: (req: unknown) => ipcRenderer.invoke("importExport:import", req),
  exportAudit: (format: "json" | "csv") => ipcRenderer.invoke("audit:export", format),
  tlsImportCert: () => ipcRenderer.invoke("tls:importCert"),
  tlsImportKey: () => ipcRenderer.invoke("tls:importKey"),
  tlsExportCert: () => ipcRenderer.invoke("tls:exportCert"),
  exportRunnerReport: (report: unknown) => ipcRenderer.invoke("runner:exportReport", report),
  shareCaptureJson: (entries: unknown[], suggestedName?: string) =>
    ipcRenderer.invoke("capture:shareJson", entries, suggestedName),
});
