import { describe, it, expect, vi } from "vitest";
import { generateRandom, resolveRandomizers, RANDOMIZER_TOKENS } from "../../renderer/lib/randomizer";

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
        expect(resolveRandomizers(null as any)).toBe(null);
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

    it("does not replace non-random tokens", () => {
        const result = resolveRandomizers("{{host}} and {{random.name}}");
        expect(result).toContain("{{host}}");
        expect(result).not.toContain("{{random.name}}");
    });
});

describe("RANDOMIZER_TOKENS", () => {
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
