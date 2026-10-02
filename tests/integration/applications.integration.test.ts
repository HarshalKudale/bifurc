/**
 * Happy-path workflow test for the **Applications** feature.
 *
 * This was the largest completely untested feature in the app: the panel, the
 * `applications:*` IPC handlers, the command generator's runtime side and the
 * process spawner had no test at all, so "Run" could have been broken end to end
 * without a single failing test.
 *
 * **It drives the `application.*` commands today, not the channels** — the ten
 * `applications:*` channels were deleted on 2026-09-21 (P6 reduction, seventh group,
 * `plan/07` §5b-octodecies). The subject is unchanged: the commands are where the
 * behaviour always lived, the channels were one-line `commandRegistry.invoke()`
 * adapters over them, and the registry is the path a containerised engine has.
 *
 * It runs against a REAL workspace on disk and spawns REAL child processes (a tiny
 * generated Node script), then asserts the user-visible outcome: state transitions,
 * exit code, and the log stream the panel renders.
 *
 * Runs in the integration project (shared process-global data-root override ⇒
 * `fileParallelism: false`).
 */

import { describe, it, expect, beforeAll, afterAll, vi } from "vitest";
import * as fs from "fs";
import * as path from "path";
import * as net from "net";
import type { CommandAction, CommandContext, CommandRegistry } from "@bifurc/engine/kernel/registry";
import { createWorkspace, TEST_WS, type WorkspaceFixture } from "./proxyHarness";

// `electron` is still mocked, because the command modules this suite now drives reach `processSpawner`
// and `portUtils`, which import it. Nothing here registers or calls a channel.
vi.mock("electron", () => ({
    ipcMain: { handle: vi.fn(), on: vi.fn() },
    BrowserWindow: { getAllWindows: vi.fn(() => []) },
    app: { getPath: vi.fn(() => "/tmp/test-user-data"), on: vi.fn(), whenReady: vi.fn(() => Promise.resolve()) },
}));

/**
 * The ten `applications:*` channels were **deleted** on 2026-09-21 (P6 reduction, seventh group,
 * `plan/07` §5b-octodecies), so this suite drives the commands those channels forwarded to — which is
 * what the channel did, minus the channel. Every one of the ten was a one-line
 * `commandRegistry.invoke()` adapter with no behaviour of its own, so nothing was lost in the move.
 *
 * The positional → payload mapping below is the *deleted bodies'* mapping, reproduced verbatim: a
 * positional call is split across two places (the wrapper injected the event and forwarded the rest;
 * this function builds the payload from the rest) —
 * `plan/07` §5b-quindecies' argument-shift trap, in the direction that keeps the signature honest.
 *
 * `registry` is assigned in `beforeAll` rather than imported statically: `vi.mock("electron")` is
 * hoisted, but the engine modules must load *after* the data-root override `createWorkspace()` installs,
 * or `workspaceFs` resolves against the developer's real user-data directory.
 */
let registry: CommandRegistry;

/** What `in-process` and every legacy `ipcMain.handle` adapter pass: a bus, no session. */
const ctx: CommandContext = { bus: {} as CommandContext["bus"] };

/** The registry's ctx, with a *real* bus — for the commands whose result is not the whole story. */
function invokeCommand<T = any>(action: CommandAction, params: unknown): T {
    return registry.invoke(action, params, ctx) as T;
}

// ── Fixtures ──────────────────────────────────────────────────────────────────

let ws: WorkspaceFixture;
const processesToStop: string[] = [];

/** Poll until `check()` is truthy, or fail after `timeoutMs`. */
async function waitFor(check: () => boolean | Promise<boolean>, timeoutMs = 20_000, intervalMs = 100): Promise<void> {
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
        if (await check()) return;
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

    // P6 §5c slice 5 moved these ten out of `applicationHandlers.ts`'s module scope, and the reduction's
    // seventh group then deleted the channels — so this call is now the commands' only wiring here, not
    // a second one. It is also the *only* suite that ever drove them, so no other file needed the move.
    const { commandRegistry } = await import("@bifurc/engine/kernel/registry");
    const { registerApplicationCommands } = await import("@bifurc/engine/applications/applicationCommands");
    registerApplicationCommands(commandRegistry);
    registry = commandRegistry;
});

afterAll(() => {
    // Make sure nothing outlives the suite.
    for (const id of processesToStop) {
        try { registry.invoke("application.stop", { appId: id }, ctx); } catch { /* already gone */ }
    }
    ws?.cleanup();
});

// ── Tests ─────────────────────────────────────────────────────────────────────

describe("Applications — happy path", () => {
    it("registers the full set of application commands", () => {
        // The ten `applications:*` channels are gone, so "registered" now means "the registry implements
        // it" — the ratchet that matters for a containerised engine. `tests/ipc/handlers.test.ts`'s
        // "registry coverage" block asserts the whole protocol set; this asserts the namespace, because
        // the failure mode this suite exists for is a command that silently is not there.
        for (const command of [
            "application.list", "application.save", "application.delete",
            "application.start", "application.stop", "application.getState",
            "application.getAllStates", "application.getLogs",
            "application.checkPort", "application.killPort",
        ]) {
            expect(registry.isRegistered(command as any), `missing command ${command}`).toBe(true);
        }
    });

    it("saves a run configuration and pre-computes the command", async () => {
        const saved = await invokeCommand("application.save", { application: appConfig({ name: "Saving App" }) });

        expect(saved.id).toBeTruthy();
        expect(saved.createdAt).toBeGreaterThan(0);
        expect(saved.resolvedCommand).toBe("node emit.js");
        expect(saved.resolvedCwd).toBe(ws.root);
        expect(saved.resolvedEnv).toEqual({});

        const listed = await invokeCommand<any[]>("application.list", { workspaceId: TEST_WS });
        expect(listed.some((a) => a.id === saved.id)).toBe(true);
    });

    it("lists an empty set for a workspace with no applications", async () => {
        expect(await invokeCommand<any[]>("application.list", { workspaceId: "ws-with-nothing" })).toEqual([]);
    });

    it("runs a saved application, streams its logs and reports the exit code", async () => {
        const saved = await invokeCommand("application.save", { application: appConfig({ name: "Runner" }) });
        processesToStop.push(saved.id);

        const started = await invokeCommand("application.start", { workspaceId: TEST_WS, appId: saved.id, mode: "run" });
        expect(started.appId).toBe(saved.id);
        expect(started.status).toBe("running");
        expect(started.pid).toBeGreaterThan(0);

        // The state is observable straight away through getState/getAllStates.
        expect((await invokeCommand("application.getState", { appId: saved.id })).status).toBe("running");
        expect((await invokeCommand<any[]>("application.getAllStates", {})).some((s) => s.appId === saved.id)).toBe(true);

        // ...and the process really finishes.
        await waitFor(async () => (await invokeCommand("application.getState", { appId: saved.id }))?.status === "exited");

        const final = await invokeCommand("application.getState", { appId: saved.id });
        expect(final.exitCode).toBe(0);
        expect(final.stoppedAt).toBeGreaterThan(0);

        const logs = await invokeCommand<any[]>("application.getLogs", { appId: saved.id });
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
        const saved = await invokeCommand("application.save", { application: appConfig({ name: "Stayer", command: "node stay.js" }) });
        processesToStop.push(saved.id);

        expect((await invokeCommand("application.start", { workspaceId: TEST_WS, appId: saved.id, mode: "run" })).status).toBe("running");

        const stopResult = await invokeCommand("application.stop", { appId: saved.id });
        expect(stopResult).toEqual({ ok: true });

        await waitFor(async () => {
            const s = await invokeCommand("application.getState", { appId: saved.id });
            return s?.status === "exited";
        }, 20_000);

        expect((await invokeCommand<any[]>("application.getLogs", { appId: saved.id })).map((l) => l.data).join(""))
            .toContain("[Stopping process...]");
    });

    it("starts a debug run and reports the debug port", async () => {
        const debugPort = await freePort();
        const saved = await invokeCommand("application.save", {
            application: appConfig({
                name: "Debugger",
                type: "node",
                command: undefined,
                debugPort,
                nodeConfig: { scriptPath: "emit.js", nodeArgs: [] },
            }),
        });
        processesToStop.push(saved.id);

        expect(saved.resolvedDebugPort).toBe(debugPort);
        expect(saved.resolvedDebugCommand).toContain(`--inspect=0.0.0.0:${debugPort}`);

        const started = await invokeCommand("application.start", { workspaceId: TEST_WS, appId: saved.id, mode: "debug" });
        expect(started.status).toBe("debugging");
        expect(started.debugPort).toBe(debugPort);

        const logs = (await invokeCommand<any[]>("application.getLogs", { appId: saved.id })).map((l) => l.data).join("");
        expect(logs).toContain(`debug port: ${debugPort}`);

        await waitFor(async () => (await invokeCommand("application.getState", { appId: saved.id }))?.status === "exited");
    });

    it("reports an error state for an unknown application instead of throwing", async () => {
        const state = await invokeCommand("application.start", { workspaceId: TEST_WS, appId: "no-such-app", mode: "run" });
        expect(state.status).toBe("error");
        expect(state.error).toContain("no-such-app");
        expect(state.pid).toBeUndefined();
    });

    it("reports an error state when the configuration has no command", async () => {
        const saved = await invokeCommand("application.save", { application: appConfig({ name: "Empty", command: "", resolvedCommand: "" }) });
        // Overwrite the pre-computed command so the spawner sees an empty one.
        const onDisk = path.join(ws.dataRoot, TEST_WS, "applications", `${saved.id}.json`);
        const raw = JSON.parse(fs.readFileSync(onDisk, "utf-8"));
        fs.writeFileSync(onDisk, JSON.stringify({ ...raw, resolvedCommand: "" }, null, 2), "utf-8");

        const state = await invokeCommand("application.start", { workspaceId: TEST_WS, appId: saved.id, mode: "run" });
        expect(state.status).toBe("error");
        expect(state.error).toContain("No command to run");
    });

    it("deletes an application and stops it first", async () => {
        const saved = await invokeCommand("application.save", { application: appConfig({ name: "Doomed", command: "node stay.js" }) });
        await invokeCommand("application.start", { workspaceId: TEST_WS, appId: saved.id, mode: "run" });

        expect(await invokeCommand("application.delete", { workspaceId: TEST_WS, id: saved.id })).toEqual({ ok: true });

        const listed = await invokeCommand<any[]>("application.list", { workspaceId: TEST_WS });
        expect(listed.some((a) => a.id === saved.id)).toBe(false);
    });

    it("checks whether a port is in use", async () => {
        const port = await freePort();
        const info = await invokeCommand("application.checkPort", { port });
        expect(info).toHaveProperty("inUse");
        expect(typeof info.inUse).toBe("boolean");
    });

    it("resolves the port-kill request without throwing", async () => {
        const port = await freePort();
        const res = await invokeCommand("application.killPort", { port });
        expect(res).toHaveProperty("ok");
    });

    it("returns null state and an empty log buffer for an application that was never started", async () => {
        expect(await invokeCommand("application.getState", { appId: "never-started" })).toBeNull();
        expect(await invokeCommand<any[]>("application.getLogs", { appId: "never-started" })).toEqual([]);
    });
});
