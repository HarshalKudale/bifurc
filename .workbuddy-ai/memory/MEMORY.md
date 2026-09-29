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
  **fails every case** (10 individual tests; the old "4/4" counted top-level *groups*): `git push` here reports
  success **without writing the remote's refs** (same shell property as
  `git fetch`/`refs/remotes/**` in `~/.workbuddy-ai/MEMORY.md`). The safe-delete shim (`EBUSY … rmdir` on
  `%TEMP%`) makes `gitStore.test.ts`'s `beforeEach` time out (**24/24 alone**) and leaves **four integration
  suites failing teardown only** — `fileOps`, `importExport`, `importExportBlob`, `importExportFormats`
  (`Hook timed out in 20000ms` in `afterAll`, all 116 tests pass). A suite-level `FAIL` beside
  `Tests N passed` is this, not a regression.
- **Last full measurement 2026-09-29: 2,742 tests / 128 files** (unit 98 / 2,281 incl. 40 skipped;
  integration 30 / 461). **Through engine-layout phase 7 it was 2,737 / 127; phase 8 added exactly +1 file /
  +5 tests — the layout ratchet — and that delta is by design.** 11 failures, **all environment**:
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

## Engine module layout — DONE, and the layering ratchet is CLEAN (branch `engine_modules`)

`Engine_Module_Plan.md` reorganised `packages/engine/src` into feature modules behind a layering rule.
**Phases 0–8 committed**: `c531c7c` plan/map/codemod · `16161ad` p1 `kernel/`+`utils/` · `083a026` p2 rebind
fan-out · `99d1444` p3 `store/` split · `1f748b7` p4 proxy family (14) · `8bedb72` p5 protocol verticals (6) ·
`1713dec` p6 `gitops/` (12) · `069fe87` p7 leftovers (44 renames + 4 added) · `fbb6fc7` p8 the ratchet ·
`7639458` the `kernel/eventBus` fix · `74bb223` + `206d34f` the layout cleanup. **The plan's move counts
were 2 right / 5 wrong** — measure, never trust the row.

**⚠ The layering rule is §6's per-module table, NOT §5.1's blanket ban** (they contradict; §6 governs,
decided 2026-09-29). **`tests/engine/layout.test.ts` enforces it** — 5 assertions, teeth verified by
mutation — and **`ALLOWED_VIOLATIONS` is now `{}`**: all 24 violations the ratchet found are FIXED
(24 → 22 → 0), each deletion forced by the staleness assertion. Exemptions: `import type` (erased) and
**the two SEAM modules** (`runtime/instances` — §4.1's fan-out, 20 edges; `runtime/entityStatus`).

**The placement rule: "two features share it" is NOT a placement rule — what a module READS is.** Stateless
+ shared → extract **down** into `utils/`. Reads a file/config/sibling → it belongs in the layer it reads
from, however many callers it has (that is why `blob/` → `workspace/blob/` and `gitops/gitStore.ts` →
`workspace/gitStore.ts`). **And before choosing a destination, count the CALLERS' modules** — a function's
own imports do not decide its home. **§5.2's ESLint half is impossible: this repo has no ESLint.** Tests
**do not** mirror `src/` (TESTING.md §3.5). **Procedures → skill `bifurc-engine-layout-phase`** (the
codemod's six blind spots + the Windows case-only-rename trap, the resolve-every-pointer sweep, the
per-phase loop, and the cleanup's two traps).

- **⚠ The map's `from` fields deliberately keep the OLD paths** — they are the codemod's input, not a
  description of the tree, and three `to` fields were moved *again* by the cleanup. `--check` drift is
  expected; the codemod's header and the map's `corrections` both say so.
- **The GUI has its own unrelated `reloadConfig`** (a UI callback in `useCaptureActions.tsx` etc.) — never
  touch it in this migration.
- **`packages/**/*.test.ts` IS in the `unit` project, and `packages/engine/tsconfig.json` covers
  `packages/engine/tests/`.**
- **⚠ A test that PINS a split must move with it.** Splitting `fileops/` broke 10 tests in
  `fileOps.integration.test.ts` because two assertions name *which module registers which command* and the
  harness had to register the new owner.
