import * as fs from "fs";
import {
  loadConfig, saveConfig, generateId, ProxyRule,
} from "@/store/config";
import {
  writeFlatEntity, readEnabledSet, writeEnabledSet, bootstrapEnabledSet, readAllEntities,
} from "@/store/workspaceFs";
import { reloadConfig } from "@/proxy/server";
import type { PreflightResult, ImportResult, CollisionStrategy } from "@/ipc/importExport/types";

function syncEnabled(wsId: string, id: string, enabled: boolean): void {
  let set = readEnabledSet(wsId, "rules");
  if (!set) set = bootstrapEnabledSet(wsId, "rules");
  if (enabled) set.add(id); else set.delete(id);
  writeEnabledSet(wsId, "rules", set);
}

function parse(filePath: string): ProxyRule[] {
  const data = JSON.parse(fs.readFileSync(filePath, "utf-8"));
  if (data?.schema !== "lp-proxy-rules-v1" || !Array.isArray(data.proxyRules)) {
    throw new Error("Not a valid lp-proxy-rules-v1 file");
  }
  return data.proxyRules as ProxyRule[];
}

export function preflight(wsId: string, filePath: string): PreflightResult {
  try {
    const rules = parse(filePath);
    const existingIds = new Set(readAllEntities<ProxyRule>(wsId, "rules").map((r) => r.id));
    const collisionIds = rules.filter((r) => existingIds.has(r.id)).map((r) => r.id);
    return { ok: true, filePath, itemCount: rules.length, collisionIds };
  } catch (err) {
    return { ok: false, error: String(err) };
  }
}

export async function run(
  wsId: string,
  filePath: string,
  strategy: CollisionStrategy,
): Promise<ImportResult> {
  try {
    const rules = parse(filePath);
    const cfg = loadConfig();
    const existingIds = new Set(cfg.proxyRules.filter((r) => r.workspaceId === wsId).map((r) => r.id));

    let imported = 0;
    let skipped = 0;

    for (const rule of rules) {
      const isCollision = existingIds.has(rule.id);
      if (isCollision && strategy === "keep") { skipped++; continue; }

      const newId = strategy === "new" ? generateId() : rule.id;
      const newRule: ProxyRule = { ...rule, id: newId, workspaceId: wsId };

      const existIdx = cfg.proxyRules.findIndex((r) => r.id === newId);
      if (existIdx !== -1) cfg.proxyRules[existIdx] = newRule;
      else cfg.proxyRules = [...cfg.proxyRules, newRule];

      writeFlatEntity(wsId, "rules", newId, newRule);
      syncEnabled(wsId, newId, newRule.enabled);
      imported++;
    }

    saveConfig(cfg);
    reloadConfig();
    return { ok: true, imported, skipped };
  } catch (err) {
    return { ok: false, error: String(err) };
  }
}
