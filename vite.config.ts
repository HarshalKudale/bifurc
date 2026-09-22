import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import path from "path";
import { readFileSync } from "fs";

const pkg = JSON.parse(readFileSync(path.resolve(__dirname, "package.json"), "utf-8"));

/**
 * The Electron renderer's build.
 *
 * `root` stays `renderer` and `outDir` stays `dist/renderer` — that pair is a contract, not a
 * preference: `src/main.ts` loads `dist/renderer/index.html`, and four scripts under `scripts/` treat
 * the path as Vite's bundle rather than as a Node module graph. Under D18 the *UI* lives in
 * `packages/gui/src`, so `renderer/` is now only the host's entry (`index.html` + `main.tsx`).
 *
 * The `@` alias is what makes the move a config change instead of a 647-import rewrite: the UI already
 * imported itself as `@/…` rather than by relative path, so repointing this one line moved 211 files
 * without editing a single import statement.
 */
export default defineConfig({
  plugins: [react()],
  root: "renderer",
  base: "./",
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "packages/gui/src"),
      "@bifurc/gui": path.resolve(__dirname, "packages/gui/src/index.ts"),
    },
  },
  // The UI now sits outside `root`. Vite derives its dev-server allow-list from the workspace root, which
  // should already cover `packages/`, but that inference is the kind of thing that fails only in `npm run
  // dev` — which this repo's own rules say the owner runs, not me. Stated explicitly rather than assumed.
  server: {
    fs: { allow: [path.resolve(__dirname)] },
  },
  define: {
    __APP_VERSION__: JSON.stringify(pkg.version),
  },
  build: {
    outDir: path.resolve(__dirname, "dist/renderer"),
    emptyOutDir: true,
    sourcemap: true,
  },
});
