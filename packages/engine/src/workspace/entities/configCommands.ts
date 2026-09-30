/**
 * `config.save` and the three `workspace.*` lifecycle commands — the engine half of
 * `packages/protocol/src/commands/config.ts`.
 *
 * ## Why these four and not the other five in that file
 *
 * `config.ts` also declares `config.get`, `env.setActive` and `workspace.setActive`, and those were
 * already registered — but from `src/ipc/handlers/coreHandlers.ts`, i.e. from the **shell**. That was
 * the mirror image of the problem this module fixes and it was deliberately left alone here: P6 is
 * about commands the registry cannot serve at all, and moving an already-registered command between
 * packages is a different change with a different blast radius. §5c took it as a separate pass, and the
 * shell half is now gone entirely — the reduction's sixth group deleted `coreHandlers.ts`'s seventeen
 * channels on 2026-09-21 and the file became `coreKinds.ts` (`plan/07` §5b-septendecies). So these four
 * and those three no longer have a shell twin at all.
 *
 * ## `config.save` is SPLIT, and the body below is already the engine half
 *
 * `config.ts`'s own note says the legacy channel also calls `updateTrayMenu()` — a shell-only side
 * effect — and that the engine half is "config persistence + server restart", with the shell
 * subscribing to `settings.changed` to update its own tray. The shell body moved here never called
 * `updateTrayMenu()`, so no split was needed to move it: it *is* the engine half, and the tray half
 * has already been the shell's job since P1.
 *
 * ## The one thing not moved verbatim, and why
 *
 * `workspace.delete`'s shell body opened with a `const ws = (cfg.workspaces ?? []).find(...)` whose
 * result it never read. That line is dropped rather than reproduced; every observable behaviour is
 * unchanged. Everything else is a character-for-character move.
 */
import type { CommandRegistry, CommandContext } from "../../kernel/registry";
import type {
    ConfigSaveParams,
    SettingsUpdateParams,
    WorkspaceAddParams,
    WorkspaceDeleteParams,
    WorkspaceRenameParams,
} from "@bifurc/protocol";
import { generateId, loadConfig, saveConfig, type AppConfig, type Workspace } from "../config";
import { initWorkspaceDir } from "../workspaceFs";
import { initWorkspaceRepo } from "../gitStore";
import { generateRandomWorkspaceName } from "../../utils/randomNames";
import { gateCreate } from "../entityCount";
// `Engine_Module_Plan.md` §4.1 — the fan-out lives in `runtime/instances.ts` now. This command used to
// import `reloadConfig` from `proxy/server` and the three port re-binders from the listeners themselves;
// both operations are the *composition*'s business, so they moved to the module that owns the listeners.
import { notifyConfigChanged, rebindChangedListeners, rebindRuntimePorts } from "../../runtime/instances";
import { loadSettings, saveSettings } from "../appSettings";

export function registerConfigCommands(registry: CommandRegistry): void {
    /**
     * `ConfigSaveParams.config` is `z.record(z.string(), z.unknown())` — deliberately loose, because
     * `AppConfig` is "a large, evolving shape that today's engine already owns and validates on load"
     * (see `config.ts`'s header). So the cast is the wire boundary's documented contract rather than a
     * shortcut, and the shell body's own `(incoming: AppConfig)` was doing exactly the same thing
     * without a runtime check.
     */
    registry.register("config.save", (params: ConfigSaveParams, ctx: CommandContext) => {
        const incoming = params.config as unknown as AppConfig;
        const prev = loadConfig();
        saveConfig(incoming);
        notifyConfigChanged();
        // `ctx.bus`, not the imported singleton: carrying the bus in the context is the whole reason
        // `CommandContext` exists, and it is what lets a test drive this with a fresh bus.
        ctx.bus.emitTyped("settings.changed", {});

        /**
         * ## `plan/17` §5 work item 7 — a save re-binds the three runtime ports
         *
         * `plan/17` §1 splits the ports by *whether a change needs a restart*: only the **companion** does, and
         * manually. The edge, the forward proxy and the webhook **re-bind at runtime**, and this call is that
         * rule.
         *
         * **The branches themselves moved to `runtime/instances.ts`'s `rebindRuntimePorts()`**
         * (`Engine_Module_Plan.md` §4.1). They were inline here — three parallel `if`s reading five fields off
         * `incoming`/`prev`, plus a `tlsChanged` predicate — and that put the *port ownership* rule in a
         * `store/` command. It belongs with the listeners: this command's job is "save the config", and *what
         * that means for each listener* is a composition concern. Each branch's own rationale (why the edge
         * may move but only when bound, why the webhook never starts, why TLS needs the proxy restarted rather
         * than merely reloaded) moved with it, verbatim.
         *
         * `prev` is still read here because the diff is what the caller knows; `rebindRuntimePorts` takes
         * both snapshots rather than re-loading, so there is one definition of "before".
         */
        rebindRuntimePorts(incoming, prev);
        return { ok: true };
    });

    /**
     * The settings half of `config.save`, as a patch. It writes `app.json` only — no workspace, no entity —
     * so a client holding a stale config cannot revert anything by changing a port.
     */
    registry.register("settings.update", (params: SettingsUpdateParams, ctx: CommandContext) => {
        const prev = loadSettings();
        const patch = Object.fromEntries(Object.entries(params).filter(([, v]) => v !== undefined));
        const next = { ...prev, ...patch };
        saveSettings(next);
        notifyConfigChanged();
        ctx.bus.emitTyped("settings.changed", {});
        rebindChangedListeners(next, prev);
        return { ok: true };
    });

    /**
     * The gate-failure branch returns `{ error: "limit_reached", ...gate }`, which is **not**
     * `WorkspaceAddResult`. That is pre-existing shell behaviour carried over unchanged rather than a
     * new mismatch, and it is left as-is: changing a command's failure shape while moving it would
     * hide the move behind a contract change. Worth a follow-up, not a silent fix.
     */
    registry.register("workspace.add", async (params: WorkspaceAddParams) => {
        const cfg = loadConfig();
        const gate = gateCreate(cfg.activeWorkspaceId, "workspace");
        if (!gate.allowed) return { error: "limit_reached", ...gate };
        const finalName = params.name.trim() || generateRandomWorkspaceName();
        const newWs: Workspace = { id: generateId(), name: finalName, createdAt: Date.now(), activeEnvironmentId: null };
        cfg.workspaces = cfg.workspaces ?? [];
        cfg.workspaces.push(newWs);
        saveConfig(cfg);
        try {
            initWorkspaceDir(newWs.id, newWs.name);
            await initWorkspaceRepo(newWs.id);
        } catch { }
        return newWs;
    });

    registry.register("workspace.rename", async (params: WorkspaceRenameParams) => {
        const cfg = loadConfig();
        const ws = (cfg.workspaces ?? []).find((w) => w.id === params.id);
        if (ws) ws.name = params.name.trim() || ws.name;
        saveConfig(cfg);
        return { ok: true };
    });

    registry.register("workspace.delete", async (params: WorkspaceDeleteParams) => {
        const cfg = loadConfig();
        cfg.workspaces = (cfg.workspaces ?? []).filter((w) => w.id !== params.id);
        if (cfg.activeWorkspaceId === params.id) {
            const first = cfg.workspaces[0];
            if (first) {
                cfg.activeWorkspaceId = first.id;
                cfg.activeEnvironmentId = first.activeEnvironmentId;
            }
        }
        saveConfig(cfg);
        // A delete can change which entities are enabled (removing a workspace removes its sets from the
        // active view), so the proxy has to re-derive. `rebindRuntimePorts` is deliberately **not** called:
        // the ports of the *host* are not a function of which workspaces exist.
        notifyConfigChanged();
        return { ok: true };
    });
}
