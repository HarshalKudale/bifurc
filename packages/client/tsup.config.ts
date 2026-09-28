import { defineConfig } from "tsup";

export default defineConfig({
  entry: ["src/**/*.ts", "!src/**/*.test.ts"],
  format: ["esm", "cjs"],
  dts: true,
  sourcemap: true,
  clean: true,
  splitting: false,
  /**
   * CRITICAL — do not bundle, for the same reason `packages/engine` does not.
   *
   * The subscription multiplexer (`src/subscriptions.ts`) holds one ref-count table as
   * module-level state. Bundling would inline a *private copy* of that module into every
   * entry point, so two `createClient()` calls reaching it through different entries would
   * not share the table — and the failure mode is silent: a duplicated transport
   * subscription rather than an error.
   */
  bundle: false,
  /**
   * `neutral`, not `node`. The client is the half that has to run in a browser bundle for
   * P7's web UI, and it touches no Node built-in — it is a pure mapping over a `Transport`.
   * The engine is `platform: "node"`; this package deliberately is not.
   */
  platform: "neutral",
  target: "es2022",
});
