# Bifurc — Project Memory

**Read `TRAPS.md` first** (`.workbuddy-ai/memory/TRAPS.md`) — failure modes not discoverable from the code.
Authorities: status → `plan/README.md` · live phase → `plan/14` · suites → `TESTING.md` · P6/§5c ledgers →
`plan/07` · protocol history → `plan/protocol-changes.md` · procedures → the `bifurc-*` skills.

## Layout — four things that silently waste a session

- **Flat repo — the app IS the root.** `workspaces: ["packages/*"]`; never `cd bifurc`. Git root is the PARENT dir
  (`I:/workspace/Bifurc-Worskspace`, empty) — the repo is `Bifurc/.git`, branch `standalone-engine`. Check
  `git rev-parse --show-toplevel` before any *write*.
- **`git rm` and `git stash` are BOTH destructive on this drive** — they cost the **object store** (`.git/refs`,
  `.git/index`, loose objects), not the tree; `git stash push` cost five commits on 2026-09-21. Recovery in
  `TRAPS.md`; use the plain-filesystem equivalent + `git add`.
- **App runs `packages/*/dist/`; tests run `src/`** → `npm run build:packages` after ANY package edit. The vitest
  alias covers `@bifurc/engine/*` **only**; `@bifurc/protocol` / `@bifurc/client` resolve to built `dist/`, so an
  un-rebuilt edit is **silently untested** — `npm test` does not build. The dts leg can fail *after* the JS leg
  prints "Build success" — grep `error TS`. Bypass the safe-delete guard with
  `env -u CODEBUDDY_SAFE_DELETE_BULK_STATE_DIR -u CODEBUDDY_TOOL_CALL_ID …`, **in the background** (foreground
  SIGTERMs the long legs).
- **No renderer edits P1–P6**; `window.api` byte-identical. `node`/`require` cannot load absolute `I:\…` module
  paths — write probes *inside* the repo with a bare specifier.
## 🔴 Open defect — the sandboxed preload kills `window.api` (found 2026-09-21; **not** P14)

`npm run dev` opens a window with **no `window.api`**: `Unable to load preload script: dist/preload.js` →
`module not found: @bifurc/client`. `src/main.ts` `webPreferences` sets `contextIsolation: true`,
`nodeIntegration: false`, never `sandbox: false` → Electron 42's default sandbox applies → the preload runs in the
`sandbox_bundle` realm, whose `require` is `preloadRequire`: it resolves **only** ids in Electron's own webpack
module map (`electron`, `events`, `url`, `timers`). **No node_modules and no filesystem resolution exists** — the
contents are handed in and evaled, so a relative `./x` fails exactly as a bare specifier does. `build:main` is plain
`tsc` + `tsc-alias` (**no bundling**), so `dist/preload.js` keeps bare requires: `@bifurc/client` (line 4) and, via
`./ipcTransport`, `@bifurc/protocol` + `@bifurc/engine/transport/types` (runtime values). Pre-P6 `src/preload.ts`
imported **only `electron`** — a self-contained 196-line table — which is why it worked for the project's whole
life. `1268bfd` (2026-09-18 17:36, P6 "route config:get through the RPC bridge") added it; the last playwright
report is 16:32 that day. **This is the mechanism behind P6 criterion 2's carried `46 failed / 0 passed`** — a
non-booting app, not flakiness. Fix = bundle the preload (esbuild/tsup, `--external:electron`) in `build:main`;
`sandbox: false` is a security downgrade. Blast radius includes the packaged app.

## Measuring the suite

- **A *delta* is the signal; the standing failure is not. Name the mechanism for every delta.** Baseline is
  **re-measured, never carried forward** — a group that opens with a fix-first commit moves it before deletion.
- **For any generated / looped / `it.each` suite, NEVER count by hand** — enumerate the run (`--reporter=json` →
  `assertionResults[].title` → classify by name), then subtract. A total low by exactly the pre-existing case count
  **looks like a missing test** rather than an arithmetic slip.
- **`Δ0` has three causes and the count separates none** — read per-file (P6 groups 1–9: Δ0 · Δ−5 · Δ−3 · Δ0 · Δ−8 ·
  **Δ−25** · Δ0 · Δ0 · **Δ−4**, the only genuine ratchet shrink). Full ledger: `plan/07`.
- **There is NO standing failure any more — every failure in a run is a flake to *identify*, not assume.**
  The P0 spike was retired 2026-09-21 (`spike/README.md`), taking `soap.execute` with it, so the set is now
  **two load-induced files and nothing else**: `gitStore.test.ts` + `gitOps.test.ts` (`EBUSY` / hook-timeout;
  **28/28 isolated in 120 s**). Separately dismissed: `certTrust.test.ts` (certutil; **34/34 isolated**).
  **The stable number is the TOTAL, not the passed/failed split** — pre-retirement two consecutive full runs
  read `4 failed / 2520 passed` and `3 failed / 2521 passed`, **both `2564` total**, the same two files, a
  different number of cases tripping. So **identify the failure** (`--reporter=json --outputFile=/tmp/f.json`,
  filter `testResults[].assertionResults[].status`) rather than assume — one run, and it is the difference
  between dismissing a failure and having dismissed it. Quote the **second** measurement.
- **A retired test's contract must be shown to have a better home, per case — never assumed.** Deleting
  `tests/spike/protocolPoc.test.ts` was safe because all 12 cases were checked individually; two would have
  been silently lost by a naive delete. **"Does anything else test X?" is not answerable by counting files
  that mention X** — both directions were wrong on the first pass (one false-negative, one false-positive).
  See `spike/README.md`.

**Current: 2560 tests — 2518 passed, 40 skipped, 2 failed** (both the load-induced pair), **105 files**.
*(Trajectory: 101 / 2,458 / 2,417 / 40 / 1 → … → 2,507 (WI 3) → 2,564 (WI 1 protocol + D17) → 2,552
(spike retired: −12 = exactly the spike file's case count) → **2,560** (WI 7: +8 = exactly the new
`workspaceCommandScope` file's cases; files +1). **Name every delta's mechanism.**)*

## Phase state — the plans are the authority

- **P0–P5 done.** P4 8/10 (extension release + audit record — neither code here); P3 item 5 open as *UI* → P7.
  **P5's spec is `packages/client/src/surface.ts` (144 keys); `src/preload.ts` is the authority, not
  `renderer/types/window.ts`.**
- **P6 done in scope** — 144/144 keys classified, zero `SPLIT`; all 51 §5c commands moved and statically guarded.
  `registerClientHandlers()` sits beside the reduced `registerIpcHandlers()` (`src/main.ts:294`). **Criterion 2
  ("11 e2e specs pass, unmodified") is CARRIED** (46 failed / 0 passed) — **the preload defect above is its known
  mechanism.** WI 2 (process split) downgraded by D14 — desktop stays in-process.
- **The reduction is CLOSED** — nine groups, one file per commit, `src/` 20 → 17 files. Re-derived from the tree
  2026-09-21 = **18 `ipcMain.handle(` sites, all keepers** (17 in `clientHandlers.ts`, 17 live / 0 dead; + 1
  client-local in `systemHandlers.ts`). `coreKinds.ts` keeps 1 `registerCrudKind<Environment>()` — **a kind is NOT
  a channel**. **Re-measure from the tree, not prose.** Ledger `plan/07`; procedure in the
  `bifurc-dead-channel-reduction` skill.

## The executor pivot — `plan/14` (D10–D17), the live phase

Deployable unit = a **workspace**, not the engine. **`plan/14` is the authority; D10–D17 live in
`plan/00-decisions.md`.** Workspace = executor · N per host via `WorkspaceRuntime` · desktop adopts the same
runtime · one RPC endpoint per host with `workspaceId` in params · process split downgraded (D14) · CLI parked
(D15) · proxy base `80` win32 / `8080` elsewhere, counting up (D16) · blob staging namespaced per workspace (D17).
`@bifurc/engine` does **not** move.

**Done:** WI 0 (gate) · WI 2 (factories + facade) · WI 3 (host/registry/`registerAll` — **the empty-registry find**:
all 18 `register*Commands()` calls lived in the shell, so a headless host got `UNKNOWN_COMMAND` for everything while
every desktop suite stayed green, *the P6 §5c class one layer up*) · WI 4 (`servedWorkspaceId()`; `setActive`
switched the **pointer only**) · WI 5 (allocator + `advisory` preflight) · WI 1's protocol half.

**Four rules, each earned expensively** (narrative in the 2026-09-21 log):

1. **An optional wire parameter is done when the *body* reads it, not when the schema accepts it.**
   `server.start {workspaceId:"A"}` returned **`{ ok: true }`** while binding a *third* server on the *active*
   workspace's port — the module facade (`workspaceId: null`) ignores the param. **The failure succeeds**, so no
   suite caught it. 🚩 **WI 7's flip is load-bearing for correctness, not a tidy-up.**
2. **A reserved-port set must not include its own base** — it is the range's start, so `findFreePortFrom(base)`
   returns `base + 1`; the host test *observed* that off-by-one and called it correct.
3. **Namespacing a store surfaces every half-threaded id, and only the integration suite sees it** (D17: 0 → 30
   failures). **Scope the caller, never relax the store**; audit by asking *"which calls pass NO id?"*.
4. **A path a header calls temporary is still the live implementation** (`proxy/server.ts`, "removed in WI 7").

**Still owed:** WI 1's `runtime.json` preferred-port plumbing from the UI/settings (allocator + registry honour
`preferred*`; only the *editor* is missing) · WI 6 (host artifact, feeds P9) · **WI 7's desktop half** — the
command layer is DONE (below); what remains is `src/main.ts:309-310` building runtimes, which needs
`createEngine()`'s own proxy lifecycle (`index.ts:255`) to move with it or the engine and a runtime would both
bind · WI 8 (five egress commands gain `workspaceId`).

**WI 7's command layer — DONE 2026-09-21.** `runtime/runtimeRegistry.ts` (the lookup the commands were missing)
+ `runtime/runtimeServerOps.ts` (how a resolved runtime's servers are driven). Resolution has **three**
outcomes and the third is load-bearing: named runtime → that runtime; no runtime but target **is** the active
workspace → the **facade** (which genuinely *is* the desktop's server, so the desktop is byte-identical);
anything else → **`absent`**. Falling through to the facade in the third case re-creates the original bug with
a success code. **A command never allocates a port** — and the reason is a measurement: `proxy/server.ts` seeds
`currentPort = 0` but `webhookServer.ts` seeds it with **9101**, so a "never started" guard written for one does
not transfer. `restart` re-binds only an **observed** port; `start` binds only a **caller-named** port.
**Mutation-checked twice** (runtime branch off → 7/8 fail; `absent`→facade → exactly the 2 that should die).

**NOT scoped, deliberately:** `services.discover` is host-wide (asserted in `index.test.ts`) ·
`fileOps/commands.ts:139-140` — **the CA is one per *machine*, not per workspace** (`caCertPath()` under
`appDataDir()`); scoping it would let a client stage under A and import into the shared CA.
**RESOLVED:** the P0 spike was **retired** 2026-09-21 (`spike/README.md`) — runnable code gone, evidence kept.
