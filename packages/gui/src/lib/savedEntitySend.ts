/**
 * When may a Send go to the engine as an **entity id** instead of as an inline request?
 *
 * ## The two input forms, and why the choice is not free
 *
 * All four protocols' `*.send` commands take a **union** (`plan/19` D24/D27): `{workspaceId, id}` names a
 * saved entity for the engine to read, and the inline form carries what the caller assembled. The engine
 * decides from the shape, so this module answers one question per Send:
 *
 * - **id form** — the engine reads the entity off disk and runs *that*.
 * - **inline form** — the renderer sends every field it holds. Required for a draft, and for anything the
 *   editor has edited but not yet written.
 *
 * The id form is only correct when the stored entity is a **faithful and complete description of the request
 * the user is looking at**. When it is not, the id form silently sends something else — a stale body, a
 * literal `{{token}}` where a value was meant — and the response looks plausible. That is the failure this
 * module exists to prevent, and it is why the gate is conservative rather than clever.
 *
 * ## `!isDraft && !hasLocalChanges` — the end state `plan/19` §7 predicted, reached 2026-09-30
 *
 * The gate had **four** terms when `plan/19` was written. Two were facts about **where the work ran**, not
 * about the request, and §1 states the general form — *"a gate that has to encode 'the client and the engine
 * would disagree here' is a bug report about the architecture"*. Both are now gone, and the order they left
 * in *is* the history of WI 3:
 *
 * | term | left when | why |
 * |---|---|---|
 * | `hasExecutableScripts` | REST's migration | `true` for REST alone and `false` for the other three — a fact about which tabs had wired scripts up, so REST's change made it a constant |
 * | `usesVariables` | **gRPC's migration**, the last of the four | the last tab that resolved in the renderer stopped, so a stored `{{token}}` can no longer make the disk and the screen disagree |
 *
 * What is left is a question the UI can answer about itself — which is what §7 predicted, and the reason the
 * two terms could be **deleted** rather than weakened. **Nothing may be re-added here**: a fifth term would be
 * a new instance of the bug this file's history is a record of, and `tests/renderer/savedEntitySend.test.ts`
 * carries cases guarding both departures.
 *
 * ## Why one module rather than four inline checks
 *
 * Four tabs ask this question. Four copies of a boolean expression is four chances for one of them to drop
 * the condition that matters, and the symptom — a request that sends a stale body — is invisible in review.
 * The conditions are also the kind that read as *already satisfied* to whoever wrote the tab: "of course
 * it's saved, it's in the sidebar".
 *
 * ## What this deliberately does not do
 *
 * It does not attempt to *detect* the difference by comparing the resolved request against the stored one.
 * That would be exact rather than conservative, and it is the obvious next thought — but it costs a full
 * resolution pass on every Send just to decide how to send, and it would still not cover the scripts, whose
 * effect on the request cannot be known without running them. A conservative gate that falls back to the
 * inline form is cheap and cannot regress.
 */

export interface SavedEntitySendInput {
    /** The tab is unsaved (a `draftTabId` is set) — there is no entity on disk to load. */
    isDraft: boolean;
    /** `useProtocolEditor`'s `hasLocalChanges`: pending autosave, or a git-sync status other than clean. */
    hasLocalChanges: boolean;
}

/**
 * True when sending by entity id produces the **same request** as sending the inline form.
 *
 * Both terms are reasons the two could differ, so this is a conjunction and there is no "mostly".
 *
 * **⚠ `isDraft` is not redundant with `hasLocalChanges`, and the reason is worth keeping.** A draft reports
 * `hasLocalChanges === false` **by construction** — there is no file for the editor to differ from — so a
 * gate checking only that would send a draft by id, and the engine would answer `NOT_FOUND` for an entity
 * that was never saved.
 */
export function canSendByEntityId(input: SavedEntitySendInput): boolean {
    return !input.isDraft && !input.hasLocalChanges;
}
