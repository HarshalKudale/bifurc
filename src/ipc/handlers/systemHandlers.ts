/**
 * The shell's remaining system handler — **one channel**, `app:checkUpdate`.
 *
 * ## What used to be here, and why it is not (P6 reduction, fifth group, 2026-09-21)
 *
 * Four channels left this file: `server:start` / `server:stop` / `server:restart` and
 * `capture:shareJson`. All four were unreachable — `window.api` *is* the client
 * (`src/preload.ts:110`) and `packages/client/src` names no channel — and all four had a twin that
 * carries their behaviour:
 *
 *  - the three `server:*` bodies were **self-contained** (`loadConfig()` → `stopServer()` /
 *    `startServer(cfg.port)`), not `commandRegistry.invoke()` delegates. Their engine halves in
 *    `@bifurc/engine/proxy/serverCommands.ts` are exact equivalents — that file's header says so
 *    character by character — so deleting them lost nothing. **This deletion is what dissolves that
 *    module's "do NOT re-point the legacy channels at this module yet" trap**, which existed only
 *    because these bodies duplicated it.
 *  - `capture:shareJson` owned a `dialog.showSaveDialog` plus the write, which is the one thing a
 *    shell channel may legitimately own. The client's `shareCaptureJson` owns the same two steps
 *    through `local.writeArtifact`, and its ordering — artifact-first, deliberately, unlike the
 *    channel's dialog-first — is pinned in `packages/client/tests/client.test.ts`.
 *
 * ## Why `app:checkUpdate` stays, and why this file is not deleted with the rest
 *
 * It is **client-local**, not legacy. `currentVersion` is the version of the thing the *user* runs
 * and the download URL must be the asset for the *user's* platform, so no engine can answer it
 * correctly on the client's behalf — an engine matching its own `process.platform` hands a Windows
 * user the AppImage. `packages/protocol` no longer declares `app.checkUpdate`; `src/preload.ts:87`
 * is the only caller, and `plan/07`'s "local handlers stay local" table is the ledger. Local
 * channels are not a reduction target, so this file outlives the group that shrank it.
 *
 * `import * as fs from "fs"` used to sit at the top of this file and was referenced by nothing —
 * not by `app:checkUpdate`, and not by any of the four bodies above either. It was removed with
 * them, because four channel bodies made the file look like it needed it.
 */
import { ipcMain, app } from "electron";

export function registerSystemHandlers() {
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
}
