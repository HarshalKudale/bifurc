/**
 * Happy-path workflow test for the **Collection Runner's persistence** —
 * `src/ipc/handlers/runnerHandlers.ts` (0% coverage). The runner's Run button is only
 * useful if the report survives: saved runs are what the History list and the "Export
 * Report" action read back.
 *
 * These drive the REAL registered handlers against a REAL workspace on disk and assert
 * the files that actually land in `requests/.runs/<folderId>/<startedAt>/`. Only
 * Electron itself (and the save dialog) is mocked.
 *
 * Runs in the integration project (shared process-global data-root override ⇒
 * `fileParallelism: false`).
 */

import { describe, it, expect, beforeAll, beforeEach, afterAll, vi } from "vitest";
import * as fs from "fs";
import * as path from "path";
import { createWorkspace, TEST_WS, type WorkspaceFixture } from "./proxyHarness";

const { handlers, dialogState } = vi.hoisted(() => ({
  handlers: new Map<string, (...args: any[]) => any>(),
  dialogState: { result: { canceled: true, filePath: undefined as string | undefined } },
}));

vi.mock("electron", () => ({
  ipcMain: {
    handle: (channel: string, fn: (...args: any[]) => any) => { handlers.set(channel, fn); },
    on: vi.fn(),
  },
  BrowserWindow: { getAllWindows: vi.fn(() => []) },
  dialog: { showSaveDialog: vi.fn(async () => dialogState.result) },
  app: {
    getPath: vi.fn(() => "/tmp/test-user-data"),
    on: vi.fn(),
    whenReady: vi.fn(() => Promise.resolve()),
    commandLine: { appendSwitch: vi.fn() },
    quit: vi.fn(),
  },
}));

function invoke<T = any>(channel: string, ...args: any[]): T {
  const fn = handlers.get(channel);
  if (!fn) throw new Error(`No handler registered for "${channel}"`);
  return fn({}, ...args) as T;
}

let ws: WorkspaceFixture;

function runsDir(folderId: string): string {
  return path.join(ws.dataRoot, TEST_WS, "requests", ".runs", folderId);
}

const FOLDER_ID = "folder-runner-1";
const STARTED_AT = 1_700_000_000_000;

function makeReport(overrides: Record<string, any> = {}) {
  return {
    folderId: FOLDER_ID,
    folderName: "Smoke Tests",
    startedAt: STARTED_AT,
    completedAt: STARTED_AT + 2_500,
    totalRequests: 2,
    totalTests: 3,
    passedTests: 2,
    failedTests: 1,
    results: [
      {
        requestId: "a", requestName: "Get Users", method: "GET", url: "http://api.localhost/users",
        status: 200, responseTime: 12, tests: [{ name: "ok", passed: true, durationMs: 1 }], testLogs: [],
      },
      {
        requestId: "b", requestName: "Get Missing", method: "GET", url: "http://api.localhost/missing",
        status: 404, responseTime: 8,
        tests: [{ name: "is 200", passed: false, error: "expected 404 to equal 200", durationMs: 1 }],
        testLogs: [],
      },
    ],
    ...overrides,
  };
}

beforeAll(async () => {
  ws = createWorkspace();
  const { registerRunnerHandlers } = await import("@/ipc/handlers/runnerHandlers");
  // P3 made `runner:exportReport` a thin client: it shows the dialog, then calls the engine's
  // `runner.exportReport` command and writes the artifact it gets back. The renderer-facing
  // contract is unchanged (`{ok:false}` on cancel, `{ok:true, filePath}` on success), so every
  // assertion below still holds — but the engine half has to be registered, exactly as
  // `src/ipc/handlers.ts` registers it in production. Without this the handler returns
  // `{ok:false, error: 'No handler registered for command "runner.exportReport".'}` and the two
  // file-writing tests fail with `ENOENT` on a file that was never written.
  const { registerBlobCommands } = await import("@bifurc/engine/blob/commands");
  const { registerFileOpsCommands } = await import("@bifurc/engine/fileOps/commands");
  const { commandRegistry } = await import("@bifurc/engine/commands/registry");
  registerBlobCommands(commandRegistry);
  registerFileOpsCommands(commandRegistry);
  registerRunnerHandlers();
});

beforeEach(() => {
  dialogState.result = { canceled: true, filePath: undefined };
});

afterAll(() => {
  ws?.cleanup();
});

// ── Saving a run ──────────────────────────────────────────────────────────────

describe("runner:saveReport", () => {
  it("writes report.json and report.html into a timestamped run directory", async () => {
    const res = await invoke("runner:saveReport", TEST_WS, makeReport());

    expect(res).toEqual({ ok: true });

    const runDir = path.join(runsDir(FOLDER_ID), String(STARTED_AT));
    expect(fs.existsSync(path.join(runDir, "report.json"))).toBe(true);
    expect(fs.existsSync(path.join(runDir, "report.html"))).toBe(true);
  });

  it("round-trips the report through the saved JSON", async () => {
    const report = makeReport({ startedAt: STARTED_AT + 1, folderName: "Round Trip" });
    await invoke("runner:saveReport", TEST_WS, report);

    const saved = JSON.parse(
      fs.readFileSync(path.join(runsDir(FOLDER_ID), String(report.startedAt), "report.json"), "utf-8"),
    );

    expect(saved.folderName).toBe("Round Trip");
    expect(saved.totalTests).toBe(3);
    expect(saved.passedTests).toBe(2);
    expect(saved.results).toHaveLength(2);
    expect(saved.results[1].tests[0].error).toContain("expected 404 to equal 200");
  });

  it("writes a standalone HTML report naming the folder", async () => {
    const report = makeReport({ startedAt: STARTED_AT + 2, folderName: "Html Folder" });
    await invoke("runner:saveReport", TEST_WS, report);

    const html = fs.readFileSync(
      path.join(runsDir(FOLDER_ID), String(report.startedAt), "report.html"),
      "utf-8",
    );

    expect(html).toContain("<!DOCTYPE html>");
    expect(html).toContain("Html Folder");
    expect(html).toContain("Get Users");
    expect(html).toContain("expected 404 to equal 200");
  });

  it("reports failure instead of throwing when the report is malformed", async () => {
    const res = await invoke("runner:saveReport", TEST_WS, { startedAt: STARTED_AT });

    expect(res.ok).toBe(false);
    expect(typeof res.error).toBe("string");
  });
});

// ── Run history ───────────────────────────────────────────────────────────────

describe("runner:getHistory", () => {
  it("lists a saved run with its summary", async () => {
    await invoke("runner:saveReport", TEST_WS, makeReport({ folderId: "folder-history" }));

    const history = await invoke("runner:getHistory", TEST_WS, "folder-history");

    expect(history).toHaveLength(1);
    expect(history[0].timestamp).toBe(STARTED_AT);
    expect(history[0].summary).toEqual({ total: 3, passed: 2, failed: 1 });
  });

  it("returns the newest run first", async () => {
    const folder = "folder-order";
    await invoke("runner:saveReport", TEST_WS, makeReport({ folderId: folder, startedAt: 1_000 }));
    await invoke("runner:saveReport", TEST_WS, makeReport({ folderId: folder, startedAt: 3_000 }));
    await invoke("runner:saveReport", TEST_WS, makeReport({ folderId: folder, startedAt: 2_000 }));

    const history = await invoke("runner:getHistory", TEST_WS, folder);

    expect(history.map((h: any) => h.timestamp)).toEqual([3_000, 2_000, 1_000]);
  });

  it("returns an empty list for a folder that has never been run", async () => {
    expect(await invoke("runner:getHistory", TEST_WS, "never-run")).toEqual([]);
  });

  it("skips run directories that contain no report", async () => {
    const folder = "folder-gaps";
    fs.mkdirSync(path.join(runsDir(folder), "12345"), { recursive: true });
    await invoke("runner:saveReport", TEST_WS, makeReport({ folderId: folder }));

    const history = await invoke("runner:getHistory", TEST_WS, folder);

    expect(history).toHaveLength(1);
    expect(history[0].timestamp).toBe(STARTED_AT);
  });

  it("returns an empty list rather than throwing for an unknown workspace", async () => {
    expect(await invoke("runner:getHistory", "ws-does-not-exist", FOLDER_ID)).toEqual([]);
  });
});

// ── Runner configuration per folder ───────────────────────────────────────────

describe("runner:saveConfig / runner:loadConfig", () => {
  it("round-trips a folder's runner configuration", async () => {
    const config = { delayMs: 250, stopOnFailure: true, iterations: 3 };

    expect(await invoke("runner:saveConfig", TEST_WS, "folder-config", config)).toEqual({ ok: true });
    expect(await invoke("runner:loadConfig", TEST_WS, "folder-config")).toEqual(config);
  });

  it("returns null when a folder has no saved configuration", async () => {
    expect(await invoke("runner:loadConfig", TEST_WS, "folder-without-config")).toBeNull();
  });

  it("listFolderIds reports only folders that have a saved configuration", async () => {
    await invoke("runner:saveConfig", TEST_WS, "folder-listed", { delayMs: 0 });
    await invoke("runner:saveReport", TEST_WS, makeReport({ folderId: "folder-only-a-run" }));

    const ids = await invoke("runner:listFolderIds", TEST_WS);

    expect(ids).toContain("folder-listed");
    expect(ids).not.toContain("folder-only-a-run");
  });

  it("listFolderIds returns an empty list for an unknown workspace", async () => {
    expect(await invoke("runner:listFolderIds", "ws-does-not-exist")).toEqual([]);
  });
});

// ── Export ────────────────────────────────────────────────────────────────────

describe("runner:exportReport", () => {
  it("writes an HTML file when a path is chosen", async () => {
    const out = path.join(ws.root, "export.html");
    dialogState.result = { canceled: false, filePath: out };

    const res = await invoke("runner:exportReport", makeReport({ folderName: "Exported" }));

    expect(res).toEqual({ ok: true, filePath: out });
    const html = fs.readFileSync(out, "utf-8");
    expect(html).toContain("Exported");
  });

  it("writes JSON when the chosen path ends in .json", async () => {
    const out = path.join(ws.root, "export.json");
    dialogState.result = { canceled: false, filePath: out };

    await invoke("runner:exportReport", makeReport());

    const parsed = JSON.parse(fs.readFileSync(out, "utf-8"));
    expect(parsed.totalTests).toBe(3);
    expect(parsed.results).toHaveLength(2);
  });

  it("does nothing when the save dialog is cancelled", async () => {
    dialogState.result = { canceled: true, filePath: undefined };

    expect(await invoke("runner:exportReport", makeReport())).toEqual({ ok: false });
  });

  it("does nothing when the dialog returns no path", async () => {
    dialogState.result = { canceled: false, filePath: undefined };

    expect(await invoke("runner:exportReport", makeReport())).toEqual({ ok: false });
  });
});
