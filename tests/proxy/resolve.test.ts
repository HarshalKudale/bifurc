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

import { resolveVars, resolveHeaders, mergeEnvVars } from "@bifurc/engine/utils/resolve";

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

    /**
     * The four below came from `tests/renderer/resolveVars.test.ts`, which `plan/19` WI 6 deleted along with
     * `packages/gui/src/lib/resolveVars.ts`. They are the cases the engine's suite did **not** already have,
     * and each pins a behaviour that is easy to lose in a rewrite: the token regex is `\w+`-bounded so a
     * partial key cannot match, lookups are case-sensitive, an empty value substitutes to nothing, and the
     * input text is never mutated.
     */
    it("does not match a partial key name", () => {
        const env = { variables: [{ key: "BASE_URL", value: "http://example.com" }] } as any;
        expect(resolveVars("{{BASE}}", env)).toBe("{{BASE}}");
    });

    it("is case-sensitive for variable keys", () => {
        const env = { variables: [{ key: "key", value: "lower" }] } as any;
        expect(resolveVars("{{KEY}}", env)).toBe("{{KEY}}");
        expect(resolveVars("{{key}}", env)).toBe("lower");
    });

    it("substitutes an empty value as nothing", () => {
        const env = { variables: [{ key: "EMPTY", value: "" }] } as any;
        expect(resolveVars("prefix{{EMPTY}}suffix", env)).toBe("prefixsuffix");
    });

    it("leaves the input text untouched", () => {
        const env = { variables: [{ key: "K", value: "v" }] } as any;
        const input = "{{K}}";
        expect(resolveVars(input, env)).toBe("v");
        expect(input).toBe("{{K}}");
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

    it("returns a new record rather than mutating the input", () => {
        const env = { variables: [{ key: "TOKEN", value: "abc" }] } as any;
        const original = { Authorization: "Bearer {{TOKEN}}" };
        const result = resolveHeaders(original, env);
        expect(result).not.toBe(original);
        expect(original.Authorization).toBe("Bearer {{TOKEN}}");
        expect(result.Authorization).toBe("Bearer abc");
    });
});

/**
 * `mergeEnvVars` — the `__global__` + active merge, and **the reason it is asserted here rather than in prose**.
 *
 * `plan/19` §8 trap 2: resolving against the *active* environment alone resolves a **shadowed** key
 * differently, and every test that uses no shadowed key passes either way. That makes the shadowed case the
 * only one worth writing down, and it is why the merge travelled with the call rather than being added beside
 * `resolveVars` (§11 finding 2).
 *
 * These cases arrived with `env.getMerged` (`plan/19` WI 6), which is the read that let `App.tsx` stop
 * computing the merge — so the renderer's copy and the engine's are now the same function or, more precisely,
 * there is only one.
 */
describe("mergeEnvVars()", () => {
    const env = (vars: [string, string][], id = "e") => ({
        id,
        name: id,
        variables: vars.map(([key, value]) => ({ id: key, key, value })),
    }) as any;

    it("returns null when there is neither a global nor an active environment", () => {
        expect(mergeEnvVars(null, null)).toBeNull();
    });

    it("returns the global environment when there is no active one", () => {
        const merged = mergeEnvVars(env([["a", "1"]], "__global__"), null);
        expect(merged?.variables.map((v: any) => [v.key, v.value])).toEqual([["a", "1"]]);
    });

    it("lets the ACTIVE value win a shadowed key, and keeps a global-only key reachable", () => {
        const merged = mergeEnvVars(
            env([["shared", "global"], ["onlyGlobal", "yes"]], "__global__"),
            env([["shared", "active"]], "active"),
        );
        const map = new Map(merged!.variables.map((v: any) => [v.key, v.value]));
        expect(map.get("shared")).toBe("active");
        expect(map.get("onlyGlobal")).toBe("yes");
        expect(merged!.variables).toHaveLength(2);
    });

    it("marks the result as a view rather than a stored entity", () => {
        const merged = mergeEnvVars(env([["a", "1"]], "__global__"), null);
        expect(merged!.id).toBe("merged");
        expect(merged!.name).toBe("merged");
    });
});
