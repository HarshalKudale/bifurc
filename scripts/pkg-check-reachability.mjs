/**
 * Audit `dist/` for *stale* compiled output — files with no counterpart in `src/` — and
 * report whether any of them is reachable from the app's two entry points by relative
 * `require`.
 *
 * Why this matters: `tsc` never deletes output, so every module that moved into
 * `packages/engine` during P2 left a compiled copy behind in `dist/`. Those copies are
 * duplicates of engine modules — and the engine's `bundle: false` build exists precisely
 * because modules like `store/paths.ts` hold module-level state. If a *live* file required a
 * stale path, `setDataRoot()` made through one copy would not be seen by the other.
 *
 * Unreachable stale files are dead weight that ships; reachable ones are a correctness bug.
 * This distinguishes the two, so the claim is measured rather than assumed.
 *
 * `scripts/clean-main-dist.mjs` prevents the condition on every build; this is the
 * instrument that says whether it is back, and how bad it is if so. Measured 2026-09-20,
 * before the cleaner landed: 92 of 116 files stale (447 KB), 0 reachable.
 *
 * Run from the repo root: `node scripts/pkg-check-reachability.mjs`
 */
import fs from "node:fs";
import path from "node:path";

const DIST = path.resolve("dist");
const entries = ["main.js", "preload.js"];

const walk = (dir) =>
  fs.readdirSync(dir, { withFileTypes: true }).flatMap((e) => {
    const p = path.join(dir, e.name);
    if (e.isDirectory()) return e.name === "renderer" ? [] : walk(p);
    return e.name.endsWith(".js") && !e.name.endsWith(".map") ? [p] : [];
  });

const all = walk(DIST);
const isStale = (abs) => {
  const rel = path.relative(DIST, abs).replace(/\.js$/, "");
  return !fs.existsSync(path.join("src", `${rel}.ts`)) && !fs.existsSync(path.join("src", `${rel}.tsx`));
};

const staleSet = new Set(all.filter(isStale));
const requires = (file) => {
  const src = fs.readFileSync(file, "utf8");
  return [...src.matchAll(/require\((["'])(\.[^"']*)\1\)/g)].map((m) => m[2]);
};

const seen = new Set();
const queue = entries.map((e) => path.join(DIST, e));
const reachedStale = [];

while (queue.length) {
  const file = queue.pop();
  const abs = path.resolve(file.endsWith(".js") ? file : `${file}.js`);
  if (seen.has(abs) || !fs.existsSync(abs)) continue;
  seen.add(abs);
  if (staleSet.has(abs)) reachedStale.push(path.relative(DIST, abs));
  for (const spec of requires(abs)) {
    queue.push(path.resolve(path.dirname(abs), spec));
  }
}

const bytes = (set) => [...set].reduce((n, f) => n + fs.statSync(f).size, 0);

console.log(`dist .js files (excl. renderer):      ${all.length}`);
console.log(`  stale (no matching src/*.ts):       ${staleSet.size}   ${(bytes(staleSet) / 1024).toFixed(0)} KB`);
console.log(`  live:                               ${all.length - staleSet.size}`);
console.log(`reachable from main.js/preload.js:    ${seen.size}`);
console.log(`  of which stale:                     ${reachedStale.length}`);
if (reachedStale.length) {
  console.log("\nREACHABLE STALE FILES (correctness risk — duplicate module state):");
  for (const f of reachedStale) console.log(`  ${f}`);
} else {
  console.log("\nNo live file reaches a stale copy — the stale set is dead weight, not a live bug.");
}
