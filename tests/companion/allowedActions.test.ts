import { describe, it, expect } from "vitest";
import { V1_COMPANION_ACTIONS } from "@bifurc/engine/transport/legacyCompanionActions";

/**
 * This set is a security boundary: it decides which IPC actions the browser
 * extension companion is allowed to invoke. The companion is a lower-trust caller
 * than the app window, so the allowlist must stay small and additive-only.
 *
 * The exact-contents assertion is intentional — adding a new action should require
 * a deliberate change to this test, which is the review gate for widening the
 * extension's privileges.
 *
 * Moved from `@bifurc/engine/companion/allowedActions` (and `ALLOWED_ACTIONS`) when P4 work
 * item 1 relocated the companion server into the transport layer. The contents are unchanged;
 * the name now says *which* version's surface this is, because the v2 mapping of these four
 * commands is deliberately not a straight alias onto the `CommandRegistry` — see
 * `packages/engine/src/transport/legacyCompanion.ts`'s header for the four verified differences,
 * the load-bearing one being that the registry's mock create can *disable an existing mock*
 * while this allowlist's contract is additive-only.
 */

const EXPECTED = ["config:get", "folder:add", "mock:add", "request:add"];

describe("companion V1_COMPANION_ACTIONS", () => {
    it("contains exactly the four additive actions", () => {
        expect([...V1_COMPANION_ACTIONS].sort()).toEqual(EXPECTED);
    });

    it("is a Set", () => {
        expect(V1_COMPANION_ACTIONS).toBeInstanceOf(Set);
    });

    it("exposes no delete, update or settings actions", () => {
        for (const action of V1_COMPANION_ACTIONS) {
            expect(action, `${action} looks destructive`).not.toMatch(/\b(delete|remove|drop|clear|reset|purge)\b/i);
            expect(action, `${action} looks like a mutation of existing state`).not.toMatch(/\b(update|edit|rename|move|set)\b/i);
        }
    });

    it("only grants `add` writes, never `add` outside the three safe kinds", () => {
        const writes = [...V1_COMPANION_ACTIONS].filter((a) => a.endsWith(":add"));
        expect(writes.sort()).toEqual(["folder:add", "mock:add", "request:add"]);
    });

    it("grants at most one read action", () => {
        const reads = [...V1_COMPANION_ACTIONS].filter((a) => a.endsWith(":get"));
        expect(reads).toEqual(["config:get"]);
    });

    it("uses the `kind:verb` naming convention", () => {
        for (const action of V1_COMPANION_ACTIONS) {
            expect(action, `${action} is not kind:verb`).toMatch(/^[a-z]+:[a-z]+$/);
        }
    });

    it("has no duplicates or empty entries", () => {
        expect(V1_COMPANION_ACTIONS.size).toBe(EXPECTED.length);
        for (const action of V1_COMPANION_ACTIONS) {
            expect(action.length).toBeGreaterThan(0);
        }
    });
});
