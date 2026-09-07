import * as fs from "fs";
import * as path from "path";
import { EntityIndex, entityDir, readIndex, sanitizeDirName, writeIndex } from "../workspaceFs";

/** Delete a folder's directory (called after all its items have been moved out). */
export function deleteEntityDir(wsId: string, kind: string, folderName: string): void {
  const dir = path.join(entityDir(wsId, kind), sanitizeDirName(folderName));
  if (fs.existsSync(dir)) fs.rmSync(dir, { recursive: true, force: true });
}

/**
 * Scan the kind directory for subdirectories not yet registered in the index,
 * auto-register them, and return the (possibly updated) index.
 */
export function autoSyncFsDirectories(wsId: string, kind: string, makeId: () => string): EntityIndex {
  const idx = readIndex(wsId, kind);
  const dir = entityDir(wsId, kind);
  if (!fs.existsSync(dir)) return idx;

  const subDirs = fs.readdirSync(dir, { withFileTypes: true })
    .filter((e) => e.isDirectory() && e.name !== "drafts" && e.name !== "capture" && e.name !== ".runs")
    .map((e) => e.name);

  let changed = false;
  for (const dirName of subDirs) {
    if (!idx.folders.some((f) => sanitizeDirName(f.name) === dirName)) {
      idx.folders.push({
        id: makeId(),
        name: dirName,
        parentId: null,
        createdAt: Date.now(),
        workspaceId: wsId,
      });
      changed = true;
    }
  }

  if (changed) writeIndex(wsId, kind, idx);
  return idx;
}
