import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";
import path from "path";
import fs from "fs";

/**
 * Custom resolve plugin: @/ resolves to src/ first, then renderer/ as fallback.
 * This allows test files to import from both src/ and renderer/ using @/ alias.
 */
function dualAliasPlugin() {
  const srcDir = path.resolve(__dirname, "src");
  const rendererDir = path.resolve(__dirname, "renderer");
  return {
    name: "dual-alias",
    resolveId(source: string) {
      if (!source.startsWith("@/")) return null;
      const rel = source.slice(2);
      const extensions = ["", ".ts", ".tsx", ".js", ".jsx"];
      // Try src/ first
      for (const ext of extensions) {
        const full = path.join(srcDir, rel + ext);
        if (fs.existsSync(full)) return full;
      }
      // Try renderer/ second
      for (const ext of extensions) {
        const full = path.join(rendererDir, rel + ext);
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

export default defineConfig({
  plugins: [react(), dualAliasPlugin()],
  test: {
    environment: "node",
    globals: true,
    include: ["tests/**/*.test.ts", "tests/**/*.test.tsx"],
    setupFiles: ["tests/setup.ts"],
    testTimeout: 15000,
    coverage: {
      provider: "v8",
      include: ["src/**/*.ts", "renderer/lib/**/*.ts", "renderer/components/**/*.tsx"],
      exclude: ["src/main.ts", "src/preload.ts"],
    },
  },
});
