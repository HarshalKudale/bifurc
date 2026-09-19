/**
 * `application.*` — the ten application run-configuration and process-management commands, moved out
 * of `src/ipc/applicationHandlers.ts` (P6 step 3e, slice 5).
 *
 * ## Why they moved (§5c)
 *
 * All ten were registered at **module scope** in the shell file, so a containerised engine (P9)
 * answered `UNKNOWN_COMMAND` for every one. The shell file's channels are thin
 * `commandRegistry.invoke()` delegations, so they were only ever as alive as the registry behind them.
 *
 * ## This is the cleanest §5c slice so far: nothing had to travel
 *
 * Every dependency already lived in this package — `store/workspaceFs`'s `readAllEntities` /
 * `writeEntity` / `deleteEntityFile`, `applications/processSpawner`, `applications/commandGenerator`'s
 * `generateResolvedCommand`, and `applications/portUtils`'s `checkPortInUse` / `killProcessOnPort`.
 * The shell file imported them from `@bifurc/engine/...` and added nothing but an `ipcMain.handle`
 * each. **A file whose entire body is `import from the engine` + `register` + `delegate` is the §5c
 * defect in its purest form** — the shell never had a reason to own this, so the move is a relocation
 * rather than a decision. That is also why it is safe: there is no behaviour in the shell to lose.
 *
 * The one shell-side dependency was `electron`'s `ipcMain`, which stays behind in the channels.
 *
 * ## `processSpawner` is a module-level singleton, and `bundle: false` is what keeps it one
 *
 * `applications/processSpawner.ts` exports a single `ProcessSpawner` instance, not a factory. This
 * module imports that instance rather than constructing one — the same object the proxy and the
 * shell's channels reach. The engine's `tsup` config is deliberately `bundle: false` for exactly this
 * reason: bundling would inline a *private copy* of the module into every entry point, so a process
 * started through one entry would be invisible to `stop()` called through another. The failure would
 * be silent — a spawned child with no way to reach it.
 *
 * ## `application.save` mutates the object it was handed, and that is preserved
 *
 * When `id` is absent it assigns `app.id` and `app.createdAt` **on the caller's object** before
 * spreading it. Kept verbatim: it is how the renderer's "new application" flow gets an id back, and
 * the renderer's declared result is the saved config. The mutation is visible here because the value
 * arrived over a transport (so it is a structured clone, not the renderer's own object) — but a
 * direct `registry.invoke()` caller, which is what the integration suite does, *does* see its own
 * object mutated. Worth knowing before anyone "cleans it up" into a pure function: that would change
 * what an in-process caller observes.
 *
 * The resolved command string is generated **at save time** (`generateResolvedCommand(app,
 * process.platform)`) and persisted merged into the config — so the platform-specific command is
 * frozen at save, not recomputed at start. That is the file's stated purpose and the reason
 * `application.start` can spawn a pre-computed command.
 *
 * ## Ordering that matters
 *
 * `application.delete` stops the process **before** deleting the entity file. On Windows a running
 * child can hold the workspace file open, so the reverse order leaves a file that cannot be removed.
 * `application.stop` resolves `{ok: true}` unconditionally, while `application.start` returns the
 * spawner's own result — asymmetric, and preserved, because the renderer's Stop button must not be
 * told it failed when there was nothing to stop.
 *
 * ## `generateId` collapsed: the fourth copy of the same five lines
 *
 * The shell file carried a private `generateId()`. It is **byte-identical** to the exported one in
 * `store/config.ts` (`Date.now().toString(36) + Math.random().toString(36).slice(2, 7)`), which
 * `store/folderCommands.ts` already imports, so this module imports it too and one copy disappears.
 * (A further private copy survives in `store/appSettings.ts` — same body, unrelated callers, left
 * alone: it is not part of a §5c move and collapsing it would be an unrelated edit.)
 *
 * Unlike `syncEnabledSet` in slice 1, this helper is **pure** — it reads no state and writes none — so
 * the collapse is hygiene rather than a correctness fix. The slice-1 collapse mattered because
 * disagreement there persisted to `enabled.json`; here a divergent copy could only produce a
 * differently-shaped id.
 */
import {
    readAllEntities, writeEntity, deleteEntityFile,
} from "../store/workspaceFs";
import { generateId } from "../store/config";
import { processSpawner } from "./processSpawner";
import { generateResolvedCommand } from "./commandGenerator";
import { checkPortInUse, killProcessOnPort } from "./portUtils";
import type { ApplicationConfig } from "./types";
import type {
    ApplicationListParams, ApplicationSaveParams, ApplicationDeleteParams, ApplicationStartParams,
    ApplicationStopParams, ApplicationGetStateParams, ApplicationGetAllStatesParams,
    ApplicationGetLogsParams, ApplicationCheckPortParams, ApplicationKillPortParams,
} from "@bifurc/protocol";
import type { CommandRegistry } from "../commands/registry";

/**
 * P2 work item 7 — all ten convert, and the reason is the schema rather than the bodies:
 * `application.save`'s `application` field is an opaque `z.record(...)`, so there is no fixed-shape
 * gap of the kind that pinned `runner.saveConfig` (which required `requestOrder` that no caller
 * sent). Every other param matches the frozen schema exactly.
 *
 * None of the ten is renderer-exposed — `src/preload.ts` has no entry for any of them and only
 * `tests/integration/applications.integration.test.ts` drives them. So this slice cannot move the
 * 144-key surface in either direction, which is a stronger guarantee than "the diff is empty".
 */
export function registerApplicationCommands(registry: CommandRegistry): void {
    registry.register("application.list", ({ workspaceId }: ApplicationListParams) => {
        return readAllEntities(workspaceId, "applications");
    });

    registry.register("application.save", ({ application }: ApplicationSaveParams) => {
        const app = application as unknown as ApplicationConfig;
        if (!app.id) {
            app.id = generateId();
            app.createdAt = Date.now();
        }

        // Generate the resolved command at save time
        const resolved = generateResolvedCommand(app, process.platform);
        const configToSave: ApplicationConfig = {
            ...app,
            ...resolved,
        };

        writeEntity(configToSave.workspaceId, "applications", configToSave.id, configToSave);
        return configToSave;
    });

    registry.register("application.delete", ({ workspaceId, id }: ApplicationDeleteParams) => {
        // Stop process if running — before the delete: on Windows a live child can hold the file open.
        processSpawner.stop(id);
        deleteEntityFile(workspaceId, "applications", id);
        return { ok: true };
    });

    registry.register("application.start", ({ workspaceId, appId, mode }: ApplicationStartParams) => {
        return processSpawner.start(workspaceId, appId, mode);
    });

    registry.register("application.stop", ({ appId }: ApplicationStopParams) => {
        processSpawner.stop(appId);
        return { ok: true };
    });

    registry.register("application.getState", ({ appId }: ApplicationGetStateParams) => {
        return processSpawner.getState(appId);
    });

    registry.register("application.getAllStates", (_params: ApplicationGetAllStatesParams) => {
        return processSpawner.getAllStates();
    });

    registry.register("application.getLogs", ({ appId }: ApplicationGetLogsParams) => {
        return processSpawner.getLogs(appId);
    });

    registry.register("application.checkPort", ({ port }: ApplicationCheckPortParams) => {
        return checkPortInUse(port);
    });

    registry.register("application.killPort", ({ port }: ApplicationKillPortParams) => {
        return killProcessOnPort(port);
    });
}
