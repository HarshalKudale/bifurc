import { defineConfig } from "tsup";

export default defineConfig({
  entry: ["src/**/*.ts", "!src/**/*.test.ts"],
  format: ["esm", "cjs"],
  dts: true,
  sourcemap: true,
  clean: true,
  splitting: false,
  bundle: false,
  /**
   * `neutral`, like `@bifurc/client` and for the same reason: a browser page loads this package. There
   * is no Node built-in on the path today — `channel.ts` uses the platform `WebSocket`, `TextEncoder`
   * and `TextDecoder`, all of which a browser provides — and this setting is what turns "no Node
   * built-in" from a claim into something the build checks.
   */
  platform: "neutral",
  target: "es2022",
});
