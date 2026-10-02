/**
 * `emitEntityStatus()` — read a workspace's two git-derived maps and emit them as **one** event.
 *
 * ## Why this is in `runtime/` and not in `kernel/eventBus.ts`
 *
 * It lived in `kernel/eventBus.ts` until engine-layout phase 8's follow-up, and it made that file
 * **violate §5.1's central claim**: *"`kernel/` imports nothing from the engine"* — the property §5.2
 * calls what makes the layering rule "mechanically checkable rather than aspirational". The function
 * reads `getWorkspaceSyncStatus()` (`gitops/statusTracker.ts`) and `getWorkspaceGitState()`
 * (`gitops/gitState.ts`), so `kernel/` imported two **values** from a feature module.
 *
 * It could not simply move to `gitops/`, because **three of its four callers are `workspace/`-side** —
 * `workspace/entities/coreCommands.ts`, `workspace/entities/entityCrudCommands.ts` and
 * `workspace/healthbarCommands.ts` (the fourth is `gitops/syncCommands.ts`) — and §6 lets `workspace/`
 * import only `kernel/` and `utils/`. Moving it sideways would have turned two `kernel → gitops` edges
 * into three `workspace → gitops` ones and changed nothing.
 *
 * So it went to `runtime/`, which §5.1 puts last in the chain precisely for this: *"`runtime/` composes;
 * it never serves."* Reading two modules' outputs and publishing them together is composition — the
 * same reason `runtime/instances.ts` holds the listener fan-out. **The arrow stays one-way:** nothing in
 * `gitops/` or `workspace/` may be imported *by* this file's dependencies in return.
 *
 * `runtime/entityStatus.ts` and `runtime/instances.ts` are the engine's **two seam modules**: the small
 * set of functions that exist because a feature needs something it is not allowed to import. The layout
 * ratchet (`tests/engine/layout.test.ts`) exempts exactly these two by name — adding a third is a
 * deliberate act, not a side effect.
 *
 * ## Why one event and not two
 *
 * Both halves are read in parallel and emitted together. A subscriber that received the file map without
 * the git state would have to fetch it separately, and between the two reads a publish could land —
 * leaving the UI showing a `modified` entity that has nothing left to sync. `getWorkspaceGitState()`
 * cannot reject (it resolves `hasRemote: false` instead), so the `Promise.all` below only ever fails on
 * the file map, matching the prior behaviour exactly.
 *
 * The `JSON.parse(JSON.stringify(...))` is not decoration: the map is a plain object built from git
 * output, and re-serialising it guarantees subscribers get a structured clone rather than a shared
 * reference they could mutate out from under the next read.
 *
 * ## History
 *
 * The body is a **verbatim move** — it is the shared replacement for the THREE near-identical local
 * `broadcastEntityStatus()` functions that used to live in the shell's `coreHandlers.ts`,
 * `entityCrudFactory.ts` and `syncHandlers.ts` (plus a fourth, slightly different copy in
 * `companionServer.ts`), each reaching into the shell's open windows directly. One implementation,
 * emitted on the bus; the shell's `rpcBridge.ts` is now the only place that fans an event out to
 * windows (step 3c deleted the `eventBridge.ts` that used to).
 */
import { getWorkspaceSyncStatus } from "../gitops/statusTracker";
import { getWorkspaceGitState } from "../gitops/gitState";
import { bus } from "../kernel/eventBus";

export function emitEntityStatus(wsId: string): void {
  Promise.all([getWorkspaceSyncStatus(wsId), getWorkspaceGitState(wsId)])
    .then(([status, git]) => {
      bus.emitTyped("sync.entityStatus", { wsId, status: JSON.parse(JSON.stringify(status)), git });
    })
    .catch(() => { /* best-effort, matches prior behaviour */ });
}
