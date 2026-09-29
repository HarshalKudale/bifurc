import { AppConfig, Environment, type WorkspaceDispatchSlice } from "../workspace/config";
import { readEnabledSet, bootstrapEnabledSet } from "../workspace/workspaceFs";

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

/**
 * The hostname from a `Host` header, with the port removed — including for an **IPv6 literal**.
 *
 * ## Why this is a function rather than a `.split(":")[0]` at each site
 *
 * Both listeners used to derive the host with `hostHeader.split(":")[0]`, which is correct for
 * `example.com:8080` and for a bare `example.com`, and **wrong for every IPv6 literal**: `[::1]:8080` split
 * on `:` yields `"["`. The two halves then disagreed with each other and with the browser about what the
 * host was.
 *
 * It was harmless while the value was only a log label (`server.ts`'s `target`). It stopped being harmless
 * when the edge began **routing** on it: `plan/15` work item 5's branch 4 is *"the GUI, by bare IP or
 * localhost"*, so a user who reached the engine at `http://[::1]/` would have got the home page and no way
 * to tell why. Fixing it at the point of use would have left the second site with the bug and no marker,
 * which is the duplication `lib/net.ts` records as the thing to avoid — hence one function, two callers.
 *
 * **Not a validator.** A malformed header is returned as it arrived rather than rejected: both callers
 * compare the result against a configured name, so an unmatched host already has an answer (the home page,
 * or a 404) and there is nothing a rejection could add.
 */
export function hostnameOf(hostHeader: string): string {
  const lower = hostHeader.toLowerCase();
  if (lower.startsWith("[")) {
    // `[::1]:8080` → `::1`. The bracketed form is the only one in which a colon is not a port separator.
    const close = lower.indexOf("]");
    return close === -1 ? lower : lower.slice(1, close);
  }
  return lower.split(":")[0];
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
  fullRules: import('../workspace/config').ProxyRule[],
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
