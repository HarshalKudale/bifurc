import type { EnvResolveTextParams, EnvGetMergedParams, EnvSetActiveParams } from "@bifurc/protocol";
import { loadConfig, saveConfig } from "../workspace/config";
import { notifyConfigChanged } from "../runtime/instances";
import type { CommandRegistry } from "../kernel/registry";
import { loadMergedEnvironment } from "./envState";
import { resolveVars } from "./resolve";

/** `env.resolveText`, `env.getMerged` and `env.setActive` — the environment commands. */
export function registerEnvCommands(registry: CommandRegistry): void {
    /**
     * `plan/19` WI 5, item 4 — resolve `{{tokens}}` for **display**.
     *
     * The health bar shows the url a check will actually probe. WI 3 moved resolution into the engine, so
     * the renderer can no longer compute that value, and this is the read-only path it asks instead of
     * carrying a second resolver (`plan/19` D22). See `packages/protocol/src/commands/config.ts` for why
     * the *check* returning the url it used would be worse: the card shows the resolved url before the
     * first check, and a service with auto-refresh off may never be checked at all.
     *
     * **The merged environment is not optional.** `loadMergedEnvironment` puts `__global__` under the
     * active environment; resolving against the active one alone resolves a *shadowed* key differently,
     * and every test that uses no shadowed key passes either way (`plan/19` §8 trap 2). It is also a pure
     * read — `envState.ts` deliberately avoids `loadWorkspaceDispatchSlice`, which writes `enabled.json`.
     */
    registry.register("env.resolveText", ({ workspaceId, text }: EnvResolveTextParams) => ({
        text: resolveVars(text, loadMergedEnvironment(workspaceId)),
    }));

    /**
     * `plan/19` WI 6 — the **merged environment itself**, as data.
     *
     * The renderer's `App.tsx` merged `__global__` with the active environment on every render and handed the
     * result down as the `activeEnv` prop, which every `EnvVarHint` lists variable names from. That merge is
     * D23's, so it is the engine's; this is the read that lets the hint keep working without the renderer
     * computing it — the **list vs logic** split D28 made for `RANDOMIZER_TOKENS`, applied to the one renderer
     * need `env.resolveText` cannot serve (a list to choose from, rather than a string with the choices made).
     *
     * **Returned as-is, and the `null` is load-bearing.** A workspace with neither a global nor an active
     * environment has no effective environment, and `EnvVarHint` renders nothing for `null` — the same
     * "nothing to choose from" it renders for an empty variable list. Answering `{id:"merged", variables:[]}`
     * would be a different claim: an environment that exists and happens to be empty.
     *
     * `loadMergedEnvironment` is the same function `env.resolveText` uses, so the list the UI offers and the
     * values the engine substitutes cannot disagree — which is the property that would have been lost by
     * leaving the merge in the renderer.
     */
    registry.register("env.getMerged", ({ workspaceId }: EnvGetMergedParams) =>
        loadMergedEnvironment(workspaceId),
    );

    registry.register("env.setActive", ({ id }: EnvSetActiveParams) => {
        if (id === "__global__") return { ok: false, error: "cannot_activate_global" };
        const cfg = loadConfig();
        cfg.activeEnvironmentId = id;
        const ws = (cfg.workspaces ?? []).find((w) => w.id === cfg.activeWorkspaceId);
        if (ws) ws.activeEnvironmentId = id;
        saveConfig(cfg);
        notifyConfigChanged();
        return { ok: true };
    });
}
