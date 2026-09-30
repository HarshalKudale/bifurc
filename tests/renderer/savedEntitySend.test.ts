/**
 * The Send-by-entity-id gate — `packages/gui/src/lib/savedEntitySend.ts`.
 *
 * ## Why this has a test when the tabs that use it do not
 *
 * The four tabs are React components with no test harness; the *decision* they share is a pure function, and
 * it is the only part of the change where being wrong is silent. Getting it wrong sends a request that
 * differs from the one on screen and returns a plausible response — no error, no log, and the user's only
 * clue is a body they did not expect.
 *
 * So the assertions fail on **each condition individually** rather than only on the whole conjunction: a test
 * that flipped every input at once would pass with either one of them deleted. The `it.each` block below is
 * that teeth-check, and it is the reason this file exists rather than four inline booleans in the tabs.
 *
 * ## The gate is down to two terms, and the last two cases guard the others from returning
 *
 * `plan/19` §7 predicted `!isDraft && !hasLocalChanges` as the end state, and the four protocols' migrations
 * reached it on 2026-09-30: `hasExecutableScripts` left when REST moved, and `usesVariables` when gRPC did —
 * gRPC being the last tab still resolving in the renderer. Both were facts about **where the work ran**, not
 * about the request.
 *
 * **`containsVariableToken` is deleted with `usesVariables`**, and so is its describe block here. It had
 * exactly one consumer — the four tabs' gates — and once they all passed `false` there was nothing left for it
 * to answer. Deleting it rather than leaving it is deliberate: an exported helper with no callers reads as a
 * capability the module still has.
 */
import { describe, it, expect } from "vitest";
import { canSendByEntityId } from "@/lib/savedEntitySend";

/** Every condition satisfied — the state a saved, unedited request is in. */
const ALLOWED = {
    isDraft: false,
    hasLocalChanges: false,
};

describe("canSendByEntityId", () => {
    it("allows the id path for a saved entity with no local changes", () => {
        expect(canSendByEntityId(ALLOWED)).toBe(true);
    });

    /**
     * Each condition, flipped **alone**. This is the block that makes both terms load-bearing: with either
     * one removed from the implementation, exactly one of these cases goes green when it should be red.
     */
    it.each([
        ["isDraft", { isDraft: true }],
        ["hasLocalChanges", { hasLocalChanges: true }],
    ] as const)("refuses the id path when %s alone is set", (_label, override) => {
        expect(canSendByEntityId({ ...ALLOWED, ...override })).toBe(false);
    });

    it("refuses when both conditions are violated", () => {
        expect(canSendByEntityId({ isDraft: true, hasLocalChanges: true })).toBe(false);
    });

    /**
     * The one that reads as already-true and is not. A draft has `hasLocalChanges === false` — by
     * construction, because `useProtocolEditor` defines it as `!draftTabId && …` — so a gate that checked only
     * `hasLocalChanges` would send a **draft** by id, asking the engine to load an entity that has never been
     * written. Asserted as its own case because the input combination is the trap, not either flag on its own.
     */
    it("refuses a draft even though a draft always reports no local changes", () => {
        expect(canSendByEntityId({ ...ALLOWED, isDraft: true, hasLocalChanges: false })).toBe(false);
    });

    /**
     * **Guards against the two deleted terms coming back**, which is why they are tests and not comments.
     *
     * `hasExecutableScripts` was `true` for the REST tab and `false` for the other three — a fact about which
     * tabs had wired scripts up — and it was deleted when REST's migration made it `false` everywhere.
     * `usesVariables` was a genuine per-request question while three tabs still resolved in the renderer, and
     * it was deleted when the last of them stopped. **A constant inside this gate reads as a check being
     * performed when none is**, so each departure is pinned: if someone re-adds either term to the
     * conjunction, one of these two cases goes red.
     */
    it("ignores a stale `hasExecutableScripts` key rather than letting it decide", () => {
        const withStaleKey = { ...ALLOWED, hasExecutableScripts: true } as typeof ALLOWED;
        expect(canSendByEntityId(withStaleKey)).toBe(true);
    });

    it("ignores a stale `usesVariables` key rather than letting it decide", () => {
        const withStaleKey = { ...ALLOWED, usesVariables: true } as typeof ALLOWED;
        expect(canSendByEntityId(withStaleKey)).toBe(true);
    });
});
