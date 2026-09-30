/**
 * `config.*`, `workspace.*`, `env.setActive` — top-level config and workspace lifecycle.
 * Config itself stays `z.record` at the wire boundary (see the note in `entity.ts`); it is a
 * large, evolving shape (`AppConfig`) that today's engine already owns and validates on load.
 */
import { z } from "zod";

export const ConfigGetParams = z.object({}).strict();
export type ConfigGetParams = z.infer<typeof ConfigGetParams>;
export type ConfigGetResult = Record<string, unknown>; // AppConfig

/**
 * **Deprecated — no client should send this.** It replaces the host settings *and* rewrites every mock,
 * mapping and environment of the active workspace from the caller's copy, so a stale copy silently reverts
 * changes made by anyone else (another client, a git pull, the engine itself). Settings go through
 * `settings.update`; entities go through `entity.*`. Kept only until its remaining test coverage moves.
 */
export const ConfigSaveParams = z.object({
  config: z.record(z.string(), z.unknown()),
});
export type ConfigSaveParams = z.infer<typeof ConfigSaveParams>;
export interface ConfigSaveResult {
  ok: boolean;
}

const port = z.number().int().min(1).max(65535);

/**
 * `settings.update` — change host-local settings by **patch**. Only the fields present are written; nothing
 * else in the settings or in any workspace is touched. A listener is re-bound only when its own setting
 * changed, and a forward proxy the user stopped stays stopped.
 */
export const SettingsUpdateParams = z.object({
  port: port.optional(),
  forwardProxyPort: port.optional(),
  companionPort: port.optional(),
  minimizeToTray: z.boolean().optional(),
  tlsEnabled: z.boolean().optional(),
  tlsCaCertPath: z.string().nullable().optional(),
  tlsCaKeyPath: z.string().nullable().optional(),
}).strict();
export type SettingsUpdateParams = z.infer<typeof SettingsUpdateParams>;
export interface SettingsUpdateResult {
  ok: boolean;
}
// NOTE (work item 1, SPLIT): the legacy `config:save` also calls `updateTrayMenu()` — a
// shell-only side effect. The ENGINE half is config persistence + server restart; the shell
// subscribes to `event.config.changed` (new, added alongside `entity.changed`) to update its
// own tray instead of the engine reaching into `@/main`.

export const WorkspaceAddParams = z.object({
  name: z.string(),
});
export type WorkspaceAddParams = z.infer<typeof WorkspaceAddParams>;
export interface WorkspaceAddResult {
  id: string;
  name: string;
  createdAt: number;
  activeEnvironmentId: string | null;
}

export const WorkspaceRenameParams = z.object({
  id: z.string(),
  name: z.string(),
});
export type WorkspaceRenameParams = z.infer<typeof WorkspaceRenameParams>;
export interface WorkspaceRenameResult {
  ok: boolean;
}

export const WorkspaceDeleteParams = z.object({
  id: z.string(),
});
export type WorkspaceDeleteParams = z.infer<typeof WorkspaceDeleteParams>;
export interface WorkspaceDeleteResult {
  ok: boolean;
}

export const WorkspaceSetActiveParams = z.object({
  id: z.string(),
});
export type WorkspaceSetActiveParams = z.infer<typeof WorkspaceSetActiveParams>;
export interface WorkspaceSetActiveResult {
  ok: boolean;
  config: Record<string, unknown>;
}

export const EnvSetActiveParams = z.object({
  id: z.string().nullable(),
});
export type EnvSetActiveParams = z.infer<typeof EnvSetActiveParams>;
export interface EnvSetActiveResult {
  ok: boolean;
}

/**
 * `env.resolveText` — resolve `{{tokens}}` in a string against the workspace's **effective** environment
 * (`plan/19` WI 5, item 4).
 *
 * ## Why this is a command and not a renderer helper
 *
 * WI 3 made the engine the resolving party: a `{{host}}` in a request reaches the engine verbatim and is
 * resolved there. That left one legitimate renderer need it cannot satisfy — **display**. The health bar
 * shows the url a check will actually probe, and after WI 3 the renderer has no resolver to compute it
 * with (`plan/19` D22; the renderer's copy is a boundary violation).
 *
 * The alternative was to let the probe return the url it used, and it is worse: the panel shows the
 * resolved url *before* the first check (a service with auto-refresh off may never be checked), so a
 * check-only path would leave the card showing `{{host}}/health` indefinitely. Resolving on demand is the
 * shape that matches what the UI is actually asking.
 *
 * ## "Effective" is the whole point — `plan/19` §8 trap 2
 *
 * The engine merges `__global__` (base) with the active environment (override) before resolving
 * (`workspace/envState.ts`), because resolving against the *active* environment alone resolves a
 * **shadowed** key differently — and every test that uses no shadowed key passes either way. A caller
 * cannot ask for the un-merged behaviour: that option existed in the renderer and is what made the UI the
 * authority for a workspace fact the engine already owned.
 *
 * ## Scope
 *
 * `read`. It observes — no file is written, no upstream is reached, and `{{random.*}}` values are minted
 * fresh on each call, which is the one thing that makes the command's output non-reproducible without
 * making it a side effect. **A caller must not treat the result as stable**; a request that needs a value
 * stable across a retry must let the engine resolve it inside the pipeline, not resolve it here.
 */
export const EnvResolveTextParams = z.object({
  workspaceId: z.string(),
  text: z.string(),
}).strict();
export type EnvResolveTextParams = z.infer<typeof EnvResolveTextParams>;
export interface EnvResolveTextResult {
  /** The input with every known `{{key}}` substituted. An unknown key is left verbatim. */
  text: string;
}

/**
 * `env.getMerged` — the workspace's **effective** environment: `__global__` with the active environment over
 * it (`plan/19` WI 6).
 *
 * ## Why this is a command and not a renderer helper
 *
 * `App.tsx` has merged these two on every render since long before the engine resolved anything, and handed
 * the merged result down as the `activeEnv` prop. `plan/19` D23 makes the **engine** the authority for which
 * environment we resolve against, and the merge is part of that decision rather than a detail (`§8` trap 2: a
 * shadowed key resolves differently if the merge is skipped, and every test that uses no shadowed key passes
 * either way). So the merge moved engine-side — and this is the read that lets the renderer keep the one thing
 * it legitimately needs it for.
 *
 * ## What the renderer still needs it for, and why that is not a boundary violation
 *
 * **Autocomplete.** `EnvVarHint` lists the variable *names* available at the cursor, and `WsStreams` /
 * `RequestPane` / `ResponsePane` each render one. That is the same **list vs logic** split D28 made for
 * `RANDOMIZER_TOKENS` and WI 5 made again for `SKIP_CURL_HEADERS`: the UI may hold a copy of the *data*, and
 * the *resolver* is never the UI's. What it may not do is compute the data, which is exactly what
 * `mergeEnvVars` in the renderer was.
 *
 * **⚠ It is not a substitute for `env.resolveText`.** This returns a *list to choose from*; that returns a
 * *string with the choices already made*. A caller that resolved a request by looking values up in this
 * environment would have re-implemented the resolver — the thing this command exists to avoid.
 *
 * ## Scope
 *
 * `read`: it observes and changes nothing. Note the contrast with `config.get`, which returns the stored
 * `environments` array and the `activeEnvironmentId` pointer — *inputs* to the merge, one of which is
 * `__global__` and neither of which is what a token resolves to.
 */
export const EnvGetMergedParams = z.object({
  workspaceId: z.string(),
}).strict();
export type EnvGetMergedParams = z.infer<typeof EnvGetMergedParams>;

/**
 * The merged environment, or `null` when the workspace has neither a global nor an active environment.
 *
 * Hand-mirrored by the renderer as `Environment | null`, which is why the shape is spelled out rather than
 * reusing the engine's `Environment`: `@bifurc/gui` has no `@bifurc/engine` dependency by design (`plan/19`
 * finding 1). `id` is the literal `"merged"` and `name` likewise — the merged environment is a *view*, not a
 * stored entity, and giving it a real id would invite someone to save it.
 */
export interface EnvGetMergedResult {
  id: string;
  name: string;
  variables: { id: string; key: string; value: string }[];
  createdAt: number;
  workspaceId: string;
}
