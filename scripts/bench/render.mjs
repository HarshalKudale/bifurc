// Draws docs/assets/bench.svg from docs/assets/bench.json: one horizontal bar chart per metric.
// Usage: node scripts/bench/render.mjs
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "../..");
const data = JSON.parse(readFileSync(join(root, "docs/assets/bench.json"), "utf8"));
const METRICS = [
  ["installMb", "Install size on disk", "MB"],
  ["idleMemoryMb", "Idle memory (private working set)", "MB"],
  ["coldStartMs", "Cold start to first window (median)", "ms"],
].filter(([key]) => data.results.some((r) => r[key] != null));

const W = 760, LABEL = 110, BAR_MAX = 520, ROW = 30, TOP = 34, GAP = 26;
let y = 12;
let body = "";
for (const [key, title, unit] of METRICS) {
  const rows = data.results.filter((r) => r[key] != null);
  const max = Math.max(...rows.map((r) => r[key]));
  body += `<text x="0" y="${y + 14}" class="t">${title}</text>`;
  y += TOP - 12;
  for (const r of rows) {
    const w = Math.max(2, Math.round((r[key] / max) * BAR_MAX));
    const us = r.name === "Bifurc";
    body += `<text x="${LABEL - 8}" y="${y + 17}" class="l" text-anchor="end">${r.name}</text>`;
    body += `<rect x="${LABEL}" y="${y + 4}" width="${w}" height="18" rx="3" class="${us ? "us" : "them"}"/>`;
    body += `<text x="${LABEL + w + 8}" y="${y + 17}" class="v">${r[key].toLocaleString("en-US")} ${unit}</text>`;
    y += ROW;
  }
  y += GAP;
}
const caption = `${data.os}, ${data.date}. Lower is better.`;
const H = y + 8;
const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="${W}" height="${H}" viewBox="0 0 ${W} ${H}" role="img" aria-label="Install size, idle memory and cold start of Bifurc compared with other apps">
<style>
  text { font: 13px system-ui, -apple-system, "Segoe UI", sans-serif; fill: #1f2328; }
  .t { font-weight: 600; } .v { fill: #57606a; } .us { fill: #4f46e5; } .them { fill: #8c959f; }
  .c { font-size: 11px; fill: #57606a; }
  @media (prefers-color-scheme: dark) { text { fill: #e6edf3; } .v, .c { fill: #9198a1; } .us { fill: #818cf8; } .them { fill: #6e7681; } }
</style>
${body}<text x="0" y="${H - 6}" class="c">${caption}</text>
</svg>
`;
writeFileSync(join(root, "docs/assets/bench.svg"), svg);
console.log("Wrote docs/assets/bench.svg");
