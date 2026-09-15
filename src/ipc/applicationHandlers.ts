/**
 * IPC handlers for application run configurations and process management.
 *
 * Save: generates the resolved command string and persists it with the config.
 * Start: reads config from disk by ID and spawns the pre-computed command.
 */

import { ipcMain } from "electron";
import type {
    ApplicationListParams, ApplicationSaveParams, ApplicationDeleteParams, ApplicationStartParams,
    ApplicationStopParams, ApplicationGetStateParams, ApplicationGetAllStatesParams,
    ApplicationGetLogsParams, ApplicationCheckPortParams, ApplicationKillPortParams,
} from "@bifurc/protocol";
import {
    readAllEntities, writeEntity, deleteEntityFile,
} from "@/store/workspaceFs";
import { processSpawner } from "@/applications/processSpawner";
import { generateResolvedCommand } from "@/applications/commandGenerator";
import { checkPortInUse, killProcessOnPort } from "@/applications/portUtils";
import type { ApplicationConfig } from "@/applications/types";
import { commandRegistry } from "@/commands/registry";
import { bus } from "@/eventBus";

function generateId(): string {
    return Date.now().toString(36) + Math.random().toString(36).slice(2, 7);
}

// P2 work item 7 — application.* is not renderer-exposed today (no `preload.ts` entry at all;
// only `tests/integration/applications.integration.test.ts` exercises it directly), and every
// param matches the frozen schema exactly (`application.save`'s `application` field is an
// opaque `z.record(...)`, so there's no fixed-shape gap like `runner.saveConfig`'s). All 10
// channels convert.
const ctx = { bus };

commandRegistry.register("application.list", ({ workspaceId }: ApplicationListParams) => {
    return readAllEntities(workspaceId, "applications");
});

commandRegistry.register("application.save", ({ application }: ApplicationSaveParams) => {
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

commandRegistry.register("application.delete", ({ workspaceId, id }: ApplicationDeleteParams) => {
    // Stop process if running
    processSpawner.stop(id);
    deleteEntityFile(workspaceId, "applications", id);
    return { ok: true };
});

commandRegistry.register("application.start", ({ workspaceId, appId, mode }: ApplicationStartParams) => {
    return processSpawner.start(workspaceId, appId, mode);
});

commandRegistry.register("application.stop", ({ appId }: ApplicationStopParams) => {
    processSpawner.stop(appId);
    return { ok: true };
});

commandRegistry.register("application.getState", ({ appId }: ApplicationGetStateParams) => {
    return processSpawner.getState(appId);
});

commandRegistry.register("application.getAllStates", (_params: ApplicationGetAllStatesParams) => {
    return processSpawner.getAllStates();
});

commandRegistry.register("application.getLogs", ({ appId }: ApplicationGetLogsParams) => {
    return processSpawner.getLogs(appId);
});

commandRegistry.register("application.checkPort", ({ port }: ApplicationCheckPortParams) => {
    return checkPortInUse(port);
});

commandRegistry.register("application.killPort", ({ port }: ApplicationKillPortParams) => {
    return killProcessOnPort(port);
});

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
