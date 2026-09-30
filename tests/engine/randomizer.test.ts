import { describe, it, expect } from "vitest";
import { generateRandom, resolveRandomizers, RANDOMIZER_TOKENS } from "@bifurc/engine/utils/randomizer";

/**
 * The `{{random.*}}` generator's cases, moved with it.
 *
 * This file was `tests/renderer/randomizer.test.ts` and imported `@/lib/randomizer` — the renderer's own
 * generator, which `plan/19` WI 6 deleted. It was reachable only through `lib/resolveVars.ts`, which is
 * deleted too, and the engine's copy (`packages/engine/src/utils/randomizer.ts`) was always the one the
 * request pipeline used.
 *
 * **The cases moved rather than being dropped with the file**, which is the same treatment
 * `tests/renderer/curlParser.test.ts` got in WI 5: the module's *behaviour* is not what the boundary rule
 * removed, only its second implementation. Deleting the suite with the file would have left 33 tokens'
 * output formats unpinned on the side that actually generates them.
 *
 * **What is still the renderer's, and where it is pinned:** the token *catalogue* stays in
 * `packages/gui/src/lib/randomizer.ts` as a copy for `RandomizerHint`'s autocomplete (D28 — the list may be
 * the UI's, the resolver may not), and `tests/gui/randomizerCatalogue.test.ts` pins that copy to the
 * authority these cases exercise. The `RANDOMIZER_TOKENS` block below therefore runs against the **engine's**
 * list, which is the one the pin compares the UI's against.
 */
describe("packages/engine/src/utils/randomizer.ts", () => {
  describe("generateRandom()", () => {
    it("generates a full name for random.name", () => {
      const result = generateRandom("random.name");
      expect(result).toMatch(/^\w+ \w+$/);
    });

    it("generates a first name for random.firstName", () => {
      const result = generateRandom("random.firstName");
      expect(result.length).toBeGreaterThan(0);
    });

    it("generates an email for random.email", () => {
      const result = generateRandom("random.email");
      expect(result).toMatch(/^[a-z]+\.[a-z]+\d+@example\.com$/);
    });

    it("generates a phone number for random.phone", () => {
      const result = generateRandom("random.phone");
      expect(result).toMatch(/^\+1-\d{3}-\d{3}-\d{4}$/);
    });

    it("generates an integer for random.int", () => {
      const result = generateRandom("random.int");
      const num = parseInt(result, 10);
      expect(num).toBeGreaterThanOrEqual(0);
      expect(num).toBeLessThanOrEqual(9999);
    });

    it("generates a float for random.float", () => {
      const result = generateRandom("random.float");
      const num = parseFloat(result);
      expect(num).toBeGreaterThanOrEqual(0);
      expect(num).toBeLessThanOrEqual(1);
    });

    it("generates a boolean for random.boolean", () => {
      const result = generateRandom("random.boolean");
      expect(["true", "false"]).toContain(result);
    });

    it("generates a UUID for random.uuid", () => {
      const result = generateRandom("random.uuid");
      expect(result).toMatch(/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/);
    });

    it("generates a short ID for random.id", () => {
      const result = generateRandom("random.id");
      expect(result).toMatch(/^[0-9a-f]{8}$/);
    });

    it("generates a URL for random.url", () => {
      const result = generateRandom("random.url");
      expect(result).toMatch(/^https:\/\/.+\/.+$/);
    });

    it("generates an IPv4 for random.ip", () => {
      const result = generateRandom("random.ip");
      expect(result).toMatch(/^\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}$/);
    });

    it("generates an IPv6 for random.ipv6", () => {
      const result = generateRandom("random.ipv6");
      expect(result.split(":")).toHaveLength(8);
    });

    it("generates a hex color for random.color", () => {
      const result = generateRandom("random.color");
      expect(result).toMatch(/^#[0-9a-f]{6}$/);
    });

    it("generates an ISO date for random.date", () => {
      const result = generateRandom("random.date");
      expect(result).toMatch(/^\d{4}-\d{2}-\d{2}$/);
    });

    it("generates a timestamp for random.timestamp", () => {
      const result = generateRandom("random.timestamp");
      const num = parseInt(result, 10);
      expect(num).toBeGreaterThan(0);
    });

    it("generates a price for random.price", () => {
      const result = generateRandom("random.price");
      expect(result).toMatch(/^\d+\.\d{2}$/);
    });

    /**
     * The `default:` branch, and it is load-bearing rather than defensive: it is what
     * `tests/gui/randomizerCatalogue.test.ts` uses to detect a catalogued key with no `case`. The UI offers
     * a key, the user inserts it, and this branch is what would hand it back verbatim — so the pin tests for
     * exactly this string.
     */
    it("returns {{key}} for unknown tokens", () => {
      expect(generateRandom("random.unknown")).toBe("{{random.unknown}}");
    });

    it("generates a city name", () => {
      const result = generateRandom("random.city");
      expect(result.length).toBeGreaterThan(0);
    });

    it("generates a country name", () => {
      const result = generateRandom("random.country");
      expect(result.length).toBeGreaterThan(0);
    });
  });

  describe("resolveRandomizers()", () => {
    it("returns empty string unchanged", () => {
      expect(resolveRandomizers("")).toBe("");
    });

    it("returns falsy value unchanged", () => {
      expect(resolveRandomizers(null as unknown as string)).toBe(null);
    });

    it("returns text without tokens unchanged", () => {
      expect(resolveRandomizers("plain text")).toBe("plain text");
    });

    it("replaces a single random token", () => {
      const result = resolveRandomizers("id={{random.uuid}}");
      expect(result).not.toContain("{{random.uuid}}");
      expect(result.length).toBeGreaterThan(3);
    });

    it("replaces multiple different tokens", () => {
      const result = resolveRandomizers("{{random.name}} ({{random.email}})");
      expect(result).not.toContain("{{random.name}}");
      expect(result).not.toContain("{{random.email}}");
    });

    it("replaces duplicate tokens with different values", () => {
      // Each occurrence gets a different random value
      const result = resolveRandomizers("{{random.int}}-{{random.int}}");
      // Both are resolved (no longer contain the token)
      expect(result).not.toContain("{{random.int}}");
    });

    /**
     * **The boundary this module must not cross.** `{{host}}` is an environment variable, and the randomizer
     * resolves only `random.*`. Collapsing the two passes is what `utils/resolve.ts`'s docblock warns against:
     * `resolveVars` skips `random.*` during its env pass and hands the result here, so the two are separable —
     * which is what lets the mocks' suite stub the generator without stubbing resolution.
     */
    it("does not replace non-random tokens", () => {
      const result = resolveRandomizers("{{host}} and {{random.name}}");
      expect(result).toContain("{{host}}");
      expect(result).not.toContain("{{random.name}}");
    });
  });

  describe("RANDOMIZER_TOKENS (the authority)", () => {
    it("contains at least 30 tokens", () => {
      expect(RANDOMIZER_TOKENS.length).toBeGreaterThanOrEqual(30);
    });

    it("each token has key, description, and example", () => {
      for (const token of RANDOMIZER_TOKENS) {
        expect(token.key).toMatch(/^random\.\w+$/);
        expect(token.description.length).toBeGreaterThan(0);
        expect(token.example.length).toBeGreaterThan(0);
      }
    });

    it("all token keys can be generated without error", () => {
      for (const token of RANDOMIZER_TOKENS) {
        const result = generateRandom(token.key);
        expect(result).not.toBe(`{{${token.key}}}`);
      }
    });
  });
});
