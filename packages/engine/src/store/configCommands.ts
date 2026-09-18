/**
 * `config.save` and the three `workspace.*` lifecycle commands — the engine half of
 * `packages/protocol/src/commands/config.ts`.
 *
 * ## Why these four and not the other five in that file
 *
 * `config.ts` also declares `config.get`, `env.setActive` and `workspace.setActive`, and those are
 * already registered — but from `src/ipc/handlers/coreHandlers.ts`, i.e. from the **shell**. That is
 * the mirror image of the problem this module fixes and it is deliberately left alone here: P6 is
 * about commands the registry cannot serve at all, and moving an already-registered command between
 * packages is a different change with a different blast radius. It is recorded in `plan/07` as a
 * follow-up, because on a containerised engine (P9) `config.get` would answer `UNKNOWN_COMMAND` for
 * exactly the same reason these four did.
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
import type { CommandRegistry, CommandContext } from "../commands/registry";
import type {
    ConfigSaveParams,
    WorkspaceAddParams,
    WorkspaceDeleteParams,
    WorkspaceRenameParams,
} from "@bifurc/protocol";
import { generateId, loadConfig, saveConfig, type AppConfig, type Workspace } from "./config";
import { initWorkspaceDir } from "./workspaceFs";
import { initWorkspaceRepo } from "./gitStore";
import { generateRandomWorkspaceName } from "../lib/randomNames";
import { gateCreate } from "../subscription/entityCount";
import { isRunning, reloadConfig, startServer, stopServer } from "../proxy/server";
import { restartCompanionServer } from "../transport/legacyCompanion";

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
        reloadConfig();
        // `ctx.bus`, not the imported singleton: carrying the bus in the context is the whole reason
        // `CommandContext` exists, and it is what lets a test drive this with a fresh bus.
        ctx.bus.emitTyped("settings.changed", {});
        const tlsChanged = incoming.tlsEnabled !== prev.tlsEnabled
            || incoming.tlsCaCertPath !== prev.tlsCaCertPath
            || incoming.tlsCaKeyPath !== prev.tlsCaKeyPath;
        if (!isRunning() || incoming.port !== prev.port || tlsChanged) {
            stopServer();
            startServer(incoming.port);
        }
        if (incoming.companionPort !== prev.companionPort) {
            // Statically imported rather than `require`d: a bare `require("@bifurc/engine/transport/
            // legacyCompanion")` only resolves because `tsc-alias` post-processes the build output,
            // which would make this user-facing path depend on the build pipeline and put it out of
            // reach of a test runner. There is no import cycle: `legacyCompanion.ts` imports only
            // `store/`, `sync/`, `proxy/` and `eventBus` — never `src/ipc`, and never this file.
            restartCompanionServer(incoming.companionPort);
        }
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
        reloadConfig();
        return { ok: true };
    });
}
