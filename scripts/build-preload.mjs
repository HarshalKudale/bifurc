/**
 * Bundle `src/preload.ts` into a **single self-contained file**, because a sandboxed preload has no module
 * resolver and cannot load anything but `electron`.
 *
 * ## The defect this fixes
 *
 * `src/main.ts:170-174` sets `contextIsolation: true` and `nodeIntegration: false` and never mentions
 * `sandbox`, so Electron's default applies — `true` since v20. A sandboxed preload does **not** run under
 * Node's module system: the browser process reads the file, hands `{filePath, contents}` to
 * `executeSandboxedPreloadScripts`, and evals it with a `require` bound to `preloadRequire`. That function's
 * whole body is a lookup in Electron's own webpack module table, ending in
 * `throw new Error("module not found: " + id)`. The table holds **`electron`, `events`, `url`, `timers`,
 * `punycode`** — and nothing else. There is no `node_modules` resolution and no filesystem resolution, so a
 * bare specifier and a relative `./sibling` fail identically; the error simply names whichever appears first.
 *
 * `build:main` is `tsc && tsc-alias` — **no bundling** — so `dist/preload.js` kept
 * `require("@bifurc/client")`, `require("./ipcTransport")` and, transitively, `require("@bifurc/protocol")`.
 * The preload never loaded at all:
 *
 * ```
 * Unable to load preload script: I:\...\dist\preload.js
 * Error: module not found: @bifurc/client
 *     at preloadRequire (VM4 sandbox_bundle:2:144196)
 *     at runPreloadScript (VM4 sandbox_bundle:2:144465)
 * ```
 *
 * **The consequence was not a broken preload alone.** `window.api` is the whole renderer surface, so every
 * `window.api.*` call is `undefined` and the app is a non-booting shell — which is the mechanism behind P6
 * criterion 2's carried `46 failed / 0 passed`. That was recorded as "carried" rather than diagnosed for
 * several sessions, and this is the diagnosis.
 *
 * ## Why bundling, and not the alternatives
 *
 * - **`sandbox: false`** would restore `require`, and it is a security downgrade — the renderer would get
 *   Node's module system back. Not an option.
 * - **Hand-writing the surface again** (the pre-`1268bfd` state) means maintaining a second copy of
 *   `@bifurc/client`'s 144-key surface by hand, which is what P5 and P6 removed.
 * - **Bundling** keeps `src/preload.ts` as the single source of truth and produces a file with exactly one
 *   surviving `require`: `electron`.
 *
 * ## Why it resolves `@bifurc/*` to `dist/` rather than to each package's source tree
 *
 * Aliasing to source (as `vitest.config.ts` does) would make the bundle build even when a package's `dist`
 * is incomplete — and an incomplete `dist` is a real, observed failure mode here: on 2026-09-21
 * `packages/engine/dist/` was found holding **161 declaration files and only 13 implementations**, so
 * `@bifurc/engine/transport/types` had no runtime file at all. Resolving through the package exports turns
 * that into a **build error** ("Could not resolve …") instead of a runtime one, which is strictly better
 * detection. It also keeps the preload built from the same artifacts the main process loads.
 *
 * ## The guard: it loads the bundle in a simulated sandbox
 *
 * A regex for `require("…")` would be fooled by the string appearing inside a string literal, and a static
 * require-walk — which `scripts/clean-main-dist.mjs` used to prove the stale `dist/` files were unreachable —
 * proves **reachability, not loadability**. It walked `require`s out of `dist/preload.js` and concluded they
 * were fine; the sandbox realm cannot resolve them at all.
 *
 * So this evaluates the finished bundle with a `require` that knows **only** the five names Electron's table
 * provides, and fails the build if anything else is asked for. That is the same failure the browser process
 * would produce, caught at build time. **The check is the point of this script** — the bundling is three
 * lines of esbuild config, and without the check the condition can come back the moment someone adds an
 * import.
 */
import fs from "node:fs";
import path from "node:path";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const esbuild = require("esbuild");

const ENTRY = path.resolve("src/preload.ts");
const OUT = path.resolve("dist/preload.js");

/**
 * The complete contents of Electron's sandboxed-preload module table.
 *
 * Sourced from `preloadRequire` in `electron.exe`'s `sandbox_bundle`, which ends in
 * `throw new Error("module not found: " + id)` — so this set is the *whole* resolver, not a subset of a
 * larger one. `punycode` is included because it is in the table; nothing in the closure uses it today.
 */
const SANDBOX_MODULES = new Set(["electron", "events", "url", "timers", "punycode"]);

if (!fs.existsSync(ENTRY)) {
  console.error(`[build:preload] no ${path.relative(process.cwd(), ENTRY)} — nothing to bundle`);
  process.exit(1);
}

await esbuild.build({
  entryPoints: [ENTRY],
  outfile: OUT,
  bundle: true,
  // `node` rather than `browser`: the preload runs in a Node-flavoured realm, and this leaves Node builtins
  // external instead of trying to shim them. The closure is verified to contain none (see the guard), so
  // nothing is left dangling.
  platform: "node",
  // CJS, because the sandbox evals the file with a `require` in scope. An ESM output would use `import`
  // syntax the realm cannot execute.
  format: "cjs",
  target: "node20",
  // The one module the sandbox can resolve. Everything else must be inlined.
  external: ["electron"],
  // Not minified, deliberately. The stack trace above is how this defect was diagnosed; a minified preload
  // would have made that far harder, and 1.8 MB of readable preload is not worth trading for it.
  minify: false,
  sourcemap: true,
  logLevel: "warning",
});

const code = fs.readFileSync(OUT, "utf8");

/**
 * Load the bundle the way the sandbox would: `require` resolves the five known names and throws
 * `module not found` for anything else, exactly as `preloadRequire` does.
 */
const electronStub = {
  contextBridge: { exposeInMainWorld: () => {} },
  ipcRenderer: { invoke: () => Promise.resolve(), on: () => {}, removeListener: () => {} },
};

const sandboxRequire = (id) => {
  if (!SANDBOX_MODULES.has(id)) throw new Error(`module not found: ${id}`);
  return id === "electron" ? electronStub : require(id);
};

const asked = new Set();
const trackingRequire = (id) => {
  asked.add(id);
  return sandboxRequire(id);
};

try {
  const module_ = { exports: {} };
  // eslint-disable-next-line no-new-func -- the point is to evaluate the emitted file as the sandbox does.
  const load = new Function(
    "require",
    "module",
    "exports",
    "__filename",
    "__dirname",
    code,
  );
  load(trackingRequire, module_, module_.exports, OUT, path.dirname(OUT));
} catch (err) {
  console.error(
    `[build:preload] the bundle does NOT load in a sandboxed preload — the app would be a non-booting shell.\n` +
      `  ${err && err.message ? err.message : String(err)}\n` +
      `  A sandboxed preload can resolve only: ${[...SANDBOX_MODULES].join(", ")}.\n` +
      `  Anything else must be bundled in. Check for a new import in src/preload.ts or its closure.`,
  );
  process.exit(1);
}

const sizeKb = (fs.statSync(OUT).size / 1024).toFixed(0);
console.log(
  `[build:preload] bundled ${path.relative(process.cwd(), ENTRY)} → ` +
    `${path.relative(process.cwd(), OUT)} (${sizeKb} KB), loads in a simulated sandbox; ` +
    `resolved at load: ${[...asked].sort().join(", ")}`,
);
