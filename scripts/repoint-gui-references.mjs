/**
 * Repoint stale `renderer/...` references at their new home under `packages/gui/src/`.
 *
 * Written for the D18 extraction, following the procedure in the `bifurc-engine-layer-move` skill:
 * after moving a layer, every reference to it must follow — imports *and* comments, because a doc
 * comment naming a path that no longer exists is a navigation break for the next reader.
 *
 * ## What it deliberately does NOT touch
 *
 * - `renderer/index.html` and `renderer/main.tsx` — they **stayed**; they are the Electron host's entry.
 * - `dist/renderer/...` — the build *output* path, which is unchanged by design (five scripts and
 *   `src/main.ts` depend on it).
 * - `renderer/types/window.ts` etc. are matched by prefix, so the two exceptions above are safe only
 *   because they are not under `types/`, `lib/`, `panels/`, `components/` or `hooks/`.
 *
 * Run with `--dry` first. Re-running after applying must report 0 rewrites — that idempotency check is
 * what proves the rewrite is complete rather than merely quiet.
 */
import fs from "node:fs";
import path from "node:path";

const ROOT = process.cwd();
const DRY = process.argv.includes("--dry");

/** Only prefixes that actually moved. Order matters only for reporting. */
const PATTERNS = [
  ["renderer/types/", "packages/gui/src/types/"],
  ["renderer/lib/", "packages/gui/src/lib/"],
  ["renderer/panels/", "packages/gui/src/panels/"],
  ["renderer/components/", "packages/gui/src/components/"],
  ["renderer/hooks/", "packages/gui/src/hooks/"],
  ["renderer/App.tsx", "packages/gui/src/App.tsx"],
];

const SKIP_DIRS = new Set([
  "node_modules", "dist", "release", "coverage", "coverage-baseline", "playwright-report",
  "test-results", ".git",
]);
const EXTS = new Set([".ts", ".tsx", ".md", ".json", ".mjs", ".py"]);
const isSkipped = (name) => SKIP_DIRS.has(name) || name.startsWith("coverage-run");

/**
 * Files whose `renderer/...` references must **not** be rewritten, and why.
 *
 * - **The dated daily logs.** They are a chronological record, and `renderer/types/window.ts` was the
 *   correct path on the day each was written. Rewriting them would make the log claim the move had
 *   already happened — the same class of error as a narrative written from a working tree and read as
 *   history. `MEMORY.md` and `TRAPS.md` are *live* pointers and are rewritten; the dated files are not.
 * - **This script**, which holds the patterns as string literals.
 */
const SKIP_FILES = [
  /\.workbuddy-ai[\\/]memory[\\/]\d{4}-\d{2}-\d{2}\.md$/,
  /scripts[\\/]repoint-gui-references\.mjs$/,
];
const isSkippedFile = (p) => SKIP_FILES.some((re) => re.test(p));

const counts = new Map();
const perFile = new Map();
let scanned = 0;

function walk(dir) {
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name);
    if (e.isDirectory()) {
      if (!isSkipped(e.name)) walk(p);
      continue;
    }
    if (!EXTS.has(path.extname(e.name))) continue;
    if (isSkippedFile(p)) continue;
    scanned++;

    const before = fs.readFileSync(p, "utf8");
    let after = before;
    const hits = [];
    for (const [from, to] of PATTERNS) {
      // Split on the pattern to count occurrences without regex-escaping surprises.
      const n = after.split(from).length - 1;
      if (n === 0) continue;
      counts.set(from, (counts.get(from) ?? 0) + n);
      hits.push(`${from} x${n}`);
      after = after.split(from).join(to);
    }
    if (after === before) continue;

    perFile.set(path.relative(ROOT, p).replace(/\\/g, "/"), hits);
    if (!DRY) fs.writeFileSync(p, after);
  }
}

walk(ROOT);

console.log(`${DRY ? "DRY RUN — " : ""}scanned ${scanned} file(s)\n`);
for (const [from, n] of [...counts].sort((a, b) => b[1] - a[1])) {
  console.log(`  ${String(n).padStart(3)}  ${from} -> ${PATTERNS.find((x) => x[0] === from)[1]}`);
}
console.log(`\n  ${perFile.size} file(s) affected, ${[...counts.values()].reduce((a, b) => a + b, 0)} reference(s)`);
if (perFile.size) {
  console.log("\n  files:");
  for (const [f, hits] of perFile) console.log(`    ${f}  [${hits.join(", ")}]`);
}
