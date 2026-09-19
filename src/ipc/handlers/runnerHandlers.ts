import { ipcMain, dialog } from "electron";
import * as fs from "fs";
import * as path from "path";
import type {
  RunnerExportReportResult,
  RunnerGetHistoryParams,
  RunnerListFolderIdsParams,
} from "@bifurc/protocol";
import { wsDir as workspaceDir } from "@bifurc/engine/store/workspaceFs";
import { renderRunnerHtml } from "@bifurc/engine/runner/reportHtml";
import { commandRegistry } from "@bifurc/engine/commands/registry";
import { bus } from "@bifurc/engine/eventBus";
import { call, writeArtifact } from "@/ipc/fileOpsClient";

// ## What is left here, and why
//
// Two different things live in this file now.
//
// **`runner.getHistory` / `runner.listFolderIds` are registered on the registry at *module scope***
// (below, outside `registerRunnerHandlers()`). That is no longer a convenience — it is the reason they
// cannot move into `packages/engine`. `plan/07` §5c records the trap: a module-scope registration takes
// effect on *import*, so relocating one behind an explicit `registerRunnerCommands()` call silently
// unregisters it for every suite that imports this handler group without running
// `registerIpcHandlers()`. Moving them means updating those suites in the same pass, not a mechanical
// cut-and-paste.
//
// **The `ipcMain.handle` channels are the legacy path.** P2 work item 7 converted only the two above;
// the rest moved in P6 once the protocol allowed it:
//   - `runner:saveReport` → `runner.saveReport` in `packages/engine/src/runner/runnerCommands.ts`
//     (step 3b-2). It writes into the *workspace* rather than to a path the user chose, so it is not
//     one of `File_Ops_Protocol.md` §4's egress channels.
//   - `runner:exportReport` is a thin client: it shows the dialog, then calls the engine's
//     `runner.exportReport` and writes the artifact it gets back.
//   - `runner:saveConfig` / `runner:loadConfig` → the engine's `runner.saveConfig` /
//     `runner.loadConfig` in **step 3d**, after `RunnerConfigSchema` was widened to `z.record`
//     (`plan/protocol-changes.md`). These two were tried and reverted in P2 for exactly that reason —
//     the frozen schema required `{requestOrder, delayMs}` while real callers save
//     `{delayMs, stopOnFailure, iterations}` — so the fix was to correct the schema, not the handler.
//   - `runner:getHistory` / `runner:listFolderIds` channels are thin adapters over the two
//     module-scope registrations above.
//
// The channel bodies stay until `registerIpcHandlers()` is deleted. Note that they are
// **self-contained duplicates** rather than delegating to the registry, which is why an integration
// suite driving channels by name keeps passing while the engine half is wrong. That is precisely what
// the "through the registry" block in `tests/integration/runnerStorage.integration.test.ts` exists to
// catch.
const ctx = { bus };

commandRegistry.register("runner.getHistory", ({ workspaceId, folderId }: RunnerGetHistoryParams) => {
  try {
    const runsDir = path.join(workspaceDir(workspaceId), "requests", ".runs", folderId);
    if (!fs.existsSync(runsDir)) return [];
    const entries = fs.readdirSync(runsDir).filter((d) => {
      return fs.statSync(path.join(runsDir, d)).isDirectory();
    });
    return entries.map((ts) => {
      const reportFile = path.join(runsDir, ts, "report.json");
      if (!fs.existsSync(reportFile)) return null;
      try {
        const data = JSON.parse(fs.readFileSync(reportFile, "utf-8"));
        return {
          timestamp: Number(ts),
          summary: {
            total: data.totalTests ?? 0,
            passed: data.passedTests ?? 0,
            failed: data.failedTests ?? 0,
          },
        };
      } catch { return null; }
    }).filter(Boolean).sort((a: any, b: any) => b.timestamp - a.timestamp);
  } catch {
    return [];
  }
});

commandRegistry.register("runner.listFolderIds", ({ workspaceId }: RunnerListFolderIdsParams) => {
  try {
    const runsDir = path.join(workspaceDir(workspaceId), "requests", ".runs");
    if (!fs.existsSync(runsDir)) return [];
    return fs.readdirSync(runsDir, { withFileTypes: true })
      .filter((e) => e.isDirectory() && fs.existsSync(path.join(runsDir, e.name, "runner.json")))
      .map((e) => e.name);
  } catch {
    return [];
  }
});

export function registerRunnerHandlers() {
  ipcMain.handle("runner:saveReport", (_e, wsId: string, report: any) => {
    try {
      const folderId = report.folderId as string;
      const ts = report.startedAt as number;
      const runDir = path.join(workspaceDir(wsId), "requests", ".runs", folderId, String(ts));
      fs.mkdirSync(runDir, { recursive: true });
      fs.writeFileSync(path.join(runDir, "report.json"), JSON.stringify(report, null, 2), "utf-8");

      // Generate HTML report — rendered by the engine (`@bifurc/engine/runner/reportHtml`), because
      // it is a generated artifact and `runner:exportReport` needs the same renderer.
      const html = renderRunnerHtml(report);
      fs.writeFileSync(path.join(runDir, "report.html"), html, "utf-8");
      return { ok: true };
    } catch (err: any) {
      return { ok: false, error: err?.message ?? "Failed to save report" };
    }
  });

  ipcMain.handle("runner:exportReport", async (_e, report: any) => {
    try {
      const folderName = (report.folderName as string) ?? "collection";
      const ts = new Date(report.startedAt as number).toISOString().replace(/[:.]/g, "-");

      // Dialog FIRST, then the engine renders — `File_Ops_Protocol.md` §3.2. The default path is
      // still built here from the report, because the renderer sends the report and the dialog has
      // to be shown before the command runs.
      const { filePath, canceled } = await dialog.showSaveDialog({
        title: "Export Runner Report",
        defaultPath: `${folderName}-report-${ts}.html`,
        filters: [
          { name: "HTML Report", extensions: ["html"] },
          { name: "JSON Report", extensions: ["json"] },
        ],
      });
      if (canceled || !filePath) return { ok: false };

      // The format is the client's to decide — only it knows what the user typed. The pre-P3
      // handler tested `filePath.endsWith(".json")` and rendered accordingly; the same test now
      // happens here and the answer travels with the request.
      const format = filePath.endsWith(".json") ? "json" : "html";

      const artifact = await call<RunnerExportReportResult>("runner.exportReport", { report, format });
      if (!artifact.ok) return { ok: false, error: artifact.error };

      try {
        writeArtifact(artifact, filePath);
      } catch (err: any) {
        return { ok: false, error: err?.message };
      }
      return { ok: true, filePath };
    } catch (err: any) {
      return { ok: false, error: err?.message };
    }
  });

  ipcMain.handle("runner:getHistory", (_e, workspaceId: string, folderId: string) =>
    commandRegistry.invoke("runner.getHistory", { workspaceId, folderId }, ctx));

  ipcMain.handle("runner:saveConfig", (_e, wsId: string, folderId: string, config: any) => {
    try {
      const runsDir = path.join(workspaceDir(wsId), "requests", ".runs", folderId);
      fs.mkdirSync(runsDir, { recursive: true });
      fs.writeFileSync(path.join(runsDir, "runner.json"), JSON.stringify(config, null, 2), "utf-8");
      return { ok: true };
    } catch {
      return { ok: false };
    }
  });

  ipcMain.handle("runner:loadConfig", (_e, wsId: string, folderId: string) => {
    try {
      const file = path.join(workspaceDir(wsId), "requests", ".runs", folderId, "runner.json");
      if (!fs.existsSync(file)) return null;
      return JSON.parse(fs.readFileSync(file, "utf-8"));
    } catch {
      return null;
    }
  });

  ipcMain.handle("runner:listFolderIds", (_e, workspaceId: string) =>
    commandRegistry.invoke("runner.listFolderIds", { workspaceId }, ctx));
}
