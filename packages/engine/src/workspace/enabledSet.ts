/**
 * Add or remove an entity from a workspace's enabled-set, bootstrapping the set from the entity files
 * on the first write.
 *
 * ## Why this module exists
 *
 * It existed **three times** before it existed here:
 * `src/ipc/handlers/utils.ts`, `src/ipc/handlers/entityCrudFactory.ts`, and a private copy in
 * `packages/engine/src/transport/legacyCompanion.ts`. All three were semantically identical — the
 * engine's used `??` where the shell's used `if (!set)`, which is the same thing because
 * `readEnabledSet()` returns `Set<string> | null` and an empty `Set` is truthy.
 *
 * That is the near-identical duplication worth collapsing rather than tolerating: this function
 * decides whether a mock, mapping or rule is **served by the proxy**, so two copies disagreeing by one
 * branch would mean the same entity behaving differently depending on which code path toggled it — and
 * the enabled-set is written to disk (`enabled.json`) rather than held in memory, so the disagreement
 * would outlive the process.
 *
 * The P6 §5c pass needed this inside the engine anyway (`entity.setEnabled` moves here from the
 * shell), so the consolidation and the move are the same edit.
 *
 * It lives in `store/` rather than `store/workspace/` because it is a *policy* helper over the fs
 * primitives (`readEnabledSet` / `bootstrapEnabledSet` / `writeEnabledSet`), not a primitive itself.
 */
import { readEnabledSet, writeEnabledSet, bootstrapEnabledSet } from "./workspaceFs";

export function syncEnabledSet(wsId: string, kind: string, id: string, enabled: boolean): void {
    const set = readEnabledSet(wsId, kind) ?? bootstrapEnabledSet(wsId, kind);
    if (enabled) set.add(id);
    else set.delete(id);
    writeEnabledSet(wsId, kind, set);
}
