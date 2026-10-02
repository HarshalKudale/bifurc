/**
 * Client-only handlers — P2 work item 5.
 *
 * Per `plan/handler-classification.md`, these channels are **CLIENT**-classified: they are only
 * meaningful on the user's machine (zoom, theme, the OS trust store, native dialogs, opening an
 * external URL, first-launch UX state) and never cross the wire to a remote engine. They live in
 * their own registration function, called directly by the Electron shell (`src/main.ts`) —
 * `registerIpcHandlers()` (`@/ipc/handlers`) does not call this file, and it must stay that way
 * once `packages/engine` exists: this file simply never moves there.
 *
 * `tls:installCA` is the one exception worth calling out: it lives here (not in the since-deleted
 * `tlsHandlers.ts`) because it mutates the *host's* trust store — a host-scoped, client-local
 * mutation per `File_Ops_Protocol.md` §6, unlike the rest of that file, which read and wrote the
 * engine's own CA files. P3 work item 5 made that split explicit: this handler now delegates to
 * `@/ipc/certLifecycle`, which fetches the certificate over the command registry and drives
 * `@/ipc/certTrust` — the per-platform install/un-trust. The engine's `certManager` no longer
 * shells out to anything.
 */
import { ipcMain, dialog, BrowserWindow } from "electron";
import * as fs from "fs";
import * as path from "path";
import type { ReadFileOptions, ReadFileResult, SavePathOptions } from "@bifurc/client";
import { BLOB_MAX_INGRESS_BYTES } from "@bifurc/protocol";
import { loadSettings, saveSettings } from "@bifurc/engine/workspace/appSettings";
import { getMainWindow } from "@/main";
import { installEngineCa, readCertStatus, uninstallEngineCa } from "@/ipc/certLifecycle";
import { mimeFor } from "@/ipc/fileOpsClient";

function getTitleBarOverlayTheme(themeId: string | null | undefined): { color: string; symbolColor: string } {
  return themeId === "light"
    ? { color: "#eff2f6", symbolColor: "#151b21" }
    : { color: "#090e12", symbolColor: "#eef2f7" };
}

export function registerClientHandlers(): void {
  // ── Zoom ────────────────────────────────────────────────────────────────────
  ipcMain.handle("zoom:get", () => {
    const s = loadSettings();
    return s.zoomLevel ?? 0;
  });

  ipcMain.handle("zoom:set", (_e, level: number) => {
    const clamped = Math.max(-5, Math.min(9, level));
    const s = loadSettings();
    saveSettings({ ...s, zoomLevel: clamped, zoomLevelSetByUser: true });
    const overlayHeight = Math.round(35 * Math.pow(1.2, clamped));
    const overlayTheme = getTitleBarOverlayTheme(s.themeId);
    BrowserWindow.getAllWindows().forEach((w) => {
      if (!w.isDestroyed()) {
        w.webContents.setZoomLevel(clamped);
        try {
          w.setTitleBarOverlay({
            color: overlayTheme.color,
            symbolColor: overlayTheme.symbolColor,
            height: overlayHeight,
          });
        } catch { }
      }
    });
    return { ok: true, zoomLevel: clamped };
  });

  // ── Theme ───────────────────────────────────────────────────────────────────
  ipcMain.handle("theme:get", () => {
    const s = loadSettings();
    return s.themeId ?? null;
  });

  ipcMain.handle("theme:set", (_e, themeId: string) => {
    const s = loadSettings();
    saveSettings({ ...s, themeId });
    const win = getMainWindow();
    if (win && !win.isDestroyed()) {
      const zoomLevel = win.webContents.getZoomLevel();
      const height = Math.round(35 * Math.pow(1.2, zoomLevel));
      const overlayTheme = getTitleBarOverlayTheme(themeId);
      win.setTitleBarOverlay({
        color: overlayTheme.color,
        symbolColor: overlayTheme.symbolColor,
        height,
      });
      win.setBackgroundColor(overlayTheme.color);
    }
    return { ok: true };
  });

  ipcMain.handle("shell:openExternal", (_e, url: string) => {
    const { shell } = require("electron");
    shell.openExternal(url);
  });

  // ── Path pickers for run configurations ────────────────────────────────
  ipcMain.handle("dialog:pickFilePath", async (_evt, title: string, filters?: Electron.FileFilter[]) => {
    const win = BrowserWindow.getFocusedWindow();
    if (!win) return null;
    const { filePaths, canceled } = await dialog.showOpenDialog(win, {
      title: title || "Select File",
      filters: filters ?? [{ name: "All Files", extensions: ["*"] }],
      properties: ["openFile"],
    });
    return canceled || !filePaths[0] ? null : filePaths[0];
  });

  ipcMain.handle("dialog:pickFolderPath", async (_evt, title: string) => {
    const win = BrowserWindow.getFocusedWindow();
    if (!win) return null;
    const { filePaths, canceled } = await dialog.showOpenDialog(win, {
      title: title || "Select Folder",
      properties: ["openDirectory", "createDirectory"],
    });
    return canceled || !filePaths[0] ? null : filePaths[0];
  });

  // ── File picker for binary body uploads ────────────────────────────────
  ipcMain.handle("dialog:openFile", async () => {
    const win = BrowserWindow.getFocusedWindow();
    const { filePaths, canceled } = await dialog.showOpenDialog(win!, {
      title: "Select File",
      properties: ["openFile"],
    });
    if (canceled || !filePaths[0]) return null;
    const filePath = filePaths[0];
    const stat = fs.statSync(filePath);
    if (stat.size > 1024 * 1024) {
      return { error: "File exceeds 1 MB limit" };
    }
    const buffer = fs.readFileSync(filePath);
    const ext = path.extname(filePath).toLowerCase().slice(1);
    const mimeMap: Record<string, string> = {
      png: "image/png", jpg: "image/jpeg", jpeg: "image/jpeg", gif: "image/gif",
      webp: "image/webp", svg: "image/svg+xml", ico: "image/x-icon", bmp: "image/bmp",
      pdf: "application/pdf", zip: "application/zip", gz: "application/gzip",
      mp3: "audio/mpeg", wav: "audio/wav", ogg: "audio/ogg",
      mp4: "video/mp4", webm: "video/webm",
      woff: "font/woff", woff2: "font/woff2", ttf: "font/ttf", otf: "font/otf",
    };
    return {
      name: path.basename(filePath),
      size: stat.size,
      base64: buffer.toString("base64"),
      mimeType: mimeMap[ext] ?? "application/octet-stream",
    };
  });

  ipcMain.handle("shell:setTitleBarOverlay", (_e, color: string, symbolColor: string) => {
    const win = getMainWindow();
    if (win && !win.isDestroyed()) {
      const zoomLevel = win.webContents.getZoomLevel();
      const height = Math.round(35 * Math.pow(1.2, zoomLevel));
      win.setTitleBarOverlay({ color, symbolColor, height });
    }
    return { ok: true };
  });

  // ── Host trust-store mutation — see the file-level comment above ──────────
  //
  // The pre-P3 body was `fs.existsSync(appDataDir()/ca-cert.pem)` then `installCA(certPath)` — the
  // engine's file, installed by an engine function. It now fetches the certificate as an artifact
  // over the command registry, so it works unchanged when the engine is in another process (P4) or
  // a container (P9). The "no CA yet" message is the same string the renderer was written against;
  // `certLifecycle.installEngineCa()` owns it.
  ipcMain.handle("tls:installCA", () => installEngineCa());

  /**
   * The other two halves of the same two-sided operation, **reclassified `transport` → `local` on
   * 2026-09-20** (`plan/07` §5b-bis). They belong in this file for the reason `tls:installCA` does —
   * the OS trust store is the host's, so the composition cannot travel — but they differ from it in
   * one way worth keeping straight: each still calls its engine half (`tls.certStatus` /
   * `tls.removeCert`) through the command registry, so the engine's part of the answer keeps crossing
   * the wire. Only the *composition* moved.
   *
   * What went wrong while they were routed, in one line each: `tlsCertStatus` delivered the engine's
   * fingerprint and paths and dropped the three trust-store fields, so the drift warning could never
   * render; `tlsRemoveCert` deleted the engine's keypair and left the OS store trusting the CA, from
   * the Settings panel's "remove certificate" button. The superseded twins — `tls:certStatus` and
   * `tls:removeCert` in `tlsHandlers.ts` — were unreachable once `window.api` became
   * `{ ...client }`, and were deleted with that file on 2026-09-21.
   */
  ipcMain.handle("client:tlsCertStatus", () => readCertStatus());
  ipcMain.handle("client:tlsRemoveCert", () => uninstallEngineCa());

  // ── Artifact egress / ingress — the client half of an engine-produced artifact ──────────────
  //
  // `File_Ops_Protocol.md` §4: the engine produces the bytes, the **user** chooses where they go, and
  // only the client can put them there. None of these are protocol commands. A remote engine must
  // never choose a path on the user's machine, which is why `@bifurc/client` takes them as injected
  // hooks instead of routing them: P7's browser downloads, P8's CLI writes to stdout or a flag, and
  // only a desktop shell shows these dialogs.
  //
  // Three channels, because the ordering rule needs the ask to be separable from the write.
  // `File_Ops_Protocol.md` §3.2 requires the dialog *before* the command — fail fast on cancel, so
  // the engine never renders a 200 MB workspace archive the user then abandons — and that is only
  // expressible if "ask where" is not bundled with "write there". `client:pickSavePath` is the ask;
  // `client:writeArtifact` is the write, and it also still offers the bundled form for the egress
  // methods whose artifact is cheap and whose dialog chrome comes from the engine's own suggestion.
  //
  // `contentBase64` is **base64**, and every write goes down as raw bytes — that is
  // `ClientLocal.writeArtifact`'s contract. It was a decoded `string` written as `"utf-8"`, which is
  // only correct for artifacts that happen to be text: `workspace-zip` is a ZIP, and round-tripping
  // it through a UTF-8 string replaces every non-ASCII byte with `U+FFFD`, producing a file that
  // looks right and will not open. Taking base64 and writing a Buffer makes every artifact safe,
  // including the five that were already text.
  ipcMain.handle("client:pickSavePath", async (_e, opts?: SavePathOptions) => {
    const win = BrowserWindow.getFocusedWindow();
    const { filePath, canceled } = await dialog.showSaveDialog(win!, {
      ...(opts?.title ? { title: opts.title } : {}),
      ...(opts?.defaultName ? { defaultPath: opts.defaultName } : {}),
      // An empty list would leave the OS dialog with no filter row at all; "All Files" is what the
      // bundled form effectively showed for an unknown extension.
      filters: opts?.filters?.length ? opts.filters : [{ name: "All Files", extensions: ["*"] }],
    });
    return canceled || !filePath ? null : filePath;
  });

  ipcMain.handle(
    "client:writeArtifact",
    async (_e, contentBase64: string, suggestedName: string, mimeType: string, destPath?: string) => {
      // The dialog-first form: the user already chose through `client:pickSavePath`, so asking again
      // would be the bug this parameter exists to prevent. `suggestedName`/`mimeType` describe the
      // artifact and are unused on this branch.
      if (destPath) return writeBytes(contentBase64, destPath);

      const win = BrowserWindow.getFocusedWindow();
      const ext = path.extname(suggestedName).replace(/^\./, "") || "txt";
      const { filePath, canceled } = await dialog.showSaveDialog(win!, {
        defaultPath: suggestedName,
        filters: [{ name: mimeType || "File", extensions: [ext] }],
      });
      if (canceled || !filePath) return { ok: false, canceled: true };
      return writeBytes(contentBase64, filePath);
    },
  );

  ipcMain.handle("client:readArtifactFile", async (_e, opts?: ReadFileOptions) => {
    let filePath = opts?.path;
    if (!filePath) {
      const win = BrowserWindow.getFocusedWindow();
      const { filePaths, canceled } = await dialog.showOpenDialog(win!, {
        ...(opts?.title ? { title: opts.title } : {}),
        // Omitted entirely when absent: `dialog:openFile` passes no `filters` either, and adding an
        // "All Files" entry here would change the dialog the two TLS import methods already show.
        ...(opts?.filters?.length ? { filters: opts.filters } : {}),
        properties: ["openFile"],
      });
      if (canceled || !filePaths[0]) return null;
      filePath = filePaths[0];
    }
    return readLocalFile(filePath);
  });

  // ── First-launch UX state — was inline in main.ts's app.whenReady() ───────
  ipcMain.handle("app:isFirstLaunch", () => {
    const s = loadSettings();
    return !s.hasSeenWelcome;
  });
  ipcMain.handle("app:completeFirstLaunch", () => {
    const s = loadSettings();
    saveSettings({ ...s, hasSeenWelcome: true });
    return { ok: true };
  });
}

/**
 * Write base64-encoded artifact bytes to a path, reporting failure rather than throwing.
 *
 * A thrown `EACCES` would surface in the renderer as a rejected promise, and the export modal has no
 * `try` around its call — it would sit on "working" forever. The whole point of returning a result is
 * that "you said no" and "the write broke" stay distinguishable.
 */
function writeBytes(
  contentBase64: string,
  filePath: string,
): { ok: true; filePath: string } | { ok: false; error: string } {
  try {
    fs.writeFileSync(filePath, Buffer.from(contentBase64, "base64"));
    return { ok: true, filePath };
  } catch (err) {
    return { ok: false, error: (err as Error)?.message ?? "Could not write the file" };
  }
}

/**
 * Read a local file for an import, capped by the protocol's ingress limit.
 *
 * **The cap is the reason this is not a `dialog:openFile` pass-through**, which is what the preload
 * used to point `readArtifactFile` at. That channel caps at 1 MB because it feeds binary request
 * bodies, whereas the legacy `importExport:preflight` accepted up to `BLOB_MAX_INGRESS_BYTES`
 * (100 MB) — so reusing it here would have silently rejected every import between 1 MB and 100 MB,
 * which is most real collections. It also returns no `filePath`, and the path is exactly what the
 * import pair needs to stop opening a second dialog.
 */
function readLocalFile(filePath: string): ReadFileResult | { error: string } {
  let size: number;
  try {
    size = fs.statSync(filePath).size;
  } catch (err) {
    return { error: `Could not read ${path.basename(filePath)}: ${String(err)}` };
  }
  if (size > BLOB_MAX_INGRESS_BYTES) {
    return {
      error:
        `"${path.basename(filePath)}" is ${size} bytes, over the ` +
        `${BLOB_MAX_INGRESS_BYTES}-byte import limit.`,
    };
  }
  try {
    return {
      name: path.basename(filePath),
      size,
      base64: fs.readFileSync(filePath).toString("base64"),
      mimeType: mimeFor(filePath),
      filePath,
    };
  } catch (err) {
    return { error: `Could not read ${path.basename(filePath)}: ${String(err)}` };
  }
}
