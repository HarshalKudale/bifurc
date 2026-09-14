/**
 * Unit tests for `serverUtils` — the layer that decides *what is active*.
 *
 * `workspaceCfg()` is the single most important function for correctness of routing:
 * it filters mappings, proxy rules and mocks down to the active workspace and the
 * enabled set. A bug here silently disables (or wrongly enables) user configuration.
 * `activeEnv()` decides which variables mock bodies resolve against.
 */

import { describe, it, expect, afterEach } from "vitest";
import { workspaceCfg, activeEnv, loadEnabledSets, mkId } from "@/proxy/serverUtils";
import type { AppConfig } from "@/store/config";
import type { Environment, LocalMapping, MockRule, ProxyRule } from "@/store/types";
import { createWorkspace, type WorkspaceFixture } from "../integration/proxyHarness";

function baseCfg(overrides: Partial<AppConfig> = {}): AppConfig {
  return {
    port: 80,
    webhookPort: 9101,
    companionPort: 9271,
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
    graphqlMocks: [],
    graphqlSchemas: [],
    graphqlRequestFolders: [],
    graphqlMockFolders: [],
    grpcRequests: [],
    grpcMocks: [],
    protoFiles: [],
    grpcRequestFolders: [],
    grpcMockFolders: [],
    grpcMockServerPort: 9102,
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

describe("workspaceCfg()", () => {
  it("returns the config untouched when there is no active workspace", () => {
    const cfg = baseCfg({ activeWorkspaceId: "", mappings: [mapping("m1")] });
    const res = workspaceCfg(cfg, [], { mocks: new Set(), mappings: new Set(), rules: new Set() });
    expect(res).toBe(cfg);
  });

  it("keeps only mappings that are in the enabled set", () => {
    const cfg = baseCfg({ mappings: [mapping("m1"), mapping("m2")] });
    const res = workspaceCfg(cfg, [], {
      mocks: new Set(),
      mappings: new Set(["m1"]),
      rules: new Set(),
    });
    expect(res.mappings.map((m) => m.id)).toEqual(["m1"]);
  });

  it("keeps only mappings belonging to the active workspace", () => {
    const cfg = baseCfg({ mappings: [mapping("m1", "ws"), mapping("m2", "other")] });
    const res = workspaceCfg(cfg, [], {
      mocks: new Set(),
      mappings: new Set(["m1", "m2"]),
      rules: new Set(),
    });
    expect(res.mappings.map((m) => m.id)).toEqual(["m1"]);
  });

  it("takes proxy rules from fullRules (not the stubbed cfg.proxyRules)", () => {
    const cfg = baseCfg({ proxyRules: [] });
    const fullRules = [proxyRule("r1"), proxyRule("r2")];
    const res = workspaceCfg(cfg, fullRules, {
      mocks: new Set(),
      mappings: new Set(),
      rules: new Set(["r1"]),
    });
    expect(res.proxyRules.map((r) => r.id)).toEqual(["r1"]);
  });

  it("accepts proxy rules with no workspaceId (legacy rules)", () => {
    const cfg = baseCfg({});
    const legacy = { ...proxyRule("r1"), workspaceId: "" } as ProxyRule;
    const res = workspaceCfg(cfg, [legacy], {
      mocks: new Set(),
      mappings: new Set(),
      rules: new Set(["r1"]),
    });
    expect(res.proxyRules.map((r) => r.id)).toEqual(["r1"]);
  });

  it("filters mocks by workspace and enabled set", () => {
    const cfg = baseCfg({ mocks: [mock("k1"), mock("k2"), mock("k3", "other")] });
    const res = workspaceCfg(cfg, [], {
      mocks: new Set(["k1", "k3"]),
      mappings: new Set(),
      rules: new Set(),
    });
    expect(res.mocks.map((m) => m.id)).toEqual(["k1"]);
  });

  it("disables everything when the enabled sets are empty", () => {
    const cfg = baseCfg({
      mappings: [mapping("m1")],
      mocks: [mock("k1")],
    });
    const res = workspaceCfg(cfg, [proxyRule("r1")], {
      mocks: new Set(),
      mappings: new Set(),
      rules: new Set(),
    });
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
