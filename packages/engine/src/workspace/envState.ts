/**
 * The environment a workspace's requests resolve against — `__global__` as the base, the active
 * environment over it. One function, because the pipeline resolves against it and the collection runner
 * will too.
 *
 * ## Why this is engine-side now
 *
 * `plan/19` D23. The renderer has merged these two on every render since long before the engine resolved
 * anything (`packages/gui/src/App.tsx:288`) and then handed the **merged** environment down to
 * `resolveVars`. That made the UI the authority for a workspace fact the engine already owns: the
 * `environments` entity kind, the `activeEnvironmentId` settings pointer and the synthetic `__global__`
 * are all engine-side already. The renderer's merge is WI 2's to delete.
 *
 * ## ⚠ The merge is not a detail — `plan/19` §8 trap 2
 *
 * Resolving against the active environment **alone** resolves a *shadowed* key differently from the UI,
 * and every test that uses no shadowed key passes. That is why `mergeEnvVars` travelled with the call
 * rather than being added beside `resolveVars` (`plan/19` §11 finding 2).
 *
 * ## A pure read, which is why it does not use `loadWorkspaceDispatchSlice`
 *
 * The obvious reuse is `loadWorkspaceDispatchSlice()`, and it is the wrong one here: it also reads
 * `mocks`/`mappings` and calls `bootstrapEnabledSet()`, which **writes** `enabled.json`. An environment
 * lookup must not repair an unrelated index as a side effect — the same objection `config.ts` raises
 * against auto-creating `__global__` in a host member's config load. So this reads the two things it
 * needs directly.
 *
 * ## `__global__` is absent, not empty, and the two mean the same thing
 *
 * `loadConfig()` auto-creates `__global__` for the active workspace; this file does not, and it does not
 * need to. The auto-created entity has **no variables**, so "absent" and "present but empty" merge to
 * the same result. Reading an absent `__global__` as "there is no global environment" would be wrong on
 * the desktop, where `loadConfig()` has usually created it already — but it changes nothing, which is
 * exactly why the shortcut is safe.
 */
import type { Environment } from "./config";
import { readAllEntities } from "./fs/fsRead";
import { loadSettings } from "./appSettings";
import { mergeEnvVars } from "../utils/resolve";
import { effectiveEnvironmentId } from "./envSelection";

/**
 * The synthetic global environment's id.
 *
 * A literal everywhere else in the engine (`config.ts:148`, `dispatch.ts:55`, `entityCrudCommands.ts`),
 * and deliberately still a literal here rather than a new exported constant: introducing one would make
 * this file a second place the value is *declared*, and the four existing sites would keep their own.
 * When someone consolidates them, this is one of the sites to fold in.
 */
const GLOBAL_ENV_ID = "__global__";

/** The workspace's effective environment — `__global__` overridden by the active environment, or `null`. */
export function loadMergedEnvironment(wsId: string): Environment | null {
  const environments = readAllEntities<Environment>(wsId, "environments");
  const globalEnv = environments.find((e) => e.id === GLOBAL_ENV_ID) ?? null;
  // Read from settings rather than from the entity list: the pointer is a *settings* field, which is
  // what `loadConfig()` resolves for the active workspace and what `loadWorkspaceDispatchSlice()` reads
  // for any workspace. An `environments` entity is not "active" by virtue of anything on itself.
  const activeId = effectiveEnvironmentId(
    environments,
    loadSettings().workspaces.find((w) => w.id === wsId)?.activeEnvironmentId,
  );
  const activeEnv = environments.find((e) => e.id === activeId) ?? null;
  return mergeEnvVars(globalEnv, activeEnv);
}
