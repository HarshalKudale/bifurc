import * as fs from "fs";
import * as path from "path";
import unzipper from "unzipper";
import {
  loadConfig, saveConfig, generateId, Workspace,
} from "../../store/config";
import { wsDir, initWorkspaceDir } from "../../store/workspaceFs";
import { initWorkspaceRepo } from "../../store/gitStore";
import { reloadConfig } from "../../proxy/server";
import { loadSettings, saveSettings } from "../../store/appSettings";
import type { PreflightResult, ImportResult, CollisionStrategy } from "../types";

/**
 * `workspace-zip` — the one importer that cannot read content.
 *
 * `unzipper.Open.file()` rejects a buffer outright (`File_Ops_Protocol.md` §9.1), so this pair
 * needs a real file on disk. The path it is given is the **engine's** copy of the uploaded blob
 * (`blob/store.blobContentPath()`), which is a file the engine already has — so unlike the old
 * `filePath`, it is not a path the client had to supply, and it never crosses the wire.
 *
 * Because of that, the blob's `filename` is not needed here: nothing in this importer derives a
 * display name from a path (the workspace name comes from `workspace.json` *inside* the archive).
 */
export function preflight(_wsId: string, sourcePath: string): PreflightResult {
  try {
    if (!fs.existsSync(sourcePath)) return { ok: false, error: "File not found" };
    // Minimal check: just ensure it's a zip
    const buf = Buffer.alloc(4);
    const fd = fs.openSync(sourcePath, "r");
    fs.readSync(fd, buf, 0, 4, 0);
    fs.closeSync(fd);
    const isZip = buf[0] === 0x50 && buf[1] === 0x4b;
    if (!isZip) return { ok: false, error: "Not a valid ZIP file" };
    return { ok: true, itemCount: 0, collisionIds: [] };
  } catch (err) {
    return { ok: false, error: String(err) };
  }
}

export async function run(
  _wsId: string,
  sourcePath: string,
  _strategy: CollisionStrategy,
): Promise<ImportResult> {
  try {
    const settings = loadSettings();
    const newWsId = generateId();

    // Determine workspace name from workspace.json inside the zip, if present
    let wsName = `Imported Workspace`;
    try {
      const directory = await unzipper.Open.file(sourcePath);
      const wsFile = directory.files.find((f: { path: string }) => f.path === "workspace.json" || f.path.endsWith("/workspace.json"));
      if (wsFile) {
        const buf = await wsFile.buffer();
        const data = JSON.parse(buf.toString("utf-8"));
        if (data.name) wsName = data.name;
      }
    } catch {}

    // De-duplicate name
    const existingNames = new Set(settings.workspaces.map((w: { name: string }) => w.name));
    let finalName = wsName;
    if (existingNames.has(finalName)) {
      let n = 2;
      while (existingNames.has(`${wsName} (${n})`)) n++;
      finalName = `${wsName} (${n})`;
    }

    const destDir = wsDir(newWsId);
    initWorkspaceDir(newWsId, finalName);

    // Extract ZIP, skip .git entries
    const directory = await unzipper.Open.file(sourcePath);
    let fileCount = 0;
    for (const file of directory.files) {
      if (file.path.startsWith(".git/") || file.path === ".git") continue;
      if (file.type === "Directory") continue;

      const destPath = path.join(destDir, file.path);
      fs.mkdirSync(path.dirname(destPath), { recursive: true });
      const buf = await file.buffer();
      fs.writeFileSync(destPath, buf);
      fileCount++;
    }

    // Update workspace.json to use the new ID and name
    const wsFilePath = path.join(destDir, "workspace.json");
    if (fs.existsSync(wsFilePath)) {
      const wsData = JSON.parse(fs.readFileSync(wsFilePath, "utf-8"));
      wsData.id = newWsId;
      wsData.name = finalName;
      fs.writeFileSync(wsFilePath, JSON.stringify(wsData, null, 2), "utf-8");
    }

    await initWorkspaceRepo(newWsId);

    // Register the new workspace in app settings
    const newWs: Workspace = { id: newWsId, name: finalName, createdAt: Date.now(), activeEnvironmentId: null };
    settings.workspaces = [...settings.workspaces, { id: newWsId, name: finalName, activeEnvironmentId: null }];
    settings.activeWorkspaceId = newWsId;
    saveSettings(settings);

    reloadConfig();

    return { ok: true, imported: fileCount };
  } catch (err) {
    return { ok: false, error: String(err) };
  }
}
