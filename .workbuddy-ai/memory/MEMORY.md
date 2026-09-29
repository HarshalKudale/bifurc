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
- **Last full measurement 2026-09-28: 2,737 tests / 127 files** (re-measured; the recorded 2,677/120 was
  stale). 11 baseline failures, **all environment**: `gitRemote` 10 (sandbox `git push` writes no remote
  refs) + `serverUtils` 1 hook timeout, plus **four teardown-only** file failures (`fileOps`, `importExport`,
  `importExportBlob`, `importExportFormats`) whose tests all pass. **⚠ A *varying* count in the
  `tests/store/`+`tests/sync/` git/temp family is the shim's contention signature — check alone before
  blaming a change.** **e2e: 11 specs rewritten 2026-09-28 — the old 46/46 figure is stale and wants an
  owner run.**

## Phase state

**Live: P18** — enterprise SSO / org workspaces / permission gating (**WI 0 done**). **P17**: only item 4's
Electron half open. **P0–P6, P14 done. P7, P9, P16 decommissioned.** `packages/gui` exports `<App />`;
`renderer/` is its only consumer. **⚠ P18 adds a SECOND authorization axis — per workspace
(`PermissionContext`), not per session (`transport/auth/scopes.ts`). Do not merge them** (D19–D21).

## Active work: engine module layout — branch `engine_modules`

`Engine_Module_Plan.md` (repo root) reorganises `packages/engine/src` (140 files / 23,443 LOC) into feature
modules behind an enforced layering rule. **Phases 0, 1 and 2 are DONE and committed** (`c531c7c` plan/map/codemod,
`16161ad` phase 1 — `kernel/` + `utils/`, phase 2 — the rebind fan-out). **Phase 3 (`workspace/` split) is next
and not started.**

- **Phase 2 delivered `runtime/instances.ts` with TWO functions, not the one §4.1 sketched** (user-ratified):
  `notifyConfigChanged()` (the **reload** fan-out — proxy facade only; edge/webhook have nothing to reload)
  **and** `rebindRuntimePorts(incoming, prev)` (the **port** fan-out, moved verbatim out of `config.save`).
  Only `config.save` rebinds; `restartEdge`/`startWebhookServer` have **zero** other engine callers.
- **The real Phase 2 surface was 29 call sites / 21 files** — 17 × `importExport/importers/*`,
  `store/configCommands.ts` ×2, `store/coreCommands.ts` ×3, `store/entityCrudCommands.ts` ×3,
  `sync/syncCommands.ts` ×4, `startup.ts` ×1 (passed **as a value** to `setAutoSyncReloadFn`, whose seam type is
  `() => void`). *Not* the plan's 23. **⚠ The GUI has its own unrelated `reloadConfig` (a UI callback in
  `useCaptureActions.tsx` etc.) — never touch it in this migration.**
- **⚠ Measuring `reloadConfig` by `grep -c` over-counts ~2× (47 vs 29)** because docblocks name it constantly.
  Filter comment lines and the definition/interface forms. A blanket rename also rewrites *docblock mentions* —
  check each: some describe a **historical** name (leave those), some current behaviour (rename those).
- **Test seam:** `tests/ipc/handlers.test.ts` mocks **only** `notifyConfigChanged` at
  `@bifurc/engine/runtime/instances` (with `importOriginal` spread) and leaves `rebindRuntimePorts` **real**, so
  the port assertions still observe real calls against the listener mocks. `facadeBinding.integration.test.ts`
  keeps importing `reloadConfig` — it tests the proxy's own surface.

- **`scripts/engine-layout-codemod.mjs` is the instrument** — `--check` (report + assert totality),
  `--emit-map`, `--apply --phase N [--write]` (dry by default). Re-running an applied phase must report **0**.
  **It rewrites `@bifurc/engine/*` specifiers only** — engine-internal *relative* imports (what phase 2 was made
  of) are hand work.
- **⚠ The rewrite has TWO surfaces.** `@bifurc/engine/*` (536 refs / 102 files / 83 subpaths at last measure)
  **and** the engine's internal *relative* imports — and 194 of the 536 are dynamic `import()` / `vi.mock`, which
  fail at **RUNTIME**, so a `from`-only find/replace is silently wrong. Do not hand-edit these.
- **⚠ §7.1's counts were wrong** (529/104/82; only `from` counted). `plan/engine-layout-map.json` has the
  measured numbers and the one deviation (`store/workspace/*` → `workspace/fs/*`, not `workspace/workspace/*`) —
  **ratified 2026-09-29, keep it**.
- §7.3 mechanics: never `git rm`; the **stale-`dist/` trap** (`npm run clean:main` before the app build,
  `node scripts/pkg-check-reachability.mjs` after each phase); `vitest.config.ts`'s coverage `exclude` names
  three engine `types.ts` paths by hand. **`packages/**/*.test.ts` IS in the `unit` project, and
  `packages/engine/tsconfig.json` covers `packages/engine/tests/`.**
