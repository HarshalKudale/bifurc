# Bifurc — Project Memory

**Read `TRAPS.md` first** (`.workbuddy-ai/memory/TRAPS.md` — the narrative record of failure modes not
discoverable from the code). Authorities: programme state → `plan/README.md` §Status · live phase →
`plan/14` · decisions → `plan/00-decisions.md` · suites → `TESTING.md` · P6/§5b/§5c ledgers → `plan/07` ·
protocol history → `plan/protocol-changes.md` · procedures → the `bifurc-*` skills. **This file is a pointer
plus the rules that cost the most; the narrative lives in those, not here.**

## 🚫 Standing rule — ask the user to run shell-bound commands

Anything needing a real desktop session / Electron / GPU (`npm run dev`, `npm run test:e2e`, any GUI) runs
**by the user**. Full reasoning in `~/.workbuddy-ai/MEMORY.md`. **An agent-shell failure is evidence about
the agent shell, not the product** — name the mechanism that makes it environment-specific, or ask for a run
instead. **Never start a build you will not wait for.**

**The evidence for a failed run is already on disk — read it before asking for a re-run.**
`screenshot: "only-on-failure"` + `trace: "on-first-retry"` leave
`test-results/<spec>-<test>-electron/error-context.md` (the error **plus a full accessibility snapshot**) and
`test-failed-1.png`. A *passing* run leaves `playwright-report/index.html`, whose embedded `report.json`
carries `stats` — decode with `re.search(r'data:application/zip;base64,(…)'` + `zipfile` to attribute a pass
to the **full** suite rather than assume it from a filtered run.

## Layout — five things that silently waste a session

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
- **Tooling: one `Edit` per file per message.** Two `Edit` calls to the *same* file in one message silently
  revert each other (the second reads a pre-first-edit snapshot). Parallel edits to *different* files are
  fine. A same-file `EBUSY` is transient — retry it alone.

## Measuring the suite

- **A *delta* is the signal; the standing failure is not. Name the mechanism for every delta.** Baseline is
  **re-measured, never carried forward**; **`Δ0` has three causes and the count separates none** — read
  per-file (ledger `plan/07`).
- **For any generated / looped / `it.each` suite, NEVER count by hand** — enumerate the run
  (`--reporter=json` → `assertionResults[].title`), then subtract. **Exception: when the claim *is* a count
  of occurrences in a source file, reading the file and asserting the count is the direct instrument.**
- **There is NO standing failure — every failure is a flake to *identify*, not assume.** The load-induced
  pair is `gitStore.test.ts` + `gitOps.test.ts` (`EBUSY`/hook-timeout; 28/28 isolated in 120 s); the retired
  `soap.execute` flake went with the P0 spike. **The stable number is the TOTAL, not the passed/failed
  split** — identify it (`--reporter=json --outputFile=/tmp/f.json`) rather than assume.
- **A retired test's contract must be shown to have a better home, per case — never assumed.**
  **"Does anything else test X?" is not answerable by counting files that mention X.**
- **An assertion must be able to fail — and one mutation is not enough to show it does.** Ask of every
  instrument: *what would it fail on?* One answering a *neighbouring* question is unguarded however green.
  **Prefer a NARROW mutation**, and **check it is well-formed first** — a malformed one measures itself.
- **A negative verdict is a claim too.** "I did not find it" is not "it is not there"; searching one
  directory is the same error as counting the files that mention a thing.
- **A checkbox and a table cell are two copies of one claim, and they drift.** Counting a document's own
  markers is a one-line cross-check on whether two places that state the same thing agree.

**Current: 2,632 tests / 113 files — 2,592 passed, 40 skipped, 0 failed** (2026-09-22, *before* the 6b
product change — re-measure). **e2e: 11 specs / 46 tests — 46 expected, 0 unexpected, 0 flaky,
owner-measured 2026-09-22.** Trajectory in `plan/07`.

## Phase state — the plans are the authority

- **P0–P6 done.** P6 criterion 2 closed at **11/11 specs, 46/46 tests**, owner-run; the seven failures
  getting there were **all test- or fixture-side, no product code changed** (table in `TRAPS.md`). P4 8/10
  (extension release + audit record — neither is code here); P3 item 5 open as *UI* → P7. **P5's spec is
  `packages/client/src/surface.ts` (144 keys); `src/preload.ts` is the authority, not
  `renderer/types/window.ts`.**
- **Dead-channel reduction CLOSED** — nine groups, one file per commit, `src/` 20 → 17 files. Re-derived
  from the tree = **18 real `ipcMain.handle(` sites, all keepers** (a bare `grep` count of 20 also catches
  one comment in `rpcBridge.ts` and the RPC bridge itself). `coreKinds.ts` keeps one
  `registerCrudKind<Environment>()` — **a kind is NOT a channel**. **Re-measure from the tree, not prose.**
- **The sandboxed-preload defect is FIXED** (`e8e535f`): `scripts/build-preload.mjs` bundles
  `src/preload.ts` with esbuild, run from `build:main`. **`build:main` is required after any
  `src/preload.ts` change.** The guard *evaluates* the bundle with a `require` that knows only Electron's
  five names — a regex or static require-walk proves reachability, not loadability.

## P14 — the executor pivot (`plan/14`, D10–D17)

Deployable unit = a **workspace**, not the engine: N per host via `WorkspaceRuntime`, desktop adopts the
same runtime, one RPC endpoint per host with `workspaceId` in params, process split downgraded (D14), CLI
parked (D15), proxy base `80` win32 / `8080` elsewhere counting up (D16), blob staging namespaced per
workspace (D17). `@bifurc/engine` does **not** move. **WI 0–8 landed; §8 is 21 met · 0 open** (6b closed by
a product change on the owner's decision). Narrative in `plan/14`; the audit story is §8.

**Five rules, each earned expensively:**

1. **An optional wire parameter is done when the *body* reads it, not when the schema accepts it.**
   `server.start {workspaceId:"A"}` returned **`{ ok: true }`** while binding a *third* server on the
   *active* workspace's port. **The failure succeeds**, so no suite caught it.
2. **A reserved-port set must not include its own base** — `findFreePortFrom(base)` returns `base + 1`; the
   host test *observed* that off-by-one and called it correct.
3. **Namespacing a store surfaces every half-threaded id, and only the integration suite sees it** (D17:
   0 → 30 failures). **Scope the caller, never relax the store**; audit by asking *"which calls pass NO id?"*.
4. **A path a header calls temporary is still the live implementation** (`proxy/server.ts`, "removed in WI 7").
5. **Rewording a defective criterion can weaken its claim** — after which the instrument agrees with the
   code instead of the decision. P14's criterion 6 went green on a *substitute* claim (reproducibility) while
   D16's stickiness rule stayed unimplemented. **Quote the decision in the criterion, and re-test the old
   claim rather than retiring it.**

**WI 7's command layer — resolution has THREE outcomes and the third is load-bearing:** named runtime →
that runtime; no runtime but the target **is** the active workspace → the **facade**; anything else →
**`absent`**. Falling through to the facade in the third case re-creates the original bug with a success
code. **A command never allocates a port.** WI 8 is a **contract-consistency fix, not a live bug fix**.

**WI 4's dispatch gap (fixed 2026-09-22) — the fix is a *loader*, not a filter.** A runtime whose workspace
was not the active pointer served **nothing**: `cfg.mocks`/`cfg.mappings` came from **A** (`loadConfig()` is
active-only) while `enabledSets` came from **B**, so the halves cancelled. **A filter cannot recover an
absent input**; `store/config.ts`'s `loadWorkspaceDispatchSlice(wsId)` builds one workspace's entities for
**any** id. **Also: adding an export to a module breaks every `vi.mock` of it** (82 failures in one file) —
**a module's surface is a fixture too.**

**WI 7's desktop half landed**, with two corrections to the plan's wording: **bind** the module facades to
the runtime's instances (do *not* resolve by `activeWorkspaceId` — WI 4 made the active pointer independent
of the running servers), and call `runtime.proxy.start(port)`, **never** `runtime.start()` (which would bind
the user-started webhook receiver).

**⚠ `--data-dir` does NOT redirect the settings file on Windows** — `settingsPath()` prefers
`%LOCALAPPDATA%/Bifurc/app.json` regardless of `setDataRoot()`. **Linux-invisible**, hence a **P9 deployment
caveat**, recorded not patched.

**NOT scoped, deliberately:** `services.discover` is host-wide · the CA is one per *machine*, not per
workspace (`fileOps/commands.ts:139-140`). **WI 1's `runtime.json` preferred-port editor is P7** — a
renderer edit, so non-negotiable #3 bars it here.

**Secondary findings, recorded not patched:** `TESTING.md`'s "`--project` + path filter breaks collection"
warning **no longer reproduces** (used 12× in one session, clean every time).
