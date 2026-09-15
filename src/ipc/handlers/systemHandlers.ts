import { ipcMain, dialog, app } from "electron";
import * as fs from "fs";
import { stopServer, startServer } from "@/proxy/server";
import { loadConfig } from "@/store/config";

export function registerSystemHandlers() {
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

      const res = await fetch("https://api.github.com/repos/HarshalKudale/bifurc/releases/latest", {
        headers: { "User-Agent": "Bifurc-App" },
      });

      let tagName = "";
      let releaseName = "";
      let releaseNotes = "";
      let publishedAt = "";
      let htmlUrl = "https://github.com/HarshalKudale/bifurc/releases";
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
        const tagsRes = await fetch("https://api.github.com/repos/HarshalKudale/bifurc/tags", {
          headers: { "User-Agent": "Bifurc-App" },
        });
        if (tagsRes.ok) {
          const tags = (await tagsRes.json()) as any[];
          if (Array.isArray(tags) && tags.length > 0) {
            tagName = tags[0].name || "";
            releaseName = tagName;
            htmlUrl = `https://github.com/HarshalKudale/bifurc/releases/tag/${tagName}`;
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
        downloadUrl: "https://github.com/HarshalKudale/bifurc/releases",
        releaseUrl: "https://github.com/HarshalKudale/bifurc/releases",
        error: err?.message || "Failed to check for updates",
      };
    }
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
