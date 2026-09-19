import { contextBridge, ipcRenderer } from "electron";

import { createClient } from "@bifurc/client";
import type { ReadFileOptions, SavePathOptions } from "@bifurc/client";

import { createIpcTransport } from "./ipcTransport";

/**
 * P6 work item 1 — the shell's RPC client, and now the **whole** `window.api`.
 *
 * ## What this file is
 *
 * It was a hand-written table of 136 `ipcRenderer.invoke` calls. It is now a one-line spread of the
 * client, plus **four** explicit overrides. The table is gone because `renderer/types/window.ts` was
 * never the contract — `src/preload.ts` was — and `@bifurc/client` was built in P5 to satisfy that
 * contract exactly, positional arguments included. So `{ ...client }` *is* the surface, and the
 * surface test in `packages/client/tests/surface.test.ts` asserts it against the real object.
 *
 * ## Why four keys are still `ipcRenderer.invoke`, and why that is not sloppiness
 *
 * Step 3b-1 routed everything that could safely be routed, 3b-2 gave the missing commands registry
 * implementations, and 3b-3 routed the last four artifact-egress methods once the client had the
 * filesystem hooks to carry them. **140 of 144 keys now route.** What is left is one category:
 *
 * **Four commands with no registry implementation at all** (`plan/07` §5b). `checkUpdate`,
 * `listAudit`, `saveRunnerConfig`, `loadRunnerConfig` are pinned in the ratchet as `SPLIT` /
 * `NARROWED` / `BLOCKED` / `BLOCKED` respectively. Routing them would turn a working method into an
 * `UNKNOWN_COMMAND`, so they stay on the channels that serve them until the protocol moves. Note
 * this is *not* a backlog: `app.checkUpdate` is resolved by P12, and the two `runner.*Config` are
 * blocked by a **passing test** that saves a config the frozen schema rejects.
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
 *
 * The three filesystem hooks are the same idea with an ordering requirement on top: the dialog has
 * to run *before* the command that produces the bytes, so the ask cannot be bundled with the write.
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

    // ── The three filesystem hooks ──────────────────────────────────────────
    //
    // `writeArtifact` is the bundled form — show the save dialog, then write — and `pickSavePath` is
    // the ask on its own, for the egress methods that must choose a destination **before** the
    // engine renders (`File_Ops_Protocol.md` §3.2). `readArtifactFile` is the ingress mirror: ask
    // (or reuse a path the user already chose), then read, returning the path so `preflightImport`
    // can hand the renderer something `importData` reuses instead of re-asking.
    //
    // None of the three is a protocol command: a remote engine must never choose a path on the
    // user's machine, and none of this would mean anything in a container.
    //
    // `contentBase64` is base64 because egress artifacts are **bytes**, not text: `workspace-zip` is
    // a ZIP. The channel decodes it once and writes the buffer, so no step in the chain is a UTF-8
    // round-trip that could turn a valid archive into a corrupt one.
    writeArtifact: (contentBase64: string, suggestedName: string, mimeType: string, destPath?: string) =>
      ipcRenderer.invoke("client:writeArtifact", contentBase64, suggestedName, mimeType, destPath),
    pickSavePath: (opts?: SavePathOptions) => ipcRenderer.invoke("client:pickSavePath", opts),
    readArtifactFile: (opts?: ReadFileOptions) => ipcRenderer.invoke("client:readArtifactFile", opts),
  },
});

contextBridge.exposeInMainWorld("api", {
  ...client,

  // ── Three commands with no registry implementation — see the header ───────
  //
  // Each of these would answer `UNKNOWN_COMMAND` through the bridge. They are pinned in the
  // "registry coverage" ratchet in `tests/ipc/handlers.test.ts`, which is what stops them from being
  // quietly routed here later without the protocol changing first.
  checkUpdate: () => ipcRenderer.invoke("app:checkUpdate"),
  saveRunnerConfig: (wsId: string, folderId: string, config: unknown) =>
    ipcRenderer.invoke("runner:saveConfig", wsId, folderId, config),
  loadRunnerConfig: (wsId: string, folderId: string) => ipcRenderer.invoke("runner:loadConfig", wsId, folderId),
});
