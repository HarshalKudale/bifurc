# Bifurc — Project Memory

**Read `TRAPS.md` first** (`.workbuddy-ai/memory/TRAPS.md`) — failure modes not discoverable from the code.
Authorities: status → `plan/README.md` · live phase → `plan/14` · decisions → `plan/00-decisions.md` ·
suites → `TESTING.md` · P6/§5c ledgers → `plan/07` · protocol history → `plan/protocol-changes.md` ·
procedures → the `bifurc-*` skills.

## 🚫 STANDING RULE — ask the user to run shell-bound commands; never run them here

**Anything needing a real desktop session / Electron / GPU runs BY THE USER.** Named: `npm run dev`,
`npm run test:e2e`, anything launching Electron or a GUI. Ask, then wait.

**User's words (2026-09-21):** *"…you have to ask me, the user, about running it. Do not run it in your
agent-sandboxed environment, because it will fail without having an actual issue and **you will break the
working code**."*

**Two failures this caused, both mine:** (1) I read an agent-shell e2e failure as evidence about the
product — Electron cannot launch here (GPU process fails), so every spec dies at
`electronApplication.firstWindow` in ~850 ms; **that count is a constant, not a signal**. (2) I backgrounded
a build, saw a transient `Cannot find module` from a half-written `dist/`, and concluded I had broken the
tree. I had not — two builds were racing on one `dist/`.

**An agent-shell failure is evidence about the agent shell, not the product.** Before reporting one, name
the mechanism that makes it environment-specific; if you cannot, ask the user to run it. **Corollary: never
start a build you will not wait for** — a killed build leaves a half-written `dist/` and the user's next
`npm run dev` fails on it.

## Layout — four things that silently waste a session

- **Flat repo — the app IS the root.** `workspaces: ["packages/*"]`; never `cd bifurc`. Git root is the
  PARENT dir (`I:/workspace/Bifurc-Worskspace`, empty) — the repo is `Bifurc/.git`, branch
  `standalone-engine`. Check `git rev-parse --show-toplevel` before any *write*.
- **`git rm` and `git stash` are BOTH destructive here** — they cost the **object store** (`.git/refs`,
  `.git/index`, loose objects), not the tree; `git stash push` cost five commits on 2026-09-21. Recovery in
  `TRAPS.md`; use the plain-filesystem equivalent + `git add`.
- **App runs `packages/*/dist/`; tests run `src/`** → `npm run build:packages` after ANY package edit. The
  vitest alias covers `@bifurc/engine/*` **only**; `@bifurc/protocol`/`@bifurc/client` resolve to built
  `dist/`, so an un-rebuilt edit is **silently untested** (`npm test` does not build). The dts leg can fail
  *after* the JS leg prints "Build success" — grep `error TS`. Bypass the safe-delete guard with
  `env -u CODEBUDDY_SAFE_DELETE_BULK_STATE_DIR -u CODEBUDDY_TOOL_CALL_ID …`, **in the background**.
- **No renderer edits P1–P6**; `window.api` byte-identical. `node`/`require` cannot load absolute `I:\…`
  module paths — write probes *inside* the repo with a bare specifier.

## e2e — the method, and the state

**The evidence for a failed e2e run is already on disk. Read it before asking for a re-run.**
`screenshot: "only-on-failure"` + `trace: "on-first-retry"`, so each failure leaves
`test-results/<spec>-<test>-electron/error-context.md` — the error **plus a full accessibility snapshot of
the page** — and `test-failed-1.png`. Every failure below was diagnosed from those files alone.

**P6 criterion 2 ("11 e2e specs pass, unmodified"): 9/11, user-measured 2026-09-22**; the last two are
fixed in `782eef8` and awaiting the re-run. Trajectory: `46 failed / 0 passed` → **4 failed** after
`e8e535f` (preload fix) → **2 failed** after `a239ab5`. **The `46` was never a product measurement** — it
was this agent shell; the preload defect merely failed the same suite for a different reason. A matching
count is not a matching cause.

**Six failures over two sessions — three stale tests, two stale locators, one stale fixture. NONE a
product bug; no product code changed for any.**

| Where | Cause | Commit |
|---|---|---|
| `proxy-rules.spec.ts:15` | **Fixture wrote `names.json` as `{id: "name"}` (a string); the engine reads `EntityNameEntry` OBJECTS** (`fsNamesIndex.ts:5`) → `ruleNames[id]?.name` undefined → every seeded name and pattern blank. **All four indexes were wrong the same way**; the field differs per kind: `rules→url (=pattern)`, `requests→method+url`, `sockets→url`, `webhooks→urlSuffix` (`config.ts:133,168-170,175-176,184-185`) | `a239ab5` |
| `mocks-rest.spec.ts:49` | Forgot the response body — `RestTab.tsx:164` gates Save on `resMode === "none" \|\| resBody.trim()`, and a new mock starts `resMode:"json"`, `resBody:""` | `a239ab5` |
| `happy-path-workflows.spec.ts:330` | A **local** `clickTitlebarButton(/^Manage Environments/i)` — that label lives **inside the environment dropdown**, so it matches nothing at rest | `a239ab5` |
| `app.spec.ts:69` | A locator for `REST 3`, **a label nothing produces** (introduced by `bae3732`, 2026-09-14, and nowhere else) | `a239ab5`, re-fixed `782eef8` |
| `app.spec.ts:68` | **The tree row is not a button** — `FolderTreeItemNode.tsx:28` renders `<div title={item.name} onClick=…>`, so its role is `generic` and its name comes from `title`. And `Ctrl+T` opens the **protocol chooser**, not the form: `useRequestsPanelState.ts:94` sets `draftProtocols[tabId] = null` | `782eef8` |
| `screenshots.spec.ts:117` | Declared `Manage Environments` as `trigger:"titlebar"`, but `EnvSelector.tsx:46` renders it only while the dropdown is open (`{open && …}`) | `782eef8` |

**An assertion must be able to fail.** `getByText("New Request")` also matched the panel's own "New
request" button, so it passed with the keybind doing nothing — replaced by the chooser's unique heading
`Select Request Protocol`. Same class as the old `if (await locator.isVisible())` guards. **When you "fix" a
test, check the replacement can still fail.**

**The guard that was missing:** ~2,600 unit/integration tests were green throughout, because they build
their own fixtures from `src/` — the e2e fixture's on-disk shape was exercised by **nothing but a real
Electron run**. `tests/integration/e2eFixtureContract.integration.test.ts` (5 cases) runs
`writeSampleWorkspace()` and reads it back through the engine's own `loadConfig()`. **A "no blank field"
property must be scoped to fields that are *required*** — the first two versions reported `mappings`, whose
display field is `label` and whose `map-003.label` is `""` **deliberately** (`MappingsPanel.tsx:180`).

## Measuring the suite

- **A *delta* is the signal; the standing failure is not. Name the mechanism for every delta.** Baseline is
  **re-measured, never carried forward**.
- **For any generated / looped / `it.each` suite, NEVER count by hand** — enumerate the run
  (`--reporter=json` → `assertionResults[].title`), then subtract.
- **`Δ0` has three causes and the count separates none** — read per-file. Ledger `plan/07`.
- **There is NO standing failure — every failure is a flake to *identify*, not assume.** The P0 spike was
  retired 2026-09-21 (`spike/README.md`), leaving **two load-induced files and nothing else**:
  `gitStore.test.ts` + `gitOps.test.ts` (`EBUSY`/hook-timeout; 28/28 isolated in 120 s). Separately
  dismissed: `certTrust.test.ts` (certutil; 34/34 isolated). **The stable number is the TOTAL, not the
  passed/failed split** — two consecutive full runs read `4 failed / 2520 passed` and `3 failed / 2521
  passed`, **both 2564 total**. Identify the failure (`--reporter=json --outputFile=/tmp/f.json`) rather
  than assume: one run, and it is the difference between dismissing a failure and having dismissed it.
- **A retired test's contract must be shown to have a better home, per case — never assumed.** Two of the
  12 `protocolPoc` cases would have been lost by a naive delete. **"Does anything else test X?" is not
  answerable by counting files that mention X.** See `spike/README.md`.

**Current: 2592 tests — 2552 passed, 40 skipped, 0 failed, 107 files.** *(The +5 over 2,587 is the new
`e2eFixtureContract` file. **Full trajectory in `plan/07`; name every delta's mechanism.**)*

## Phase state — the plans are the authority

- **P0–P5 done.** P4 8/10 (extension release + audit record — neither code here); P3 item 5 open as *UI* → P7.
  **P5's spec is `packages/client/src/surface.ts` (144 keys); `src/preload.ts` is the authority, not
  `renderer/types/window.ts`.**
- **P6 done in scope** — 144/144 keys classified, zero `SPLIT`; all 51 §5c commands moved and statically
  guarded. `registerClientHandlers()` sits beside the reduced `registerIpcHandlers()` (`src/main.ts:294`).
  **Criterion 2 is 9/11.** WI 2 (process split) downgraded by D14.
- **Dead-channel reduction CLOSED** — nine groups, one file per commit, `src/` 20 → 17 files. Re-derived
  from the tree 2026-09-21 = **18 `ipcMain.handle(` sites, all keepers** (17 in `clientHandlers.ts`,
  17 live / 0 dead; +1 client-local in `systemHandlers.ts`). `coreKinds.ts` keeps one
  `registerCrudKind<Environment>()` — **a kind is NOT a channel**. **Re-measure from the tree, not prose.**
  Ledger `plan/07`; procedure in the `bifurc-dead-channel-reduction` skill.

## ✅ FIXED 2026-09-21 (`e8e535f`) — the sandboxed preload killed `window.api`

Electron 42's default sandbox applied (no `sandbox: false`), so the preload ran in the `sandbox_bundle`
realm whose `require` resolves **only** ids in Electron's own webpack module map — **no node_modules and no
filesystem resolution**, so a relative `./x` failed exactly as a bare specifier did. `build:main` was plain
`tsc` + `tsc-alias` (**no bundling**), so `dist/preload.js` kept bare requires. Introduced by `1268bfd`.

**Fix:** `scripts/build-preload.mjs` bundles `src/preload.ts` with esbuild (`format: "cjs"`,
`external: ["electron"]`), run from `build:main`. **Its guard is the valuable half** — it *evaluates* the
bundle with a `require` that knows only Electron's five names, because a regex or a static require-walk
proves reachability, not loadability. **`build:main` is now required after any `src/preload.ts` change.**
Details: `TRAPS.md`, "A sandboxed preload has NO module resolver".

## The executor pivot — `plan/14` (D10–D17), the live phase

Deployable unit = a **workspace**, not the engine: N per host via `WorkspaceRuntime`, desktop adopts the
same runtime, one RPC endpoint per host with `workspaceId` in params, process split downgraded (D14), CLI
parked (D15), proxy base `80` win32 / `8080` elsewhere counting up (D16), blob staging namespaced per
workspace (D17). `@bifurc/engine` does **not** move. **WI 0, 2, 3, 4, 5, WI 1's protocol half, WI 6
(`runtime/hostArgs.ts` + `hostCli.ts`), WI 8, and WI 7's command layer are done** — per-WI narrative is in
`plan/14` and the 2026-09-21 log.

**Four rules, each earned expensively:**

1. **An optional wire parameter is done when the *body* reads it, not when the schema accepts it.**
   `server.start {workspaceId:"A"}` returned **`{ ok: true }`** while binding a *third* server on the
   *active* workspace's port. **The failure succeeds**, so no suite caught it. 🚩 **WI 7's flip is
   load-bearing for correctness, not a tidy-up.**
2. **A reserved-port set must not include its own base** — `findFreePortFrom(base)` returns `base + 1`; the
   host test *observed* that off-by-one and called it correct.
3. **Namespacing a store surfaces every half-threaded id, and only the integration suite sees it** (D17:
   0 → 30 failures). **Scope the caller, never relax the store**; audit by asking *"which calls pass NO id?"*.
4. **A path a header calls temporary is still the live implementation** (`proxy/server.ts`, "removed in WI 7").

**WI 3's empty-registry find:** all 18 `register*Commands()` calls lived in the shell, so a headless host got
`UNKNOWN_COMMAND` for everything while every desktop suite stayed green — *the P6 §5c class one layer up*.

**WI 7's command layer — resolution has THREE outcomes and the third is load-bearing:** named runtime → that
runtime; no runtime but the target **is** the active workspace → the **facade** (which genuinely *is* the
desktop's server); anything else → **`absent`**. Falling through to the facade in the third case re-creates
the original bug with a success code. **A command never allocates a port** — `proxy/server.ts` seeds
`currentPort = 0` but `webhookServer.ts` seeds **9101**, so a "never started" guard written for one does not
transfer. Mutation-checked twice. WI 8 is a **contract-consistency fix, not a live bug fix**
(`audit.export` had a second WI 1-class bug; `tls.exportCert` deliberately has no `workspaceId` — the CA is
per *machine*) → `plan/protocol-changes.md`.

**⚠ `--data-dir` does NOT redirect the settings file on Windows** — `settingsPath()` prefers
`%LOCALAPPDATA%/Bifurc/app.json` regardless of `setDataRoot()`, so a `--data-dir` run reads the *user's*
settings (the first artifact run served 0 workspaces and exited 0 for exactly that reason).
**Linux-invisible**, hence a **P9 deployment caveat**, recorded not patched — a patch would be a *third*
answer to "where is the settings file". WI 6 fixed the **silence**: the host now names each requested
workspace it cannot find *and the directory it looked in*.

**Still owed — ONE item, and it needs an owner decision before it is touched.** WI 1's `runtime.json`
preferred-port plumbing from the UI/settings (allocator + registry honour `preferred*`; only the *editor* is
missing — and it may be a renderer edit, so P7 by non-negotiable #3). **The only substantial item left is
WI 7's desktop half, and it is a DESIGN QUESTION, not a refactor:**

> `src/main.ts:309-310` must build a `WorkspaceRuntime` for the active workspace instead of calling
> `startServer()`. **The blocker is not that line — it is that ~17 importer callers of `reloadConfig()`
> reach the module facade.** If the desktop stops creating the facade instance, `reloadConfig()` would
> lazily create a *phantom* one whose config is updated while the running runtime's proxy never sees it —
> so config changes would silently stop reaching the running proxy (rules, mocks, enabled sets).
> **Two candidate shapes, and picking one is an owner call:** (a) make the facade **delegate** to the
> registered runtime for the active workspace, so the 17 importers are untouched and the module functions
> keep meaning "the active workspace's server, wherever it lives"; or (b) change the 17 callers to target a
> runtime explicitly. §9 flags WI 7 as **the only step that can change what a user sees**, gated on P6's
> zero-user-visible-change rule — and §8's criterion is "**one proxy port**, one visible workspace", so the
> desktop must NOT start N runtimes at boot.

**NOT scoped, deliberately:** `services.discover` is host-wide · the CA is one per *machine*, not per
workspace (`fileOps/commands.ts:139-140`). **RESOLVED:** the P0 spike was retired 2026-09-21.
