# Bifurc — Project Memory

**Pointers, not records.** Status → `plan/README.md` §Status · decisions → `plan/00-decisions.md` · suites →
`TESTING.md` · ports → `plan/17` §1 · **traps → `.workbuddy-ai/memory/TRAPS.md` (grep it before diagnosing
anything)** · procedures → the `bifurc-*` skills · per-day → `memory/YYYY-MM-DD.md`.

> **⚠ ONE BUILD TARGET: the Electron desktop app.** `web/` + the container image are **deleted**.
> **`npm run dev` / `test:e2e` are the user's to run** — an agent-shell failure is evidence about the *shell*,
> not the product.

## Build & layout

- **Flat repo — the app IS the root.** `git rm`, `git stash`, `git checkout -- <file>` are ALL destructive;
  `cp` to `/tmp` first. **⚠ `rm` is a shim, and `git rm` under `scripts/` wiped the whole directory** → delete
  individual files with `python -c "import os; os.remove(...)"`, small batches, verify each, **commit first**.
- **App runs `packages/*/dist/`; tests run `src/`** → `npm run build:packages` after a package edit (needs
  `CODEBUDDY_SAFE_DELETE_ENABLED=0`). Only `@bifurc/engine/*` aliases to `src`. Directory renames blocked; a
  *file* `git mv` works. **⚠ One `Edit` per file per message.**
- **⚠ `build:packages` omits `@bifurc/gui`**; **Vitest resolves `@/…` via its own alias plugin** — a green test
  is not proof the bundle resolves. **⚠ No tsconfig covers `tests/`** → `typecheck` green while `tests/` broken.
  `packages/gui` is not in `npm run typecheck` — sole runner `npm run typecheck --workspace @bifurc/gui`
  (**~33 pre-existing errors; the instrument is *which files*, not the count**).
- **⚠ `package-lock.json`'s workspace set is a coupling** — check with `npm ci --dry-run`.

## Measuring the suite

- **A *delta* is the signal; name its mechanism.** Baseline **re-measured, never carried forward**. Judge a
  mutation by **exit code**. **Freeze the tree while a run is in flight.** **⚠ A single-project run is NOT the
  full run** — unit + integration = the total. `getFreePort()` is a **reservation, not a guarantee** (TRAPS 112).
- **⚠ Standing ENVIRONMENT failures — deterministic, NOT to be fixed.** `gitRemote.integration.test.ts` fails
  every case (10; sandbox `git push` reports success **without writing the remote's refs**). The safe-delete shim
  (`EBUSY … rmdir` on `%TEMP%`) times out `gitStore.test.ts`'s `beforeEach` (**24/24 alone**) and leaves four
  teardown-only suite `FAIL`s (`fileOps`, `importExport`, `importExportBlob`, `importExportFormats` —
  `Hook timed out in 20000ms` in `afterAll`; all their tests pass). **⚠ Never blame a change for a count in the
  git/temp family — check alone first.**
- **Last full measurement 2026-09-29: 2,742 tests / 128 files** (unit 98 / 2,281 incl. 40 skipped; integration
  30 / 461). 11 failures, **all environment**. **e2e: 11 specs rewritten 2026-09-28 — the old 46/46 figure is
  stale and wants an owner run.**

## Protocol commands & the client surface

- **⚠ Adding a command touches SEVEN places, four of which THROW** — procedure + count pins: skill
  `bifurc-protocol-module-add`. **`@bifurc/protocol` resolves from `dist/` in tests** → `build:packages` before
  any test sees the new list.
- **Counts: protocol 107 commands / 13 events; client surface 154** (pinned by `packages/client/tests/
  surface.test.ts` + `client.test.ts`; `src/preload.ts` needs no edit for a new key).
- **`packages/gui/src/lib/savedEntitySend.ts` is `!isDraft && !hasLocalChanges`** — both domain terms gone;
  `tests/renderer/savedEntitySend.test.ts` pins each departure. **⚠ A draft reports `hasLocalChanges === false`
  by construction.** **⚠ The four protocol editors hang jsdom** (CodeMirror + layout) and no e2e spec drives
  Send → tab *wiring* is typecheck + review only.

## ⚠ The UI is a CLIENT — `plan/19` (P19, D22–D30)

**Owner's rule (2026-09-29): the renderer holds no domain logic and no domain state.** Test any line in
`packages/gui/src`: *would this differ if the UI were a terminal, a web page, or a test harness?* If yes → engine
work. Engine-owned: `{{env}}` + `{{random.*}}` resolution, pre/post/test scripts, the collection-run loop **and
its run state**, cURL parsing, report rendering, WebSocket. UI state that legitimately stays: open tabs, drafts,
layout, theme, **"is this tab saved / does the editor differ from disk"**. New protocol modules land
**engine-first** (`Protocol_Module_Contract.md`, repo root).

- **⚠ TWO renderer ratchets, deliberately separate** — `tests/gui/boundary.test.ts` (engine **logic** by name)
  and `tests/gui/protocolModule.test.ts` (engine **I/O** by construction site: `new WebSocket`, `http.request`,
  `net.connect`, bare `fetch(`…). Both keyed `file#construct`, **can only shrink**; the staleness assertion forces
  each retirement. protocolModule's scan **doubles as its vacuity guard** — with `{}` that is now an explicit
  match / must-not-match control table checked both ways against `CONSTRUCTS`.
- **⚠ The UI needs to *display* resolved values → `env.resolveText { workspaceId, text }`** (merges `__global__`
  over the active env; idempotent but **not deterministic** — `{{random.*}}`). **⚠ `entity id → relPath` rides
  `sync.getEntityStatus`'s `EntityStatusPayload.paths`** (`workspace/entityPaths.ts`), **NOT `config.get`** —
  `config.save` hands its payload straight to `saveConfig()`, so a computed field on a config entity would be
  **persisted to disk**.
- **⚠ D28 (list-vs-logic split, applied 3×):** the engine owns the authority (`RANDOMIZER_TOKENS`,
  `SKIP_CURL_HEADERS`, `isBinaryContentType`); the UI keeps a **copy** for autocomplete/presentation, and **the
  pin test is what makes the copy legitimate** (`randomizerCatalogue`, `curlHeaderSkip`, `binaryContentType`).
- **⚠ Two traps invisible in a green suite:** never resolve in both places even briefly (a resolved value can
  itself contain `{{…}}` → double-resolution); and `mergeEnvVars` (global over active) is engine-side now.
- **⚠ A render-time `window.api` read added to `App` inherits a hazard**: every other call there is inside an
  event handler; a mount effect meets a partial bridge stub and throws (`App.test.tsx`). Guard it.

## Phase state

- **P19 — COMPLETE 2026-09-30 (WI 1–7). Both ratchets `{}`.** **⚠ One item left on the table:
  `packages/gui/src/lib/blocks.ts`** (`buildBlockMock` / `blockKey` / `foundBlocksFolder` /
  `ensureBlocksFolderId`) — same class as WI 5 item 2, still in the renderer, **absent from `plan/19` §1's table
  so no ratchet sees it**; needs its own work item before it can be allowlisted.
- **MQTT is next, under `Protocol_Module_Contract.md`** (skill `bifurc-protocol-module-add`).
  **`packages/engine/src/websocket/` is now real** (`wsRegistry.ts` + `wsCommands.ts`, WI 6).
- **P18** enterprise SSO / org workspaces / permission gating (**WI 0 done**). **⚠ P18 adds a SECOND
  authorization axis — per workspace (`PermissionContext`), not per session (`transport/auth/scopes.ts`); do not
  merge them** (D19–D21). **P17**: only item 4's Electron half open. **P0–P6, P14 done. P7, P9, P16
  decommissioned.** `packages/gui` exports `<App />`; `renderer/` is its only consumer.

## Engine module layout — DONE, ratchet CLEAN (branch `engine_modules`)

`Engine_Module_Plan.md` reorganised `packages/engine/src` into feature modules behind a layering rule. **Phases
0–8 committed. The plan's move counts were 2 right / 5 wrong — measure, never trust the row.** Enforcement is
`tests/engine/layout.test.ts` — **SEVEN assertions**, `ALLOWED_VIOLATIONS` `{}` (TESTING.md §3.6); **⚠ the rule
is §6's per-module table, NOT §5.1's blanket ban.** Placement: **what a module READS decides its home, not how
many callers share it** (stateless + shared → down into `utils/`; reads a file/config/sibling → the layer it
reads from). Tests do **not** mirror `src/` (TESTING.md §3.5). Procedures → skill `bifurc-engine-layout-phase`.
**⚠ A test that PINS a split must move with it**; the GUI's own `reloadConfig` is unrelated — never touch it.
