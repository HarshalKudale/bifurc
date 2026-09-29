/**
 * The per-workspace **entity-status cache** — the `wsId → { relPath → status }` map that
 * `gitops/statusTracker.ts` populates and several writers invalidate.
 *
 * ## Why this is a `workspace/` file and not a `gitops/` one
 *
 * The cache is *derived from* git status, so it reads as a `gitops` concern — and it lived in
 * `gitops/statusTracker.ts` until engine-layout phase 7. It is here instead because of the layering
 * rule (`Engine_Module_Plan.md` §5.1/§6): **`workspace/` may import only `kernel/` and `utils/`**,
 * while feature modules may import `workspace/`. Two of the three invalidators are `workspace/`-side —
 * `workspace/entities/entityCrudCommands.ts` (on delete) and `workspace/healthbarCommands.ts` (on
 * `healthbar.saveServices`) — so a cache owned by `gitops/` forced a `workspace → gitops` import that
 * §6 forbids.
 *
 * That edge already existed before this extraction (`entityCrudCommands.ts` imported
 * `../../gitops/statusTracker` for exactly this function), and phase 3 introduced it when `store/` was
 * split. Moving the cache down is the minimal repair that makes §4.5's assignment of the `healthbar.*`
 * commands to `workspace/` legal: the arrow now runs `gitops → workspace` (allowed) instead of
 * `workspace → gitops` (forbidden).
 *
 * The `gitops` side is unaffected — `statusTracker.ts` re-exports `invalidateCache` so its two internal
 * callers (`gitOps.ts`, `syncCommands.ts`) keep importing it from where they always have.
 *
 * ## What it is not
 *
 * Not a general-purpose cache. It holds exactly one thing, is populated in exactly one place
 * (`getWorkspaceSyncStatus`), and is evicted by writers that know the workspace's git state has moved.
 * It is deliberately a module-level `Map` rather than a class: it is process-global state keyed by
 * workspace, which is what every caller wants and what the previous single-file version already was.
 */

/** A single entity file's git status, as the renderer reads it. */
export type EntitySyncStatus = "clean" | "modified" | "new" | "deleted";

const _cache = new Map<string, Record<string, EntitySyncStatus>>();

/** Replace the cached snapshot for `wsId` — called by `getWorkspaceSyncStatus()` after a fresh read. */
export function setCachedStatus(wsId: string, status: Record<string, EntitySyncStatus>): void {
  _cache.set(wsId, status);
}

/** The last snapshot read for `wsId`, or `{}` when none was ever taken. */
export function getCachedStatus(wsId: string): Record<string, EntitySyncStatus> {
  return _cache.get(wsId) ?? {};
}

/**
 * Drop the snapshot for `wsId`, so the next `getWorkspaceSyncStatus()` re-reads git rather than serving
 * a stale map. Called on every write that can change a workspace's git status — entity delete, entity
 * revert (`gitOps.ts`), the sync commands, and `healthbar.saveServices`.
 */
export function invalidateCache(wsId: string): void {
  _cache.delete(wsId);
}
