/**
 * Happy-path workflow test for the **Applications** feature.
 *
 * This was the largest completely untested feature in the app: the panel, the
 * `applications:*` IPC handlers, the command generator's runtime side and the
 * process spawner had no test at all, so "Run" could have been broken end to end
 * without a single failing test.
 *
 * It drives the REAL registered IPC handlers against a REAL workspace on disk and
 * spawns REAL child processes (a tiny generated Node script), then asserts the
 * user-visible outcome: state transitions, exit code, and the log stream the panel
 * renders.
 *
 * Runs in the integration project (shared process-global data-root override ⇒
 * `fileParallelism: false`).
 */

import { describe, it, expect, beforeAll, afterAll, vi } from "vitest";
import * as fs from "fs";
import * as path from "path";
import * as net from "net";
import { createWorkspace, TEST_WS, type WorkspaceFixture } from "./proxyHarness";

// ── Capture the handlers the feature registers ────────────────────────────────
// Same technique as tests/ipc/handlers.test.ts: swap ipcMain for a registry so the
// production handler functions can be invoked directly.

const { handlers } = vi.hoisted(() => ({ handlers: new Map<string, (...args: any[]) => any>() }));

vi.mock("electron", () => ({
    ipcMain: {
        handle: (channel: string, fn: (...args: any[]) => any) => { handlers.set(channel, fn); },
        on: vi.fn(),
    },
    BrowserWindow: { getAllWindows: vi.fn(() => []) },
    app: { getPath: vi.fn(() => "/tmp/test-user-data"), on: vi.fn(), whenReady: vi.fn(() => Promise.resolve()) },
}));

/** Invoke a registered handler the way ipcMain would (first arg is the event). */
function invoke<T = any>(channel: string, ...args: any[]): T {
    const fn = handlers.get(channel);
    if (!fn) throw new Error(`No handler registered for "${channel}"`);
    return fn({}, ...args) as T;
}

// ── Fixtures ──────────────────────────────────────────────────────────────────

let ws: WorkspaceFixture;
const processesToStop: string[] = [];

/** Poll until `check()` is truthy, or fail after `timeoutMs`. */
async function waitFor(check: () => boolean, timeoutMs = 20_000, intervalMs = 100): Promise<void> {
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
        if (check()) return;
        await new Promise((r) => setTimeout(r, intervalMs));
    }
    throw new Error(`Condition not met within ${timeoutMs}ms`);
}

function freePort(): Promise<number> {
    return new Promise((resolve, reject) => {
        const srv = net.createServer();
        srv.on("error", reject);
        srv.listen(0, "127.0.0.1", () => {
            const addr = srv.address();
            const port = typeof addr === "object" && addr ? addr.port : 0;
            srv.close(() => resolve(port));
        });
    });
}

function appConfig(over: Record<string, any> = {}): any {
    return {
        id: "",
        name: "Integration App",
        type: "shell",
        workingDirectory: ws.root,
        command: "node emit.js",
        args: [],
        createdAt: 0,
        workspaceId: TEST_WS,
        resolvedCommand: "",
        resolvedCwd: "",
        resolvedEnv: {},
        ...over,
    };
}

beforeAll(async () => {
    ws = createWorkspace();

    // Two throwaway programs the app will "run".
    fs.writeFileSync(
        path.join(ws.root, "emit.js"),
        `console.log("hello-from-app");\nconsole.error("warn-from-app");\n`,
        "utf-8",
    );
    fs.writeFileSync(path.join(ws.root, "stay.js"), `setInterval(() => {}, 1000);\n`, "utf-8");

    const { registerApplicationHandlers } = await import("@/ipc/applicationHandlers");
    registerApplicationHandlers();
});

afterAll(() => {
    // Make sure nothing outlives the suite.
    for (const id of processesToStop) {
        try { invoke("applications:stop", id); } catch { /* already gone */ }
    }
    ws?.cleanup();
});

// ── Tests ─────────────────────────────────────────────────────────────────────

describe("Applications — happy path", () => {
    it("registers the full set of application channels", () => {
        for (const channel of [
            "applications:list", "applications:save", "applications:delete",
            "applications:start", "applications:stop", "applications:getState",
            "applications:getAllStates", "applications:getLogs",
            "applications:checkPort", "applications:killPort",
        ]) {
            expect(handlers.has(channel), `missing handler ${channel}`).toBe(true);
        }
    });

    it("saves a run configuration and pre-computes the command", () => {
        const saved = invoke("applications:save", appConfig({ name: "Saving App" }));

        expect(saved.id).toBeTruthy();
        expect(saved.createdAt).toBeGreaterThan(0);
        expect(saved.resolvedCommand).toBe("node emit.js");
        expect(saved.resolvedCwd).toBe(ws.root);
        expect(saved.resolvedEnv).toEqual({});

        const listed = invoke<any[]>("applications:list", TEST_WS);
        expect(listed.some((a) => a.id === saved.id)).toBe(true);
    });

    it("lists an empty set for a workspace with no applications", () => {
        expect(invoke<any[]>("applications:list", "ws-with-nothing")).toEqual([]);
    });

    it("runs a saved application, streams its logs and reports the exit code", async () => {
        const saved = invoke("applications:save", appConfig({ name: "Runner" }));
        processesToStop.push(saved.id);

        const started = invoke("applications:start", TEST_WS, saved.id, "run");
        expect(started.appId).toBe(saved.id);
        expect(started.status).toBe("running");
        expect(started.pid).toBeGreaterThan(0);

        // The state is observable straight away through getState/getAllStates.
        expect(invoke("applications:getState", saved.id).status).toBe("running");
        expect(invoke<any[]>("applications:getAllStates").some((s) => s.appId === saved.id)).toBe(true);

        // ...and the process really finishes.
        await waitFor(() => invoke("applications:getState", saved.id)?.status === "exited");

        const final = invoke("applications:getState", saved.id);
        expect(final.exitCode).toBe(0);
        expect(final.stoppedAt).toBeGreaterThan(0);

        const logs = invoke<any[]>("applications:getLogs", saved.id);
        const text = logs.map((l) => l.data).join("");
        expect(text).toContain("> node emit.js");
        expect(text).toContain(`cwd: ${ws.root}`);
        expect(text).toContain("hello-from-app");
        expect(text).toContain("warn-from-app");
        expect(text).toContain("[Process exited with code 0]");

        // Log chunks are tagged with their stream so the panel can colour them.
        expect(logs.some((l) => l.stream === "stdout" && l.data.includes("hello-from-app"))).toBe(true);
        expect(logs.some((l) => l.stream === "stderr" && l.data.includes("warn-from-app"))).toBe(true);
        expect(logs.every((l) => typeof l.ts === "number")).toBe(true);
    });

    it("stops a long-running application", async () => {
        const saved = invoke("applications:save", appConfig({ name: "Stayer", command: "node stay.js" }));
        processesToStop.push(saved.id);

        expect(invoke("applications:start", TEST_WS, saved.id, "run").status).toBe("running");

        const stopResult = invoke("applications:stop", saved.id);
        expect(stopResult).toEqual({ ok: true });

        await waitFor(() => {
            const s = invoke("applications:getState", saved.id);
            return s?.status === "exited";
        }, 20_000);

        expect(invoke<any[]>("applications:getLogs", saved.id).map((l) => l.data).join(""))
            .toContain("[Stopping process...]");
    });

    it("starts a debug run and reports the debug port", async () => {
        const debugPort = await freePort();
        const saved = invoke("applications:save", appConfig({
            name: "Debugger",
            type: "node",
            command: undefined,
            debugPort,
            nodeConfig: { scriptPath: "emit.js", nodeArgs: [] },
        }));
        processesToStop.push(saved.id);

        expect(saved.resolvedDebugPort).toBe(debugPort);
        expect(saved.resolvedDebugCommand).toContain(`--inspect=0.0.0.0:${debugPort}`);

        const started = invoke("applications:start", TEST_WS, saved.id, "debug");
        expect(started.status).toBe("debugging");
        expect(started.debugPort).toBe(debugPort);

        const logs = invoke<any[]>("applications:getLogs", saved.id).map((l) => l.data).join("");
        expect(logs).toContain(`debug port: ${debugPort}`);

        await waitFor(() => invoke("applications:getState", saved.id)?.status === "exited");
    });

    it("reports an error state for an unknown application instead of throwing", () => {
        const state = invoke("applications:start", TEST_WS, "no-such-app", "run");
        expect(state.status).toBe("error");
        expect(state.error).toContain("no-such-app");
        expect(state.pid).toBeUndefined();
    });

    it("reports an error state when the configuration has no command", () => {
        const saved = invoke("applications:save", appConfig({ name: "Empty", command: "", resolvedCommand: "" }));
        // Overwrite the pre-computed command so the spawner sees an empty one.
        const onDisk = path.join(ws.dataRoot, TEST_WS, "applications", `${saved.id}.json`);
        const raw = JSON.parse(fs.readFileSync(onDisk, "utf-8"));
        fs.writeFileSync(onDisk, JSON.stringify({ ...raw, resolvedCommand: "" }, null, 2), "utf-8");

        const state = invoke("applications:start", TEST_WS, saved.id, "run");
        expect(state.status).toBe("error");
        expect(state.error).toContain("No command to run");
    });

    it("deletes an application and stops it first", () => {
        const saved = invoke("applications:save", appConfig({ name: "Doomed", command: "node stay.js" }));
        invoke("applications:start", TEST_WS, saved.id, "run");

        expect(invoke("applications:delete", TEST_WS, saved.id)).toEqual({ ok: true });

        const listed = invoke<any[]>("applications:list", TEST_WS);
        expect(listed.some((a) => a.id === saved.id)).toBe(false);
    });

    it("checks whether a port is in use", async () => {
        const port = await freePort();
        const info = await invoke("applications:checkPort", port);
        expect(info).toHaveProperty("inUse");
        expect(typeof info.inUse).toBe("boolean");
    });

    it("resolves the port-kill request without throwing", async () => {
        const port = await freePort();
        const res = await invoke("applications:killPort", port);
        expect(res).toHaveProperty("ok");
    });

    it("returns null state and an empty log buffer for an application that was never started", () => {
        expect(invoke("applications:getState", "never-started")).toBeNull();
        expect(invoke<any[]>("applications:getLogs", "never-started")).toEqual([]);
    });
});
