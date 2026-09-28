/**
 * The per-kind "how do I label this entity?" resolver, published where any module can reach it.
 *
 * ## Why this is its own module
 *
 * Each kind declares its label shape exactly once, as `CrudFactoryOpts.getNameEntry` in
 * `store/kinds/*.ts`, and `entityCrudCommands.ts` consumes it when it writes a pending deletion. But
 * `sync/gitOps.ts` needs the *same* answer when a revert re-syncs `names.json`, and it cannot reach
 * `entityCrudCommands`'s registry without importing that whole module — which drags in
 * `proxy/server` and `commands/registry` behind it. A second copy of the table in `gitOps` is what
 * the code did instead, and the two copies had already drifted (`grpcRequests`).
 *
 * So the registry **pushes** its resolver here at registration time and the consumer **pulls** from
 * here. The dependency direction is `entityCrudCommands → kindNameEntries ← gitOps`: this module
 * imports nothing but a type, so it cannot participate in a cycle, and there is still exactly one
 * authority for the mapping.
 *
 * A consumer that runs without any kinds registered — every unit test that imports the engine's
 * functions directly instead of going through `registerAll` — gets `null` and falls back to its own
 * table. See `extractNameEntry()` in `sync/gitOps.ts` for that fallback and its reasoning.
 */
import type { EntityNameEntry } from "./workspace/fsNamesIndex";

type NameEntryResolver = (entity: any) => EntityNameEntry | null;

const resolvers = new Map<string, NameEntryResolver>();

/** Called by `registerCrudKind()`. Last registration for a kind wins, matching the registry itself. */
export function registerKindNameEntry(kind: string, resolver: NameEntryResolver): void {
  resolvers.set(kind, resolver);
}

/**
 * The registered label entry for an entity, or `null` when the kind is unregistered, declares no
 * `getNameEntry`, or throws. Never throws: every caller is a best-effort cleanup step, and a
 * malformed entity must not turn a successful revert into a failed one.
 */
export function getKindNameEntry(kind: string, entity: unknown): EntityNameEntry | null {
  const resolver = resolvers.get(kind);
  if (!resolver || !entity || typeof entity !== "object") return null;
  try {
    return resolver(entity) ?? null;
  } catch {
    return null;
  }
}

/** Test-only: drop every registration, so one suite's registrations cannot leak into the next. */
export function clearKindNameEntries(): void {
  resolvers.clear();
}
