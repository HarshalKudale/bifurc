/**
 * Unit tests for `matchProxyRule` — the function that decides whether a proxy rule
 * applies and where it points. Previously this had no direct tests at all; it was only
 * exercised incidentally through a fully-mocked server test that asserted
 * "http.request was called" without checking the target.
 */

import { describe, it, expect } from "vitest";
import { matchProxyRule } from "@/proxy/proxyHandler";
import type { LocalMapping, ProxyRule } from "@bifurc/engine/store/types";

function mapping(overrides: Partial<LocalMapping> = {}): LocalMapping {
  return {
    id: "map-1",
    domain: "api.localhost",
    target: "127.0.0.1:3000",
    enabled: true,
    workspaceId: "ws",
    ...overrides,
  };
}

function rule(overrides: Partial<ProxyRule> = {}): ProxyRule {
  return {
    id: "rule-1",
    name: "Rule",
    pattern: "http://api.test/data",
    useRegex: false,
    targetType: "external",
    targetMappingId: "",
    targetExternal: "127.0.0.1:4000",
    requestScript: "",
    responseScript: "",
    enabled: true,
    createdAt: 1,
    workspaceId: "ws",
    ...overrides,
  };
}

describe("matchProxyRule()", () => {
  it("returns no match when there are no rules", () => {
    expect(matchProxyRule([], "http://api.test/data", [])).toEqual({
      matched: false,
      rule: null,
      target: null,
    });
  });

  it("matches an exact (non-regex) pattern", () => {
    const r = rule({ pattern: "http://api.test/data" });
    const res = matchProxyRule([r], "http://api.test/data", []);
    expect(res.matched).toBe(true);
    expect(res.rule).toBe(r);
  });

  it("does not match a substring when useRegex is false", () => {
    const r = rule({ pattern: "http://api.test/data" });
    expect(matchProxyRule([r], "http://api.test/data/extra", []).matched).toBe(false);
  });

  it("matches a regex pattern", () => {
    const r = rule({ pattern: "^http://api\\.test/v\\d+/.*$", useRegex: true });
    expect(matchProxyRule([r], "http://api.test/v2/items", []).matched).toBe(true);
  });

  it("skips an invalid regex instead of throwing", () => {
    const r = rule({ pattern: "[invalid(", useRegex: true });
    expect(() => matchProxyRule([r], "http://api.test/data", [])).not.toThrow();
    expect(matchProxyRule([r], "http://api.test/data", []).matched).toBe(false);
  });

  it("resolves an external target from targetExternal", () => {
    const r = rule({ targetType: "external", targetExternal: "127.0.0.1:4000" });
    expect(matchProxyRule([r], "http://api.test/data", []).target).toBe("127.0.0.1:4000");
  });

  it("resolves a mapping target from the referenced mapping", () => {
    const m = mapping({ id: "map-api", target: "127.0.0.1:5000" });
    const r = rule({ targetType: "mapping", targetMappingId: "map-api" });
    expect(matchProxyRule([r], "http://api.test/data", [m]).target).toBe("127.0.0.1:5000");
  });

  it("reports a match with a null target when the referenced mapping is missing", () => {
    const r = rule({ targetType: "mapping", targetMappingId: "nope" });
    const res = matchProxyRule([r], "http://api.test/data", []);
    expect(res.matched).toBe(true);
    expect(res.target).toBeNull();
  });

  it("reports a match with a null target when targetExternal is empty", () => {
    const r = rule({ targetType: "external", targetExternal: "" });
    const res = matchProxyRule([r], "http://api.test/data", []);
    expect(res.matched).toBe(true);
    expect(res.target).toBeNull();
  });

  it("returns the first matching rule when several match", () => {
    const first = rule({ id: "first", pattern: ".*", useRegex: true, targetExternal: "127.0.0.1:1" });
    const second = rule({ id: "second", pattern: ".*", useRegex: true, targetExternal: "127.0.0.1:2" });
    const res = matchProxyRule([first, second], "http://api.test/data", []);
    expect(res.rule?.id).toBe("first");
    expect(res.target).toBe("127.0.0.1:1");
  });

  it("does not consider rule.enabled — the caller pre-filters via the enabled set", () => {
    // This documents the contract: workspaceCfg() strips disabled rules before this runs.
    const r = rule({ enabled: false });
    expect(matchProxyRule([r], "http://api.test/data", []).matched).toBe(true);
  });
});
