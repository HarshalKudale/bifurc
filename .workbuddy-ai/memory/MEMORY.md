# Bifurc — Project Memory

**Pointers, not records.** Authorities: state → `plan/README.md` §Status · decisions →
`plan/00-decisions.md` · suites → `TESTING.md` · ports → `plan/17` §1 · edge/mapping → `plan/15` · **the
numbered trap registry (239 KB, ~2400 entries) → `.workbuddy-ai/memory/TRAPS.md`** · procedures → the
`bifurc-*` skills · per-day detail → `.workbuddy-ai/memory/YYYY-MM-DD.md`. **Grep TRAPS.md before
diagnosing anything** — most traps are already recorded there.

> **⚠ ONE BUILD TARGET: the Electron desktop app.** `web/` + the container image are **deleted** (P7/P9
> decommissioned, P16 never built). **`npm run dev` / `test:e2e` are the user's to run** — an agent-shell
> failure is evidence about the *shell*, not the product.

## Build & layout — the rules that bite immediately

- **Flat repo — the app IS the root.** **`git rm`, `git stash`, `git checkout -- <file>` are ALL
  destructive**; `cp` to `/tmp` first. **⚠ `rm` is a shim, and `git rm` under `scripts/` wiped the whole
  directory** → use `python -c "import os; os.remove(...)"`, small batches, verify each, **commit first**.
- **App runs `packages/*/dist/`; tests run `src/`** → `npm run build:packages` after a package edit (only
  `@bifurc/engine/*` aliases to `src`; the rest resolve `dist/`, so an un-rebuilt edit is **silently
  untested**). Needs `CODEBUDDY_SAFE_DELETE_ENABLED=0`. **Directory renames are blocked**; a *file* `git mv`
  works. **⚠ One `Edit` per file per message** — two same-file edits silently revert.
- **⚠ `build:packages` omits `@bifurc/gui`** (no `dist`; `build:renderer` bundles its `src`). **Vitest
  resolves `@/…` through its own alias plugin, not the bundler's** — a green test is not proof the bundle
  resolves.
- **⚠ No tsconfig covers `tests/`** (but `typecheck:e2e` covers `e2e/`), so `typecheck` is green while
  `tests/` is broken. **`packages/gui` is not in `npm run typecheck` either** — sole runner
  `npm run typecheck --workspace @bifurc/gui` (**~33 errors, all pre-existing at HEAD — the instrument is
  *which files*, not the count**). `tsc` after any type-level change; vitest does not typecheck.
- **⚠ `package-lock.json`'s workspace set is a coupling** — check with `npm ci --dry-run`, never reasoning.

## Measuring the suite

- **A *delta* is the signal; name its mechanism.** Baseline **re-measured, never carried forward**. Judge a
  mutation by **exit code, not count**. **Freeze the tree while a run is in flight.** **⚠ A single-project
  run is NOT the full run** — reconcile **unit + integration = the full total**. `getFreePort()` returns a
  **reservation, not a guarantee** — de-dupe every port after the first (TRAPS 112).
- **⚠ Standing ENVIRONMENT failures — deterministic, NOT to be fixed.** `gitRemote.integration.test.ts`
  **4/4 fail**: `git push` here reports success **without writing the remote's refs** (same shell property as
  `git fetch`/`refs/remotes/**` in `~/.workbuddy-ai/MEMORY.md`). The safe-delete shim (`EBUSY … rmdir` on
  `%TEMP%`) makes `gitStore.test.ts`'s `beforeEach` time out (**24/24 alone**) and leaves **four integration
  suites failing teardown only** — `fileOps`, `importExport`, `importExportBlob`, `importExportFormats`
  (`Hook timed out in 20000ms` in `afterAll`, all 116 tests pass). A suite-level `FAIL` beside
  `Tests N passed` is this, not a regression.
- **Last full measurement 2026-09-29: 2,737 tests / 127 files** — **re-confirmed unchanged after engine-layout
  phase 3** (unit 97 / 2,276 incl. 40 skipped; integration 30 / 461). 11 failures, **all environment**:
  `gitRemote` 10 (sandbox `git push` writes no remote refs) + **5 hook timeouts in the
  `tests/store/`+`tests/sync/` family incl. `serverUtils`** (varying set — the shim's contention signature;
  5 this run, 1 in phase 2's), plus **four teardown-only** suite `FAIL`s (`fileOps`, `importExport`,
  `importExportBlob`, `importExportFormats`, `Hook timed out in 20000ms` in `afterAll`, all their tests pass).
  **⚠ Never blame a change for a count in that git/temp family — check alone first.** **e2e: 11 specs rewritten
  2026-09-28 — the old 46/46 figure is stale and wants an owner run.**

## Phase state

**Live: P18** — enterprise SSO / org workspaces / permission gating (**WI 0 done**). **P17**: only item 4's
Electron half open. **P0–P6, P14 done. P7, P9, P16 decommissioned.** `packages/gui` exports `<App />`;
`renderer/` is its only consumer. **⚠ P18 adds a SECOND authorization axis — per workspace
(`PermissionContext`), not per session (`transport/auth/scopes.ts`). Do not merge them** (D19–D21).

## Active work: engine module layout — branch `engine_modules`

`Engine_Module_Plan.md` (repo root) reorganises `packages/engine/src` (140 files / 23,443 LOC) into feature
modules behind an enforced layering rule. **Phases 0, 1, 2 and 3 are DONE and committed** (`c531c7c`
plan/map/codemod, `16161ad` phase 1 — `kernel/` + `utils/`, `083a026` phase 2 — the rebind fan-out,
`99d1444` phase 3 — the `store/` split). **Phase 4 (proxy family) is next and not started.**

### The engine-layout codemod — see skill `bifurc-engine-layout-phase`

**Phases 0–3 done** (`c531c7c` plan/map/codemod · `16161ad` p1 `kernel/`+`utils/` · `083a026` p2 rebind
fan-out · `99d1444` p3 `store/` split). **Phase 4 (proxy family) next.** The codemod's six blind spots, the
resolve-every-pointer sweep, and the per-phase checklist live in that skill — **load it before starting a
phase** rather than re-deriving them.

State the skill does not carry:

- **`store/` now holds exactly 4 files** — `gitStore.ts` (phase 6) + `kinds/{graphql,grpc,soap}Kinds.ts`
  (phase 5). `subscription/` is gone. `store/` never fills again; it drains.
- **⚠ §7.1's counts were wrong** (529/104/82; only `from` counted). `plan/engine-layout-map.json` has the
  measured numbers, a **`phaseStatus`** block, and the ratified `store/workspace/*` → `workspace/fs/*`
  deviation. **⚠ The map's `from` fields deliberately keep the OLD paths** — they are the codemod's input,
  not a description of the tree. The map is not "wrong" when it names a moved file.
- **The GUI has its own unrelated `reloadConfig`** (a UI callback in `useCaptureActions.tsx` etc.) — never
  touch it in this migration.
- §7.3 mechanics: never `git rm`; the **stale-`dist/` trap** (`npm run clean:main` before the app build,
  `node scripts/pkg-check-reachability.mjs` after each phase — engine's own `dist/` self-cleans, so the gate
  is the app's root `dist/`). **`packages/**/*.test.ts` IS in the `unit` project, and
  `packages/engine/tsconfig.json` covers `packages/engine/tests/`.**
