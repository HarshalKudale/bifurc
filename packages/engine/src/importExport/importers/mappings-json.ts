import {
  loadConfig, saveConfig, generateId, LocalMapping,
} from "../../store/config";
import {
  writeFlatEntity, readEnabledSet, writeEnabledSet, bootstrapEnabledSet, readAllEntities,
} from "../../store/workspaceFs";
import { reloadConfig } from "../../proxy/server";
import type { PreflightResult, ImportResult, CollisionStrategy, ImportSource } from "../types";

function syncEnabled(wsId: string, id: string, enabled: boolean): void {
  let set = readEnabledSet(wsId, "mappings");
  if (!set) set = bootstrapEnabledSet(wsId, "mappings");
  if (enabled) set.add(id); else set.delete(id);
  writeEnabledSet(wsId, "mappings", set);
}

function parse(content: string): LocalMapping[] {
  const data = JSON.parse(content);
  if (data?.schema !== "lp-mappings-v1" || !Array.isArray(data.mappings)) {
    throw new Error("Not a valid lp-mappings-v1 file");
  }
  return data.mappings as LocalMapping[];
}

export function preflight(wsId: string, source: ImportSource): PreflightResult {
  try {
    const mappings = parse(source.content);
    const existingIds = new Set(readAllEntities<LocalMapping>(wsId, "mappings").map((m) => m.id));
    const collisionIds = mappings.filter((m) => existingIds.has(m.id)).map((m) => m.id);
    return { ok: true, itemCount: mappings.length, collisionIds };
  } catch (err) {
    return { ok: false, error: String(err) };
  }
}

export async function run(
  wsId: string,
  source: ImportSource,
  strategy: CollisionStrategy,
): Promise<ImportResult> {
  try {
    const mappings = parse(source.content);
    const cfg = loadConfig();
    const existingIds = new Set(cfg.mappings.filter((m) => m.workspaceId === wsId).map((m) => m.id));

    let imported = 0;
    let skipped = 0;

    for (const mapping of mappings) {
      const isCollision = existingIds.has(mapping.id);
      if (isCollision && strategy === "keep") { skipped++; continue; }

      const newId = strategy === "new" ? generateId() : mapping.id;
      const newMapping: LocalMapping = { ...mapping, id: newId, workspaceId: wsId };

      const existIdx = cfg.mappings.findIndex((m) => m.id === newId);
      if (existIdx !== -1) cfg.mappings[existIdx] = newMapping;
      else cfg.mappings = [...cfg.mappings, newMapping];

      writeFlatEntity(wsId, "mappings", newId, newMapping);
      syncEnabled(wsId, newId, newMapping.enabled);
      imported++;
    }

    saveConfig(cfg);
    reloadConfig();
    return { ok: true, imported, skipped };
  } catch (err) {
    return { ok: false, error: String(err) };
  }
}
