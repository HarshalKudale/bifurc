import * as fs from "fs";
import * as path from "path";
import { entityDir, readAllEntities } from "../workspaceFs";

export interface EntityNameEntry { name: string; method?: string; url?: string; urlSuffix?: string; endpointUrl?: string; operationName?: string; soapActionPattern?: string; serviceName?: string;[key: string]: string | undefined; }

function namesFile(wsId: string, kind: string): string {
  return path.join(entityDir(wsId, kind), "names.json");
}

export function readNamesIndex(wsId: string, kind: string): Record<string, EntityNameEntry> {
  try { return JSON.parse(fs.readFileSync(namesFile(wsId, kind), "utf-8")); } catch { return {}; }
}

function writeNamesIndex(wsId: string, kind: string, names: Record<string, EntityNameEntry>): void {
  const f = namesFile(wsId, kind);
  fs.mkdirSync(path.dirname(f), { recursive: true });
  fs.writeFileSync(f, JSON.stringify(names, null, 2), "utf-8");
}

export function upsertNameEntry(wsId: string, kind: string, id: string, entry: EntityNameEntry): void {
  const names = readNamesIndex(wsId, kind);
  names[id] = entry;
  writeNamesIndex(wsId, kind, names);
}

export function removeNameEntry(wsId: string, kind: string, id: string): void {
  const names = readNamesIndex(wsId, kind);
  delete names[id];
  writeNamesIndex(wsId, kind, names);
}

export function bootstrapNamesIndex<T extends { id: string; name: string; method?: string; url?: string }>(
  wsId: string, kind: string,
): Record<string, EntityNameEntry> {
  const entities = readAllEntities<T>(wsId, kind);
  const names: Record<string, EntityNameEntry> = {};
  for (const e of entities) {
    names[e.id] = { name: e.name, ...(e.method ? { method: e.method } : {}), ...(e.url ? { url: e.url } : {}) };
  }
  writeNamesIndex(wsId, kind, names);
  return names;
}
