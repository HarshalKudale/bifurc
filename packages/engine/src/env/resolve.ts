/**
 * Environment-variable and header resolution — the engine's **one** resolver.
 *
 * ## Why this file exists
 *
 * `plan/19` §1 row 1: resolution was written twice — privately inside `mocks/mockHandler.ts`, and again
 * in the renderer (`packages/gui/src/lib/resolveVars.ts`). The renderer's copy is P19 WI 2's to delete;
 * **this** module is where the engine's copy moved, so that `mocks/` and the request pipeline WI 3 adds
 * can share one implementation instead of two that agree by discipline alone.
 *
 * `Engine_Module_Plan.md` §5.1's placement rule decides the home: the functions are stateless and
 * shared, so they are extracted **down** into `utils/`. They read no file, no config and no sibling
 * module's state — `Environment` arrives as an argument, and it is imported as a **type**, which the
 * layering ratchet exempts because `import type` is erased at compile time and so creates no runtime
 * edge from `utils/` to `workspace/`.
 *
 * ## The `{{random.*}}` delegation is load-bearing, not tidiness
 *
 * `resolveVars` deliberately does **not** mint random values. It skips `random.*` keys during the env
 * pass and then hands the result to `resolveRandomizers`. That split is what lets the mocks' suite stub
 * the generator without stubbing resolution (`tests/proxy/mockHandler.test.ts` mocks the randomizer
 * module for exactly this reason), and it is what makes the statement *"a randomizer is
 * non-deterministic, so the stored entity holds the token, never the value"* true — which is a premise
 * `packages/gui/src/lib/savedEntitySend.ts` relies on and `plan/19` §5 turns into the reason the
 * pipeline command carries its body as text. Collapsing the two passes here would change both.
 *
 * ## What is deliberately **not** here yet
 *
 * `mergeEnvVars` — the global + active merge — is part of WI 2 (D23) and is **not** in this file. It
 * moves with the **call**, not with the function: the merge decides *which* environment to resolve
 * against, and until the engine is the resolving party (WI 3) a second merge implementation would exist
 * purely to be unused, while the renderer's `App.tsx:288` merge stayed authoritative. Landing it here
 * now would create the duplication `plan/19` §8 trap 1 warns about instead of removing one. It arrives
 * in the same change that makes the engine resolve.
 */
import type { Environment } from "../workspace/config";
import { resolveRandomizers } from "../utils/randomizer";

/** Replace `{{KEY}}` with env vars, then `{{random.*}}`. Unknown keys are left verbatim. */
export function resolveVars(text: string, env: Environment | null): string {
  if (!text) return text;
  let result = text;
  if (env) {
    result = result.replace(/\{\{(\w+)\}\}/g, (_match, key) => {
      // Skip random.* tokens - handled separately below
      if (key.startsWith("random.")) return _match;
      const v = env.variables.find((v) => v.key === key);
      return v !== undefined ? v.value : _match;
    });
  }
  // Then resolve randomizers
  return resolveRandomizers(result);
}

/**
 * Resolve every value in a headers record, leaving the keys untouched.
 *
 * Landed with `resolveVars` as the other half of the same surface — a resolver split across two files is
 * how the two copies this module exists to remove came about in the first place. `pipeline/` is its
 * caller.
 */
export function resolveHeaders(
  headers: Record<string, string>,
  env: Environment | null,
): Record<string, string> {
  const out: Record<string, string> = {};
  for (const [k, v] of Object.entries(headers)) out[k] = resolveVars(v, env);
  return out;
}

/**
 * Merge global env vars (base) with active env vars (override) into a single `Environment`.
 *
 * `packages/gui/src/App.tsx:288` has done this on every render since long before the engine resolved
 * anything, and then passed the **merged** environment down to `resolveVars` — which made the UI the
 * authority for a workspace fact the engine already owns. `plan/19` D23 makes this the authority, and
 * §8 trap 2 makes the merge part of the move rather than a detail: resolving against the *active*
 * environment alone resolves a **shadowed** key differently from the UI, and every test that uses no
 * shadowed key passes either way.
 *
 * **Why it was not here from the start, and why that timing is the point.** A merge with no engine-side
 * caller would have been a *second* implementation while `App.tsx`'s stayed authoritative and ran on
 * every render — `plan/19` §8 trap 1's duplication, introduced rather than removed. So it travels with
 * the **call**, not with the function (`plan/19` §11 finding 2), and `pipeline/` is that call.
 *
 * Insertion order is the renderer's — global first, active second — so the active value wins a
 * collision. Order is otherwise immaterial: `resolveVars` looks keys up with `.find()`, and the map
 * de-duplicates by key.
 */
export function mergeEnvVars(
  globalEnv: Environment | null,
  activeEnv: Environment | null,
): Environment | null {
  if (!globalEnv && !activeEnv) return null;
  const map = new Map<string, Environment["variables"][number]>();
  for (const v of globalEnv?.variables ?? []) map.set(v.key, v);
  for (const v of activeEnv?.variables ?? []) map.set(v.key, v);
  return {
    id: "merged",
    name: "merged",
    variables: [...map.values()],
    createdAt: 0,
    workspaceId: "",
  };
}
