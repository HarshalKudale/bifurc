/**
 * IPC handlers for application run configurations and process management.
 *
 * ## Channels only — the registry half now lives in the engine
 *
 * All ten `application.*` commands used to be registered at **module scope** in this file, which is
 * the `plan/07` §5c defect: a containerised engine (P9) answered `UNKNOWN_COMMAND` for every one.
 * Step 3e slice 5 (2026-09-19) moved them into
 * `packages/engine/src/applications/applicationCommands.ts` behind `registerApplicationCommands()`,
 * called from `src/ipc/handlers.ts`.
 *
 * Every channel below **delegates** through `commandRegistry.invoke()` rather than duplicating the
 * body — unlike `runnerHandlers.ts`, where three of the five are self-contained copies. That
 * distinction decides what breaks: a delegating channel is only ever as alive as the registry behind
 * it, so a suite that registers this handler group by hand **must** also call
 * `registerApplicationCommands(commandRegistry)`, or the channel exists with nothing behind it. The
 * failure is `No handler registered` at *invocation* time, so nothing catches it at build time.
 * `tests/integration/applications.integration.test.ts` is the one such suite.
 *
 * Nothing else changed here: the resolved-command generation at save time, the stop-before-delete
 * ordering, and `application.save` mutating the object it was handed are all in the engine module now,
 * documented there. `generateId` went with them — the private copy here was byte-identical to
 * `store/config.ts`'s exported one.
 */
import { ipcMain } from "electron";
import type { ApplicationConfig } from "@bifurc/engine/applications/types";
import { commandRegistry } from "@bifurc/engine/commands/registry";
import { bus } from "@bifurc/engine/eventBus";

// P2 work item 7 — application.* is not renderer-exposed today (no `preload.ts` entry at all; only
// `tests/integration/applications.integration.test.ts` exercises it directly), and every param matches
// the frozen schema exactly (`application.save`'s `application` field is an opaque `z.record(...)`, so
// there is no fixed-shape gap like `runner.saveConfig`'s). All 10 channels convert.
const ctx = { bus };

export function registerApplicationHandlers(): void {
    // ── CRUD ──────────────────────────────────────────────────────────────────

    ipcMain.handle("applications:list", (_e, workspaceId: string) =>
        commandRegistry.invoke("application.list", { workspaceId }, ctx));

    ipcMain.handle("applications:save", (_e, application: ApplicationConfig) =>
        commandRegistry.invoke("application.save", { application }, ctx));

    ipcMain.handle("applications:delete", (_e, workspaceId: string, id: string) =>
        commandRegistry.invoke("application.delete", { workspaceId, id }, ctx));

    // ── Process control ─────────────────────────────────────────────────────────

    ipcMain.handle(
        "applications:start",
        (_e, workspaceId: string, appId: string, mode: "run" | "debug") =>
            commandRegistry.invoke("application.start", { workspaceId, appId, mode }, ctx),
    );

    ipcMain.handle("applications:stop", (_e, appId: string) =>
        commandRegistry.invoke("application.stop", { appId }, ctx));

    ipcMain.handle("applications:getState", (_e, appId: string) =>
        commandRegistry.invoke("application.getState", { appId }, ctx));

    ipcMain.handle("applications:getAllStates", () =>
        commandRegistry.invoke("application.getAllStates", {}, ctx));

    ipcMain.handle("applications:getLogs", (_e, appId: string) =>
        commandRegistry.invoke("application.getLogs", { appId }, ctx));

    ipcMain.handle("applications:checkPort", (_e, port: number) =>
        commandRegistry.invoke("application.checkPort", { port }, ctx));

    ipcMain.handle("applications:killPort", (_e, port: number) =>
        commandRegistry.invoke("application.killPort", { port }, ctx));
}
