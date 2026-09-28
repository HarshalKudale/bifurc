# Bifurc — Project Memory

**Read `TRAPS.md` first.** Authorities: state → `plan/README.md` §Status · decisions → `plan/00-decisions.md` ·
suites → `TESTING.md` · **ports → `plan/17` §1** · edge/mapping → `plan/15` · procedures → the `bifurc-*` skills.
**A pointer, not a record** — anything already stated in full over there is not repeated here.

> **⚠ ONE BUILD TARGET SINCE 2026-09-26: the Electron desktop app.** `web/` and the container image are
> **deleted**; P7/P9 decommissioned, P16 never built. **`npm run dev` / `test:e2e` are the user's to run** — an
> agent-shell failure is evidence about the shell, not the product.

## Layout

- **Flat repo — the app IS the root**; the parent dir's `.git` is **empty** and is not a publish path. Branch
  `standalone-engine`. **`git rm`, `git stash`, `git checkout -- <file>` are ALL destructive** — `cp` to `/tmp`
  first. **⚠ `rm` is a shim and `git rm` under `scripts/` wipes the whole directory** — use
  `python -c "import os; os.remove(...)"`, small batches, verify each.
- **App runs `packages/*/dist/`; tests run `src/`** → `npm run build:packages` after a package edit (only
  `@bifurc/engine/*` aliases to `src`; the rest resolve `dist/`, so an un-rebuilt edit is **silently
  untested**). Needs `CODEBUDDY_SAFE_DELETE_ENABLED=0`. **Directory renames blocked**; a *file* `git mv` works.
  **⚠ One `Edit` per file per message** — two same-file edits silently revert each other.
- **⚠ `build:packages` omits `@bifurc/gui`** — no `dist`; `build:renderer` bundles its `src` **as source**.
  **Vitest resolves `@/…` through its own alias plugin, not the bundler's** — a green test is not proof the
  bundle resolves. Build it.
- **⚠ No tsconfig covers `tests/`** (but `npm run typecheck:e2e` does cover `e2e/`) — `npm run typecheck` is
  green while `tests/` is broken. Run the suite.
- **⚠ `package-lock.json`'s workspace set is a coupling** — `npm ci` fails both ways (a manifest the lockfile
  lacks; a lockfile entry whose directory is gone → npm asks the public registry, 404). Check with
  `npm ci --dry-run`, never by reasoning.

## Measuring the suite

- **A *delta* is the signal; name its mechanism.** Baseline **re-measured, never carried forward**. **There is
  no standing *product* failure — every failure is a flake to identify.** Judge a mutation by **exit code, not
  count**. ⚠ The qualifier is new (2026-09-28): there **are** standing *environment* failures — see the
  `gitRemote` entry below — and they are deterministic, not flakes.
- **⚠ Freeze the tree while a run is in flight.** **⚠ A single-project run is NOT the full run** — reconcile
  **unit + integration = the full total**.
- **⚠ A fixture's `cleanup()` is not local** — a second fixture un-points the store for every case after it.
  **⚠ A new suite that binds ports degrades every "scan lands on `base + N`" assertion** — remedy: the shared
  `holdBaseWithFreeSuccessors()`.
- **⚠ `gitStore.test.ts`'s `beforeEach` can time out at 10 s** under the safe-delete shim (`EBUSY … rmdir` on a
  `%TEMP%` dir, `node-safe-delete-shim.cjs`). Took one unit case down 2026-09-28; the file passes **24/24 in
  isolation**. Environment, not product.
- **⚠ Same shim, same signature, in `afterAll`: four integration suites fail *teardown only*.** Measured
  2026-09-28 — `fileOps`, `importExport`, `importExportBlob`, `importExportFormats` each report
  `Hook timed out in 20000ms` in their `afterAll` (the `f.cleanup()` + `fs.rmSync(outDir)` block), while
  **all 116 of their tests pass**, in a full run *and* alone. So a suite-level `FAIL` with `Tests N passed`
  beside it is this, not a regression. `CODEBUDDY_SAFE_DELETE_ENABLED=0` does not help — that is the
  evidence it is contention rather than the shim.
- **⚠ `tests/integration/gitRemote.integration.test.ts` fails 4/4 in this sandbox, deterministically.**
  `remoteHead()` shells out to `git --git-dir <remote> rev-parse refs/heads/<branch>`; `git push` here
  reports success **without writing the remote's `refs/heads/*`**, so the value is `null`. **Reproduces with
  the file run alone** (with `gitSync` passing beside it), so it is *not* a flake and *not* attributable to
  any change — and `grep -rn "packages/gui" tests/integration/` matches only a comment, so no integration
  test can even load the GUI. Same shell property as the `git fetch` / `refs/remotes/**` rule in
  `~/.workbuddy-ai/MEMORY.md`. **Do not try to fix these.**

**Current: 2,677 tests / 120 files — 2,217 unit + 460 integration, 40 skipped; 0 product failures
(2026-09-28, after the right sidebar gained its variables face).** Unit **90 files** (2,176 pass + 40 skipped
+ 1 hook-timeout flake), integration **30 files** (456 pass + 4 `gitRemote` sandbox failures + 4 teardown-only
suite timeouts).
**e2e: 11 specs, all rewritten 2026-09-28 for the autosave move — the old 46/46 figure is stale and wants an
owner run.** (The two non-hermetic tests `engineSmoke` / `hostStartup`, and the `forwardProxyPort` fix, are
recorded in `TESTING.md`.)

## The autosave move (2026-09-28)

- **Create is the only explicit write.** A draft has no server id, so a **Create** button writes it; once it
  exists every change autosaves **1 s after the last keystroke**. Sync / Revert / Delete stay intentional.
  `Ctrl+S` = `saveNow()` = "flush now, do not wait for the debounce".
- **One hook: `packages/gui/src/hooks/useDebouncedAutosave.ts`.** Load-bearing: **silent arming** (the first
  snapshot seen while `enabled` becomes the baseline without writing, so draft → saved id cannot write straight
  back), **baseline advances only on success**, **unmount flushes**. `flush()` cancels the pending timer, so
  `Ctrl+S`/Sync cannot write the same edit twice. **The effect's returned cleanup *is* the debounce reset** — an
  extra `clearTimer()` is redundant, and removing it changes nothing (mutation-tested).
- **⚠ A store echo re-seeds local form state and clobbers an in-flight keystroke.** `ProxyRuleDetailsPanel`
  swallows it (`swallowNextResetRef`); `AddServiceModal` seeds once per *opening* (`seededForRef` keyed on
  `id ?? "__new__"`).
- **⚠ Mappings rows and Health Bar cards are editors now** — real `<Input>`s, so **they carry no text and
  `tr:has-text(...)` cannot find them**; match on `input[value="…"]`. Mappings' modal is create-only; Health
  Bar's edits on change and confirms with `Done`.
- **e2e:** `AUTOSAVE_DEBOUNCE_MS` + `awaitAutosave(page)` in `e2e/helpers/index.ts` mirror the 1 s delay — two
  files in different packages, kept in step by comment only.

## The right sidebar is multi-face (2026-09-28)

- **`components/sidebar/RightSidebar.tsx` owns the width, and only it.** `.right-sidebar` (renamed from
  `.history-sidebar`) animates it; a closed sidebar is **zero-width, not unmounted**. Two faces —
  `"history"` (`HistorySidebar`, now **content-only**: no `open` prop, returns a fragment) and `"variables"`
  (`VariablesSidebar`). **⚠ `HistorySidebar` now re-mounts per open** — its load effects lost their `!open`
  guard because *being mounted is being open*.
- **⚠ The mode IS the open state** — `RightSidebarMode | null` in `App.tsx`. A separate `open` boolean
  would be a second source of truth and could express *open with no face*. Both toggle rules fall out of it:
  press the showing face's control to close, press the other to swap in place. `historyOpen` is derived and
  still threaded to the panels unchanged.
- **The variables face keeps two groups on purpose** — `mergeEnvVars` overlays env on `__global__`, so a
  shadowed global is marked `overridden` rather than dropped; a merged list hides the one fact you need when
  a substitution surprises you. Title-bar toggle sits beside `EnvSelector`; `aria-pressed` carries the state.
  Pinned by `tests/renderer/rightSidebar.test.tsx` (11 cases, mutation-checked).

## Phase state

**Live: P18** — enterprise SSO / org workspaces / permission gating (opened 2026-09-26; **WI 0 done**). **P17**:
only item 4's Electron half open. **P0–P6, P14 done. P7, P9, P16 decommissioned.** `packages/gui` still exports
`<App />`; `renderer/` is its only consumer. **⚠ P18 adds a SECOND authorization axis — per workspace
(`PermissionContext`), not per session (`transport/auth/scopes.ts`). Do not merge them** — see
`plan/00-decisions.md` D19–D21. Local actor ⇒ allow-all; org actor ⇒ fail closed with its **own** error code.

### Invariants that are *traps*, not documentation

- **⚠ `bindEdgeInstance()` has no lazy default** — unbound is **503**, not a fallback. **Three compositions bind
  the edge** — `createEngine()`, `runtime/host.ts`, and **`src/main.ts`, which never calls `createEngine()`**;
  grep the **factory**, not the entry point. A `config.save` **re-binds** edge, proxy and webhook (WI 7).
- **⚠ Since 2026-09-24 the engine is NOT cloud-deployable: `--domain` is REMOVED** and is a **fatal
  unrecognised flag**. Survivors: the bind opt-ins and `hello`'s `hostDomain` — which **is** the wire field.
- **⚠ `packages/gui` is NOT in `npm run typecheck`.** Sole runner: `npm run typecheck --workspace @bifurc/gui`
  (**33 pre-existing errors across 21 files**, all present at HEAD). **The instrument is *which files*, not the
  count.** **`tsc` after any type-level change — vitest does not typecheck.**
- **⚠ The browser's transport is auth-wrapped; Electron's is not** — `hello` is mandatory before the first real
  request. **`build:main` after any `src/preload.ts` change.**
- **Mocks are REST-only**; the four *request* kinds and their schemas all stay. **Client surface = 132 keys**
  (`surface.ts` is the spec; `BifurcApi` declares 128 — the gap stays 4).
- **The companion is RETIRED as a bridge**; the extension becomes an ordinary authenticated client — engine half
  DONE, extension half deliberately NOT. **`onCompanionRefresh` was KEPT** — misleadingly named but *live*.
