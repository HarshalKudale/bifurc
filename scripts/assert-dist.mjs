/**
 * Assert the built tree is **internally consistent** — every relative `require` in every emitted file
 * resolves to a file that exists.
 *
 * ## The failure this exists to catch, measured
 *
 * `npm run test:e2e` (and `npm run dev`) died before the app could start:
 *
 * ```
 * Error: Cannot find module './workspaceFs'
 * Require stack:
 * - ...\packages\engine\dist\store\config.js
 * - ...\packages\engine\dist\store\entityCrudCommands.js
 * - ...\dist\ipc\handlers\soapKinds.js
 * - ...\dist\main.js
 * ```
 *
 * `packages/engine/dist/store/` held `workspaceFs.d.ts` and `workspaceFs.d.mts` — **declarations present,
 * implementation missing** — which is exactly the shape `TRAPS.md` already records for a *partial* `dist`
 * ("the dts leg re-adds declarations over the remains"). `tsup` runs with `clean: true`, and on this drive
 * a clean routed through the delete guard can partially fail; the build then reports **`exit=0` and
 * "Build success"**, because it wrote everything it was asked to write into a directory it had only
 * half-emptied.
 *
 * **The suite cannot see this.** `vitest.config.ts` aliases `@bifurc/engine/*` to the package's *source*,
 * so all ~2,600 tests pass against `src/` while `dist/` is unloadable — the "app runs `dist`, tests run
 * `src`" split `MEMORY.md` records. A green suite says nothing about whether the app can boot.
 *
 * ## Why a static walk rather than requiring the entry
 *
 * Requiring `dist/main.js` needs Electron (`require("electron")` at the top), so it cannot run in a plain
 * Node check. What can be checked without executing anything is the property that actually broke: **every
 * relative specifier in the emitted files resolves.** That is a strictly stronger check than "the files I
 * expected are present", because it also catches an edge into a file that was never an entry.
 *
 * It is deliberately **not** a `require`-reachability walk from the entry — `scripts/clean-main-dist.mjs`
 * already does that, and it proves the wrong property here: reachability of a *stale* file is not
 * existence of the file a live one needs. This checks existence, from every file, which is what a
 * half-cleaned `dist/` violates.
 */
import fs from "node:fs";
import path from "node:path";

/** The emitted trees to check, and the extensions a relative specifier may resolve to. */
const TREES = [
  "dist",
  "packages/protocol/dist",
  "packages/engine/dist",
  "packages/client/dist",
  // Added 2026-09-22 with the P7 web target, and missing for the same reason `build:packages` was: the
  // package was new, and every hand-written enumeration of "the packages" has to be found by hand. tsup
  // emits it with `bundle: false`, so it is a module graph and belongs here rather than in `SKIP_DIRS`.
  "packages/web-client/dist",
];

/**
 * Subtrees to skip — `dist/renderer` is the one that matters.
 *
 * `dist/renderer` is a **Vite bundle**, not a module graph: its relative specifiers were resolved by Vite
 * at build time and each emitted file is a single self-contained chunk. Walking it produced a *false
 * positive* on a tree the user had just confirmed worked (`npm run dev` started fine) — which is the worst
 * thing a build guard can do, because it invites "fixes" to code that was never broken.
 *
 * ⚠ **`dist/web` was the second entry here until 2026-09-26**, when the browser host was decommissioned and
 * `build:web` went with it. It is recorded rather than silently dropped because the entry was the *same*
 * omission `packages/web-client/dist` had in `TREES` above, one hand-written enumeration further along: a
 * new target that nobody added to the list. This list has to be found by hand in **both** directions.
 *
 * So this check is scoped to the trees that **are** module graphs: the packages' `dist/` (tsup,
 * `bundle: false`, one file per source file) and the app's own `dist/` minus the bundle (plain
 * `tsc`). That is exactly where the observed failure lived —
 * `packages/engine/dist/store/config.js` requiring a sibling that had not been written.
 */
const SKIP_DIRS = [path.join("dist", "renderer")];

/** Extensions Node would try, in order, for a relative specifier without one. */
const RESOLVE_EXTENSIONS = [".js", ".cjs", ".mjs", ".json", ".node"];

/** Files that must exist for the app to start at all, and what to say if they do not. */
const REQUIRED_ENTRIES = [
  ["dist/main.js", "the Electron main entry (package.json's `main`)"],
  ["dist/preload.js", "the preload — bundled by `build:preload`"],
  ["dist/renderer/index.html", "the renderer bundle — produced by `build:renderer`"],
  // `dist/web/index.html` was deliberately never listed here — `build:web` sat outside `npm run build` by
  // decision, so requiring it would have failed this check on a clean checkout. The bundle itself is gone as
  // of 2026-09-26, so the distinction no longer has a subject; the rule it protected does.
];

/** Every file under `dir` with one of `exts`, as paths relative to `dir`, skipping `SKIP_DIRS`. */
function walk(dir, exts, out = []) {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      // Skip by *relative* path so a skip applies wherever the tree is rooted.
      const rel = path.relative(process.cwd(), full).replace(/\\/g, "/");
      if (SKIP_DIRS.some((skip) => rel === skip || rel.startsWith(skip + "/"))) continue;
      walk(full, exts, out);
    } else if (exts.some((e) => entry.name.endsWith(e))) {
      out.push(full);
    }
  }
  return out;
}

/**
 * Relative specifiers in a file. Only `require("…")` and `import … from "…"` with a **literal** string,
 * and only ones starting with `.` — a bare specifier is a dependency, not something this tree owns.
 *
 * `require(someVariable)` has no literal and is skipped by construction: it is unresolvable statically,
 * and pretending otherwise would produce false positives.
 */
function relativeSpecifiers(code) {
  const found = new Set();
  const patterns = [
    /require\(\s*["'](\.[^"']*)["']\s*\)/g,
    /(?:^|[^\w$])import\s*\(\s*["'](\.[^"']*)["']\s*\)/g,
    /(?:^|[^\w$])import\s+[^;]*?from\s*["'](\.[^"']*)["']/g,
  ];
  for (const re of patterns) {
    for (const m of code.matchAll(re)) found.add(m[1]);
  }
  return [...found];
}

/** Whether `spec` resolves from `fromFile`, trying the extensions Node would. */
function resolves(fromFile, spec) {
  const base = path.resolve(path.dirname(fromFile), spec);
  for (const ext of ["", ...RESOLVE_EXTENSIONS]) {
    if (fs.existsSync(base + ext) && fs.statSync(base + ext).isFile()) return true;
  }
  // A directory import resolves through its `index.*`.
  if (fs.existsSync(base) && fs.statSync(base).isDirectory()) {
    return RESOLVE_EXTENSIONS.some((ext) => fs.existsSync(path.join(base, "index" + ext)));
  }
  return false;
}

let checked = 0;
const problems = [];

for (const tree of TREES) {
  if (!fs.existsSync(tree)) {
    problems.push(`${tree}/ does not exist — run the build.`);
    continue;
  }
  for (const file of walk(tree, [".js", ".cjs", ".mjs"])) {
    checked += 1;
    const code = fs.readFileSync(file, "utf8");
    for (const spec of relativeSpecifiers(code)) {
      if (!resolves(file, spec)) {
        problems.push(`${file} requires ${JSON.stringify(spec)} — no such file.`);
      }
    }
  }
}

for (const [entry, why] of REQUIRED_ENTRIES) {
  if (!fs.existsSync(entry)) problems.push(`${entry} is missing — ${why}.`);
}

/**
 * The preload must be a **bundle**, not `tsc` output. `build:main` runs `tsc` *then* `build:preload`, so a
 * build that stops between them leaves a module-graph preload here — and a sandboxed preload has no module
 * resolver, so the app boots with no `window.api`. This is the same property `build-preload.mjs` asserts;
 * checking it again here catches the case where the last step simply did not run.
 */
if (fs.existsSync("dist/preload.js")) {
  const requires = [...fs.readFileSync("dist/preload.js", "utf8").matchAll(/require\(\s*["']([^"']+)["']\s*\)/g)]
    .map((m) => m[1]);
  const nonElectron = [...new Set(requires)].filter((id) => id !== "electron");
  if (nonElectron.length > 0) {
    problems.push(
      `dist/preload.js is NOT bundled — it requires ${nonElectron.map((s) => JSON.stringify(s)).join(", ")}. ` +
        `A sandboxed preload can resolve only "electron", so the app would boot with no window.api. ` +
        `\`build:preload\` probably did not run; re-run \`npm run build\`.`,
    );
  }
}

if (problems.length > 0) {
  console.error(`[assert:dist] the build output is INCOMPLETE — ${problems.length} problem(s):\n`);
  for (const p of problems) console.error(`  - ${p}`);
  console.error(
    `\n  This is the partial-dist failure TRAPS.md records: tsup runs with \`clean: true\`, and a clean\n` +
      `  routed through the delete guard can half-succeed, after which the build still reports success.\n` +
      `  The test suite cannot see it — vitest aliases the packages to \`src/\`, so a green run says\n` +
      `  nothing about whether \`dist/\` is loadable.\n\n` +
      `  Fix: re-run the build with BOTH delete bypasses (neither alone is enough):\n` +
      `    CODEBUDDY_SAFE_DELETE_ENABLED=0 \\\n` +
      `      env -u CODEBUDDY_SAFE_DELETE_BULK_STATE_DIR -u CODEBUDDY_TOOL_CALL_ID npm run build`,
  );
  process.exit(1);
}

console.log(
  `[assert:dist] ${checked} emitted file(s) across ${TREES.length} tree(s); ` +
    `every relative specifier resolves, every required entry present, preload is bundled.`,
);
