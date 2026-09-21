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
- **Baseline: 101 files / 2,492 tests — 2,451 passed, 40 skipped, 1 failed** (`soap.execute`,
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
- **The reduction — `plan/07` §5b-decies (inventory) / §5b-undecies (criterion) / §5b-duodecies (group 1) /
  §5b-terdecies (group 2) / §5b-quaterdecies (group 3) / §5b-quindecies (group 4) / §5b-sedecies (group 5).**
  Measured 2026-09-21 (`ipcMain.handle(` **call sites**): **55 literal channels in the 6 handler files = 18
  live + 37 dead → 37 to go across 4 files**, one handler group per commit. Decidable by construction:
  `window.api` *is* the client (`src/preload.ts:110`) and `packages/client/src` names no channel, so only
  preload's `local` block's 18 are reachable. `clientHandlers.ts` **17 live / 0 dead → untouchable**;
  `systemHandlers.ts` is now **1 live / 0 dead and out of scope** (`app:checkUpdate` is client-local, not
  legacy), so **all four remaining files are all-dead** and the rest are whole-file deletions:
  `coreHandlers.ts` (17, next), `applicationHandlers.ts` (10), `runnerHandlers.ts` (6),
  `importExportHandlers.ts` (4). §5b-decies' "68" is a **different partition** from its 75 dead (it excludes
  the dialog-owners) — do not reconcile them. **Read the inventory in the *latest* §5b-* section, not the
  nearest one** — each section's table is a dated reading.
- **The dialog-owner risk subset is 3, not 6** — three sections carried "6" forward without re-deriving it.
  The real sequence is **8 → 5 → 4 → 4 → 4 → 3** (§5b-undecies widened §5b-decies' 7 by adding
  `tls:generate`; group 1 took 8 → 5, group 2 → 4, group 5 → 3). What remains: `runner:exportReport`,
  `importExport:export`, `importExport:preflight`. **Re-derive subset counts with `grep -n "dialog\."` over the
  remaining files, never from the previous paragraph** — the four ledger sections agreed with each other and
  were all wrong (TRAPS.md, "A count written into prose is never re-derived").
- **The criterion is `owns a step the engine does not perform`** — §5b-decies said `dialog.*` and was one
  channel short. `tls:generate` owns a *path*, and that mattered: `tlsGenerate` / `tlsImportCert` /
  `tlsImportKey` were routed as 1:1 `transport` calls in step 3b-1 and had been **silently dropping
  `certPath` / `keyPath` / `path` ever since** — the Settings panel's Generate and Import buttons were no-ops
  while the CA sat on disk. Fixed by reclassifying them `shim` (now `transport` 67 / `shim` 6) and reading the
  paths from the `tlsCertStatus` local hook. **Never cast the object you build** — cast only the boundary
  `unknown`. A *payload-shape* difference counts too: `folder:add`'s channel **spread** its argument where the
  client picks two fields — inert only because the renderer contract excludes `workspaceId`, so check the
  contract, not the intuition.
- **Four groups deleted:** `tlsHandlers.ts` (6, delta 0 — *re-point*), `syncHandlers.ts` (19, delta −5 —
  *generated list*), `folderHandlers.ts` (4, delta −3 — **both at once**, read per-file), `crudHandlers.ts`
  → **`crudKinds.ts`** (5, delta 0 — **never had any**; a rename, so `git ls-files src/` is unchanged at 20).
  `expectedChannels` in `handlers.test.ts` is the only thing that turns a channel name into a test (one `it`
  per entry), so **delta 0 has two causes and the count cannot separate them** — re-pointed cases (`tls`) or
  no cases at all (`crud`).
  **A suite that drives a channel *by name* fails loudly (`No handler registered`) — "failures up" means
  RE-POINT it (drive the command), not delete the test**; `handlers.test.ts`'s `entity.*` / `history.*` /
  `folder.*` helpers are the idiom, and a many-call-site suite gets a shared test harness
  (`tests/integration/deletedChannelHarness.ts`) rather than N rewrites — but that harness is **not a second
  contract**. The spike's retired-channel fixture grows only with a **driven** channel (7 of 10 now; group 4
  added none).
- **A file's name is a claim, and the reduction keeps having to fix the ones that stop being true.**
  `*Handlers.ts` → `*Kinds.ts` for soap/graphql/grpc in stage 3b, and now `crudHandlers.ts` → `crudKinds.ts`
  — §5b-octies had *deliberately* kept the name because the file registered "six kinds **and** five live
  `webhook*` channels", and deleting the five expired the premise. When you rename, fix the **comments at the
  call sites too**: five of them said the call "wires the *channels* and the per-kind options", false in its
  first half. A stale comment on a *renamed* function is worse than on a deleted one — the deleted one fails
  loudly, this one reads as current.
- **A guard whose floor is a file count fails as the tree shrinks — and the tree shrinking is the work.**
  `handlers.test.ts`'s §5c anti-vacuity check was `src/` > 20 `.ts` files; the reduction hit exactly 20.
  Now expressed as **presence** (`preload.ts`, `ipc/handlers.ts`, `ipc/rpcBridge.ts`). **`V1_COMPANION_ACTIONS`
  / `EXTENSION_FROZEN_COMMANDS` are NOT channel lists** — same `kind:verb` strings, separate surface with its
  own dispatch (`legacyCompanion.ts`), so a grep for a channel name will wrongly suggest it is live.
- In this repo **never `git rm`** — it deletes all of `src/`; use `rm` + `git add` (TRAPS.md). Procedure →
  `bifurc-dead-channel-reduction`.

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
