import * as fs from "fs";
import * as path from "path";
import { entityDir, readAllEntities } from "../workspaceFs";

export function readEnabledSet(wsId: string, kind: string): Set<string> | null {
  const file = path.join(entityDir(wsId, kind), "enabled.json");
  try {
    const ids = JSON.parse(fs.readFileSync(file, "utf-8")) as string[];
    return new Set(ids);
  } catch {
    return null;
  }
}

export function writeEnabledSet(wsId: string, kind: string, ids: Set<string>): void {
  const file = path.join(entityDir(wsId, kind), "enabled.json");
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, JSON.stringify(Array.from(ids), null, 2), "utf-8");
}

export function bootstrapEnabledSet<T extends { id: string; enabled?: boolean }>(
  wsId: string, kind: string,
): Set<string> {
  const entities = readAllEntities<T>(wsId, kind);
  const enabled = new Set(entities.filter((e) => e.enabled !== false).map((e) => e.id));
  writeEnabledSet(wsId, kind, enabled);
  return enabled;
}
