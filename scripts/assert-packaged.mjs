#!/usr/bin/env node
/**
 * Assert that a *packaged* Bifurc bundle can actually resolve what its compiled output
 * requires. Run after `electron-builder`, before the artifact is published.
 *
 * ## Why this is a script and not a unit test
 *
 * The property is about the **artifact**, not about `src/`. The unit suite runs against
 * sources; a bundle can be unresolvable while every test is green — that is exactly how the
 * icon regression shipped (see `plan/12`): `nativeImage.createFromPath()` on a missing path
 * returns an empty image *without throwing*, so the app shipped a blank tray icon and no
 * error. A missing engine is the same class of failure but worse: the app cannot start.
 *
 * ## What it checks
 *
 *  1. The **whole require graph** resolves inside the archive. Starting at `dist/main.js` and
 *     `dist/preload.js`, every specifier of every file reached is resolved with Node's own
 *     algorithm — `createRequire(<the requiring file>)`, not a re-implementation of it — and
 *     each result must land inside the extracted archive. A specifier-only scan would miss a
 *     *relative* require pointing at a file `build.files` dropped, which is the shape a
 *     packaging change actually breaks in. Two specifiers are legitimately absent from the
 *     archive and are skipped: Node builtins, and `electron`, which the runtime injects.
 *  2. `@bifurc/client` loads for real. It transitively requires `@bifurc/protocol`, so this
 *     also exercises a bare specifier *inside* a packaged package — the case that makes each
 *     packaged package's `package.json` (and therefore its `exports` map) load-bearing.
 *
 * ## Why the specifiers are read out of the bundle rather than listed here
 *
 * A hard-coded list is a copy of the code under test, and copies drift silently: add an
 * engine import tomorrow and a hand-written list still passes. The list is derived from the
 * artifact itself, so it grows with the app.
 *
 * ## The non-vacuity guards are the point
 *
 * A wrong path or a wrong layout makes every assertion trivially true — the classic
 * "guard that reads as coverage while covering nothing" (`TRAPS.md`). So before asserting
 * the real property this script requires that the extraction produced files, that `dist/`
 * holds compiled output, and that specifiers were actually found. It also has a
 * `--negative` mode that removes `node_modules/@bifurc` and requires the probe to **fail**,
 * so the probe's failure mode is observed rather than assumed.
 *
 * ## Usage
 *
 *   node scripts/assert-packaged.mjs release/win-unpacked/resources/app.asar
 *   node scripts/assert-packaged.mjs release/win-unpacked          # a *-unpacked dir also works
 *   node scripts/assert-packaged.mjs <archive> --negative          # must FAIL
 *   node scripts/assert-packaged.mjs <archive> --drop=<path>       # must FAIL
 *   node scripts/assert-packaged.mjs <archive> --keep              # keep the extraction
 */
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { builtinModules, createRequire } from "node:module";

const args = process.argv.slice(2);
const negative = args.includes("--negative");
const keep = args.includes("--keep");
const target = args.find((a) => !a.startsWith("--"));

if (!target) {
  console.error("usage: node scripts/assert-packaged.mjs <app.asar | *-unpacked dir> [--negative] [--keep]");
  process.exit(2);
}

/** Accept an asar, a `resources/` dir, or a `*-unpacked` dir — and find the asar. */
function findAsar(input) {
  const abs = path.resolve(input);
  if (!fs.existsSync(abs)) throw new Error(`no such path: ${abs}`);
  if (fs.statSync(abs).isFile()) return abs;
  const direct = path.join(abs, "resources", "app.asar");
  if (fs.existsSync(direct)) return direct;
  const nested = path.join(abs, "app.asar");
  if (fs.existsSync(nested)) return nested;
  throw new Error(`could not find app.asar under ${abs}`);
}

const asarPath = findAsar(target);
console.log(`archive: ${path.relative(process.cwd(), asarPath)}`);

// `@electron/asar` is a devDependency of the app (electron-builder ships it); resolve it
// from the repo so the script works from any cwd inside the project.
const require = createRequire(path.join(process.cwd(), "assert-packaged.cjs"));
let asar;
try {
  asar = require("@electron/asar");
} catch {
  console.error("FAIL: @electron/asar is not installed — run `npm ci` first.");
  process.exit(2);
}

const extractRoot = fs.mkdtempSync(path.join(os.tmpdir(), "bifurc-asar-"));
asar.extractAll(asarPath, extractRoot);

const fail = (msg) => {
  console.error(`\nFAIL: ${msg}`);
  if (!keep) fs.rmSync(extractRoot, { recursive: true, force: true });
  else console.error(`(extraction kept at ${extractRoot})`);
  process.exit(1);
};

// ── Non-vacuity guard 1: the extraction produced something ────────────────────
const extractedCount = (function count(dir) {
  let n = 0;
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    n += e.isDirectory() ? count(path.join(dir, e.name)) : 1;
  }
  return n;
})(extractRoot);
if (extractedCount < 100) {
  fail(`extracted only ${extractedCount} files from the archive — wrong archive or bad layout.`);
}
console.log(`extracted files: ${extractedCount}`);

// ── Non-vacuity guard 2: there is compiled app output to inspect ──────────────
const distDir = path.join(extractRoot, "dist");
if (!fs.existsSync(distDir)) fail("the archive has no dist/ — is this a Bifurc bundle?");

const distFiles = [];
(function walk(dir) {
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name);
    // `dist/renderer` and `dist/web` are Vite bundles: browser bundles, not Node, so their requires are
    // resolved by Vite at build time and are not this check's business.
    if (e.isDirectory()) {
      if (e.name !== "renderer" && e.name !== "web") walk(p);
    } else if (e.name.endsWith(".js") && !e.name.endsWith(".map")) {
      distFiles.push(p);
    }
  }
})(distDir);
if (distFiles.length === 0) fail("dist/ contains no compiled .js — nothing to verify.");
console.log(`packaged dist .js files: ${distFiles.length}`);

// ── The property: the whole require graph resolves inside the archive ────────
//
// Not just the `@bifurc/...` specifiers. Those were the *known* risk, but the check that
// matters is the one the runtime will actually perform: starting at the two entry points,
// resolve **every** specifier of **every** file reached, using Node's own algorithm
// (`createRequire(<the requiring file>).resolve`) rather than a re-implementation of it.
//
// A relative require pointing at a file `build.files` dropped is invisible to a
// specifier-only scan, and it is exactly how a packaging change breaks the app — the icon
// regression was the same shape. Two specifiers are legitimately absent from the archive
// and are skipped: Node builtins, and `electron`, which the Electron runtime injects.
const RUNTIME_PROVIDED = new Set(["electron"]);
const isBuiltin = (spec) =>
  spec.startsWith("node:") || builtinModules.includes(spec);

const specifiers = new Set();
for (const f of distFiles) {
  const src = fs.readFileSync(f, "utf8");
  for (const m of src.matchAll(/require\((["'])(@bifurc\/[^"']*)\1\)/g)) specifiers.add(m[2]);
}

// ── Non-vacuity guard 3: the reader actually found specifiers ────────────────
if (specifiers.size === 0) {
  fail(
    "found no @bifurc/* requires in the packaged output. Either the engine is genuinely " +
      "absent (a real failure) or the scan is broken — both need investigating, so this fails.",
  );
}
console.log(`distinct @bifurc specifiers: ${specifiers.size}`);

// `--negative` removes the whole `@bifurc` directory; `--drop=<path>` removes a single file,
// which is the sharper mutation for the *relative*-require half of the walk. Both make the
// probe's failure mode observable instead of assumed (`TRAPS.md`: a guard whose failure mode
// you have never seen is a guard you are guessing about).
const dropArg = args.find((a) => a.startsWith("--drop="));
const expectFailure = negative || Boolean(dropArg);

if (negative) {
  // Remove the one thing under test, so the probe's failure mode is observed.
  const hidden = path.join(extractRoot, "node_modules", "@bifurc");
  const moved = `${hidden}.hidden`;
  if (!fs.existsSync(hidden)) fail("--negative needs node_modules/@bifurc to exist; it does not.");
  fs.renameSync(hidden, moved);
}

if (dropArg) {
  const rel = dropArg.slice("--drop=".length);
  const target = path.join(extractRoot, rel);
  if (!fs.existsSync(target)) fail(`--drop=${rel} is not in the archive — pick a file that is.`);
  fs.rmSync(target);
  console.log(`--drop: removed ${rel} — the walk must now fail`);
}

const relative = (p) => path.relative(extractRoot, p);

// Whose `require` is it? **Ours must resolve; a third party's optional dependency is its own
// business.** `dist/` and `node_modules/@bifurc/` are ours — and they are exactly what a
// packaging change can break. This is a deliberate ownership boundary rather than a
// guard-detection heuristic: measured, our own emitted output has **no** `try`-guarded
// `require`, so a first-party specifier has no legitimate reason to be unresolvable, while
// `ws`'s `bufferutil`/`utf-8-validate`, `debug`'s `supports-color` and `unzipper`'s
// `@aws-sdk/client-s3` are all genuinely optional and reachable only through code paths
// Bifurc never takes.
const isOurs = (p) =>
  p.startsWith(distDir) || p.includes(`${path.sep}node_modules${path.sep}@bifurc${path.sep}`);

const failures = [];
const thirdParty = [];
const seen = new Set();
const queue = [path.join(distDir, "main.js"), path.join(distDir, "preload.js")];

for (const entry of queue) {
  if (!fs.existsSync(entry)) fail(`the packaged archive has no ${relative(entry)} — nothing to walk.`);
}

while (queue.length > 0) {
  const file = queue.shift();
  if (seen.has(file) || !fs.existsSync(file)) continue;
  seen.add(file);

  const fileRequire = createRequire(file);
  const src = fs.readFileSync(file, "utf8");
  for (const m of src.matchAll(/require\((["'])([^"']+)\1\)/g)) {
    const spec = m[2];
    if (isBuiltin(spec) || RUNTIME_PROVIDED.has(spec)) continue;
    try {
      const resolved = fileRequire.resolve(spec);
      if (!resolved.startsWith(extractRoot)) {
        failures.push(`${spec} (from ${relative(file)}) escaped the archive -> ${resolved}`);
        continue;
      }
      queue.push(resolved);
    } catch (err) {
      const where = `${spec} (from ${relative(file)})`;
      // Reported, never fatal, and never silent — so a genuinely missing first-party module
      // cannot hide among them.
      if (isOurs(file)) failures.push(`${where} -> ${err.code}`);
      else thirdParty.push(where);
    }
  }
}

// ── Non-vacuity guard 4: the walk actually traversed the bundle ──────────────
// Two files walked would make every assertion above trivially true.
if (seen.size < 20) {
  fail(`the walk reached only ${seen.size} files — the require graph was not traversed.`);
}
console.log(`files reached from the entry points: ${seen.size}`);

let clientLoad = "not attempted";
try {
  const client = createRequire(path.join(extractRoot, "probe.cjs"))("@bifurc/client");
  clientLoad =
    typeof client.createClient === "function" ? "createClient() present" : "loaded, but createClient is missing";
  if (typeof client.createClient !== "function") failures.push("@bifurc/client loaded without createClient");
} catch (err) {
  clientLoad = `LOAD FAILED: ${err.message}`;
  failures.push(`@bifurc/client failed to load: ${err.message}`);
}

console.log(`@bifurc/client load: ${clientLoad}`);

if (expectFailure) {
  if (failures.length === 0) {
    fail("MUTATION CHECK DID NOT FAIL — this probe cannot detect the condition it exists for.");
  }
  console.log(
    `\nMUTATION CHECK failed as required (${failures.length} failures) — the probe detects the condition.`,
  );
  if (!keep) fs.rmSync(extractRoot, { recursive: true, force: true });
  process.exit(0);
}

if (failures.length > 0) {
  console.error("");
  for (const f of failures) console.error(`  UNRESOLVED ${f}`);
  fail(`${failures.length} of our own specifiers do not resolve across ${seen.size} files.`);
}

if (thirdParty.length > 0) {
  console.log(`\nthird-party optional requires, absent by design (${thirdParty.length}):`);
  for (const t of thirdParty) console.log(`  optional  ${t}`);
}

if (!keep) fs.rmSync(extractRoot, { recursive: true, force: true });
console.log(
  `\nPASS: ${seen.size} files walked from the entry points, every first-party specifier resolves ` +
    `(${specifiers.size} distinct @bifurc ones), @bifurc/client loads.`,
);
