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
 * (`src/ipc/handlers/utils.ts`) and is now `store/enabledSet.ts`. Note the asymmetry that was recorded
 * here at move time: `entity.setEnabled`'s channel adapter kept an explicit `invalid_kind` guard *ahead*
 * of `commandRegistry.invoke()`, because `EntitySetEnabledParams.kind` is a strict 6-value enum and an
 * invalid kind would otherwise throw `BAD_REQUEST` instead of resolving the
 * `{ok: false, error: "invalid_kind"}` that channel has always returned.
 *
 * ## Where that guard went (2026-09-21) — and why it was not simply deleted
 *
 * The channel that owned it is gone: the reduction's sixth group deleted all seventeen of
 * `coreHandlers.ts`'s channels and the file became `coreKinds.ts` (`plan/07` §5b-septendecies). The
 * guard is **client-facing behaviour** (`packages/gui/src/types/window.ts:41` declares
 * `Promise<{ok: boolean; error?: string}>`, and this file's own note said it "stays where the channel
 * is"), so it had to land somewhere the client can reach before the channel could go — and
 * **non-negotiable #1 forbids touching the renderer**. It now lives in
 * `packages/client/src/index.ts`, validated against `EntitySetEnabledParams.shape.kind` rather than a
 * second copy of the six names, and is pinned by a case in `packages/client/tests/client.test.ts`.
 *
 * The sibling case is worth knowing about, because the two are not the same shape: `entity.load`'s
 * channel applied `toProtocolKind()` and the client twin had **dropped** it, so `loadEntity` was
 * answering `BAD_REQUEST` for the two engine-internal kind names two panels actually pass. That was a
 * live bug, fixed in the same fix-first commit (`174a01d`); this guard is only latent, because no
 * renderer caller can reach it today. **Both were carried over anyway — the rule is that a channel's
 * owned step moves to the twin, not that it moves only when it is currently reachable.**
 */
import type {
    ConfigGetParams,
    EnvSetActiveParams,
    EntityLoadParams,
    EntitySetEnabledParams,
    WorkspaceSetActiveParams,
} from "@bifurc/protocol";
import { loadConfig, loadEntity, saveConfig } from "../config";
import { readEnabledSet } from "../workspaceFs";
import { syncEnabledSet } from "../enabledSet";
import { notifyConfigChanged } from "../../runtime/instances";
import { emitEntityStatus } from "../../runtime/entityStatus";
import {
    mayAffectUnnamedEntities,
    type CommandContext,
    type CommandRegistry,
} from "../../kernel/registry";
import { toEngineKind } from "../../kernel/kinds";

/**
 * The six kinds that carry an enabled/disabled flag, tracked in `enabled.json` rather than on the
 * entity itself. `EntitySetEnabledParams.kind` is already restricted to exactly these six
 * engine-internal strings (see `entityKindMap.ts`'s own note on why it needs no translation).
 */
const ENABLED_STATE_KINDS = new Set(["mocks", "mappings", "rules"]);

export function registerCoreCommands(registry: CommandRegistry): void {
    registry.register("config.get", (_params: ConfigGetParams) => loadConfig());

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

    /**
     * Switch the active workspace **and re-point the forward proxy at it** — the `plan/17` §1 rule.
     *
     * ## This call was removed in P14 and is restored here, deliberately
     *
     * P14 WI 4 (2026-09-21) dropped the `reloadConfig()` this command used to make — the seam is
     * `notifyConfigChanged()` today — and the reasoning was
     * sound *for that model*: D10–D12 gave each workspace its own runtime and its own port, so the UI pointer
     * and the proxies were independent, and a workspace's rules should keep serving while you edited another
     * one. Re-pointing the single process-wide proxy would have stopped a port from serving what it had been
     * serving.
     *
     * **`plan/17` §1 replaces the model, so it replaces the conclusion.** There is now exactly **one** forward
     * proxy for the whole host, and the plan states what a workspace switch means: *"a workspace is a
     * **project**: switching workspaces changes the **context** — which rules, mocks and webhook endpoints are
     * in force — while the ports and the infrastructure stay put."* With one proxy, "the context" *is* what
     * that proxy dispatches with, so the switch **must** re-point it or the model is not implemented at all.
     *
     * **This is not a revert, and the difference is the port.** P14's objection was that re-pointing *stopped
     * a port from serving*. Now the port keeps serving and only the rules behind it change — which is exactly
     * what `plan/17` §5 work item 3 asks for: the proxy *"loads the active workspace's rules and mocks at
     * start and **swaps its in-memory config when the active workspace changes** — not per-request
     * resolution."* This command is that swap.
     *
     * The **edge** needs no equivalent: its `resolveConfig` is read per request, so it follows the pointer for
     * free. That asymmetry is `plan/15` §3.1's fresh-versus-cached distinction and is unchanged by `plan/17`.
     *
     * `env.setActive` below keeps its own `notifyConfigChanged()` for the reason it always had one — an environment
     * *is* proxy state (which rules and enabled sets are live). The two commands look symmetric and now
     * genuinely are, which is the distinction between a workspace and an environment surviving the model
     * change rather than being flattened by it.
     */
    registry.register("workspace.setActive", ({ id }: WorkspaceSetActiveParams) => {
        const cfg = loadConfig();
        const ws = (cfg.workspaces ?? []).find((w) => w.id === id);
        if (!ws) return { ok: false };
        cfg.activeWorkspaceId = id;
        cfg.activeEnvironmentId = ws.activeEnvironmentId;
        saveConfig(cfg);
        // The swap. See the block above for why this is back after P14 removed it.
        notifyConfigChanged();
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
        notifyConfigChanged();
        emitEntityStatus(workspaceId);
        return { ok: true };
    });
}
