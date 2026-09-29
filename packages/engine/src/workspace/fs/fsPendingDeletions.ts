import * as fs from "fs";
import * as path from "path";
import { entityDir, sanitizeDirName, wsDir } from "../workspaceFs";

// ── Pending deletions ─────────────────────────────────────────────────────────

export interface PendingDeletion {
  id: string;
  folderId: string | null;
  name: string;
  method?: string;
  url?: string;
  urlSuffix?: string;
}

interface PendingDeletionsFile {
  [kind: string]: PendingDeletion[];
}

function pendingDeletionsFile(wsId: string): string {
  return path.join(wsDir(wsId), "pending-deletions.json");
}

function readPendingDeletions(wsId: string): PendingDeletionsFile {
  try { return JSON.parse(fs.readFileSync(pendingDeletionsFile(wsId), "utf-8")); } catch { return {}; }
}

function writePendingDeletions(wsId: string, data: PendingDeletionsFile): void {
  fs.writeFileSync(pendingDeletionsFile(wsId), JSON.stringify(data, null, 2), "utf-8");
}

export function addPendingDeletion(wsId: string, kind: string, entry: PendingDeletion): void {
  const data = readPendingDeletions(wsId);
  if (!data[kind]) data[kind] = [];
  if (!data[kind].some((e) => e.id === entry.id)) data[kind].push(entry);
  writePendingDeletions(wsId, data);
}

export function removePendingDeletion(wsId: string, kind: string, id: string): void {
  const data = readPendingDeletions(wsId);
  if (data[kind]) data[kind] = data[kind].filter((e) => e.id !== id);
  writePendingDeletions(wsId, data);
}

export function getPendingDeletions(wsId: string, kind: string): PendingDeletion[] {
  return readPendingDeletions(wsId)[kind] ?? [];
}

export function clearPendingDeletions(wsId: string, kind: string): void {
  const data = readPendingDeletions(wsId);
  delete data[kind];
  writePendingDeletions(wsId, data);
}
