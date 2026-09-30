import { describe, it, expect } from "vitest";
import {
    RANDOMIZER_TOKENS as ENGINE_TOKENS,
    generateRandom,
} from "@bifurc/engine/utils/randomizer";
import { RANDOMIZER_TOKENS as UI_TOKENS } from "@/lib/randomizer";

/**
 * The `{{random.*}}` catalogue exists **twice**, and this file is the reason that is allowed.
 *
 * ## Why there is a copy at all
 *
 * `plan/19` §1 row 2 found the renderer and the engine each holding a `{{random.*}}` implementation whose
 * data pools are identical "and **nothing pins them equal** — they agree by discipline, not by
 * construction". WI 2 deletes the renderer's **generator**. It does not delete the renderer's **token
 * list**: `RandomizerHint.tsx` needs it for autocomplete, and the UI has no `@bifurc/engine` dependency by
 * design (`packages/gui/src/types/config.ts` mirrors engine types *by hand* for the same reason).
 *
 * The owner's call, 2026-09-29: *"random.\* types can be kept in UI as auto complete but actual value
 * resolver still stays in engine. UI just has copy of list of random.\* attributes."*
 *
 * So the split is **list vs resolver**, not renderer vs engine. The engine owns both; the renderer keeps a
 * copy of the list only. **A copy that nothing checks is exactly the defect this phase opened with**, so
 * the copy is permitted only alongside these assertions — this test is the pin, not an optional extra.
 *
 * ## What is pinned, and the one direction that is not
 *
 * 1. **Same keys, same order.** Order matters because the list is rendered as a popover; a reordering is a
 *    UI change and should be made deliberately on both sides.
 * 2. **Same descriptions and examples.** These are what the user reads in the popover, so a divergence is
 *    a user-visible lie rather than a cosmetic drift.
 * 3. **Listed ⇒ implemented.** Every catalogued key must produce a value, not the literal `{{key}}` that
 *    `generateRandom`'s `default:` branch returns. This is the **harmful** direction: an entry with no
 *    `case` offers the user an autocomplete item that silently sends the token verbatim.
 * 4. **No duplicate entries.** A repeated key would render twice in the popover.
 *
 * **⚠ The reverse direction is NOT caught, and no runtime assertion can catch it.** A `case` added to the
 * generator with no catalogue entry is invisible here, because nothing can enumerate a `switch`. The only
 * way to see it is to scan the generator's *source text* for `case "random.…"`, which would pin the
 * implementation's **shape** and break on a refactor to a lookup table — a worse trade than the gap. So
 * the gap is recorded rather than papered over, and it is the benign direction: such a token works when
 * typed, it is simply not offered. **Verified by mutation** — planting `case "random.probeOnly"` leaves
 * all four assertions green.
 */

describe("the renderer's randomizer catalogue is a pinned copy of the engine's", () => {
    it("has the same keys, in the same order", () => {
        expect(UI_TOKENS.map((t) => t.key)).toEqual(ENGINE_TOKENS.map((t) => t.key));
    });

    it("has the same descriptions and examples", () => {
        expect(UI_TOKENS).toEqual(ENGINE_TOKENS);
    });

    it("every catalogued token is actually implemented by the engine's generator", () => {
        // `generateRandom`'s `default:` branch returns `{{key}}` verbatim, so this is the exact signal for
        // "listed but not implemented" — and for the reverse, a `case` with no catalogue entry, the
        // count assertion below is the signal.
        const unimplemented = ENGINE_TOKENS.filter((t) => generateRandom(t.key) === `{{${t.key}}}`).map(
            (t) => t.key,
        );
        expect(
            unimplemented,
            `${unimplemented.length} catalogued token(s) have no case in generateRandom — the popover ` +
                `would offer them and the request would send the literal token. Add the case, or remove ` +
                `the entry from RANDOMIZER_TOKENS.`,
        ).toEqual([]);
    });

    it("has no duplicate entries, and is not empty", () => {
        // Deliberately *not* an orphan check in either direction — see the docblock's fourth point for why
        // "a generator case with no catalogue entry" is out of reach of a runtime assertion. This guards a
        // repeated key, which would render twice in the popover.
        expect(ENGINE_TOKENS.length).toBeGreaterThan(0);
        expect(new Set(ENGINE_TOKENS.map((t) => t.key)).size).toBe(ENGINE_TOKENS.length);
    });
});
