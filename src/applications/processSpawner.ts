/**
 * Process Spawner — flat singleton that manages running application processes.
 * Reads the pre-computed command from disk and spawns it. No type dispatching.
 */

import { ChildProcess, spawn } from "child_process";
import { BrowserWindow } from "electron";
import { readEntity } from "@/store/workspaceFs";
import {
    ApplicationConfig,
    AppProcessState,
    AppProcessStatus,
    AppLogChunk,
} from "./types";

interface RunningProcess {
    proc: ChildProcess;
    state: AppProcessState;
    logBuffer: AppLogChunk[];
}

const MAX_LOG_LINES = 5000;

class ProcessSpawner {
    private processes: Map<string, RunningProcess> = new Map();
    private mainWindow: BrowserWindow | null = null;

    setMainWindow(win: BrowserWindow): void {
        this.mainWindow = win;
    }

    /**
     * Start an application by ID.
     * Reads the config (with pre-computed command) from disk, then spawns it.
     */
    start(wsId: string, appId: string, mode: "run" | "debug"): AppProcessState {
        // Stop existing process for this app if running
        if (this.processes.has(appId)) {
            this.stop(appId);
        }

        // Read config from disk (contains resolvedCommand, resolvedCwd, resolvedEnv)
        const config = readEntity<ApplicationConfig>(wsId, "applications", appId);
        if (!config) {
            const errorState: AppProcessState = {
                appId,
                status: "error",
                error: `Application config not found: ${appId}`,
            };
            this.sendToRenderer("app:statusChange", errorState);
            return errorState;
        }

        // Pick command based on mode
        const command = mode === "debug" && config.resolvedDebugCommand
            ? config.resolvedDebugCommand
            : config.resolvedCommand;

        if (!command || !command.trim()) {
            const errorState: AppProcessState = {
                appId,
                status: "error",
                error: "No command to run. Save the configuration to generate the command.",
            };
            this.sendToRenderer("app:statusChange", errorState);
            return errorState;
        }

        const cwd = config.resolvedCwd || config.workingDirectory || process.cwd();
        const env = { ...process.env, ...(config.resolvedEnv || {}) };

        // Prepare state
        const state: AppProcessState = {
            appId,
            status: "starting",
            startedAt: Date.now(),
            debugPort: mode === "debug" ? config.resolvedDebugPort : undefined,
        };

        const logBuffer: AppLogChunk[] = [];

        // Emit system log: what we're about to run
        this.emitLog(appId, logBuffer, "system", `> ${command}\n`);
        this.emitLog(appId, logBuffer, "system", `  cwd: ${cwd}\n`);
        if (mode === "debug" && config.resolvedDebugPort) {
            this.emitLog(appId, logBuffer, "system", `  debug port: ${config.resolvedDebugPort}\n`);
        }
        this.emitLog(appId, logBuffer, "system", `\n`);

        // Spawn — shell:true so the command string is interpreted by the OS shell
        const proc = spawn(command, [], {
            cwd,
            env,
            shell: true,
            stdio: ["ignore", "pipe", "pipe"],
            windowsHide: true,
        });

        const pid = proc.pid ?? 0;
        state.pid = pid;
        state.status = mode === "debug" ? "debugging" : "running";

        const entry: RunningProcess = { proc, state, logBuffer };
        this.processes.set(appId, entry);

        // Stream stdout
        proc.stdout?.on("data", (data: Buffer) => {
            this.emitLog(appId, entry.logBuffer, "stdout", data.toString());
        });

        // Stream stderr
        proc.stderr?.on("data", (data: Buffer) => {
            this.emitLog(appId, entry.logBuffer, "stderr", data.toString());
        });

        // Handle spawn error (e.g. command not found)
        proc.on("error", (err) => {
            this.emitLog(appId, entry.logBuffer, "system", `\n[ERROR] ${err.message}\n`);
            entry.state.status = "error";
            entry.state.error = err.message;
            this.sendToRenderer("app:statusChange", {
                appId,
                status: "error",
                error: err.message,
            });
        });

        // Handle exit
        proc.on("exit", (code, signal) => {
            const exitMsg = signal
                ? `\n[Process terminated by signal ${signal}]\n`
                : `\n[Process exited with code ${code}]\n`;
            this.emitLog(appId, entry.logBuffer, "system", exitMsg);

            entry.state.status = "exited";
            entry.state.exitCode = code;
            entry.state.stoppedAt = Date.now();

            this.sendToRenderer("app:statusChange", {
                appId,
                status: "exited",
                exitCode: code,
                pid: entry.state.pid,
            });
        });

        // Notify renderer of running state
        this.sendToRenderer("app:statusChange", {
            appId,
            status: state.status,
            pid,
            debugPort: state.debugPort,
            startedAt: state.startedAt,
        });

        return state;
    }

    /**
     * Stop a running application.
     */
    stop(appId: string): void {
        const entry = this.processes.get(appId);
        if (!entry || !entry.proc) return;

        entry.state.status = "stopping";
        this.emitLog(appId, entry.logBuffer, "system", "\n[Stopping process...]\n");
        this.sendToRenderer("app:statusChange", { appId, status: "stopping" });

        const proc = entry.proc;
        const pid = proc.pid;

        // Kill process tree
        if (process.platform === "win32") {
            // Windows: taskkill with /T (tree) and /F (force)
            spawn("taskkill", ["/pid", String(pid), "/T", "/F"], { shell: true });
        } else {
            // Unix: SIGTERM to process group
            try {
                process.kill(-pid!, "SIGTERM");
            } catch {
                try { proc.kill("SIGTERM"); } catch { /* already dead */ }
            }
        }

        // Force kill after 5 seconds if still alive
        const forceKillTimer = setTimeout(() => {
            if (entry.proc && !entry.proc.killed) {
                this.emitLog(appId, entry.logBuffer, "system", "[Force killing...]\n");
                if (process.platform === "win32") {
                    spawn("taskkill", ["/pid", String(pid), "/T", "/F"], { shell: true });
                } else {
                    try { process.kill(-pid!, "SIGKILL"); } catch {
                        try { proc.kill("SIGKILL"); } catch { /* dead */ }
                    }
                }
            }
        }, 5000);

        proc.on("exit", () => clearTimeout(forceKillTimer));
    }

    /**
     * Get state of a specific app.
     */
    getState(appId: string): AppProcessState | null {
        return this.processes.get(appId)?.state ?? null;
    }

    /**
     * Get all process states.
     */
    getAllStates(): AppProcessState[] {
        return Array.from(this.processes.values()).map(e => e.state);
    }

    /**
     * Get log buffer for an app.
     */
    getLogs(appId: string): AppLogChunk[] {
        return this.processes.get(appId)?.logBuffer ?? [];
    }

    /**
     * Stop all running processes (called on app quit).
     */
    stopAll(): void {
        for (const [appId] of this.processes) {
            this.stop(appId);
        }
    }

    // ── Private helpers ─────────────────────────────────────────────────────────

    private emitLog(
        appId: string,
        buffer: AppLogChunk[],
        stream: "stdout" | "stderr" | "system",
        data: string,
    ): void {
        const chunk: AppLogChunk = { appId, stream, data, ts: Date.now() };
        buffer.push(chunk);
        if (buffer.length > MAX_LOG_LINES) {
            buffer.splice(0, buffer.length - MAX_LOG_LINES);
        }
        this.sendToRenderer("app:log", chunk);
    }

    private sendToRenderer(channel: string, data: unknown): void {
        if (this.mainWindow && !this.mainWindow.isDestroyed()) {
            this.mainWindow.webContents.send(channel, data);
        }
    }
}

export const processSpawner = new ProcessSpawner();
