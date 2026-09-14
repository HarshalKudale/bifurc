import * as fs from "fs";
import * as path from "path";
import { app } from "electron";
import { getPendingDeletions } from "./workspace/fsPendingDeletions";

export interface WorkspaceFile {
  id: string;
  name: string;
  createdAt: number;
  activeEnvironmentId: string | null;
}

export interface FolderEntry {
  id: string;
  name: string;
  parentId: string | null;
  createdAt: number;
  workspaceId: string;
}

export interface EntityIndex {
  folders: FolderEntry[];
  order: string[];
}

// ── Path helpers ──────────────────────────────────────────────────────────────

let _dataRootOverride: string | null = null;

export function setDataRootOverride(root: string | null): void {
  _dataRootOverride = root;
}

export function dataRoot(): string {
  if (_dataRootOverride) return _dataRootOverride;
  // Windows: use AppData\Local instead of Roaming
  if (process.platform === "win32" && process.env.LOCALAPPDATA) {
    return path.join(process.env.LOCALAPPDATA, "Bifurc", "data");
  }
  return path.join(app.getPath("userData"), "data");
}

export function wsDir(wsId: string): string {
  return path.join(dataRoot(), wsId);
}

export function entityDir(wsId: string, kind: string): string {
  return path.join(wsDir(wsId), kind);
}

/** Sanitize a folder name to be safe as a filesystem directory name. */
export function sanitizeDirName(name: string): string {
  return name.replace(/[<>:"/\\|?*\x00-\x1f]/g, "").trim() || "unnamed";
}

function entityFile(wsId: string, kind: string, id: string, folderName?: string | null): string {
  const base = entityDir(wsId, kind);
  if (folderName) return path.join(base, sanitizeDirName(folderName), `${id}.json`);
  return path.join(base, `${id}.json`);
}

function indexFile(wsId: string, kind: string): string {
  return path.join(entityDir(wsId, kind), "index.json");
}

// ── Workspace init ────────────────────────────────────────────────────────────

export function initWorkspaceDir(wsId: string, name: string): void {
  const dirs = [
    wsDir(wsId),
    entityDir(wsId, "mappings"),
    entityDir(wsId, "rules"),
    entityDir(wsId, "environments"),
    entityDir(wsId, "mocks"),
    entityDir(wsId, "requests"),
    entityDir(wsId, "sockets"),
    entityDir(wsId, "capture"),
    entityDir(wsId, "webhooks"),
    entityDir(wsId, "graphqlRequests"),
    entityDir(wsId, "graphqlMocks"),
    entityDir(wsId, "soapRequests"),
    entityDir(wsId, "soapMocks"),
    entityDir(wsId, "grpcRequests"),
    entityDir(wsId, "grpcMocks"),
    entityDir(wsId, "applications"),
  ];
  for (const d of dirs) fs.mkdirSync(d, { recursive: true });

  const wsFile = path.join(wsDir(wsId), "workspace.json");
  if (!fs.existsSync(wsFile)) {
    const wf: WorkspaceFile = { id: wsId, name, createdAt: Date.now(), activeEnvironmentId: null };
    fs.writeFileSync(wsFile, JSON.stringify(wf, null, 2), "utf-8");
  }

  const gitignore = path.join(wsDir(wsId), ".gitignore");
  if (!fs.existsSync(gitignore)) {
    fs.writeFileSync(gitignore, "capture/\n*.tmp\n", "utf-8");
  }
}

// ── Generic entity read/write ─────────────────────────────────────────────────

/** Remove any existing file for this entity ID across root and all sub-folders. */
function removeExistingEntityFile(wsId: string, kind: string, id: string): void {
  const dir = entityDir(wsId, kind);
  if (!fs.existsSync(dir)) return;
  const rootFile = path.join(dir, `${id}.json`);
  if (fs.existsSync(rootFile)) { fs.unlinkSync(rootFile); return; }
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    if (entry.isDirectory() && entry.name !== "drafts" && entry.name !== "capture" && entry.name !== ".runs") {
      const f = path.join(dir, entry.name, `${id}.json`);
      if (fs.existsSync(f)) { fs.unlinkSync(f); return; }
    }
  }
}

/**
 * Write an entity file to the correct location.
 * If folderName is provided, writes to {kind}/{folderName}/{id}.json.
 * Otherwise writes to {kind}/{id}.json.
 * Any pre-existing copy of this entity (in any sub-folder) is removed first.
 * `enabled` is always stripped — enabled state lives exclusively in enabled.json.
 */
export function writeEntity(wsId: string, kind: string, id: string, data: object, folderName?: string | null): void {
  removeExistingEntityFile(wsId, kind, id);
  const file = entityFile(wsId, kind, id, folderName);
  fs.mkdirSync(path.dirname(file), { recursive: true });
  const { enabled: _enabled, ...rest } = data as Record<string, unknown>;
  void _enabled;
  fs.writeFileSync(file, JSON.stringify(rest, null, 2), "utf-8");
}

/** Delete an entity file wherever it lives (root level or any sub-folder). */
export function deleteEntityFile(wsId: string, kind: string, id: string): void {
  removeExistingEntityFile(wsId, kind, id);
}

export * from "./workspace/fsPendingDeletions";

export * from "./workspace/fsRead";

export * from "./workspace/fsNamesIndex";

/**
 * Scan entity dir and return stubs: id + folderId from directory structure.
 * Also includes pending-deletion entries (so deleted entities still show in tree).
 */
export function readEntityStubs(
  wsId: string, kind: string,
): Array<{ id: string; folderId: string | null }> {
  const dir = entityDir(wsId, kind);
  if (!fs.existsSync(dir)) return [];
  const stubs: Array<{ id: string; folderId: string | null }> = [];
  const seenIds = new Set<string>();
  const idx = readIndex(wsId, kind);
  const folderDirToId = new Map(idx.folders.map((f) => [sanitizeDirName(f.name), f.id]));
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    if (entry.isFile() && entry.name.endsWith(".json") &&
      entry.name !== "index.json" && entry.name !== "enabled.json" && entry.name !== "names.json") {
      const id = entry.name.slice(0, -5);
      stubs.push({ id, folderId: null });
      seenIds.add(id);
    } else if (entry.isDirectory() && entry.name !== "drafts" && entry.name !== "capture" && entry.name !== ".runs") {
      const folderId = folderDirToId.get(entry.name) ?? null;
      const subdir = path.join(dir, entry.name);
      for (const f of fs.readdirSync(subdir)) {
        if (f.endsWith(".json") && f !== "index.json" && f !== "enabled.json" && f !== "names.json") {
          const id = f.slice(0, -5);
          stubs.push({ id, folderId });
          seenIds.add(id);
        }
      }
    }
  }
  // Append pending-deletion stubs (entity file gone, waiting for git commit)
  const pending = getPendingDeletions(wsId, kind);
  for (const p of pending) {
    if (!seenIds.has(p.id)) stubs.push({ id: p.id, folderId: p.folderId });
  }
  return stubs;
}

// ── Index helpers ─────────────────────────────────────────────────────────────

export function readIndex(wsId: string, kind: string): EntityIndex {
  try { return JSON.parse(fs.readFileSync(indexFile(wsId, kind), "utf-8")) as EntityIndex; }
  catch { return { folders: [], order: [] }; }
}

export function writeIndex(wsId: string, kind: string, idx: EntityIndex): void {
  const f = indexFile(wsId, kind);
  fs.mkdirSync(path.dirname(f), { recursive: true });
  fs.writeFileSync(f, JSON.stringify(idx, null, 2), "utf-8");
}

// ── Entity-path resolver (for git staging) ────────────────────────────────────

/**
 * Compute the relative path for a (not yet written) entity.
 * If folderName is provided: {kind}/{sanitizedFolderName}/{id}.json
 * Otherwise: {kind}/{id}.json
 */
export function entityRelPath(kind: string, id: string, folderName?: string | null): string {
  if (folderName) return `${kind}/${sanitizeDirName(folderName)}/${id}.json`;
  return `${kind}/${id}.json`;
}

/**
 * Find the current relative path of an existing entity file.
 * Returns null if the entity does not exist on disk.
 */
export function findEntityRelPath(wsId: string, kind: string, id: string): string | null {
  const dir = entityDir(wsId, kind);
  if (!fs.existsSync(dir)) return null;
  if (fs.existsSync(path.join(dir, `${id}.json`))) return `${kind}/${id}.json`;
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    if (entry.isDirectory() && entry.name !== "drafts" && entry.name !== "capture" && entry.name !== ".runs") {
      if (fs.existsSync(path.join(dir, entry.name, `${id}.json`))) {
        return `${kind}/${entry.name}/${id}.json`;
      }
    }
  }
  return null;
}

export * from "./workspace/fsDirectorySync";

// ── Flat entity kinds (no subfolder, stored directly in kind/) ────────────────

export function writeFlatEntity(wsId: string, kind: string, id: string, data: object): void {
  const file = path.join(entityDir(wsId, kind), `${id}.json`);
  fs.mkdirSync(path.dirname(file), { recursive: true });
  const { enabled: _enabled, ...rest } = data as Record<string, unknown>;
  void _enabled;
  fs.writeFileSync(file, JSON.stringify(rest, null, 2), "utf-8");
}

export function deleteFlatEntityFile(wsId: string, kind: string, id: string): void {
  const file = path.join(entityDir(wsId, kind), `${id}.json`);
  if (fs.existsSync(file)) fs.unlinkSync(file);
}

export function flatEntityRelPath(kind: string, id: string): string {
  return `${kind}/${id}.json`;
}

export * from "./workspace/fsEnabledSet";

// ── Find an entity file's absolute path ──────────────────────────────────────

export function findEntityFile(wsId: string, kind: string, id: string): string | null {
  const dir = entityDir(wsId, kind);
  if (!fs.existsSync(dir)) return null;
  const rootFile = path.join(dir, `${id}.json`);
  if (fs.existsSync(rootFile)) return rootFile;
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    if (entry.isDirectory() && entry.name !== "drafts" && entry.name !== "capture" && entry.name !== ".runs") {
      const f = path.join(dir, entry.name, `${id}.json`);
      if (fs.existsSync(f)) return f;
    }
  }
  return null;
}
