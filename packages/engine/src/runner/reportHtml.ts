/**
 * The collection-run HTML report — rendered by the **engine**, written by the **client**.
 *
 * Moved out of `src/ipc/handlers/runnerHandlers.ts` in P3. `runner:exportReport` used to build this
 * HTML and write it to a user-chosen path in one step; `File_Ops_Protocol.md` §4 splits those, and
 * the rendering half belongs on the engine side of the boundary because it is a *generated
 * artifact*. The dialog and the `writeFileSync` stayed in the shell.
 *
 * The markup is unchanged from the shell version, character for character, because it is what the
 * user sees when they open an exported report. Two things were typed rather than left as `any`:
 * `report` is now the protocol's `RunReport`, and `completedAt` is optional there, so the duration
 * falls back to `startedAt` (0.00s) instead of rendering `NaN`.
 */
import type { RunnerExportReportParams } from "@bifurc/protocol";

type RunReport = RunnerExportReportParams["report"];
type RunResult = NonNullable<RunReport["results"]>[number];
type TestEntry = NonNullable<RunResult["tests"]>[number];

export function renderRunnerHtml(report: RunReport): string {
  const completedAt = report.completedAt ?? report.startedAt;
  const duration = ((completedAt - report.startedAt) / 1000).toFixed(2);
  const timestamp = new Date(report.startedAt).toISOString();
  const esc = (s: unknown) =>
    String(s).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");

  return `<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<title>Run Report - ${esc(report.folderName)}</title>
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
<h1>Collection Run: ${esc(report.folderName)}</h1>
<div class="meta">${timestamp} &bull; Duration: ${duration}s</div>
</div>
<div class="summary">
<div class="stat"><div class="value">${report.totalRequests}</div><div class="label">Requests</div></div>
<div class="stat"><div class="value passed">${report.passedTests}</div><div class="label">Passed</div></div>
<div class="stat"><div class="value failed">${report.failedTests}</div><div class="label">Failed</div></div>
<div class="stat"><div class="value">${duration}s</div><div class="label">Duration</div></div>
</div>
${(report.results ?? []).map((r: RunResult) => `
<div class="request">
<div class="req-header">
<span class="method">${esc(r.method)}</span>
<span class="name">${esc(r.requestName)}</span>
${r.status != null ? `<span class="status">${r.status}</span>` : ""}
<span class="time">${r.responseTime}ms</span>
</div>
${(r.tests?.length || r.error) ? `<div class="tests">
${r.error ? `<div class="test-item failed">✗ Error: ${esc(r.error)}</div>` : ""}
${(r.tests ?? []).map((t: TestEntry) => `<div class="test-item ${t.passed ? "passed" : "failed"}">${t.passed ? "✓" : "✗"} ${esc(t.name)}${t.error ? ` — ${esc(t.error)}` : ""}</div>`).join("")}
</div>` : ""}
</div>`).join("")}
</body>
</html>`;
}
