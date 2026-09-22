/**
 * Remove the app's *compiled* output before `tsc` re-emits it — everything under `dist/`
 * except `dist/renderer/` and `dist/web/`, which belong to Vite (`build:renderer`, `build:web`).
 *
 * ## Why this exists
 *
 * `tsc` never deletes its output. P2 moved ~50 modules out of `src/` into
 * `packages/engine`, and every one of them left a compiled copy behind in `dist/`.
 * Measured on 2026-09-20: **92 of 116** `dist/*.js` files had no counterpart in `src/`
 * — 447 KB of a second, frozen copy of the engine, shipped inside the packaged asar.
 *
 * They are unreachable today (verified by walking relative `require`s from `dist/main.js`
 * and `dist/preload.js`: 0 of the 92 are reached), so this is not a live bug. It is worse
 * than dead weight in two ways:
 *
 *  1. **They drift.** `dist/shutdown.js` — the pre-move copy of what is now
 *     `packages/engine/src/shutdown.ts` — requires `@bifurc/engine/companion/companionServer`,
 *     a module that no longer exists under any name (the companion server is now
 *     `packages/engine/src/transport/legacyCompanion.ts`). A packaged build ships a file
 *     whose imports cannot resolve.
 *  2. **They are the hazard `bundle: false` exists to prevent.** The engine does not bundle
 *     because `store/paths.ts` holds the data root as module-level state; a second copy of
 *     that module means a `setDataRoot()` one copy never sees. Nothing reaches the stale
 *     copies *today*, and a single relative `require` added tomorrow would.
 *
 * Deleting them is the fix; re-emitting is what `tsc` does next. This runs on every
 * `build:main`, so the condition cannot come back.
 *
 * ## Why the two Vite bundles are excluded
 *
 * Vite emits content-hashed bundles there with `emptyOutDir: true`, and it owns those
 * directories. They are *not* tsc output — their files have no `src/` counterpart, so any
 * "delete what has no source" heuristic would classify a whole bundle as stale and delete
 * the app's UI. `build:main` must not touch what `build:renderer` or `build:web` owns.
 *
 * `web` joined `renderer` here under D18, when the browser host became a second Vite build.
 */
import fs from "node:fs";
import path from "node:path";

const DIST = path.resolve("dist");
const KEEP = new Set(["renderer", "web"]);

if (!fs.existsSync(DIST)) {
  console.log("[clean:main] no dist/ yet — nothing to do");
  process.exit(0);
}

const sizeOf = (p) => {
  const st = fs.lstatSync(p);
  if (st.isDirectory()) {
    return fs.readdirSync(p).reduce((n, name) => n + sizeOf(path.join(p, name)), 0);
  }
  return st.size;
};

let removed = 0;
let bytes = 0;
for (const entry of fs.readdirSync(DIST)) {
  if (KEEP.has(entry)) continue;
  const target = path.join(DIST, entry);
  bytes += sizeOf(target);
  fs.rmSync(target, { recursive: true, force: true });
  removed++;
}

console.log(
  `[clean:main] removed ${removed} entr${removed === 1 ? "y" : "ies"} from dist/ ` +
    `(${(bytes / 1024).toFixed(0)} KB), kept: ${[...KEEP].join(", ")}`,
);
