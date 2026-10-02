/**
 * Which environment is selected for a workspace.
 *
 * `__global__` is never "selected": it is always the base layer a request resolves against, and the selected
 * environment overrides it key by key (`envState.ts`). So the only real choice is among the *other*
 * environments, and when there is exactly one of them there is nothing to choose — it is the selection.
 *
 * Pure and import-free so `config.ts`, `envState.ts` and the CRUD commands can all use it without a cycle.
 */
const GLOBAL_ENV_ID = "__global__";

/**
 * The selection in force: the stored pointer if it names an environment that exists, otherwise the one
 * non-global environment if there is exactly one, otherwise `null`.
 */
export function effectiveEnvironmentId(
  environments: ReadonlyArray<{ id: string }>,
  storedId: string | null | undefined,
): string | null {
  if (storedId && storedId !== GLOBAL_ENV_ID && environments.some((e) => e.id === storedId)) return storedId;
  const others = environments.filter((e) => e.id !== GLOBAL_ENV_ID);
  return others.length === 1 ? others[0].id : null;
}
