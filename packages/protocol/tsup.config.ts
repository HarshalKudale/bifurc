import { defineConfig } from "tsup";

export default defineConfig({
  entry: ["src/index.ts"],
  format: ["esm", "cjs"],
  dts: true,
  sourcemap: true,
  clean: true,
  splitting: false,
  // No Node built-ins on the critical path (P1 acceptance criterion) — fail loudly
  // if a future command schema accidentally pulls one in.
  platform: "neutral",
});
