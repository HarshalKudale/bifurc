import { describe, it, expect } from "vitest";
import { gateCreate, gateEnable } from "@/subscription/entityCount";

/**
 * These two functions are the licensing/plan gate for creating and enabling
 * entities. Right now they are stubs that always allow — which is exactly the kind
 * of thing that regresses silently, because "always allow" and "correctly allow
 * within the free-tier limit" look identical until a paying customer complains.
 *
 * The tests below pin the CURRENT contract (unrestricted) so that adding a real
 * limit forces a deliberate update here, and they assert the shape the callers
 * already rely on (`{ allowed }`, with `current`/`limit` optional).
 */

const KINDS = ["mocks", "mappings", "proxyRules", "requests", "websockets", "webhooks", "environments"];

describe("gateCreate", () => {
    it("allows creation when no limits are enforced", () => {
        expect(gateCreate("ws-1", "mocks")).toEqual({ allowed: true });
    });

    it("allows creation for every entity kind", () => {
        for (const kind of KINDS) {
            expect(gateCreate("ws-1", kind), `kind ${kind}`).toEqual({ allowed: true });
        }
    });

    it("allows creation regardless of workspace", () => {
        expect(gateCreate("", "mocks").allowed).toBe(true);
        expect(gateCreate("some-other-ws", "mocks").allowed).toBe(true);
    });

    it("returns a plain GateResult with only `allowed` populated", () => {
        const res = gateCreate("ws-1", "mocks");
        expect(res.current).toBeUndefined();
        expect(res.limit).toBeUndefined();
    });

    it("does not mutate state between calls", () => {
        const first = gateCreate("ws-1", "mocks");
        const second = gateCreate("ws-1", "mocks");
        expect(first).toEqual(second);
    });
});

describe("gateEnable", () => {
    it("allows enabling when no limits are enforced", () => {
        expect(gateEnable("ws-1", "mocks")).toEqual({ allowed: true });
    });

    it("allows enabling for every entity kind", () => {
        for (const kind of KINDS) {
            expect(gateEnable("ws-1", kind), `kind ${kind}`).toEqual({ allowed: true });
        }
    });

    it("returns a plain GateResult with only `allowed` populated", () => {
        const res = gateEnable("ws-1", "mocks");
        expect(res.current).toBeUndefined();
        expect(res.limit).toBeUndefined();
    });
});
