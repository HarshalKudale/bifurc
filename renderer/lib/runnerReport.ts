/**
 * Runner report persistence - saves run reports as JSON + HTML inside the
 * collection folder's .runs/ directory with timestamped sub-folders.
 */

import { CollectionRunReport } from "@/lib/collectionRunner";

/**
 * Save a runner report to the filesystem via IPC.
 */
export async function saveRunnerReport(wsId: string, report: CollectionRunReport): Promise<{ ok: boolean; error?: string }> {
    return window.api.saveRunnerReport(wsId, report);
}

/**
 * Get saved run history for a folder.
 */
export async function getRunHistory(wsId: string, folderId: string): Promise<{ timestamp: number; summary: { total: number; passed: number; failed: number } }[]> {
    return window.api.getRunHistory(wsId, folderId);
}

/**
 * Generate a standalone HTML report from a run report.
 */
export function generateHtmlReport(report: CollectionRunReport): string {
    const duration = ((report.completedAt - report.startedAt) / 1000).toFixed(2);
    const timestamp = new Date(report.startedAt).toISOString();

    return `<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>Collection Run Report - ${escapeHtml(report.folderName)}</title>
<style>
  * { margin: 0; padding: 0; box-sizing: border-box; }
  body { font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; background: #1a1a2e; color: #e0e0e0; padding: 24px; }
  .header { margin-bottom: 24px; padding-bottom: 16px; border-bottom: 1px solid #333; }
  .header h1 { font-size: 20px; color: #fff; margin-bottom: 8px; }
  .header .meta { font-size: 12px; color: #888; }
  .summary { display: flex; gap: 24px; margin-bottom: 24px; padding: 16px; background: #222; border-radius: 8px; }
  .summary .stat { text-align: center; }
  .summary .stat .value { font-size: 24px; font-weight: bold; }
  .summary .stat .label { font-size: 11px; color: #888; text-transform: uppercase; }
  .passed { color: #4caf50; }
  .failed { color: #f44336; }
  .request { margin-bottom: 12px; border: 1px solid #333; border-radius: 6px; overflow: hidden; }
  .request .req-header { display: flex; align-items: center; gap: 8px; padding: 10px 12px; background: #252535; }
  .request .method { font-size: 11px; font-weight: bold; color: #64b5f6; font-family: monospace; }
  .request .name { font-size: 13px; flex: 1; }
  .request .status { font-weight: bold; font-family: monospace; }
  .request .time { font-size: 11px; color: #888; }
  .tests { padding: 8px 12px; }
  .test-item { display: flex; gap: 8px; padding: 4px 0; font-size: 12px; font-family: monospace; }
  .test-item .icon { flex-shrink: 0; }
  .test-item .error { color: #f44336; font-size: 11px; margin-top: 2px; }
  .status-2xx { color: #4caf50; }
  .status-3xx { color: #ff9800; }
  .status-4xx { color: #f44336; }
  .status-5xx { color: #f44336; }
</style>
</head>
<body>
<div class="header">
  <h1>Collection Run: ${escapeHtml(report.folderName)}</h1>
  <div class="meta">${timestamp} &bull; Duration: ${duration}s</div>
</div>
<div class="summary">
  <div class="stat"><div class="value">${report.totalRequests}</div><div class="label">Requests</div></div>
  <div class="stat"><div class="value passed">${report.passedTests}</div><div class="label">Tests Passed</div></div>
  <div class="stat"><div class="value failed">${report.failedTests}</div><div class="label">Tests Failed</div></div>
  <div class="stat"><div class="value">${duration}s</div><div class="label">Duration</div></div>
</div>
${report.results.map((r, i) => `
<div class="request">
  <div class="req-header">
    <span class="method">${r.method}</span>
    <span class="name">${escapeHtml(r.requestName)}</span>
    ${r.status !== null ? `<span class="status status-${Math.floor(r.status / 100)}xx">${r.status}</span>` : ""}
    <span class="time">${r.responseTime}ms</span>
  </div>
  ${r.tests.length > 0 || r.error ? `<div class="tests">
    ${r.error ? `<div class="test-item failed"><span class="icon">✗</span><span>Error: ${escapeHtml(r.error)}</span></div>` : ""}
    ${r.tests.map((t) => `<div class="test-item ${t.passed ? "passed" : "failed"}"><span class="icon">${t.passed ? "✓" : "✗"}</span><span>${escapeHtml(t.name)}${t.error ? `<div class="error">${escapeHtml(t.error)}</div>` : ""}</span></div>`).join("")}
  </div>` : ""}
</div>`).join("")}
</body>
</html>`;
}

function escapeHtml(s: string): string {
    return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
}
