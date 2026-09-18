/**
 * The companion browser extension's **v1** allowlist — the four `kind:verb` action names the
 * extension is permitted to invoke.
 *
 * Moved here from `src/companion/allowedActions.ts` by P4 work item 1. The file's old home was
 * the reason for the move, not its contents: `src/companion/` stops making sense once the browser
 * extension is just another client, which is the whole point of P4.
 *
 * **The contents are frozen.** `../bifurc-extension` is an external consumer on its own release
 * cycle, and `tests/companion/allowedActions.test.ts` pins this set **exactly** so that widening
 * the extension's privileges requires a deliberate change to that test — it is the review gate.
 *
 * The old file's security note still governs, and it is load-bearing for how these four commands
 * may be migrated: the companion is a **lower-trust** caller (anything running in the user's
 * browser), so this set must stay small and **additive-only**. That constraint is precisely why
 * the v2 mapping of these four commands is *not* the obvious one — see the header of
 * `legacyCompanion.ts`.
 */
export const V1_COMPANION_ACTIONS: ReadonlySet<string> = new Set([
    "mock:add",
    "request:add",
    "folder:add",
    "config:get",
]);
