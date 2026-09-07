import * as fs from "fs";
import * as path from "path";
import { entityDir, findEntityFile } from "../workspaceFs";

const SKIP_FILES = new Set(["index.json", "enabled.json", "names.json", "pending-deletions.json"]);

export function readAllEntities<T>(wsId: string, kind: string): T[] {
  const dir = entityDir(wsId, kind);
  if (!fs.existsSync(dir)) return [];
  const results: T[] = [];
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    if (entry.isDirectory() && entry.name !== "drafts" && entry.name !== "capture" && entry.name !== ".runs") {
      const subdir = path.join(dir, entry.name);
      for (const f of fs.readdirSync(subdir)) {
        if (f.endsWith(".json") && !SKIP_FILES.has(f)) {
          try { results.push(JSON.parse(fs.readFileSync(path.join(subdir, f), "utf-8")) as T); } catch { }
        }
      }
    } else if (entry.isFile() && entry.name.endsWith(".json") && !SKIP_FILES.has(entry.name)) {
      try { results.push(JSON.parse(fs.readFileSync(path.join(dir, entry.name), "utf-8")) as T); } catch { }
    }
  }
  return results;
}

export function readEnabledEntities<T extends { id: string }>(
  wsId: string, kind: string, enabledIds: Set<string> | null,
): T[] {
  if (enabledIds === null) return readAllEntities<T>(wsId, kind);
  if (enabledIds.size === 0) return [];
  const dir = entityDir(wsId, kind);
  if (!fs.existsSync(dir)) return [];
  const results: T[] = [];
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    if (entry.isFile() && entry.name.endsWith(".json") && !SKIP_FILES.has(entry.name)) {
      const id = entry.name.slice(0, -5);
      if (enabledIds.has(id)) {
        try { results.push(JSON.parse(fs.readFileSync(path.join(dir, entry.name), "utf-8")) as T); } catch { }
      }
    } else if (entry.isDirectory() && entry.name !== "drafts" && entry.name !== "capture" && entry.name !== ".runs") {
      const subdir = path.join(dir, entry.name);
      for (const f of fs.readdirSync(subdir)) {
        if (f.endsWith(".json") && !SKIP_FILES.has(f)) {
          const id = f.slice(0, -5);
          if (enabledIds.has(id)) {
            try { results.push(JSON.parse(fs.readFileSync(path.join(subdir, f), "utf-8")) as T); } catch { }
          }
        }
      }
    }
  }
  return results;
}

export function readEntity<T>(wsId: string, kind: string, id: string): T | null {
  const file = findEntityFile(wsId, kind, id);
  if (!file) return null;
  try { return JSON.parse(fs.readFileSync(file, "utf-8")) as T; } catch { return null; }
}
