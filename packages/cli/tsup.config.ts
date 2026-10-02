import { defineConfig } from "tsup";

export default defineConfig({
  entry: ["src/index.ts"],
  format: ["cjs"],
  platform: "node",
  target: "node20",
  sourcemap: true,
  clean: true,
  // `@bifurc/*` stay external (they are dependencies), so the CLI shares the engine's one copy of each module.
});
