/**
 * `@bifurc/engine` — package entry point.
 *
 * **Provisional (P2 work item 8, in progress).** The plan's end state for this file is the public
 * API `createEngine(opts) -> { start(), stop(), registry, bus, status() }` with **nothing else
 * exported** (`plan/03-phase-2-engine-extraction.md`, work item 8). That is not reachable yet:
 * the storage layer, the proxy, sync, the event bus, the application supervisor, the companion
 * server and the command registry have all physically moved into this package, but `startup.ts`
 * and `shutdown.ts` — and the `createEngine()` that ties them together — are still in the app's
 * `src/` and follow in the last step of the same work item.
 *
 * Until then this barrel exposes the moved layers as **namespaces** rather than flattened
 * star-exports. Two reasons:
 *
 *  1. `store/config.ts` re-exports from `store/workspaceFs.ts` and `store/types.ts`, and
 *     `WorkspaceSyncConfig` / `WorkspaceSyncMeta` are declared in both `appSettings.ts` and
 *     `types.ts`. Flattening with `export *` would make those names ambiguous. (`sync/types.ts`
 *     adds a third potential clash, for the same reason.)
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
export * as eventBus from "./eventBus";
export * as proxy from "./proxy/server";
export * as sync from "./sync/syncManager";
export * as applications from "./applications/processSpawner";
export * as companion from "./companion/companionServer";
export * as commands from "./commands/registry";
