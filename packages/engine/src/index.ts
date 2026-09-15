/**
 * `@bifurc/engine` — package entry point.
 *
 * **Provisional (P2 work item 8, in progress).** The plan's end state for this file is the public
 * API `createEngine(opts) -> { start(), stop(), registry, bus, status() }` with **nothing else
 * exported** (`plan/03-phase-2-engine-extraction.md`, work item 8). That is not reachable yet:
 * only the storage layer has physically moved into this package so far. The proxy, sync,
 * applications, commands and transport layers are still in the app's `src/` and follow in later
 * steps of the same work item.
 *
 * Until then this barrel exposes the moved layer as **namespaces** rather than flattened
 * star-exports. Two reasons:
 *
 *  1. `store/config.ts` re-exports from `store/workspaceFs.ts` and `store/types.ts`, and
 *     `WorkspaceSyncConfig` / `WorkspaceSyncMeta` are declared in both `appSettings.ts` and
 *     `types.ts`. Flattening with `export *` would make those names ambiguous.
 *  2. Namespacing makes the interim surface obviously provisional, so nobody starts depending on
 *     `import { loadConfig } from "@bifurc/engine"` before P6 freezes the real API.
 *
 * During P2–P5 the shell deep-imports `@bifurc/engine/store/config` and friends directly; this
 * entry exists so the package has a real, buildable `main`/`types` target.
 */

export * as paths from "./store/paths";
export * as types from "./store/types";
export * as appSettings from "./store/appSettings";
export * as config from "./store/config";
export * as workspaceFs from "./store/workspaceFs";
export * as gitStore from "./store/gitStore";
export * as subscription from "./subscription/entityCount";
export * as randomNames from "./lib/randomNames";
export * as randomizer from "./lib/randomizer";
