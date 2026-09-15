import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";
import path from "path";
import fs from "fs";

/**
 * Custom resolve plugin: @/ resolves to src/ first, then renderer/ as fallback.
 * This allows test files to import from both src/ and renderer/ using @/ alias.
 *
 * A directory must NOT be returned as a resolved id — Vite cannot load one. Without the
 * `isDirectory()` guards, `@/lib/strings` matched `renderer/lib/strings/` on the empty
 * extension and shadowed the `index.ts` fallback below, so any module importing the
 * `strings` barrel was untestable.
 */
function dualAliasPlugin() {
  const srcDir = path.resolve(__dirname, "src");
  const rendererDir = path.resolve(__dirname, "renderer");
  const isDir = (p: string): boolean => {
    try { return fs.statSync(p).isDirectory(); } catch { return false; }
  };
  return {
    name: "dual-alias",
    resolveId(source: string) {
      if (!source.startsWith("@/")) return null;
      const rel = source.slice(2);
      const extensions = ["", ".ts", ".tsx", ".js", ".jsx"];
      // Try src/ first
      for (const ext of extensions) {
        const full = path.join(srcDir, rel + ext);
        if (ext === "" && isDir(full)) break;
        if (fs.existsSync(full)) return full;
      }
      // Try renderer/ second
      for (const ext of extensions) {
        const full = path.join(rendererDir, rel + ext);
        if (ext === "" && isDir(full)) break;
        if (fs.existsSync(full)) return full;
      }
      // Try as directory with index
      for (const ext of extensions) {
        const full = path.join(srcDir, rel, "index" + ext);
        if (fs.existsSync(full)) return full;
      }
      for (const ext of extensions) {
        const full = path.join(rendererDir, rel, "index" + ext);
        if (fs.existsSync(full)) return full;
      }
      return null;
    },
  };
}

const plugins = [react(), dualAliasPlugin()];

/**
 * Shared defaults for every project. Each project redeclares these because Vitest
 * does not inherit `test` options from the root config into `test.projects`.
 */
const shared = {
  environment: "node" as const,
  globals: true,
  setupFiles: ["tests/setup.ts"],
  testTimeout: 15000,
};

export default defineConfig({
  plugins,
  test: {
    ...shared,
    /**
     * Two projects so the fast feedback loop (unit) can be run on its own, and the
     * heavier real-socket integration suite can be run separately in CI or before a
     * release. `npm test` still runs everything.
     */
    projects: [
      {
        plugins,
        test: {
          ...shared,
          name: "unit",
          include: ["tests/**/*.test.ts", "tests/**/*.test.tsx", "packages/**/*.test.ts"],
          // Integration tests live under tests/integration and bind real ports.
          exclude: ["tests/integration/**"],
        },
      },
      {
        plugins,
        test: {
          ...shared,
          name: "integration",
          include: ["tests/integration/**/*.test.ts"],
          // Real servers, real sockets — allow a little more headroom.
          testTimeout: 20000,
          hookTimeout: 20000,
          // Each integration file starts its own proxy + upstream servers. Running
          // files in parallel makes the output unreadable and can exhaust ports.
          fileParallelism: false,
        },
      },
    ],
    coverage: {
      provider: "v8",
      reporter: ["text", "text-summary", "html", "json-summary", "lcov"],
      reportsDirectory: "coverage",
      include: [
        "src/**/*.ts",
        "renderer/lib/**/*.ts",
        // Both extensions: the capture/search/tab-reducer logic lives in `.ts` files under
        // renderer/components, and matching only `.tsx` hid it from the report entirely —
        // even modules that already had passing tests.
        "renderer/components/**/*.ts",
        "renderer/components/**/*.tsx",
        "renderer/panels/**/*.tsx",
      ],
      exclude: [
        // Process entry points — exercised by launching the app, not by unit tests.
        "src/main.ts",
        "src/preload.ts",
        // Type-only modules.
        "**/*.d.ts",
        "src/store/types.ts",
        "src/sync/types.ts",
        "src/applications/types.ts",
        "src/ipc/importExport/types.ts",
        "renderer/components/modals/import-export/types.ts",
        "renderer/components/search/searchTypes.ts",
        "renderer/components/sidebar/FolderTree.types.ts",
        // Barrel file with no logic.
        "renderer/components/ui/index.ts",
        // Pure constant / presentational modules with no logic worth asserting.
        "renderer/lib/strings/**",
        "renderer/lib/icons.tsx",
      ],
      /**
       * Coverage thresholds act as a ratchet: they are set just below the level the
       * suite achieves today so that new untested code cannot silently drag coverage
       * down. They are deliberately low — the suite currently covers a small fraction
       * of the renderer. Raise these numbers as gaps are closed; never lower them to
       * make CI pass.
       *
       * Current actuals (see TESTING.md): statements 46.14%, branches 31.30%,
       * functions 34.34%, lines 48.39%.
       */
      thresholds: {
        statements: 46,
        branches: 31,
        functions: 34,
        lines: 48,
      },
    },
  },
});
