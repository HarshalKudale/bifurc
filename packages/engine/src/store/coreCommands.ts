/**
 * `config.get`, `env.setActive`, `workspace.setActive`, `entity.load`, `entity.setEnabled` — the five
 * commands `plan/07` §5c named as being registered **from the shell**.
 *
 * ## Why they moved (P6 §5c)
 *
 * These were registered at **module scope** in `src/ipc/handlers/coreHandlers.ts`, which made them
 * invisible to a containerised engine: the implementation lived in a file that only exists in the
 * Electron shell, so a headless host (P9) answered `UNKNOWN_COMMAND` for commands the protocol
 * declares and the client advertises. The ratchet in `tests/ipc/handlers.test.ts` could not see the
 * problem either — in *this* process the shell had registered them, so the registry looked complete.
 *
 * §5c recorded five commands. The real count is **51**; this module is the first slice, and it is the
 * one §5c named explicitly.
 *
 * ## What did not change
 *
 * Every handler body is moved **verbatim**. The `ipcMain.handle` adapters in `coreHandlers.ts` were
 * already thin delegations (`commandRegistry.invoke("config.get", {}, ctx)`), so the wire behaviour is
 * the same code reached by a different door — which is what makes this a move rather than a rewrite.
 *
 * The one dependency that had to travel is `syncEnabledSet`, which lived in a *shell* file
 * (`src/ipc/handlers/utils.ts`) and is now `store/enabledSet.ts`. Note the asymmetry that survives
 * deliberately: `entity.setEnabled`'s channel adapter keeps an explicit `invalid_kind` guard *ahead*
 * of `commandRegistry.invoke()`, because `EntitySetEnabledParams.kind` is a strict 6-value enum and an
 * invalid kind would otherwise throw `BAD_REQUEST` instead of resolving the
 * `{ok: false, error: "invalid_kind"}` that channel has always returned. That guard is client-facing
 * behaviour and stays where the channel is; this module only owns the engine half.
 */
import type {
    ConfigGetParams,
    EnvSetActiveParams,
    EntityLoadParams,
    EntitySetEnabledParams,
    WorkspaceSetActiveParams,
} from "@bifurc/protocol";
import { loadConfig, loadEntity, saveConfig } from "./config";
import { readEnabledSet } from "./workspaceFs";
import { syncEnabledSet } from "./enabledSet";
import { reloadConfig } from "../proxy/server";
import { emitEntityStatus } from "../eventBus";
import {
    mayAffectUnnamedEntities,
    type CommandContext,
    type CommandRegistry,
} from "../commands/registry";
import { toEngineKind } from "../commands/entityKindMap";

/**
 * The six kinds that carry an enabled/disabled flag, tracked in `enabled.json` rather than on the
 * entity itself. `EntitySetEnabledParams.kind` is already restricted to exactly these six
 * engine-internal strings (see `entityKindMap.ts`'s own note on why it needs no translation).
 */
const ENABLED_STATE_KINDS = new Set(["mocks", "mappings", "rules", "graphqlMocks", "soapMocks", "grpcMocks"]);

export function registerCoreCommands(registry: CommandRegistry): void {
    registry.register("config.get", (_params: ConfigGetParams) => loadConfig());

    registry.register("env.setActive", ({ id }: EnvSetActiveParams) => {
        if (id === "__global__") return { ok: false, error: "cannot_activate_global" };
        const cfg = loadConfig();
        cfg.activeEnvironmentId = id;
        const ws = (cfg.workspaces ?? []).find((w) => w.id === cfg.activeWorkspaceId);
        if (ws) ws.activeEnvironmentId = id;
        saveConfig(cfg);
        reloadConfig();
        return { ok: true };
    });

    registry.register("workspace.setActive", ({ id }: WorkspaceSetActiveParams) => {
        const cfg = loadConfig();
        const ws = (cfg.workspaces ?? []).find((w) => w.id === id);
        if (!ws) return { ok: false };
        cfg.activeWorkspaceId = id;
        cfg.activeEnvironmentId = ws.activeEnvironmentId;
        saveConfig(cfg);
        reloadConfig();
        return { ok: true, config: loadConfig() };
    });

    registry.register("entity.load", ({ workspaceId, kind, id }: EntityLoadParams) => {
        const engineKind = toEngineKind(kind);
        const entity = loadEntity(workspaceId, engineKind, id);
        if (!entity) return { ok: false };
        if (ENABLED_STATE_KINDS.has(engineKind)) {
            const set = readEnabledSet(workspaceId, engineKind);
            return { ok: true, entity: { ...entity as object, enabled: set ? set.has(id) : false } };
        }
        return { ok: true, entity };
    });

    registry.register("entity.setEnabled", async ({ workspaceId, kind, id, enabled }: EntitySetEnabledParams, ctx: CommandContext) => {
        // Enabling an entity resolves conflicts by *disabling* same-signature siblings, which reaches
        // entities the caller never named — the same privilege `entity.create`'s `onAddConflict`
        // exercises, so it takes the same gate and must agree with it. See
        // `mayAffectUnnamedEntities()`. The toggle itself is unconditional: only the
        // sibling-disabling half is narrowed.
        const mayResolveConflicts = mayAffectUnnamedEntities(ctx);

        if (enabled && mayResolveConflicts && kind === "mocks") {
            const cfg = loadConfig();
            const target = (cfg.mocks ?? []).find((m) => m.id === id);
            if (target) {
                const sig = `${target.method.toUpperCase()}|${target.urlPattern}|${target.capturedBody ?? ""}`;
                for (const m of cfg.mocks) {
                    if (m.id !== id && m.enabled) {
                        const mSig = `${m.method.toUpperCase()}|${m.urlPattern}|${m.capturedBody ?? ""}`;
                        if (mSig === sig) syncEnabledSet(workspaceId, "mocks", m.id, false);
                    }
                }
            }
        }
        if (enabled && mayResolveConflicts && kind === "rules") {
            const cfg = loadConfig();
            const target = (cfg.proxyRules ?? []).find((r) => r.id === id);
            if (target) {
                const sig = `${target.useRegex ? "re" : "exact"}|${target.pattern}`;
                for (const r of cfg.proxyRules) {
                    if (r.id !== id && r.enabled) {
                        const rSig = `${r.useRegex ? "re" : "exact"}|${r.pattern}`;
                        if (rSig === sig) syncEnabledSet(workspaceId, "rules", r.id, false);
                    }
                }
            }
        }

        syncEnabledSet(workspaceId, kind, id, enabled);
        reloadConfig();
        emitEntityStatus(workspaceId);
        return { ok: true };
    });
}
