import { ipcMain, dialog, BrowserWindow, app } from "electron";
import * as fs from "fs";
import * as path from "path";
import { loadSettings, saveSettings } from "@/store/appSettings";
import { stopServer, startServer } from "@/proxy/server";
import { loadConfig } from "@/store/config";
import { getMainWindow } from "@/main";

export function registerSystemHandlers() {
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
    BrowserWindow.getAllWindows().forEach((w) => {
      if (!w.isDestroyed()) {
        w.webContents.setZoomLevel(clamped);
        try { w.setTitleBarOverlay({ height: overlayHeight }); } catch { }
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
    return { ok: true };
  });

  ipcMain.handle("server:restart", () => {
    const cfg = loadConfig();
    stopServer();
    startServer(cfg.port);
    return { ok: true };
  });

  ipcMain.handle("server:stop", () => {
    stopServer();
    return { ok: true };
  });

  ipcMain.handle("server:start", () => {
    const cfg = loadConfig();
    startServer(cfg.port);
    return { ok: true };
  });

  ipcMain.handle("app:checkUpdate", async () => {
    try {
      let currentVersion = "0.1.0";
      try {
        if (app && typeof app.getVersion === "function") {
          currentVersion = app.getVersion();
        }
      } catch {
        // fallback in environments where app is not running
      }

      const res = await fetch("https://api.github.com/repos/HarshalKudale/local-panel/releases/latest", {
        headers: { "User-Agent": "LocalPanel-App" },
      });

      let tagName = "";
      let releaseName = "";
      let releaseNotes = "";
      let publishedAt = "";
      let htmlUrl = "https://github.com/HarshalKudale/local-panel/releases";
      let downloadUrl = "";
      let assetName = "";

      if (res.ok) {
        const data = (await res.json()) as any;
        tagName = data.tag_name || "";
        releaseName = data.name || tagName;
        releaseNotes = data.body || "";
        publishedAt = data.published_at || "";
        htmlUrl = data.html_url || htmlUrl;

        const assets = (data.assets || []) as Array<{ name: string; browser_download_url: string }>;
        const platform = process.platform;
        let matchedAsset: { name: string; browser_download_url: string } | undefined;

        if (platform === "win32") {
          matchedAsset = assets.find((a) => a.name.toLowerCase().endsWith(".exe"));
        } else if (platform === "darwin") {
          matchedAsset = assets.find((a) => a.name.toLowerCase().endsWith(".dmg") || a.name.toLowerCase().endsWith(".zip"));
        } else {
          matchedAsset = assets.find((a) => a.name.toLowerCase().endsWith(".appimage") || a.name.toLowerCase().endsWith(".deb"));
        }

        if (matchedAsset) {
          downloadUrl = matchedAsset.browser_download_url;
          assetName = matchedAsset.name;
        } else {
          downloadUrl = htmlUrl;
        }
      } else {
        const tagsRes = await fetch("https://api.github.com/repos/HarshalKudale/local-panel/tags", {
          headers: { "User-Agent": "LocalPanel-App" },
        });
        if (tagsRes.ok) {
          const tags = (await tagsRes.json()) as any[];
          if (Array.isArray(tags) && tags.length > 0) {
            tagName = tags[0].name || "";
            releaseName = tagName;
            htmlUrl = `https://github.com/HarshalKudale/local-panel/releases/tag/\${tagName}`;
            downloadUrl = htmlUrl;
          }
        }
      }

      if (!tagName) {
        return {
          ok: false,
          hasUpdate: false,
          currentVersion,
          latestVersion: currentVersion,
          downloadUrl: htmlUrl,
          releaseUrl: htmlUrl,
          error: "No release tags found on GitHub repository",
        };
      }

      const parseSemver = (v: string) =>
        v.replace(/^v/, "").split("-")[0].split(".").map((n) => parseInt(n, 10) || 0);
      const pLatest = parseSemver(tagName);
      const pCurrent = parseSemver(currentVersion);
      let hasUpdate = false;
      const len = Math.max(pLatest.length, pCurrent.length);
      for (let i = 0; i < len; i++) {
        const l = pLatest[i] || 0;
        const c = pCurrent[i] || 0;
        if (l > c) {
          hasUpdate = true;
          break;
        }
        if (l < c) {
          break;
        }
      }

      return {
        ok: true,
        hasUpdate,
        currentVersion,
        latestVersion: tagName,
        releaseName,
        releaseNotes,
        publishedAt,
        downloadUrl: downloadUrl || htmlUrl,
        releaseUrl: htmlUrl,
        assetName,
      };
    } catch (err: any) {
      return {
        ok: false,
        hasUpdate: false,
        currentVersion: "0.1.0",
        latestVersion: "0.1.0",
        downloadUrl: "https://github.com/HarshalKudale/local-panel/releases",
        releaseUrl: "https://github.com/HarshalKudale/local-panel/releases",
        error: err?.message || "Failed to check for updates",
      };
    }
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

  ipcMain.handle("capture:shareJson", async (_e, entries: unknown[], suggestedName?: string) => {
    try {
      const { filePath, canceled } = await dialog.showSaveDialog({
        title: "Share Captured Requests",
        defaultPath: suggestedName || "captured-requests.json",
        filters: [{ name: "JSON", extensions: ["json"] }],
      });
      if (canceled || !filePath) return { ok: false, canceled: true };
      fs.writeFileSync(filePath, JSON.stringify(entries, null, 2), "utf-8");
      return { ok: true, filePath };
    } catch (err: any) {
      return { ok: false, error: err?.message ?? "Share failed" };
    }
  });

}
