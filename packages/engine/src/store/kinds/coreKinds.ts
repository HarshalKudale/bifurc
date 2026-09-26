/**
 * The last `EntityKind` the shell declares — **no channels**.
 *
 * ## It lives in the engine now (2026-09-23, `plan/15` work item 10)
 *
 * Moved from `src/ipc/handlers/coreKinds.ts` to `packages/engine/src/store/kinds/coreKinds.ts`, with its
 * four siblings, because the container it populates is read by the engine-side `entity.*` commands and a
 * headless host could not reach a shell file — so `entity.create` for `environments` threw
 * `is not routed through the CommandRegistry yet` on the one host the browser UI talks to. See
 * `crudKinds.ts`'s header for the full shape; `registerAllEngineKinds()` is the engine-side caller.
 *
 * Renamed from `coreHandlers.ts` on 2026-09-21, when the P6 reduction's sixth group deleted the file's
 * seventeen channels. The name follows the same rule `crudHandlers.ts` → `crudKinds.ts` followed one
 * group earlier: a file named `*Handlers` that registers no handlers is a lie, and the three siblings
 * stage 3b renamed for the same reason (`soapKinds.ts`, `graphqlKinds.ts`, `grpcKinds.ts`).
 *
 * ## What left, and why the criterion had to be re-applied per channel
 *
 * The seventeen channels were all dead by reachability — `window.api` **is** the client
 * (`src/preload.ts:110`) and `packages/client/src` names no channel, so nothing in the shell reaches
 * them. But "dead" is not "deletable": the criterion is **"owns a step the engine does not perform"**
 * (`plan/07` §5b-undecies), and it has to be applied by *reading the command*, not inherited from the
 * classification, because this file is where the bodies stopped being one-line delegates:
 *
 *  - **Thirteen were `commandRegistry.invoke()` delegates** — `config:get`, `env:setActive`,
 *    `workspace:setActive`, `entity:load`, `entity:setEnabled`. Every one resolves to a body in
 *    `packages/engine/src/store/coreCommands.ts` that is behaviourally identical.
 *  - **Four were self-contained and already duplicated** — `config:save`, `workspace:add`,
 *    `workspace:rename`, `workspace:delete` against `packages/engine/src/store/configCommands.ts`.
 *    That file's header records the one difference: `workspace.delete`'s shell body opened with a
 *    `const ws = (cfg.workspaces ?? []).find(...)` whose result it never read, and that dead line is
 *    dropped rather than reproduced. Nothing observable moves.
 *  - **`services:discover`, `request:replay`, `server:status`, `proxy:status` and the three
 *    `healthbar:*` were direct calls** to functions the engine already owns — `discoverServices`,
 *    `replayRequest`, `isRunning`/`getPort`/`getServerError`, and `miscCommands.ts`'s workspace file IO.
 *    The commands are registered by `registerServerCommands()` / `registerMiscCommands()`.
 *
 * **The one channel that owned a step the engine did not perform was `entity:load`**, and it is why
 * this group began with a **fix-first** commit rather than a deletion (`174a01d`). Its body applied
 * `toProtocolKind()` — the map whose only non-identity entries are `rules → proxyRules` and
 * `sockets → wsConnections` — and the client twin forwarded the caller's `kind` verbatim. Two panels
 * pass the engine-internal names (`packages/gui/src/panels/ProxyRulesPanel.tsx`'s `"rules"`,
 * `packages/gui/src/panels/WebSocketsPanel.tsx`'s `"sockets"`) and `EntityLoadParams.kind` is the strict
 * `EntityKind` enum, so both were being answered `BAD_REQUEST` and both wrap the call in
 * `.catch(() => {})` — the failure surfaced as "the entity has no fields". Deleting the channel first
 * would have destroyed the only correct implementation of a live behaviour, exactly as group 1 would
 * have for the TLS paths. `entity:setEnabled`'s `invalid_kind` guard travelled with it for the same
 * reason, even though it is unreachable today: `window.ts:41` declares
 * `Promise<{ok: boolean; error?: string}>` and `store/coreCommands.ts:24-28` records the guard as
 * client-facing behaviour. Both now live in `packages/client/src/index.ts`, pinned by two cases in
 * `packages/client/tests/client.test.ts`.
 *
 * The `const ctx = { bus }` that sat here died with the channels — the registry calls carry their own
 * context, which is the whole reason `CommandContext` exists.
 *
 * ## The five commands that used to be registered here (P6 §5c, moved 2026-09-19)
 *
 * `config.get`, `env.setActive`, `workspace.setActive`, `entity.load` and `entity.setEnabled` were
 * registered **at module scope in this file**. That made them invisible to a containerised engine —
 * the implementation lived in a file only the Electron shell has, so a headless host answered
 * `UNKNOWN_COMMAND` for commands the protocol declares and `@bifurc/client` advertises — and it made
 * the registry-coverage ratchet blind, because in *this* process the shell had registered them.
 *
 * They now live in `packages/engine/src/store/coreCommands.ts` behind `registerCoreCommands()`, called
 * from `src/ipc/handlers.ts` alongside the other `register*Commands` modules. Every body moved
 * verbatim: the adapters here were already thin `commandRegistry.invoke()` delegations, so the wire
 * behaviour is the same code reached through a different door. A suite that registers handler *groups*
 * without running `registerIpcHandlers()` must call `registerCoreCommands(commandRegistry)` itself —
 * that is the cost of an explicit call site, and it is the point: a module side effect cannot be seen
 * by a ratchet, an explicit call can.
 *
 * ## Why the file outlives the group
 *
 * Because `environments` is a **kind**, not a channel — and a kind registration is the shell declaring
 * which entities it serves, which is what `registerCrudKinds()` and its three siblings do. It is not a
 * reduction target and never was. `plan/07` §5b-sedecies predicted "every remaining group is a
 * whole-file deletion"; that prediction is **wrong for this file for the same reason it was wrong for
 * `systemHandlers.ts`** — the file holds a non-channel registration. The seventeen *channels* are gone;
 * the file is reduced. Folding this one call into `crudKinds.ts` is a **merge**, a different change
 * with a different blast radius, and it is recorded as a follow-up rather than smuggled in here.
 */
import { registerCrudKind } from "../entityCrudCommands";
import { Environment } from "../config";

export function registerCoreKinds() {
  // `environments` — the last `EntityKind` still holding out of the CRUD collapse (see the
  // "What's left" note in `plan/03-phase-2-engine-extraction.md` work item 7). It fits
  // `CrudFactoryOpts` exactly like `mappings` does (flat storage, no folders, no enabled
  // state) plus one thing none of the twelve original CRUD kinds needed: a create-gate check
  // (`gateKind`, consulted by `entity.create`'s registered handler) and two environment-only
  // delete-time quirks (the "__global__" guard and the active-environment reset), both now
  // handled inside the engine's `entityCrudCommands.ts`, keyed off `opts.kind === "environments"`.
  registerCrudKind<Environment>({
    kind: "environments",
    configKey: "environments",
    isFlat: true,
    gateKind: "environment",
  });
}
