/**
 * IPC handlers for application run configurations and process management.
 *
 * Save: generates the resolved command string and persists it with the config.
 * Start: reads config from disk by ID and spawns the pre-computed command.
 */

import { ipcMain } from "electron";
import {
    readAllEntities, writeEntity, deleteEntityFile,
} from "@/store/workspaceFs";
import { processSpawner } from "@/applications/processSpawner";
import { generateResolvedCommand } from "@/applications/commandGenerator";
import { checkPortInUse, killProcessOnPort } from "@/applications/portUtils";
import type { ApplicationConfig } from "@/applications/types";

function generateId(): string {
    return Date.now().toString(36) + Math.random().toString(36).slice(2, 7);
}

export function registerApplicationHandlers(): void {
    // ── CRUD ──────────────────────────────────────────────────────────────────

    ipcMain.handle("applications:list", (_e, wsId: string) => {
        return readAllEntities(wsId, "applications");
    });

    ipcMain.handle("applications:save", (_e, app: ApplicationConfig) => {
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

    ipcMain.handle("applications:delete", (_e, wsId: string, id: string) => {
        // Stop process if running
        processSpawner.stop(id);
        deleteEntityFile(wsId, "applications", id);
        return { ok: true };
    });

    // ── Process control ─────────────────────────────────────────────────────────

    ipcMain.handle(
        "applications:start",
        (_e, wsId: string, appId: string, mode: "run" | "debug") => {
            return processSpawner.start(wsId, appId, mode);
        },
    );

    ipcMain.handle("applications:stop", (_e, appId: string) => {
        processSpawner.stop(appId);
        return { ok: true };
    });

    ipcMain.handle("applications:getState", (_e, appId: string) => {
        return processSpawner.getState(appId);
    });

    ipcMain.handle("applications:getAllStates", () => {
        return processSpawner.getAllStates();
    });

    ipcMain.handle("applications:getLogs", (_e, appId: string) => {
        return processSpawner.getLogs(appId);
    });

    ipcMain.handle("applications:checkPort", (_e, port: number) => {
        return checkPortInUse(port);
    });

    ipcMain.handle("applications:killPort", (_e, port: number) => {
        return killProcessOnPort(port);
    });
}
