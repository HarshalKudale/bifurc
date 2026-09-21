# Bifurc — Project Memory

**Read `TRAPS.md` first** (`.workbuddy-ai/memory/TRAPS.md`) — failure modes that cost real time and are not
discoverable from the code. Status → `plan/README.md`; the executor pivot → `plan/14`; suites →
`TESTING.md`; the P6 / §5c / §5b ledgers → `plan/07`; procedures → the `bifurc-*` skills.

## Layout

- **Flat repo — the app IS the root.** `workspaces: ["packages/*"]`; **never `cd bifurc`**.
- **App runs `packages/*/dist/`; tests run `src/`** → **`npm run build:packages` after ANY package edit.**
  The vitest alias covers `@bifurc/engine/*` **only**; `@bifurc/protocol`/`@bifurc/client` resolve to built
  `dist/`, so an un-rebuilt edit is **silently untested** — `npm test` doesn't build.
- **No renderer edits P1–P6**; `window.api` byte-identical.
- **Baseline: 101 files / 2,500 tests — 2,459 passed, 40 skipped, 1 failed** (`soap.execute`,
  environmental, `tests/spike/protocolPoc.test.ts`). A *delta* is the signal, the 1 is not — **name the
  mechanism** for any delta, never wave at the number.

## Phase state

- **P0–P5 done.** P4 8/10 (extension release + an audit record — neither code here); P3 item 5 open as *UI* →
  P7. **P5's spec is `packages/client/src/surface.ts` (144 keys); `src/preload.ts` is the authority, not
  `renderer/types/window.ts`.**
- **P6 done in scope** — 144/144 keys classified, zero `SPLIT`; all 51 §5c commands moved and statically
  guarded. `registerIpcHandlers()` is **reduced, not deleted**; `registerClientHandlers()` sits beside it
  (`src/main.ts:294`). **Criterion 2 ("11 e2e specs pass, unmodified") is CARRIED** — owner's terminal only;
  re-measured 2026-09-20 = the documented 46 failed / 0 passed.
- **P6 stage 3b COMPLETE — 57 of 57 channels** (`plan/07` §5b-sexies/§5b-septies). Slice 1 (9): protocol
  passthroughs, 20 call sites re-pointed at `commandRegistry.invoke()`. Slice 2 (48):
  `src/ipc/handlers/entityCrudFactory.ts` **deleted**; callers call the engine's `registerCrudKind()` /
  `registerSimpleKind()` directly. **Split-then-delete was necessary, not optional** — the wrapper also did
  the container insert, so deleting it outright breaks all 16 kinds with the misleading *"not routed through
  the CommandRegistry yet"*. −18 tests vs stage 3a: `handlers.test.ts` generates **one `it` per
  `expectedChannels` entry**. `*Handlers.ts` → `*Kinds.ts` for soap/graphql/grpc; **"six caller files"
  corrected to five**.
- **The reduction is classified — `plan/07` §5b-decies. 93 literal channels = 18 live + 75 dead**, decidable
  because `window.api` *is* the client (`src/preload.ts:110`) and `packages/client/src` names no channel;
  only preload's `local` block's 18 are reachable. `clientHandlers.ts` **17 live / 0 dead → untouchable**;
  every other file all-dead except `systemHandlers.ts` (`app:checkUpdate`). No `ipcMain.on(` in `src/`.
- **The risk criterion is `owns a step the engine does not perform`** — §5b-decies said `dialog.*`, which was
  **one channel short** (§5b-undecies). `tls:generate` owns a *path*. That mattered: `tlsGenerate` /
  `tlsImportCert` / `tlsImportKey` were routed as 1:1 `transport` calls in step 3b-1 and had been **silently
  dropping `certPath` / `keyPath` / `path` ever since**, so the Settings panel's Generate and Import buttons
  were no-ops while the CA sat on disk. Fixed 2026-09-21 by reclassifying them `shim` (`transport` 70 → 67,
  `shim` 3 → 6) and reading the paths from the `tlsCertStatus` local hook.
- **The reduction's first group is done: `tlsHandlers.ts` deleted** (§5b-duodecies). Suite delta **0** — a
  *re-point*, because `expectedChannels` never listed these six. **Inventory re-measured 2026-09-21
  (`ipcMain.handle(` call sites): 87 literal channels in the 9 handler files = 18 live + 69 dead → 69 to go
  across 8 files**, one handler group per commit. §5b-decies' "68" is a *different partition* from its 75
  dead (it excludes the 7 dialog-owners), so don't reconcile them. In this repo **never `git rm`** — it
  deletes all of `src/`; use `rm` + `git add` (TRAPS.md). Procedure → the `bifurc-dead-channel-reduction`
  skill.

## The executor pivot — `plan/14` (D10–D17), planned, no code written

**The deployable unit is a WORKSPACE, not the engine.** `plan/14` is the authority; D10–D17 in
`plan/00-decisions.md`. Workspace = executor (D10) · N per host via `WorkspaceRuntime` (D11) · the desktop
adopts the same runtime (D12) · one RPC endpoint per host, `workspaceId` in params (D13) · **P6 work item 2
(the process split) downgraded** (D14) · CLI parked (D15) · **proxy base port `80` on win32 / `8080`
elsewhere, counting up** (D16) · **blob staging namespaced per workspace** (D17). `@bifurc/engine` does
**not** move.

- **§12 — the P0–P6 reconciliation.** One of six phases was wrong: §2.2's "blob staging is already
  per-workspace" is **false** (`blobRoot()` = `<dataDir>/blobs`,
  `packages/engine/src/blob/store.ts:140-142`; `blob.*` carries no `workspaceId`,
  `packages/protocol/src/commands/blob.ts:106,140,159,181`). **D17 resolves it.** Also: auth is **already
  built** (`transport/auth/{token,scopes,authenticated}.ts`) so WI 3 needs none; `createEngine()`
  (`packages/engine/src/index.ts:210`) holds nearly all of WI 0's inventory.
- **Three traps:** the port must NOT live in `workspace.json` (git-tracked → syncs → collides);
  **`config.save` must be SPLIT, not deleted**; allocation must come from the real `listen()`, **never**
  `portUtils.checkPortInUse` (probe-then-bind is a TOCTOU race). Leave `store/paths.ts` and
  `applications/processSpawner.ts` singletons — `bundle: false` keeps them single.
