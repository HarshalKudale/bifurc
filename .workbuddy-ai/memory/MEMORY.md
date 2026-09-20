# Bifurc — Project Memory

**Read `TRAPS.md` here first** — the failure modes that cost real time and are not discoverable from the
code. Then: status → `plan/README.md`; **the executor pivot + the P0–P6 reconciliation → `plan/14` §12**;
suites → `TESTING.md`; the P6 / §5c / §5b ledgers → `plan/07`; procedures → the `bifurc-*` skills.

## Layout

- **Flat repo — the app IS the root.** `workspaces: ["packages/*"]`; **never `cd bifurc`**.
- **App runs `packages/*/dist/`; tests run `src/`** → **`npm run build:packages` after ANY package edit.**
  The vitest alias covers `@bifurc/engine/*` **only**, so `@bifurc/protocol`/`@bifurc/client` resolve to
  built `dist/` — an un-rebuilt edit is **silently untested**; `npm test` doesn't build.
- **No renderer edits P1–P6**; `window.api` byte-identical. P6 is closed in scope.

## Baseline

**101 files / 2,514 tests — 2,473 passed, 40 skipped, 1 failed.** The 1 is `soap.execute`, environmental,
in `tests/spike/protocolPoc.test.ts`. A *delta* is the signal; the 1 is not — and **account for a delta by
naming its mechanism**, never by waving at the number.

## Phase state

- **P0–P5 done** (P4 8/10: the extension release and an audit record, neither code here; P3 item 5 open as
  *UI* → P7). P2 extracted the engine — 52 files, 4 layers, `createEngine()`. **P5's spec is
  `packages/client/src/surface.ts` (144 keys); `src/preload.ts` is the authority, not
  `renderer/types/window.ts`.**
- **P6 done in scope:** 144/144 keys classified, zero `SPLIT`; all 51 commands of the §5c pass moved and
  statically guarded. `registerIpcHandlers()` is **reduced, not deleted** — `registerClientHandlers()`
  already sits beside it (`src/main.ts:294`). **Criterion 2 ("11 e2e specs pass, unmodified") is CARRIED**
  — owner's terminal only; re-measured 2026-09-20 = the documented 46 failed / 0 passed.
- **P6 step 3 — stage 3b is 9 of 57 channels done** (`plan/07` §5b-sexies): the nine protocol-client
  passthroughs deleted, 20 test call sites re-pointed at `commandRegistry.invoke()`. The **48 generated**
  channels wait on **splitting `entityCrudFactory`** (`registerEntityCrudHandlers` also calls
  `registerCrudKind`, which fills the map `entity.*` dispatches through) + judging `handlers.test.ts`'s 75
  adapter cases per case.

## The executor pivot — `plan/14` (D10–D16), planned 2026-09-20, no code written

**The deployable unit is a WORKSPACE, not the engine.** `plan/14` is the authority; D10–D16 are in
`plan/00-decisions.md`. Workspace = executor (D10) · N per host via `WorkspaceRuntime` (D11) · the desktop
adopts the same runtime (D12) · one RPC endpoint per host, `workspaceId` in params (D13) · **P6 work item 2
(the process split) downgraded** (D14) · CLI parked (D15) · **proxy base port per environment — `80` on
win32, `8080` elsewhere, counting up** (D16). `@bifurc/engine` does **not** move.

- **§12 — the reconciliation with P0–P6.** Six phases audited; **one was wrong**: §2.2 claims blob staging
  is "already per-workspace" — **false**. `blobRoot()` is `<dataDir>/blobs`
  (`packages/engine/src/blob/store.ts:140-142`) and `blob.*` carries no `workspaceId`
  (`packages/protocol/src/commands/blob.ts:106,140,159,181`), pinned by `tests/blob/store.test.ts:62` for
  the Docker volume. Blob staging is a **third app-level thing** §2.3's table has no row for. **OPEN
  DECISION (§12.1):** namespace blobs per workspace (recommended) or declare them app-level; either way the
  root stays under `dataDir()`.
- **§12 also found:** auth is **already built** (`transport/auth/{token,scopes,authenticated}.ts`), so
  non-negotiable #2 holds and WI 3 needs no new auth — despite `plan/13`'s unticked auth items;
  `createEngine()` (`packages/engine/src/index.ts:210-259`) holds nearly all of WI 0's inventory; P5's
  144 keys survive D13 but its workspace-scoped **signatures** move.
- **Three traps in the plan:** the port must NOT live in `workspace.json` (git-tracked → syncs → collides);
  **`config.save` must be SPLIT, not deleted** (proxy-restart half leaves, companion half stays);
  allocation must come from the real `listen()`, **never** `portUtils.checkPortInUse` (probe-then-bind is a
  TOCTOU race). Leave `store/paths.ts` and `applications/processSpawner.ts` singletons — `bundle: false`
  keeps them single, and if the design needs a second instance the design is wrong.
