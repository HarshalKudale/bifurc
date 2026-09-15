import { defineConfig } from "tsup";

export default defineConfig({
  /**
   * One entry per module, with the directory structure preserved. During P2–P5 the
   * Electron shell still deep-imports engine internals (`@bifurc/engine/store/config`);
   * P6 narrows the public surface to `createEngine()` alone. A single-entry bundle
   * would make the interim deep imports impossible.
   */
  entry: ["src/**/*.ts", "!src/**/*.test.ts"],
  format: ["esm", "cjs"],
  dts: true,
  sourcemap: true,
  clean: true,
  splitting: false,
  /**
   * CRITICAL — do not bundle.
   *
   * `store/paths.ts` holds the resolved data root as module-level state, and
   * `store/config.ts` / `store/gitStore.ts` hold caches the same way. Bundling would
   * inline a *private copy* of those modules into every entry point, so a
   * `setDataRoot()` call made through one entry would not be seen by the modules
   * reached through another. The failure mode is silent — paths would fall back to
   * the wrong root rather than throwing. Emitting one output file per input file keeps
   * the module graph — and therefore every singleton — exactly as written.
   */
  bundle: false,
  platform: "node",
  target: "node20",
});
