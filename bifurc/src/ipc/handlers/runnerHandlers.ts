import { ipcMain, dialog } from "electron";
import * as fs from "fs";
import * as path from "path";
import { wsDir as workspaceDir } from "@/store/workspaceFs";

export function registerRunnerHandlers() {
  ipcMain.handle("runner:saveReport", (_e, wsId: string, report: any) => {
    try {
      const folderId = report.folderId as string;
      const ts = report.startedAt as number;
      const runDir = path.join(workspaceDir(wsId), "requests", ".runs", folderId, String(ts));
      fs.mkdirSync(runDir, { recursive: true });
      fs.writeFileSync(path.join(runDir, "report.json"), JSON.stringify(report, null, 2), "utf-8");

      // Generate HTML report
      const html = generateRunnerHtml(report);
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
      const { filePath, canceled } = await dialog.showSaveDialog({
        title: "Export Runner Report",
        defaultPath: `${folderName}-report-${ts}.html`,
        filters: [
          { name: "HTML Report", extensions: ["html"] },
          { name: "JSON Report", extensions: ["json"] },
        ],
      });
      if (canceled || !filePath) return { ok: false };
      const content = filePath.endsWith(".json")
        ? JSON.stringify(report, null, 2)
        : generateRunnerHtml(report);
      fs.writeFileSync(filePath, content, "utf-8");
      return { ok: true, filePath };
    } catch (err: any) {
      return { ok: false, error: err?.message };
    }
  });

  ipcMain.handle("runner:getHistory", (_e, wsId: string, folderId: string) => {
    try {
      const runsDir = path.join(workspaceDir(wsId), "requests", ".runs", folderId);
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

  ipcMain.handle("runner:listFolderIds", (_e, wsId: string) => {
    try {
      const runsDir = path.join(workspaceDir(wsId), "requests", ".runs");
      if (!fs.existsSync(runsDir)) return [];
      return fs.readdirSync(runsDir, { withFileTypes: true })
        .filter((e) => e.isDirectory() && fs.existsSync(path.join(runsDir, e.name, "runner.json")))
        .map((e) => e.name);
    } catch {
      return [];
    }
  });
}

function generateRunnerHtml(report: any): string {
  const duration = ((report.completedAt - report.startedAt) / 1000).toFixed(2);
  const timestamp = new Date(report.startedAt).toISOString();
  const esc = (s: string) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");

  return `<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<title>Run Report - \${esc(report.folderName)}</title>
<style>
* { margin: 0; padding: 0; box-sizing: border-box; }
body { font-family: system-ui, sans-serif; background: #1a1a2e; color: #e0e0e0; padding: 24px; }
.header { margin-bottom: 24px; padding-bottom: 16px; border-bottom: 1px solid #333; }
.header h1 { font-size: 20px; color: #fff; margin-bottom: 8px; }
.meta { font-size: 12px; color: #888; }
.summary { display: flex; gap: 24px; margin-bottom: 24px; padding: 16px; background: #222; border-radius: 8px; }
.stat { text-align: center; }
.stat .value { font-size: 24px; font-weight: bold; }
.stat .label { font-size: 11px; color: #888; text-transform: uppercase; }
.passed { color: #4caf50; }
.failed { color: #f44336; }
.request { margin-bottom: 12px; border: 1px solid #333; border-radius: 6px; overflow: hidden; }
.req-header { display: flex; align-items: center; gap: 8px; padding: 10px 12px; background: #252535; }
.method { font-size: 11px; font-weight: bold; color: #64b5f6; font-family: monospace; }
.name { font-size: 13px; flex: 1; }
.status { font-weight: bold; font-family: monospace; }
.time { font-size: 11px; color: #888; }
.tests { padding: 8px 12px; }
.test-item { display: flex; gap: 8px; padding: 4px 0; font-size: 12px; font-family: monospace; }
</style>
</head>
<body>
<div class="header">
<h1>Collection Run: \${esc(report.folderName)}</h1>
<div class="meta">\${timestamp} &bull; Duration: \${duration}s</div>
</div>
<div class="summary">
<div class="stat"><div class="value">\${report.totalRequests}</div><div class="label">Requests</div></div>
<div class="stat"><div class="value passed">\${report.passedTests}</div><div class="label">Passed</div></div>
<div class="stat"><div class="value failed">\${report.failedTests}</div><div class="label">Failed</div></div>
<div class="stat"><div class="value">\${duration}s</div><div class="label">Duration</div></div>
</div>
\${(report.results ?? []).map((r: any, i: number) => \`
<div class="request">
<div class="req-header">
<span class="method">\${r.method}</span>
<span class="name">\${esc(r.requestName)}</span>
\${r.status != null ? \`<span class="status">\${r.status}</span>\` : ""}
<span class="time">\${r.responseTime}ms</span>
</div>
\${(r.tests?.length || r.error) ? \`<div class="tests">
\${r.error ? \`<div class="test-item failed">✗ Error: \${esc(r.error)}</div>\` : ""}
\${(r.tests ?? []).map((t: any) => \`<div class="test-item \${t.passed ? "passed" : "failed"}">\${t.passed ? "✓" : "✗"} \${esc(t.name)}\${t.error ? \` — \${esc(t.error)}\` : ""}</div>\`).join("")}
</div>\` : ""}
</div>\`).join("")}
</body>
</html>`;
}
