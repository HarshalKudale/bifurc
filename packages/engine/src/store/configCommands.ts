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
import type { CommandRegistry, CommandContext } from "../kernel/registry";
import type {
    ConfigSaveParams,
    WorkspaceAddParams,
    WorkspaceDeleteParams,
    WorkspaceRenameParams,
} from "@bifurc/protocol";
import { generateId, loadConfig, saveConfig, type AppConfig, type Workspace } from "./config";
import { initWorkspaceDir } from "./workspaceFs";
import { initWorkspaceRepo } from "./gitStore";
import { generateRandomWorkspaceName } from "../utils/randomNames";
import { gateCreate } from "../subscription/entityCount";
import { isRunning, reloadConfig, startServer } from "../proxy/server";
// `plan/17` §5 work item 7 — the edge's binding, added so a save can re-bind the one runtime port a command
// could not previously reach. See `proxy/edge.ts`'s `bindEdgeInstance()` for why this facade is bound-only
// where the other two have lazy defaults.
import { isEdgeRunning, restartEdge } from "../proxy/edge";
import { isWebhookServerRunning, startWebhookServer } from "../proxy/webhookServer";

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

        /**
         * ## `plan/17` §5 work item 7 — a save re-binds the three runtime ports
         *
         * `plan/17` §1 splits the ports by *whether a change needs a restart*: only the **companion** does, and
         * manually. The edge, the forward proxy and the webhook **re-bind at runtime** — and until this change
         * only one of them did, on a port that was no longer its own.
         *
         * **What this replaced, and why the old rule was right when it was written.** The body used to be a
         * single `if (!isRunning() || incoming.port !== prev.port || tlsChanged)` that re-bound *the proxy* on
         * `getPort() || incoming.port`. That was correct after `plan/15` work item 3 and is wrong now: it read
         * `settings.port` to decide *whether* to act while refusing to use it as *where* to act, because the
         * port it names had just become the **edge's**. The visible consequence was stated in this comment and
         * is the thing work item 7 exists to remove: *"changing the port in Settings no longer moves this
         * listener."* Now it does — it moves the listener that port actually belongs to.
         *
         * **Three branches, one per port, each with its own rule about whether it may move** (`plan/17` §1).
         * They are deliberately parallel, and each is a single call because every `start()` in this engine
         * already does `if (server) stop()` first — so `start(newPort)` **is** the rebind, and an explicit
         * `stopServer()` before it would be a second way to say the same thing that could drift.
         *
         * **A bind failure is not thrown**, here or anywhere: it lands in the listener's `getError()` and is
         * reported through `status()`. That is the engine's oldest bind contract, and it is what makes the
         * "revive a listener that is down" arms below safe — a save is the natural moment to retry a port the
         * user has just been told is occupied.
         */

        /**
         * **The edge** — `settings.port`, the port every `*.localhost` mapping points at.
         *
         * This is the reversal of the rule the block above used to state. `plan/17` §1 gives the edge its own
         * runtime rule (*"bound, not scanned"*), and a user who changes it in Settings means it: a mapping URL
         * that stops working because the port only takes effect on the next boot is the failure mode this
         * removes. The `!isEdgeRunning()` arm mirrors the forward proxy's — a save retries an edge that failed
         * to bind, which is the recovery path for a port that was occupied at boot.
         */
        if (!isEdgeRunning() || incoming.port !== prev.port) {
            restartEdge(incoming.port);
        }

        /**
         * **The forward proxy** — `settings.forwardProxyPort`, the port a browser's proxy setting names.
         *
         * The port is now read **straight from the setting** rather than from `getPort() ||`. `plan/17` §2
         * makes D16's remembered resolved port dead: there is no scan, so there is no resolved port that could
         * differ from the configured one, and `getPort()` here would only re-assert a number the caller already
         * has. That also removes the old asymmetry where `settings.port` decided *whether* to restart while
         * `getPort()` decided *where* — the two questions now have one answer each.
         *
         * `tlsChanged` is kept and belongs here: `proxy/server.ts`'s `start()` is what loads the CA
         * (`loadCA`), so a TLS toggle needs the proxy restarted on **its own** port. `reloadConfig()` above
         * swaps the config but does not touch the CA.
         */
        if (!isRunning() || incoming.forwardProxyPort !== prev.forwardProxyPort || tlsChanged) {
            startServer(incoming.forwardProxyPort);
        }

        /**
         * **The webhook receiver** — host-level, and **off by default** (`plan/17` §1).
         *
         * So a port change **moves a receiver that is running and never starts one**. Starting it is the
         * user's action (`webhookServer.start`, from the Webhooks panel), and a settings save that quietly
         * bound a port nobody asked for would be this command inventing a feature toggle. `isWebhookServerRunning()`
         * is what makes that distinction; the ordering of the two conditions is irrelevant but reads better with
         * the cheap check second.
         */
        if (incoming.webhookPort !== prev.webhookPort && isWebhookServerRunning()) {
            startWebhookServer(incoming.webhookPort);
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
