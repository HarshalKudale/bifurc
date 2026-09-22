import { defineConfig } from "vite";
import path from "path";

/**
 * The web target: a page, not an Electron renderer.
 *
 * Work item 5 says "reuse the Vite config". This reuses the **tooling** — Vite 8 is already a
 * devDependency and the output shape matches `dist/renderer` — but not the config *file*, and that is
 * deliberate rather than an oversight. The root `vite.config.ts` sets `root: "renderer"`, registers
 * `@vitejs/plugin-react`, and aliases `@` into the renderer tree. Every one of those would silently
 * apply to a browser page that shares it: a React plugin for a target with no React, and an alias that
 * resolves to the Electron renderer's components. Two targets that must be able to diverge should not
 * be one config.
 *
 * `base: "./"` matches the renderer's choice, so the bundle is servable from a subpath as well as from
 * a root — which is what makes the "engine serves the UI" option in work item 5 available later.
 */
export default defineConfig({
  root: __dirname,
  base: "./",
  build: {
    outDir: path.resolve(__dirname, "dist"),
    emptyOutDir: true,
    sourcemap: true,
    target: "es2022",
  },
});
