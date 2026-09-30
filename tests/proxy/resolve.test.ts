import { describe, it, expect, vi } from "vitest";

/**
 * The resolver's own suite — moved here **with the function**, from `tests/proxy/mockHandler.test.ts`,
 * when `plan/19` WI 2 extracted `resolveVars` down out of `packages/engine/src/mocks/mockHandler.ts`
 * into `packages/engine/src/utils/resolve.ts`. A test that pins a split moves with it: leaving these
 * cases under the mocks' file while importing the function from `utils/` would send a reader looking for
 * the resolver's tests to the wrong place, which is the failure the move is meant to avoid rather than
 * cause.
 *
 * The randomizer is stubbed for the same reason it was stubbed over there, and the reason is the
 * behaviour under test: `resolveVars` **delegates** `{{random.*}}` instead of generating values, so a
 * real generator would make the assertion depend on randomness. Stubbing it is how "the delegation
 * happened, and it happened after the env pass" is stated.
 */
vi.mock("@bifurc/engine/utils/randomizer", () => ({
    resolveRandomizers: vi.fn((text: string) => text),
}));

import { resolveVars, resolveHeaders } from "@bifurc/engine/utils/resolve";

describe("resolveVars()", () => {
    it("returns text as-is when env is null", () => {
        expect(resolveVars("hello {{name}}", null)).toBe("hello {{name}}");
    });

    it("returns empty string for empty input", () => {
        expect(resolveVars("", null)).toBe("");
    });

    it("returns falsy value unchanged", () => {
        expect(resolveVars(undefined as any, null)).toBe(undefined);
    });

    it("replaces known env variables", () => {
        const env = { variables: [{ key: "host", value: "localhost" }] } as any;
        expect(resolveVars("http://{{host}}/api", env)).toBe("http://localhost/api");
    });

    it("leaves unknown variables as {{key}}", () => {
        const env = { variables: [{ key: "a", value: "1" }] } as any;
        expect(resolveVars("{{a}} and {{b}}", env)).toBe("1 and {{b}}");
    });

    it("does not resolve random.* tokens itself (delegates to resolveRandomizers)", () => {
        const env = { variables: [] } as any;
        const result = resolveVars("id={{random.uuid}}", env);
        expect(result).toBe("id={{random.uuid}}");
    });

    it("replaces multiple occurrences of the same variable", () => {
        const env = { variables: [{ key: "v", value: "X" }] } as any;
        expect(resolveVars("{{v}}-{{v}}", env)).toBe("X-X");
    });
});

describe("resolveHeaders()", () => {
    it("resolves every value and leaves the keys untouched", () => {
        const env = { variables: [{ key: "host", value: "localhost" }] } as any;
        expect(resolveHeaders({ "x-host": "{{host}}", accept: "application/json" }, env)).toEqual({
            "x-host": "localhost",
            accept: "application/json",
        });
    });

    it("returns an empty record for no headers, with or without an env", () => {
        expect(resolveHeaders({}, null)).toEqual({});
        expect(resolveHeaders({}, { variables: [] } as any)).toEqual({});
    });
});
