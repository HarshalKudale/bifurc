import { describe, it, expect } from "vitest";
import { ALLOWED_ACTIONS } from "@bifurc/engine/companion/allowedActions";

/**
 * This set is a security boundary: it decides which IPC actions the browser
 * extension companion is allowed to invoke. The companion is a lower-trust caller
 * than the app window, so the allowlist must stay small and additive-only.
 *
 * The exact-contents assertion is intentional — adding a new action should require
 * a deliberate change to this test, which is the review gate for widening the
 * extension's privileges.
 */

const EXPECTED = ["config:get", "folder:add", "mock:add", "request:add"];

describe("companion ALLOWED_ACTIONS", () => {
    it("contains exactly the four additive actions", () => {
        expect([...ALLOWED_ACTIONS].sort()).toEqual(EXPECTED);
    });

    it("is a Set", () => {
        expect(ALLOWED_ACTIONS).toBeInstanceOf(Set);
    });

    it("exposes no delete, update or settings actions", () => {
        for (const action of ALLOWED_ACTIONS) {
            expect(action, `${action} looks destructive`).not.toMatch(/\b(delete|remove|drop|clear|reset|purge)\b/i);
            expect(action, `${action} looks like a mutation of existing state`).not.toMatch(/\b(update|edit|rename|move|set)\b/i);
        }
    });

    it("only grants `add` writes, never `add` outside the three safe kinds", () => {
        const writes = [...ALLOWED_ACTIONS].filter((a) => a.endsWith(":add"));
        expect(writes.sort()).toEqual(["folder:add", "mock:add", "request:add"]);
    });

    it("grants at most one read action", () => {
        const reads = [...ALLOWED_ACTIONS].filter((a) => a.endsWith(":get"));
        expect(reads).toEqual(["config:get"]);
    });

    it("uses the `kind:verb` naming convention", () => {
        for (const action of ALLOWED_ACTIONS) {
            expect(action, `${action} is not kind:verb`).toMatch(/^[a-z]+:[a-z]+$/);
        }
    });

    it("has no duplicates or empty entries", () => {
        expect(ALLOWED_ACTIONS.size).toBe(EXPECTED.length);
        for (const action of ALLOWED_ACTIONS) {
            expect(action.length).toBeGreaterThan(0);
        }
    });
});
