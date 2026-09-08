import { describe, it, expect } from "vitest";
import { resolveVars, resolveHeaders } from "@/lib/resolveVars";
import type { Environment } from "@/types";

// ── Helpers ──────────────────────────────────────────────────────────────────

function makeEnv(vars: Record<string, string>): Environment {
  return {
    id: "env-1",
    name: "Test",
    createdAt: Date.now(),
    variables: Object.entries(vars).map(([key, value]) => ({ id: key, key, value })),
  };
}

// ── resolveVars ───────────────────────────────────────────────────────────────

describe("renderer/lib/resolveVars.ts", () => {
  describe("resolveVars()", () => {
    it("returns the original text when env is null", () => {
      expect(resolveVars("hello {{NAME}}", null)).toBe("hello {{NAME}}");
    });

    it("returns the original text when text is empty", () => {
      expect(resolveVars("", makeEnv({ NAME: "world" }))).toBe("");
    });

    it("replaces a single {{KEY}} token with the environment variable value", () => {
      const env = makeEnv({ BASE_URL: "http://localhost:3000" });
      expect(resolveVars("{{BASE_URL}}/api", env)).toBe("http://localhost:3000/api");
    });

    it("replaces multiple tokens in one string", () => {
      const env = makeEnv({ HOST: "localhost", PORT: "3000" });
      expect(resolveVars("{{HOST}}:{{PORT}}", env)).toBe("localhost:3000");
    });

    it("replaces the same token appearing multiple times", () => {
      const env = makeEnv({ X: "42" });
      expect(resolveVars("{{X}} and {{X}}", env)).toBe("42 and 42");
    });

    it("leaves unrecognised tokens untouched", () => {
      const env = makeEnv({ KNOWN: "value" });
      expect(resolveVars("{{KNOWN}} {{UNKNOWN}}", env)).toBe("value {{UNKNOWN}}");
    });

    it("does not replace tokens with partial key names", () => {
      // {{BASE}} should not match a variable called BASE_URL
      const env = makeEnv({ BASE_URL: "http://example.com" });
      expect(resolveVars("{{BASE}}", env)).toBe("{{BASE}}");
    });

    it("handles text with no tokens at all", () => {
      const env = makeEnv({ KEY: "val" });
      expect(resolveVars("no tokens here", env)).toBe("no tokens here");
    });

    it("handles environment with no variables (returns text unchanged)", () => {
      const env = makeEnv({});
      expect(resolveVars("{{MISSING}}", env)).toBe("{{MISSING}}");
    });

    it("handles variable with empty string value", () => {
      const env = makeEnv({ EMPTY: "" });
      expect(resolveVars("prefix{{EMPTY}}suffix", env)).toBe("prefixsuffix");
    });

    it("is case-sensitive for variable keys", () => {
      const env = makeEnv({ key: "lower" });
      // {{KEY}} should NOT match variable with key "key"
      expect(resolveVars("{{KEY}}", env)).toBe("{{KEY}}");
      expect(resolveVars("{{key}}", env)).toBe("lower");
    });

    it("handles env with null (returns original text)", () => {
      expect(resolveVars("{{FOO}} bar", null)).toBe("{{FOO}} bar");
    });

    it("replaces a token that spans the entire text", () => {
      const env = makeEnv({ FULL: "replaced" });
      expect(resolveVars("{{FULL}}", env)).toBe("replaced");
    });
  });

  // ── resolveHeaders ────────────────────────────────────────────────────────

  describe("resolveHeaders()", () => {
    it("returns headers unchanged when env is null", () => {
      const headers = { Authorization: "Bearer {{TOKEN}}", Accept: "application/json" };
      expect(resolveHeaders(headers, null)).toEqual(headers);
    });

    it("resolves tokens in all header values", () => {
      const env = makeEnv({ TOKEN: "secret123", VERSION: "v2" });
      const headers = {
        Authorization: "Bearer {{TOKEN}}",
        "X-Api-Version": "{{VERSION}}",
        "Content-Type": "application/json",
      };

      const result = resolveHeaders(headers, env);

      expect(result["Authorization"]).toBe("Bearer secret123");
      expect(result["X-Api-Version"]).toBe("v2");
      expect(result["Content-Type"]).toBe("application/json");
    });

    it("returns a new object without mutating the original headers", () => {
      const env = makeEnv({ TOKEN: "abc" });
      const original = { Authorization: "Bearer {{TOKEN}}" };

      const result = resolveHeaders(original, env);

      expect(result).not.toBe(original);
      expect(original["Authorization"]).toBe("Bearer {{TOKEN}}");
      expect(result["Authorization"]).toBe("Bearer abc");
    });

    it("handles empty headers object", () => {
      const env = makeEnv({ KEY: "val" });
      expect(resolveHeaders({}, env)).toEqual({});
    });

    it("leaves header values unchanged when they contain no tokens", () => {
      const env = makeEnv({ TOKEN: "xyz" });
      const headers = { Accept: "application/json" };
      expect(resolveHeaders(headers, env)).toEqual({ Accept: "application/json" });
    });

    it("leaves unrecognised tokens in header values untouched", () => {
      const env = makeEnv({});
      const headers = { Authorization: "Bearer {{TOKEN}}" };
      expect(resolveHeaders(headers, env)).toEqual({ Authorization: "Bearer {{TOKEN}}" });
    });
  });
});
