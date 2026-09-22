import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import path from "path";
import { readFileSync } from "fs";

const pkg = JSON.parse(readFileSync(path.resolve(__dirname, "../package.json"), "utf-8"));

/**
 * The browser host's build (D18).
 *
 * ## What changed, and why the previous reasoning here was wrong
 *
 * This config used to argue that the web target must **not** share the renderer's config, because the
 * renderer's config registers `@vitejs/plugin-react` and aliases `@` into the renderer tree — "a React
 * plugin for a target with no React, and an alias that resolves to the Electron renderer's components".
 *
 * That was true of the framework-free probe page this file was written for. It is **not** true of this
 * target: the browser host imports the same `<App />`, so it needs React, and it needs the same `@`
 * alias. The split that matters is therefore not *desktop config vs web config* but **shared-UI config
 * vs per-host config** — both hosts now carry the React plugin and the same two aliases, and differ only
 * in `root`, `outDir`, and where `__APP_VERSION__` is read from.
 *
 * ## The two aliases
 *
 * `@` points into `packages/gui/src` rather than into a tree beside this file. That is what lets the UI's
 * 647 `@/…` imports resolve unchanged in *both* hosts — the reason the extraction moved 211 files
 * without editing an import. `@bifurc/gui` names the same package's entry, so the import reads as the
 * dependency it actually is.
 *
 * ## Output
 *
 * `base: "./"` matches the renderer's choice, so the bundle is servable from a subpath — which is what
 * makes the "engine serves the UI" option in work item 5 available later.
 *
 * `outDir` is `dist/web`, a **sibling of `dist/renderer` rather than inside it**: the two builds are
 * separate by the owner's decision. `scripts/assert-dist.mjs` skips both, because a Vite bundle is not a
 * Node module graph.
 */
export default defineConfig({
  plugins: [react()],
  root: __dirname,
  base: "./",
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "../packages/gui/src"),
      "@bifurc/gui": path.resolve(__dirname, "../packages/gui/src/index.ts"),
    },
  },
  // The UI sits outside `root`. Vite infers a dev-server allow-list from the workspace root, which
  // should already cover `packages/` — but that inference is the kind of thing that fails only under
  // `npm run dev`, which this repo's rules say the owner runs rather than me. Stated, not assumed.
  server: {
    fs: { allow: [path.resolve(__dirname, "..")] },
  },
  define: {
    // The UI's `AboutSection` reads this. The renderer's config defines it too — and both must, or the
    // browser build fails at *runtime* on a bare identifier instead of at build time.
    __APP_VERSION__: JSON.stringify(pkg.version),
  },
  build: {
    outDir: path.resolve(__dirname, "../dist/web"),
    emptyOutDir: true,
    sourcemap: true,
    target: "es2022",
  },
});
