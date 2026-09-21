/**
 * P14 work item 7 — the **runtime registry**: the lookup the command layer was missing.
 *
 * ## The gap this closes, measured rather than inferred
 *
 * `createWorkspaceHost()` returns its runtimes as a `ReadonlyMap` on the handle it hands back, so they
 * are reachable by whoever *called* it and by nobody else. But the commands serve **two** consumers — the
 * host's RPC endpoint and the desktop's IPC — and the command registry is a module-level singleton that
 * neither owns. So `registerServerCommands()` had no way to reach a runtime and called the **module
 * facade** instead (`proxy/server.ts`'s `startServer` / `isRunning` / `getPort`), whose lazy default
 * instance is `createProxyServer({ workspaceId: null })` — i.e. *the active workspace*.
 *
 * Measured on a host with two workspaces, before this module existed:
 *
 * ```
 * host: ws-A proxy=81 webhook=9101     ws-B proxy=82 webhook=9102
 * server.status                 → { running: false, port: 0 }    ← two members ARE listening
 * server.start {workspaceId: A} → { ok: true }                    ← accepted, IGNORED
 * server.status                 → { running: false, port: 8791 }  ← a THIRD server
 * ```
 *
 * **The failure succeeds**, which is why no suite caught it: `server.start` returned `{ ok: true }` after
 * binding a server nobody asked for, on the active workspace's configured port, while a member for that
 * same workspace was already on 82. WI 1's protocol half made the ten commands *accept* `workspaceId`;
 * this module is what makes them *read* it.
 *
 * ## The resolution rule, and why the third case exists
 *
 * | Request | Answer |
 * |---|---|
 * | `workspaceId` names a registered runtime | **that runtime's** proxy / webhook |
 * | resolves to the active workspace, no runtime registered | the **module facade** — the in-process engine |
 * | `workspaceId` names anything else | **`absent`** — no runtime on this host |
 *
 * The second row is the desktop: `createEngine()` binds one proxy through the facade, so on a desktop the
 * facade *is* the active workspace's server. Keeping it means **zero behaviour change for the desktop**,
 * which is WI 7's acceptance criterion (P6's rule: no user-visible change).
 *
 * **The third row is the one that matters, and it is the one a naive fix omits.** Falling through to the
 * facade when an explicitly-named workspace has no runtime would re-create exactly the bug above: the
 * command would act on the *active* workspace while the caller asked for another, and report success. So
 * `absent` is a distinct outcome that callers must handle, not a synonym for "use the facade". A workspace
 * is only ever served by a runtime that is actually registered for it.
 *
 * ## Why module-level state is acceptable here — and the constraint it carries
 *
 * `TRAPS.md` records that `store/paths.ts` and `applications/processSpawner.ts` are singletons *because*
 * `tsup` runs `bundle: false` to keep them one instance per process, and that a second copy of either is
 * the hazard. This module is a different kind of singleton: it holds **live object references**, not
 * configuration. There is no second copy to make — a registry of the runtimes in *this* process is only
 * meaningful in this process, and a duplicate would hold the same objects.
 *
 * The constraint it does carry: **one workspace host per process.** `clearRuntimes()` is called when a host
 * stops, so a second concurrent host would evict the first one's entries. That is the same limitation
 * `host.ts`'s header already records for a different reason ("an engine *and* N runtimes in one process
 * would have two authorities over the same servers"), and D10–D12 make the host the deployable unit, so
 * one per process is the design rather than a restriction this module imposes.
 *
 * ## It deliberately does not import the proxy or webhook modules
 *
 * The `WorkspaceRuntime` import below is **type-only**, so it is erased and the emitted JavaScript has no
 * import at all. That matters because the dependency would otherwise be circular:
 * `runtime/workspaceRuntime.ts` imports `proxy/server.ts` (a value), so a *value* import from
 * `proxy/serverCommands.ts` back into `runtime/` would close a loop. `import type` keeps the graph
 * acyclic — `serverCommands → runtimeRegistry → (nothing)`.
 */
import type { WorkspaceRuntime } from "./workspaceRuntime";
import { loadConfig } from "../store/config";

/** Registered runtimes, keyed by workspace id. See the header on why module scope is right here. */
const runtimes = new Map<string, WorkspaceRuntime>();

/**
 * Register a runtime so workspace-scoped commands can reach it. Called by the host as it creates each
 * runtime — **before** `start()`, so a command arriving during startup resolves to the right instance
 * rather than to the facade.
 */
export function registerRuntime(runtime: WorkspaceRuntime): void {
  runtimes.set(runtime.workspaceId, runtime);
}

/** Remove one runtime. Called when a workspace is stopped individually. */
export function unregisterRuntime(workspaceId: string): void {
  runtimes.delete(workspaceId);
}

/**
 * Forget every runtime. Called by `WorkspaceHost.stop()`.
 *
 * Without this a stopped host would leave its runtimes registered, so a later `server.status` would report
 * a workspace that is no longer served — the same class of stale answer as `currentPort` surviving `stop()`
 * (`proxy/server.ts`'s recorded trap), one layer up.
 */
export function clearRuntimes(): void {
  runtimes.clear();
}

/** The runtime for `workspaceId`, or `undefined`. Exposed for tests and for a host's own introspection. */
export function getRuntime(workspaceId: string): WorkspaceRuntime | undefined {
  return runtimes.get(workspaceId);
}

/** The registered workspace ids, in insertion order. */
export function runtimeWorkspaceIds(): string[] {
  return [...runtimes.keys()];
}

/**
 * Where a workspace-scoped command should act. A three-way discriminated union rather than
 * `WorkspaceRuntime | null`, because **"no runtime, use the facade" and "no runtime, and none is coming"
 * are different answers** and collapsing them is the bug this module exists to fix — see the header's
 * third row.
 */
export type RuntimeScope =
  | { kind: "runtime"; workspaceId: string; runtime: WorkspaceRuntime }
  | { kind: "facade"; workspaceId: string }
  | { kind: "absent"; workspaceId: string };

/**
 * Resolve a command's target, per the header's table.
 *
 * `activeWorkspaceId` is read **per call**, never captured: on a host it can change under a running
 * process, and `proxy/server.ts`'s `servedWorkspaceId()` records the same requirement for the same reason
 * ("capturing it once would silently freeze the desktop on whichever workspace was active at startup").
 */
export function resolveRuntimeScope(requestedWorkspaceId?: string): RuntimeScope {
  const activeWorkspaceId = loadConfig().activeWorkspaceId;
  const workspaceId = requestedWorkspaceId ?? activeWorkspaceId;

  const runtime = runtimes.get(workspaceId);
  if (runtime) return { kind: "runtime", workspaceId, runtime };

  // No runtime for the target. The in-process engine serves exactly one workspace through the facade, and
  // that workspace is the active one — so this is the desktop's path, unchanged.
  if (workspaceId === activeWorkspaceId) return { kind: "facade", workspaceId };

  return { kind: "absent", workspaceId };
}

/** The message an `absent` resolution produces. One place, so `server.*` and `webhookServer.*` agree. */
export function absentRuntimeError(workspaceId: string): string {
  return `No runtime is registered for workspace "${workspaceId}" on this host.`;
}

/**
 * The `workspaceId` echo for a status result, present **only when the caller supplied one**.
 *
 * Both `ServerStatusResult` and `WebhookServerStatusResult` carry an optional `workspaceId` documented as
 * *"echoed so a multiplexed client can match response to request"* — and a client that is multiplexing is
 * exactly a client that sends an id. Returning it unconditionally would change the payload of every
 * existing status call **including the desktop's**, and `tests/ipc/handlers.test.ts:764` pins that payload
 * with `toEqual` precisely so a change to it is a decision rather than a side effect. WI 7's acceptance
 * criterion is P6's rule — zero user-visible change — so the echo is conditional.
 *
 * Defined here rather than in each command module because two copies of a response-shaping rule is the
 * drift `./scope`'s own header warns about, one layer up.
 */
export function echoWorkspace(paramsWorkspaceId: string | undefined, workspaceId: string): { workspaceId?: string } {
  return paramsWorkspaceId === undefined ? {} : { workspaceId };
}
