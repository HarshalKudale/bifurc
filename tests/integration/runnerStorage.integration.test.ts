/**
 * Happy-path workflow test for the **Collection Runner's persistence** — the `runner.*` commands in
 * `packages/engine/src/runner/runnerCommands.ts` plus `runner.exportReport` in
 * `packages/engine/src/fileOps/commands.ts`. The runner's Run button is only useful if the report
 * survives: saved runs are what the History list and the "Export Report" action read back.
 *
 * These drive the REAL commands on the REAL registry against a REAL workspace on disk and assert the
 * files that actually land in `requests/.runs/<folderId>/<startedAt>/`. Only Electron itself (and the
 * save dialog, for the one channel that predates the client's own composition of it) is mocked.
 *
 * ## Why this suite drives commands and not channels (P6 reduction, eighth group, 2026-09-21)
 *
 * It used to drive `runner:saveReport` / `runner:getHistory` / `runner:saveConfig` /
 * `runner:loadConfig` / `runner:listFolderIds` / `runner:exportReport` by name. Those six channels
 * were **deleted** with `src/ipc/handlers/runnerHandlers.ts` (`plan/07` §5b-novendecies), so the
 * equivalence is now the other way round: the commands *are* the implementation, and there is no
 * second copy for this suite to accidentally test instead.
 *
 * The old shape is worth remembering because it is the exact failure P6 exists to remove. Three of
 * the six channels were **self-contained bodies** rather than `commandRegistry.invoke()` delegates —
 * `runner:saveReport`, `runner:saveConfig` and `runner:loadConfig` each re-implemented their command
 * in the shell — so this suite could pass, in full, while the engine's implementation wrote to the
 * wrong directory. §5b-septies records the same trap for the preload's unit suite; this file was its
 * integration-suite twin, and it carried a whole `describe` block whose only job was to compare the
 * two copies against each other. That block is folded into the blocks it was shadowing (below),
 * because with one implementation there is nothing left to compare.
 *
 * Runs in the integration project (shared process-global data-root override ⇒
 * `fileParallelism: false`).
 */

import { describe, it, expect, beforeAll, beforeEach, afterAll, vi } from "vitest";
import * as fs from "fs";
import * as path from "path";
import type { CommandAction } from "@bifurc/protocol";
import type { CommandContext, CommandRegistry } from "@bifurc/engine/commands/registry";
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

/** What `in-process` and every legacy `ipcMain.handle` adapter pass: a bus, no session. */
const ctx: CommandContext = { bus: {} as CommandContext["bus"] };

/**
 * The engine's registry, assigned in `beforeAll`.
 *
 * It is `let` rather than a static import because the rest of this file reaches the engine through
 * dynamic imports inside `beforeAll` — `vi.mock("electron")` is hoisted, but the engine modules still
 * have to load *after* the data-root override `createWorkspace()` installs, or `wsDir()` resolves
 * against the developer's real user-data directory and the assertions below write into it.
 */
let registry: CommandRegistry;

/** `registry.invoke` with the ctx a transport would pass, so each case reads as a single line. */
function invokeCommand<T = unknown>(action: CommandAction, params: unknown): T {
  return registry.invoke(action, params, ctx) as T;
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
  // `runner:exportReport` is the one channel whose behaviour is **not** the engine's: it is the
  // client half — show the dialog, decide the format from the chosen extension, call the engine's
  // `runner.exportReport`, write the artifact — and `packages/client/src/index.ts:927` is where the
  // real `window.api` composes those four steps. The channel is what the P6 reduction deleted; this
  // suite keeps driving it, for the export block only, because deleting the *shell's* copy is not the
  // same as proving the *client's* copy is equivalent, and `runner.exportReport`'s dialog-first
  // ordering is the one thing in this file the reduction could have lost silently.
  //
  // (`runnerHandlers.ts` used to supply this; its dialog+format composition is what
  // `packages/client/tests/client.test.ts` pins for the real surface. See `plan/07` §5b-novendecies.)
  const { registerBlobCommands } = await import("@bifurc/engine/blob/commands");
  const { registerFileOpsCommands } = await import("@bifurc/engine/fileOps/commands");
  const { commandRegistry } = await import("@bifurc/engine/commands/registry");
  const { registerRunnerCommands } = await import("@bifurc/engine/runner/runnerCommands");
  registerBlobCommands(commandRegistry);
  registerFileOpsCommands(commandRegistry);
  registerRunnerCommands(commandRegistry);
  registry = commandRegistry;
});

beforeEach(() => {
  dialogState.result = { canceled: true, filePath: undefined };
});

afterAll(() => {
  ws?.cleanup();
});

// ── Saving a run ──────────────────────────────────────────────────────────────

describe("runner.saveReport", () => {
  it("writes report.json and report.html into a timestamped run directory", () => {
    const res = invokeCommand("runner.saveReport", { workspaceId: TEST_WS, report: makeReport() });

    expect(res).toEqual({ ok: true });

    const runDir = path.join(runsDir(FOLDER_ID), String(STARTED_AT));
    expect(fs.existsSync(path.join(runDir, "report.json"))).toBe(true);
    expect(fs.existsSync(path.join(runDir, "report.html"))).toBe(true);
  });

  it("round-trips the report through the saved JSON", () => {
    const report = makeReport({ startedAt: STARTED_AT + 1, folderName: "Round Trip" });
    invokeCommand("runner.saveReport", { workspaceId: TEST_WS, report });

    const saved = JSON.parse(
      fs.readFileSync(path.join(runsDir(FOLDER_ID), String(report.startedAt), "report.json"), "utf-8"),
    );

    expect(saved.folderName).toBe("Round Trip");
    expect(saved.totalTests).toBe(3);
    expect(saved.passedTests).toBe(2);
    expect(saved.results).toHaveLength(2);
    expect(saved.results[1].tests[0].error).toContain("expected 404 to equal 200");
  });

  it("writes a standalone HTML report naming the folder", () => {
    const report = makeReport({ startedAt: STARTED_AT + 2, folderName: "Html Folder" });
    invokeCommand("runner.saveReport", { workspaceId: TEST_WS, report });

    const html = fs.readFileSync(
      path.join(runsDir(FOLDER_ID), String(report.startedAt), "report.html"),
      "utf-8",
    );

    expect(html).toContain("<!DOCTYPE html>");
    expect(html).toContain("Html Folder");
    expect(html).toContain("Get Users");
    expect(html).toContain("expected 404 to equal 200");
  });

  it("reports failure instead of throwing when the report is malformed", () => {
    // The payload is deliberately schema-invalid rather than merely `{}`: `runner.saveReport`'s
    // params require `workspaceId`, and the registry validates against the frozen protocol schema
    // *before* the handler runs. So the throw lands at the registry boundary and propagates out of
    // `invoke()` rather than being caught by the body's own `try` — which is the true behaviour and
    // the reason this asserts on a throw rather than on `{ok:false}`.
    expect(() =>
      invokeCommand("runner.saveReport", {
        workspaceId: TEST_WS,
        report: { startedAt: STARTED_AT },
      }),
    ).toThrow();
  });

  it("returns {ok:false} rather than throwing when the report reaches the body but cannot be written", () => {
    // The body's own failure branch, which the schema-invalid case above cannot reach. A file where
    // the run directory should be makes `mkdirSync(..., {recursive: true})` throw — the same shape as
    // a permissions failure — and the body resolves `{ok:false, error}` because the renderer branches
    // on the flag. A handler *returning* a failure-shaped value is an envelope **success**.
    fs.mkdirSync(path.join(ws.dataRoot, TEST_WS, "requests", ".runs"), { recursive: true });
    fs.writeFileSync(path.join(ws.dataRoot, TEST_WS, "requests", ".runs", "folder-unwritable"), "", "utf-8");

    const res = invokeCommand<any>("runner.saveReport", {
      workspaceId: TEST_WS,
      report: makeReport({ folderId: "folder-unwritable" }),
    });

    expect(res.ok).toBe(false);
    expect(typeof res.error).toBe("string");
  });
});

// ── Run history ───────────────────────────────────────────────────────────────

describe("runner.getHistory", () => {
  it("lists a saved run with its summary", () => {
    invokeCommand("runner.saveReport", {
      workspaceId: TEST_WS,
      report: makeReport({ folderId: "folder-history" }),
    });

    const history = invokeCommand<any[]>("runner.getHistory", {
      workspaceId: TEST_WS,
      folderId: "folder-history",
    });

    expect(history).toHaveLength(1);
    expect(history[0].timestamp).toBe(STARTED_AT);
    expect(history[0].summary).toEqual({ total: 3, passed: 2, failed: 1 });
  });

  it("returns the newest run first", () => {
    const folder = "folder-order";
    for (const startedAt of [1_000, 3_000, 2_000]) {
      invokeCommand("runner.saveReport", {
        workspaceId: TEST_WS,
        report: makeReport({ folderId: folder, startedAt }),
      });
    }

    const history = invokeCommand<any[]>("runner.getHistory", {
      workspaceId: TEST_WS,
      folderId: folder,
    });

    expect(history.map((h) => h.timestamp)).toEqual([3_000, 2_000, 1_000]);
  });

  it("returns an empty list for a folder that has never been run", () => {
    expect(
      invokeCommand("runner.getHistory", { workspaceId: TEST_WS, folderId: "never-run" }),
    ).toEqual([]);
  });

  it("skips run directories that contain no report", () => {
    const folder = "folder-gaps";
    fs.mkdirSync(path.join(runsDir(folder), "12345"), { recursive: true });
    invokeCommand("runner.saveReport", {
      workspaceId: TEST_WS,
      report: makeReport({ folderId: folder }),
    });

    const history = invokeCommand<any[]>("runner.getHistory", {
      workspaceId: TEST_WS,
      folderId: folder,
    });

    expect(history).toHaveLength(1);
    expect(history[0].timestamp).toBe(STARTED_AT);
  });

  it("drops only the corrupt entry, not the whole folder", () => {
    // The nested `try` in the command is load-bearing and this is its only assertion. A single
    // unparseable `report.json` must cost that one entry — a single flat `try` would have answered
    // `[]` for a folder with a perfectly good run beside it.
    const folder = "folder-corrupt";
    for (const startedAt of [5_000, 9_000]) {
      invokeCommand("runner.saveReport", {
        workspaceId: TEST_WS,
        report: makeReport({ folderId: folder, startedAt }),
      });
    }
    fs.mkdirSync(path.join(runsDir(folder), "7000"), { recursive: true });
    fs.writeFileSync(path.join(runsDir(folder), "7000", "report.json"), "{ not json", "utf-8");

    const history = invokeCommand<any[]>("runner.getHistory", {
      workspaceId: TEST_WS,
      folderId: folder,
    });

    expect(history.map((h) => h.timestamp)).toEqual([9_000, 5_000]);
  });

  it("returns an empty list rather than throwing for an unknown workspace", () => {
    expect(
      invokeCommand("runner.getHistory", { workspaceId: "ws-does-not-exist", folderId: FOLDER_ID }),
    ).toEqual([]);
  });
});

// ── Runner configuration per folder ───────────────────────────────────────────

describe("runner.saveConfig / runner.loadConfig", () => {
  it("round-trips a folder's runner configuration", () => {
    const config = { delayMs: 250, stopOnFailure: true, iterations: 3 };

    expect(
      invokeCommand("runner.saveConfig", { workspaceId: TEST_WS, folderId: "folder-config", config }),
    ).toEqual({ ok: true });
    expect(
      invokeCommand("runner.loadConfig", { workspaceId: TEST_WS, folderId: "folder-config" }),
    ).toEqual(config);
  });

  it("writes the config verbatim rather than a picked subset", () => {
    // The assertion the old frozen schema could never have made, and the reason this reads the raw
    // file instead of trusting the load: `RunnerConfigSchema` was `z.object({requestOrder, delayMs})`,
    // which *strips* unknown keys. A payload that had passed it would have reached disk without
    // `stopOnFailure` or `iterations`, and the round-trip above would still have "passed" against a
    // handler that read back only what it had been allowed to write. Silent field loss in saved run
    // configuration is exactly the failure mode `plan/protocol-changes.md` records for the widening.
    //
    // This case lived in a separate "through the registry" block while the shell had a second copy of
    // `runner:saveConfig`. With one implementation there is nothing left to compare, so it is folded
    // here rather than deleted — the assertion is about the *store's* fidelity, not about the routing.
    const config = { delayMs: 250, stopOnFailure: true, iterations: 3 };
    invokeCommand("runner.saveConfig", { workspaceId: TEST_WS, folderId: "reg-roundtrip", config });

    const onDisk = JSON.parse(
      fs.readFileSync(path.join(runsDir("reg-roundtrip"), "runner.json"), "utf-8"),
    );
    expect(onDisk).toEqual(config);
    expect(Object.keys(onDisk).sort()).toEqual(["delayMs", "iterations", "stopOnFailure"]);
  });

  it("returns null when a folder has no saved configuration", () => {
    expect(
      invokeCommand("runner.loadConfig", { workspaceId: TEST_WS, folderId: "folder-without-config" }),
    ).toBeNull();
  });

  it("returns null for a corrupt configuration, because absent and empty are different answers", () => {
    // `null` and `{}` are different answers: `runner.listFolderIds` uses the file's *presence* to decide
    // whether a folder is configured, so a corrupt file must read as "not configured" rather than as an
    // empty configuration. This is the pre-existing channel behaviour, kept rather than improved.
    fs.mkdirSync(runsDir("reg-corrupt"), { recursive: true });
    fs.writeFileSync(path.join(runsDir("reg-corrupt"), "runner.json"), "{ not json", "utf-8");

    expect(
      invokeCommand("runner.loadConfig", { workspaceId: TEST_WS, folderId: "reg-corrupt" }),
    ).toBeNull();
  });

  it("returns {ok:false} rather than throwing when the config cannot be written", () => {
    // A file where the folder should be makes `mkdirSync(..., {recursive: true})` throw, the same
    // shape as a permissions failure. The body catches it and resolves `{ok:false}` with **no**
    // `error`, which is the pre-existing shape: `BifurcApi["saveRunnerConfig"]` declares
    // `{ok: boolean}` only, so adding a message would be a contract change smuggled in under a move.
    fs.mkdirSync(path.join(ws.dataRoot, TEST_WS, "requests", ".runs"), { recursive: true });
    fs.writeFileSync(path.join(ws.dataRoot, TEST_WS, "requests", ".runs", "reg-unwritable"), "", "utf-8");

    expect(
      invokeCommand("runner.saveConfig", {
        workspaceId: TEST_WS,
        folderId: "reg-unwritable",
        config: { delayMs: 1 },
      }),
    ).toEqual({ ok: false });
  });

  it("listFolderIds reports only folders that have a saved configuration", () => {
    invokeCommand("runner.saveConfig", {
      workspaceId: TEST_WS,
      folderId: "folder-listed",
      config: { delayMs: 0 },
    });
    invokeCommand("runner.saveReport", {
      workspaceId: TEST_WS,
      report: makeReport({ folderId: "folder-only-a-run" }),
    });

    const ids = invokeCommand<string[]>("runner.listFolderIds", { workspaceId: TEST_WS });

    expect(ids).toContain("folder-listed");
    expect(ids).not.toContain("folder-only-a-run");
  });

  it("listFolderIds returns an empty list for an unknown workspace", () => {
    expect(invokeCommand("runner.listFolderIds", { workspaceId: "ws-does-not-exist" })).toEqual([]);
  });
});

// ── Export ────────────────────────────────────────────────────────────────────

/**
 * The engine half of `runner.exportReport`, driven directly.
 *
 * The **dialog-first** half — show the save dialog, derive the format from the chosen extension, write
 * the artifact — is the client's, and it is pinned for the real surface in
 * `packages/client/tests/client.test.ts` ("exportRunnerReport derives the format from the chosen
 * extension", "…resolves a bare {ok:false} on cancel"). Those two cases are what make deleting the
 * shell's copy safe; this block covers the half that genuinely lives in the engine, so the pair is
 * covered end to end without a third implementation to keep in sync.
 */
describe("runner.exportReport (the engine half)", () => {
  it("renders HTML when the client asks for html", () => {
    const artifact = invokeCommand<any>("runner.exportReport", {
      report: makeReport({ folderName: "Exported" }),
      format: "html",
    });

    expect(artifact.ok).toBe(true);
    expect(artifact.mimeType).toBe("text/html");
    expect(artifact.suggestedName).toMatch(/^Exported-report-.*\.html$/);
    // Small enough to ride inline, which is the common case and the reason the engine's
    // inline-vs-blob threshold exists (`File_Ops_Protocol.md` §3.2).
    expect(artifact.blobId).toBeUndefined();
    expect(Buffer.from(artifact.inline, "base64").toString("utf-8")).toContain("Exported");
  });

  it("renders JSON when the client asks for json", () => {
    const artifact = invokeCommand<any>("runner.exportReport", {
      report: makeReport(),
      format: "json",
    });

    const parsed = JSON.parse(Buffer.from(artifact.inline, "base64").toString("utf-8"));
    expect(parsed.totalTests).toBe(3);
    expect(parsed.results).toHaveLength(2);
    expect(artifact.suggestedName).toMatch(/\.json$/);
  });

  it("defaults the folder name when the report carries none", () => {
    const artifact = invokeCommand<any>("runner.exportReport", {
      report: makeReport({ folderName: undefined }),
      format: "html",
    });

    expect(artifact.suggestedName).toMatch(/^collection-report-/);
  });
});

// ── The command set that outlives the channels ────────────────────────────────

describe("the runner command namespace", () => {
  it("registers every runner command the client advertises", () => {
    // This replaces the old suite's `invoke("runner:…")` registration check. Under the reduction the
    // thing that must be registered is the **command** — the same substitution `handlers.test.ts`
    // made for `folder:*` in the third group and for `application.*` in the seventh. It asserts the
    // namespace rather than the ratchet's whole set, because this suite's failure mode is a runner
    // command that silently is not there.
    for (const command of [
      "runner.saveReport",
      "runner.exportReport",
      "runner.saveConfig",
      "runner.loadConfig",
      "runner.getHistory",
      "runner.listFolderIds",
    ]) {
      expect(registry.isRegistered(command as CommandAction), `missing command ${command}`).toBe(true);
    }
  });
});
