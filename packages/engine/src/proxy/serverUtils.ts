import { AppConfig, Environment, type WorkspaceDispatchSlice } from "../store/config";
import { readEnabledSet, bootstrapEnabledSet } from "../store/workspaceFs";

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

/**
 * The config proxy dispatch should use: `cfg`'s global settings, with the three entity kinds replaced by
 * **one workspace's**. (P14 WI 4's gap, closed 2026-09-22.)
 *
 * ## The fix, and why it is a parameter rather than a lookup
 *
 * This used to read `cfg.activeWorkspaceId` and filter `cfg.mocks` / `cfg.mappings` by it. That is the wrong
 * id, and it was the visible half of the defect: `cfg` comes from `loadConfig()`, which is
 * active-workspace-only, while `fullRules` and `enabledSets` come from the instance's own workspace. The two
 * halves were then cross-filtered — the active workspace's entities tested against the instance's enabled
 * sets — so a host member matched nothing and served nothing, silently. `loadWorkspaceDispatchSlice(wsId)`
 * supplies the matching half; its docblock carries the measurement.
 *
 * `activeWorkspaceId` and `activeEnvironmentId` are overridden as well, so that everything downstream —
 * `activeEnv()`, script and variable resolution, the `pages.ts` mapping list — agrees with the entities
 * above instead of pointing back at the active pointer's workspace.
 *
 * The `!m.workspaceId ||` guard is the same one `proxyRules` has always used: an entity whose own
 * `workspaceId` disagrees with the directory it was found in is a stale file, and dropping it is the
 * conservative reading. `loadConfig()` injects `workspaceId ?? wsId` for mappings, so in practice only
 * hand-written or migrated files can reach the guard at all.
 */
export function workspaceCfg(
  cfg: AppConfig,
  fullRules: import('../store/config').ProxyRule[],
  enabledSets: EnabledSets,
  slice: WorkspaceDispatchSlice,
): AppConfig {
  const wsId = slice.workspaceId;
  return {
    ...cfg,
    activeWorkspaceId: wsId,
    activeEnvironmentId: slice.activeEnvironmentId,
    environments: slice.environments,
    mappings: slice.mappings.filter((m) => (!m.workspaceId || m.workspaceId === wsId) && enabledSets.mappings.has(m.id)),
    proxyRules: fullRules.filter((r) => (!r.workspaceId || r.workspaceId === wsId) && enabledSets.rules.has(r.id)),
    mocks: slice.mocks.filter((m) => (!m.workspaceId || m.workspaceId === wsId) && enabledSets.mocks.has(m.id)),
  };
}
