/**
 * The one place the workspace-scoping field is defined (P14 D13 / D17).
 *
 * ## Why this is shared rather than repeated
 *
 * Every workspace-scoped command needs the same optional `workspaceId`, and the temptation is to
 * inline `{ workspaceId: z.string().optional() }` in each command file. That is four copies of one
 * contract, and the plan's own history is a catalogue of what happens to a count or a shape that
 * exists in two places (`plan/07` §5b: five engine commands quoted as "six" across four sections).
 * The field's *type* is also the thing a client depends on most directly, so a divergence would be
 * a compatibility break that no single file looks wrong enough to catch.
 *
 * ## Why it is optional, and what absent means
 *
 * `.optional()` is **the whole compatibility story** and not a convenience. The desktop constructs
 * one engine and addresses "whatever the user has selected", so it has no id to send — and D13
 * chose *one RPC endpoint per host, workspace id in params* precisely so that an id-less caller
 * keeps working. A host member always sends one. Both are valid, so the field cannot be required,
 * and a test asserting only "present is accepted" would pass if someone made it required and
 * broke every desktop caller.
 *
 * ## Why every consumer must call `.strict()` on the result
 *
 * Merging this object into a `.strict()` schema keeps the strictness only if `.strict()` is the
 * **outermost** call. `Base.strict().merge(WorkspaceScope)` silently produces a schema where
 * unknown keys are *permitted*, because `merge` returns a fresh object schema and the strictness
 * did not survive. **Measured on zod 4.6.5, not assumed** — the two orders disagree:
 *
 * ```
 * Base.strict().merge(Scope).safeParse({ a: "x", bogus: 1 }).success   → true    ← wrong, silently
 * Base.merge(Scope).strict().safeParse({ a: "x", bogus: 1 }).success   → false   ← correct
 * ```
 *
 * A `.strict()` command that merged this in the wrong order would stop rejecting unknown keys
 * while still passing every "the valid fixture parses" test — the fixtures that exist to pin
 * `.strict()` are the ones in `fixtures.ts`, and they are near-misses on *known* fields in most
 * rows, so the loss of strictness would be invisible. Always `Base.merge(WorkspaceScope).strict()`,
 * never `Base.strict().merge(WorkspaceScope)`. For a command with no other params,
 * `WorkspaceScope.strict()` is the whole schema and the order does not arise.
 */
import { z } from "zod";

export const WorkspaceScope = z.object({
  /** The workspace to act on. Omitted → the active workspace (the desktop's behaviour). */
  workspaceId: z.string().optional(),
});

export type WorkspaceScope = z.infer<typeof WorkspaceScope>;
