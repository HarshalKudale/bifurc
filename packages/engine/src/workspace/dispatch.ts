/**
 * **Deriving one workspace's dispatch config** — the enabled sets, the active environment, and the
 * `AppConfig` the proxy and the edge actually route against.
 *
 * ## Why this is a `workspace/` module
 *
 * These four functions lived in `utils/server.ts` until engine-layout's cleanup. That file was created by
 * phase 4's split of `proxy/serverUtils.ts`, on the reasoning that **both proxy halves** need it — which is
 * true, and is why it cannot live in either half. But `utils/` is defined by §5.1 as **pure and stateless,
 * with no filesystem and no config reads**, and `loadEnabledSets()` reads `enabled.json` through
 * `workspaceFs` and bootstraps it when absent. So the file was in the wrong layer: it is *persistence
 * access*, which is what `workspace/` is for.
 *
 * Putting it here also makes both callers legal without either importing the other: `proxy/` and
 * `reverseproxy/` may both import `workspace/` (§6), so `utils/` stops being a place where a non-pure
 * function hides. The two genuinely pure helpers that shared the old file went to `utils/` instead —
 * `mkId` → `utils/id.ts`, `hostnameOf` → `utils/net.ts`.
 *
 * ## The split, and what stayed behind
 *
 * `utils/server.ts` held six exports and they were not one concern:
 *
 * | export | home now | why |
 * |---|---|---|
 * | `EnabledSets`, `loadEnabledSets` | here | reads `enabled.json`; bootstraps it when absent |
 * | `activeEnv`, `workspaceCfg` | here | derive a dispatch config from a workspace's entities |
 * | `mkId` | `utils/id.ts` | pure: `Date.now()` + `Math.random()` |
 * | `hostnameOf` | `utils/net.ts` | pure: `Host`-header parsing, beside `isLoopbackHost`/`normalizeHostname` |
 *
 * `activeEnv` and `workspaceCfg` are pure functions of their arguments, so `utils/` would have been
 * defensible for them — but they exist only to answer "what should this workspace dispatch?", they take
 * `AppConfig`/`WorkspaceDispatchSlice`/`EnabledSets` from this module's siblings, and splitting four
 * functions across two layers to honour a purity test that has no practical consequence would be tidier on
 * paper and worse to navigate. They stay with the data they derive from.
 */
import type { AppConfig, Environment, ProxyRule, WorkspaceDispatchSlice } from "./config";
import { readEnabledSet, bootstrapEnabledSet } from "./workspaceFs";

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
  fullRules: ProxyRule[],
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
