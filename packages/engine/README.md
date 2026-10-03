# @bifurc/engine — deprecated

**This is the TypeScript engine, and it is scheduled for decommissioning.** The engine is `packages/engine-rust`
(the `bifurc-engine` executable); the desktop UI, the `bifurc` CLI and the packaged installs all use it.

What still depends on this package, and so keeps it alive until they go:

- the Electron shell (`src/` at the repository root);
- the parity harness, `scripts/engine-parity.mjs`, which runs every scenario against this engine and the Rust one;
- the UI's fallback when no `bifurc-engine` can be found (it logs that it is starting the Node engine).

Do not add features here. A behavior change goes into `packages/engine-rust` and, only while the harness still
runs, into this package as the minimum that keeps the two scenarios identical.
