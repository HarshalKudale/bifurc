import { AppConfig, Environment } from "@/store/config";
import { readEnabledSet, bootstrapEnabledSet } from "@/store/workspaceFs";

export interface EnabledSets {
  mocks: Set<string>;
  mappings: Set<string>;
  rules: Set<string>;
}

export function loadEnabledSets(wsId: string): EnabledSets {
  const load = (kind: string): Set<string> => {
    const existing = readEnabledSet(wsId, kind);
    if (existing !== null) return existing;
    return bootstrapEnabledSet(wsId, kind);
  };
  return { mocks: load("mocks"), mappings: load("mappings"), rules: load("rules") };
}

export function mkId(): string {
  return Date.now().toString(36) + Math.random().toString(36).slice(2, 6);
}

export function activeEnv(cfg: AppConfig): Environment | null {
  const globalEnv = (cfg.environments ?? []).find((e) => e.id === "__global__") ?? null;
  const selected = cfg.activeEnvironmentId
    ? (cfg.environments ?? []).find((e) => e.id === cfg.activeEnvironmentId) ?? null
    : null;
  if (!globalEnv && !selected) return null;
  const map = new Map<string, { id: string; key: string; value: string }>();
  for (const v of globalEnv?.variables ?? []) map.set(v.key, v);
  for (const v of selected?.variables ?? []) map.set(v.key, v);
  return { id: "merged", name: "merged", variables: [...map.values()], createdAt: 0, workspaceId: "" };
}

export function workspaceCfg(cfg: AppConfig, fullRules: import('@/store/config').ProxyRule[], enabledSets: EnabledSets): AppConfig {
  const wsId = cfg.activeWorkspaceId;
  if (!wsId) return cfg;
  return {
    ...cfg,
    mappings: cfg.mappings.filter((m) => m.workspaceId === wsId && enabledSets.mappings.has(m.id)),
    proxyRules: fullRules.filter((r) => (!r.workspaceId || r.workspaceId === wsId) && enabledSets.rules.has(r.id)),
    mocks: cfg.mocks.filter((m) => m.workspaceId === wsId && enabledSets.mocks.has(m.id)),
  };
}
