/**
 * Build-time constants for the browser host.
 *
 * `__APP_VERSION__` is injected by `web/vite.config.ts`'s `define` and reported to the engine in the
 * `hello` handshake, so its log says which build connected. Declared here rather than reached for
 * through `@bifurc/gui`'s `env.d.ts`, because that file is the *UI's* declaration and this host's
 * tsconfig does not include it — a shared global declaration reached across a package boundary is how
 * one host's config silently starts defining another's.
 */
declare const __APP_VERSION__: string;

/**
 * Vite turns an imported image into the URL of the emitted asset.
 *
 * Declared here rather than inherited from `packages/gui/src/env.d.ts`, for the same reason
 * `__APP_VERSION__` is: that file is the *UI's* declaration, and this host's tsconfig does not include it.
 */
declare module "*.png" {
  const src: string;
  export default src;
}
