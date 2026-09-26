/**
 * Unit tests for `serverUtils` — the layer that decides *what is active*.
 *
 * `workspaceCfg()` is the single most important function for correctness of routing:
 * it filters mappings, proxy rules and mocks down to the active workspace and the
 * enabled set. A bug here silently disables (or wrongly enables) user configuration.
 * `activeEnv()` decides which variables mock bodies resolve against.
 */

import { describe, it, expect, afterEach } from "vitest";
import { workspaceCfg, activeEnv, loadEnabledSets, mkId } from "@bifurc/engine/proxy/serverUtils";
import type { AppConfig } from "@bifurc/engine/store/config";
import type { Environment, LocalMapping, MockRule, ProxyRule } from "@bifurc/engine/store/types";
import { createWorkspace, type WorkspaceFixture } from "../integration/proxyHarness";

function baseCfg(overrides: Partial<AppConfig> = {}): AppConfig {
  return {
    port: 80,
    webhookPort: 9101,
    minimizeToTray: true,
    tlsEnabled: false,
    tlsCaCertPath: null,
    tlsCaKeyPath: null,
    workspaces: [{ id: "ws", name: "WS", createdAt: 0, activeEnvironmentId: null }],
    activeWorkspaceId: "ws",
    mappings: [],
    proxyRules: [],
    ruleFolders: [],
    mocks: [],
    requests: [],
    mockFolders: [],
    requestFolders: [],
    wsConnections: [],
    wsFolders: [],
    webhooks: [],
    webhookFolders: [],
    graphqlRequests: [],
    graphqlSchemas: [],
    graphqlRequestFolders: [],
    grpcRequests: [],
    protoFiles: [],
    grpcRequestFolders: [],
    environments: [],
    activeEnvironmentId: null,
    ...overrides,
  };
}

function mapping(id: string, workspaceId = "ws", enabled = true): LocalMapping {
  return { id, domain: `${id}.localhost`, target: "127.0.0.1:3000", enabled, workspaceId };
}

function mock(id: string, workspaceId = "ws", enabled = true): MockRule {
  return {
    id,
    name: id,
    method: "GET",
    urlPattern: `http://x/${id}`,
    useRegex: false,
    enabled,
    capturedHeaders: {},
    capturedBody: "",
    responseStatus: 200,
    responseHeaders: {},
    responseBody: "{}",
    createdAt: 1,
    workspaceId,
  };
}

function proxyRule(id: string, workspaceId = "ws", enabled = true): ProxyRule {
  return {
    id,
    name: id,
    pattern: `http://x/${id}`,
    useRegex: false,
    targetType: "external",
    targetMappingId: "",
    targetExternal: "127.0.0.1:4000",
    requestScript: "",
    responseScript: "",
    enabled,
    createdAt: 1,
    workspaceId,
  };
}

// ── workspaceCfg ──────────────────────────────────────────────────────────────

/**
 * The slice the proxy passes in: the **instance's** workspace with its own entities.
 *
 * Built by hand rather than from `loadWorkspaceDispatchSlice()`, so this stays a unit test of the filter —
 * the loader is covered against real files by `tests/integration/workspaceDispatchScope.integration.test.ts`.
 */
function slice(
  workspaceId: string,
  over: {
    mocks?: MockRule[];
    mappings?: LocalMapping[];
    environments?: Environment[];
    activeEnvironmentId?: string | null;
  } = {},
) {
  return {
    workspaceId,
    mocks: over.mocks ?? [],
    mappings: over.mappings ?? [],
    environments: over.environments ?? [],
    activeEnvironmentId: over.activeEnvironmentId ?? null,
  };
}

const NO_SETS = { mocks: new Set<string>(), mappings: new Set<string>(), rules: new Set<string>() };

describe("workspaceCfg()", () => {
  it("uses the SLICE's workspace, not cfg.activeWorkspaceId", () => {
    // **The case the 2026-09-22 fix exists for, and the only one that fails if the id is read out of the
    // config again.** The slice names `ws`; the config names `other` and holds `other`'s entities. Before
    // the fix this function filtered `cfg`'s entities by `cfg.activeWorkspaceId`, so a proxy instance
    // serving `ws` got `other`'s mocks and mappings cross-filtered against `ws`'s enabled sets — which
    // matched nothing, and left the instance serving nothing at all, silently.
    const cfg = baseCfg({
      activeWorkspaceId: "other",
      mappings: [mapping("m-other", "other")],
      mocks: [mock("k-other", "other")],
    });
    const res = workspaceCfg(
      cfg,
      [],
      { mocks: new Set(["k-other"]), mappings: new Set(["m-other"]), rules: new Set() },
      slice("ws"),
    );

    expect(res.activeWorkspaceId).toBe("ws");
    expect(res.mappings).toHaveLength(0);
    expect(res.mocks).toHaveLength(0);
  });

  it("takes environments and activeEnvironmentId from the slice", () => {
    // Same class: `loadConfig()` resolves both for the *active* workspace, so a host member would otherwise
    // resolve mock variables against another workspace's environment.
    const envA = { id: "env-a", name: "A", variables: [], createdAt: 0, workspaceId: "ws" } as Environment;
    const envB = { id: "env-b", name: "B", variables: [], createdAt: 0, workspaceId: "other" } as Environment;
    const cfg = baseCfg({ activeEnvironmentId: "env-b", environments: [envB] });

    const res = workspaceCfg(cfg, [], NO_SETS, slice("ws", { environments: [envA], activeEnvironmentId: "env-a" }));

    expect(res.activeEnvironmentId).toBe("env-a");
    expect(res.environments.map((e) => e.id)).toEqual(["env-a"]);
  });

  it("keeps only mappings that are in the enabled set", () => {
    const res = workspaceCfg(
      baseCfg(),
      [],
      { ...NO_SETS, mappings: new Set(["m1"]) },
      slice("ws", { mappings: [mapping("m1"), mapping("m2")] }),
    );
    expect(res.mappings.map((m) => m.id)).toEqual(["m1"]);
  });

  it("drops a mapping whose own workspaceId disagrees with the slice's", () => {
    const res = workspaceCfg(
      baseCfg(),
      [],
      { ...NO_SETS, mappings: new Set(["m1", "m2"]) },
      slice("ws", { mappings: [mapping("m1", "ws"), mapping("m2", "other")] }),
    );
    expect(res.mappings.map((m) => m.id)).toEqual(["m1"]);
  });

  it("takes proxy rules from fullRules (not the stubbed cfg.proxyRules)", () => {
    const cfg = baseCfg({ proxyRules: [] });
    const res = workspaceCfg(
      cfg,
      [proxyRule("r1"), proxyRule("r2")],
      { ...NO_SETS, rules: new Set(["r1"]) },
      slice("ws"),
    );
    expect(res.proxyRules.map((r) => r.id)).toEqual(["r1"]);
  });

  it("accepts proxy rules with no workspaceId (legacy rules)", () => {
    const legacy = { ...proxyRule("r1"), workspaceId: "" } as ProxyRule;
    const res = workspaceCfg(baseCfg(), [legacy], { ...NO_SETS, rules: new Set(["r1"]) }, slice("ws"));
    expect(res.proxyRules.map((r) => r.id)).toEqual(["r1"]);
  });

  it("filters mocks by workspace and enabled set", () => {
    const res = workspaceCfg(
      baseCfg(),
      [],
      { ...NO_SETS, mocks: new Set(["k1", "k3"]) },
      slice("ws", { mocks: [mock("k1"), mock("k2"), mock("k3", "other")] }),
    );
    expect(res.mocks.map((m) => m.id)).toEqual(["k1"]);
  });

  it("disables everything when the enabled sets are empty", () => {
    const res = workspaceCfg(
      baseCfg(),
      [proxyRule("r1")],
      NO_SETS,
      slice("ws", { mappings: [mapping("m1")], mocks: [mock("k1")] }),
    );
    expect(res.mappings).toHaveLength(0);
    expect(res.mocks).toHaveLength(0);
    expect(res.proxyRules).toHaveLength(0);
  });
});

// ── activeEnv ─────────────────────────────────────────────────────────────────

describe("activeEnv()", () => {
  const env = (id: string, vars: Array<[string, string]>): Environment => ({
    id,
    name: id,
    variables: vars.map(([key, value], i) => ({ id: `${id}-v${i}`, key, value })),
    createdAt: 0,
    workspaceId: "ws",
  });

  it("returns null when there are no environments", () => {
    expect(activeEnv(baseCfg({ environments: [] }))).toBeNull();
  });

  it("returns the global environment's variables when nothing else is selected", () => {
    const cfg = baseCfg({
      environments: [env("__global__", [["host", "global-host"]])],
      activeEnvironmentId: null,
    });
    const res = activeEnv(cfg);
    expect(res?.variables.find((v) => v.key === "host")?.value).toBe("global-host");
  });

  it("merges global and selected environment, with the selected one winning", () => {
    const cfg = baseCfg({
      environments: [
        env("__global__", [["host", "global-host"], ["only-global", "g"]]),
        env("env-dev", [["host", "dev-host"], ["only-dev", "d"]]),
      ],
      activeEnvironmentId: "env-dev",
    });
    const res = activeEnv(cfg);
    expect(res?.variables.find((v) => v.key === "host")?.value).toBe("dev-host");
    expect(res?.variables.find((v) => v.key === "only-global")?.value).toBe("g");
    expect(res?.variables.find((v) => v.key === "only-dev")?.value).toBe("d");
  });

  it("returns null when only a non-existent environment is selected", () => {
    const cfg = baseCfg({ environments: [], activeEnvironmentId: "missing" });
    expect(activeEnv(cfg)).toBeNull();
  });
});

// ── loadEnabledSets ───────────────────────────────────────────────────────────

describe("loadEnabledSets()", () => {
  let ws: WorkspaceFixture;
  afterEach(() => ws?.cleanup());

  it("reads the enabled sets from enabled.json on disk", () => {
    ws = createWorkspace({
      mappings: [mapping("m1"), mapping("m2")],
      mocks: [mock("k1")],
      rules: [proxyRule("r1")],
      disabledMappings: ["m2"],
    });
    const sets = loadEnabledSets("ws-test");
    expect([...sets.mappings].sort()).toEqual(["m1"]);
    expect([...sets.mocks]).toEqual(["k1"]);
    expect([...sets.rules]).toEqual(["r1"]);
  });
});

// ── mkId ──────────────────────────────────────────────────────────────────────

describe("mkId()", () => {
  it("produces a non-empty, url-safe id", () => {
    const id = mkId();
    expect(id).toMatch(/^[0-9a-z]+$/);
    expect(id.length).toBeGreaterThan(6);
  });

  it("is collision-resistant for a realistic batch", () => {
    // NOTE: mkId() appends only 4 base-36 random characters (~1.6M space) to a
    // millisecond timestamp. Generating a large batch inside one millisecond has a
    // small but real birthday-collision probability, so this asserts "almost all
    // unique" rather than "perfectly unique". See the testing report for the
    // follow-up on widening the id space.
    const ids = new Set(Array.from({ length: 500 }, () => mkId()));
    expect(ids.size).toBeGreaterThanOrEqual(490);
  });
});
