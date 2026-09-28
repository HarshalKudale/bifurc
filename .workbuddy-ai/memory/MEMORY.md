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

**⚠ The `0 product failures` claim above was wrong, and the figure has NOT been re-measured since the
uniform-git-tracking change (2026-09-28).** `tests/renderer/rightSidebar.test.tsx` →
`TitleBar — the variables toggle` was **2 standing failures at HEAD**: the test rendered `TitleBar` and
looked for the variables button, but `TitleBar.tsx` has no `variablesOpen` prop — the toggle and the
environment selector both live in `GlobalFooter`, and that is the decision (the toggle opens the expansion
of the selector's current value, so they belong in one row). **Fixed by retargeting the two cases at
`GlobalFooter`**; the file is **11/11** now, so its "11 cases, mutation-checked" claim is finally true.
**When a test names a component that does not carry the prop it looks for, suspect the test before the
component** — and check `git show HEAD:<file>` before assuming a recent change caused it. Measured after
the change: the three touched unit directories (`tests/sync`, `tests/store`, `tests/renderer`) are
**37 files / 542 tests, all passing**; `gitSync.integration.test.ts` **30/30**; `typecheck:renderer`
**20 errors**, every one on a line the change did not touch (was **38** at the first checkpoint, so 18
fewer).

## The uniform git-tracking layer (2026-09-28)

**One status→affordance decision, one handler, for every entity kind.** `lib/entityGit.ts` owns the
decision (`entityGitView`, `folderGitRollup`, `PANEL_ENTITY_KIND`); `hooks/useEntityGitActions.ts` owns the
writes (`publish` / `revert` / `publishPanel` — **two writers, not seven**); every surface renders from them.

- **⚠ The status model has TWO axes, and the second one is the bug that was reported.** `entitySyncStatus`
  is per-file vs HEAD; `workspaceGitState` (`{hasRemote, branch, ahead, behind}`) is vs the **remote**.
  A file committed but **not pushed** reads `clean` on the first axis, so every `status !== "clean"` gate
  offered it **no button** — it could not be pushed from its own row. Both axes arrive in one payload
  (`sync.getEntityStatus` → `{files, git}`, and the same `git` on the `sync.entityStatus` event).
- **⚠ `ahead` is measured LOCALLY against `refs/remotes/origin/<branch>` — there is no fetch.** It can be
  stale after someone else pushes; the remedy is the pull path, not a fetch here. A repo with no `origin`
  is **never** ahead, so a local-only workspace offers no Sync on a clean entity.
- **⚠ `git rev-parse --verify --quiet <missing-ref>` exits 0 WITH EMPTY OUTPUT on this git.** Testing ref
  existence by exit code silently failed, and the *next* line (`rev-list origin/<b>...HEAD`) threw into the
  outer catch — so a repo with a remote reported **no remote at all**. Test the **output**, not the exit
  code. (`tests/sync/gitState.test.ts` caught this; nothing else would have.)
- **⚠ `ALL_ENTITY_KINDS` / `FLAT_ENTITY_KINDS` in `store/workspaceFs.ts` are the one kind vocabulary.**
  Four copies had drifted; `statusTracker`'s had only requests/mocks/sockets, so a never-committed deleted
  `rule`/`webhook`/`graphqlRequest`/`mapping`/… had a pending-deletion record nothing read.
- **⚠ Flat kinds (`mappings`, `environments`) now record pending deletions too, and `loadConfig()` rebuilds
  their stubs.** Without it, deleting a *published* mapping removed the file, removed it from the config and
  recorded nothing — unrecoverable and unpublishable. **Corollary trap: `upsertNameEntry` must be guarded by
  `!isFlat` at all THREE sites** (`createEntityCore`, `updateEntityCore`, `discardChanges`) — a flat kind has
  no names index, so it wrote a stray `{kind}/names.json` that the next `git add -A -- {kind}/` would commit.
- **⚠ Revert has ONE implementation: `gitOps.discardChanges()`.** `publishService.restoreEntity()` was a
  second, never-called one that skipped the pending-deletion cleanup, the `names.json` re-sync and the cache
  invalidation — deleted. Do not add a third.
- **⚠ Reads fail open, writes fail closed.** `status === undefined` means *unknown*, not *clean*: no
  Publish/Sync/Revert, but History stays available (refusing it would make the button flicker on the refresh
  that opening History triggers).
- **⚠ The folder menu's rollup is a PREFIX match, so descendants count** — and the root row is a folder
  (`folderId === null`) and gets the item too. The old menu published `kind/Folder/` only and skipped
  subfolders, and offered the item for a clean folder.
- **⚠ `PANEL_ENTITY_KIND` replaced three drifted maps.** `webhooks` was in the publish handler and in
  *neither* footer map, so the webhooks panel reported "up to date" with unpublishable work on screen;
  `graphqlRequests`/`soapRequests`/`grpcRequests` were missing from the prefix map.
- **`publishService`'s staging is NOT the broken part** — `g.status()` uses `--null`, so paths are
  unquoted and a folder name with spaces works (pinned by the "Solo Folder" case). The `unquoteGitPath`
  machinery there is for the *raw* `--porcelain` pre-status.
- **⚠ `{ok: true}` does NOT mean "something happened" — the result carries the COMMIT ID.** Publish and
  Sync are both `git add` + `git commit`, so both produce a hash; `ok: true` is also what the engine
  returns when it **staged nothing**. The contract (`PublishResult` in `sync/publishService.ts`):
  `{ok: true, commit: "<40-hex>"}` = a commit was created; `{ok: true, commit: null}` = nothing was
  staged (correct for an already-committed file, and test-pinned); `{ok: false, error}` = a step failed.
  **`commit` is absent, not `null`, on a revert** — `git checkout` creates no commit, and absent vs
  `null` are different statements ("this operation does not commit" vs "a commit was expected and did
  not happen"). A surface that renders only "success" cannot tell a publish from a no-op, which is what
  *"clicking publish does nothing"* actually was: correct behaviour, silently. The card now shows
  `Published · <7-char sha>` with the full id in `title`.
- **⚠ `{ok: false}` does not mean "nothing happened" either.** `publishEntities()` stages *before* it
  commits and pushes, so a failed **push** leaves a real commit behind. `useEntityGitActions.write()`
  therefore refreshes the config **and** the status on **failure** too; refreshing only on success left
  the UI on the pre-write state — "still says not published" with a commit present.
- **⚠ `git add`'s failure must never be swallowed.** It was (`catch { /* path may not exist */ }`), which
  folded *"git refused to stage this"* into *"there was nothing to stage"* — so a broken publish
  announced itself as `commit: null` and the UI said **"already committed"** while its own status map
  said `new`. Now `findUnstageablePaths()` checks every requested non-folder path against
  `git ls-files` whenever nothing was staged: **tracked ⇒ a genuine no-op**, **not tracked ⇒
  `{ok:false}` naming the path**, with the two causes worth checking — a stale **`.git/index.lock`**
  (`git status` reads fine while every `git add` fails; a killed git process leaves one) or a
  `.gitignore` rule. A path that does not exist is an error too. **A silent `catch` around a git
  command is the general trap here**: it converts every failure into the one outcome that looks like
  success.
- **⚠ A card must not claim a state its own status contradicts.** `commit: null` says "Nothing to
  publish" only when the card agrees the entity is tracked; if it still reads unpublished, it reports
  the *disagreement*. Pinned by `tests/renderer/mappingsPublishRoundTrip.test.tsx` — the file that
  mounts the real hook and the real panel over **one shared status state**, which is the only way to
  see whether a write reaches the card. `mappingsPublish.test.tsx` fakes the git actions and
  `entityGitActions.test.tsx` only asserts the refresh is *called*; neither can.
- **⚠ Verify the MIDDLE of a chain, not its ends.** The mapping-publish report survived two rounds
  because `tests/renderer/mappingsPublish.test.tsx` *faked* the binder and
  `tests/sync/mappingPublish.test.ts` was *handed* the path — so a wrong path in
  `useEntityGitActions.entityPath` was invisible to both, and its symptom (engine stages nothing,
  returns `{ok:true}`) is identical to a broken button. `tests/renderer/entityGitActions.test.tsx` pins
  the hook's exact payloads; that is the file to extend when a write goes missing.
- **Not done:** the protocol tab editors (`useProtocolEditor.hasLocalChanges`) still gate on
  `isDirty || status !== "clean"`, so a *clean-but-unpushed* entity shows no Sync there;
  `ProxyRuleDetailsPanel` is wired, the other six editors are not. `onPublishPaths` is supported by the
  hook but not bound by the four panels, so a multi-selection still makes one commit per entity.


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
