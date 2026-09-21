# Bifurc — Project Memory

**Read `TRAPS.md` first** (`.workbuddy-ai/memory/TRAPS.md`) — failure modes that cost real time and are not
discoverable from the code. Status → `plan/README.md`; the executor pivot → `plan/14`; suites → `TESTING.md`;
the P6 / §5c ledgers → `plan/07`; procedures → the `bifurc-*` skills.

## Layout

- **Flat repo — the app IS the root.** `workspaces: ["packages/*"]`; **never `cd bifurc`**.
  **The git root is the PARENT dir** (`I:/workspace/Bifurc-Worskspace`, empty) — real repo is `Bifurc/.git`
  on branch `standalone-engine`. Check `git rev-parse --show-toplevel` before any *write*; **never `git stash`**
  (TRAPS.md, "Git / env" — it deleted `.git/refs` twice).
- **App runs `packages/*/dist/`; tests run `src/`** → **`npm run build:packages` after ANY package edit.**
  The vitest alias covers `@bifurc/engine/*` **only**; `@bifurc/protocol`/`@bifurc/client` resolve to built
  `dist/`, so an un-rebuilt edit is **silently untested** — `npm test` doesn't build.
- **No renderer edits P1–P6**; `window.api` byte-identical.

## Baseline

**2026-09-21, after P14 WI 3 (partial: runtime + companion switch): 103 files / 2,482 tests — 2,440 passed,
40 skipped, 2 failed.** Isolated, the true figure is **2,481 passed / 1 failed** — the 1 being the standing
environmental `soap.execute` (`tests/spike/protocolPoc.test.ts`). The other is the documented load flake;
isolated together `gitStore` + `gitOps` are **28/28 in 102 s**.
*(Trajectory: 101 / 2,458 / 2,417 / 40 / 1 at the pre-P14 baseline → 2,467 (WI 2 step 1, +9) → 2,476
(WI 2 step 2, +9) → 2,482 (WI 3 partial, **+2 files / +6 tests**). **Name every delta's mechanism** — the
WI 3 six separate exactly as 2 new files' 3 cases + 3 CA cases, and the +9 isolation cases were WO 2's,
already banked. Skipped held at 40 throughout; no command-count move.)*

- **A *delta* is the signal, the 1 is not — name the mechanism, never wave at the number.** A group that
  opens with a fix-first commit moves the baseline before the deletion starts — re-measure it.
- **Δ0 has three causes and the count separates none of them** — read per-file. Groups 1–9 deltas:
  Δ0 (re-point) · Δ−5 (generated) · Δ−3 · Δ0 (never had cases) · Δ−8 · **Δ−25** · Δ0 (no cases AND a
  re-point) · Δ0 (both, plus a duplicated impl) · **Δ−4** (the only genuine ratchet shrink).
- **Two unrelated load-induced flakes, both dismissed only by isolating:** `tests/store/gitStore.test.ts`
  (10,017 ms timeout; **24/24** isolated) and `tests/sync/gitOps.test.ts` (`EBUSY: rmdir`; **4/4** isolated,
  18 s). A full run reporting >1 failure is *usually* this — and the **second** measurement is the one to quote.

## Phase state

- **P0–P5 done.** P4 8/10 (extension release + audit record — neither code here); P3 item 5 open as *UI* → P7.
  **P5's spec is `packages/client/src/surface.ts` (144 keys); `src/preload.ts` is the authority, not
  `renderer/types/window.ts`.**
- **P6 done in scope** — 144/144 keys classified, zero `SPLIT`; all 51 §5c commands moved and statically
  guarded; `registerClientHandlers()` sits beside the reduced `registerIpcHandlers()` (`src/main.ts:294`).
  **Criterion 2 ("11 e2e specs pass, unmodified") is CARRIED** — owner's terminal only (46 failed / 0 passed).
  **P6 WI 2 (the process split) is downgraded by D14** — desktop stays in-process.
- **The reduction is CLOSED** — full ledger in `plan/07`; only the transferable rules below.

### The reduction — `plan/07` §5b-decies … §5b-vicendecies — **COMPLETE / CLOSED**

- **Nine groups, one file per commit, `src/` 20 → 17 files**, ending with three deletions
  (`applicationHandlers.ts` 10 Δ0, `runnerHandlers.ts` 6 Δ0, `importExportHandlers.ts` 4 **Δ−4** — the only
  genuine ratchet shrink). **Δ0 has three causes and the count separates none of them** — read per-file.
  **Nothing remains:** re-derived 2026-09-21 = **18 `ipcMain.handle(` sites, all keepers** (17 in
  `clientHandlers.ts`, 17 live / 0 dead; + 1 client-local in `systemHandlers.ts`). `coreKinds.ts` keeps 1
  `registerCrudKind<Environment>()` — **a kind is NOT a channel**. **Re-measure from the tree, not prose.**
- **Its transferable rules all live in `TRAPS.md` now** (the empty-`expectedChannels` cause of Δ0, the
  state-not-calls twin rule, re-point-don't-delete, a name is a claim, file-count guards, `git rm`). Full
  ledger in `plan/07`; procedure in the `bifurc-dead-channel-reduction` skill. Kept here only as a pointer.

## The executor pivot — `plan/14` (D10–D17) — **STARTED 2026-09-21**

**The deployable unit is a WORKSPACE, not the engine.** `plan/14` is the authority; D10–D17 in
`plan/00-decisions.md`. Workspace = executor · N per host via `WorkspaceRuntime` · desktop adopts the same
runtime · one RPC endpoint per host with `workspaceId` in params · **P6 WI 2 (process split) downgraded (D14)**
· CLI parked (D15) · **proxy base port `80` on win32 / `8080` elsewhere, counting up (D16)** · **blob staging
namespaced per workspace (D17)**. `@bifurc/engine` does **not** move.

- **WI 0 DONE — the gate, measured; the record is `plan/14` §13.** Three stateful modules
  (`proxy/server.ts`, `proxy/webhookServer.ts`, `transport/legacyCompanion.ts`), 22 reader sites,
  **11 lifecycle call sites of which exactly one is the shell** (`src/main.ts:309-310` = WI 7's single edit).
  Verdict **mechanical**; the readiness predicate is the one non-mechanical part — `index.ts:242`'s
  `waitUntilReady(isRunning, getServerError)`, and `isRunning()` must re-point at the *instance*, never wrap
  the global. **`activeWorkspaceId`'s 9 readers are mostly NOT the proxy** — only two are the server's
  (`server.ts:30-31`, `:122-123`), so **WI 4's scope is `proxy/server.ts` only**; the rest are command bodies
  using it as a param default.
- **WI 2 DONE — both factories, webhook receiver first as §6 requires.** `createWebhookServer({ workspaceId })`
  and `createProxyServer({ workspaceId, ownsCA })` each own their state; the module functions became **a facade
  over one lazily-created default instance**, kept until WI 7 (§9 rollback); **one file each, zero caller
  changes, zero `window.api` movement**, and in both cases the pre-existing cases passed with no edits — a
  facade's real proof is that its consumers cannot tell. Three facts pinned: **`MAX_ACTIVE_WEBHOOKS` stays a
  module constant** (per-workspace scopes the *map*, not the number; the renderer holds a duplicate `5`);
  **`webhookEmitter` must NOT be per instance**; **the `(workspaceId, urlSuffix)` routing key is WI 1's**, not
  WI 2's. Transferable hazards → `TRAPS.md` "De-singletonizing" (diff-before-delete when the factory body was
  derived from the original; a re-export block is part of the facade; `loadX()`/`unloadX()` is not made
  per-instance by wrapping it; correction-not-code).
- **§12 — the P0–P6 reconciliation (detail in `plan/14` §12).** §2.2's "blob staging is already
  per-workspace" is **false** (D17 namespaces it instead). **Auth is already built** (`transport/auth/*`), so
  WI 3 needs none — the checklist's unticked auth items are stale. `plan/14`'s "known callers" list is dated:
  two of its six were deleted by the reduction.
- **Three traps:** the port must NOT live in `workspace.json` (git-tracked → syncs → collides);
  **`config.save` must be SPLIT, not deleted** (proxy half `configCommands.ts:64-67` out, companion half
  `:68-75` stays — the two halves are *different functions*, so it is a clean block deletion); allocation must
  come from the real `listen()`, **never** `portUtils.checkPortInUse` (TOCTOU race).
- **§6's order:** prove each factory on the webhook receiver before `proxy/server.ts`. **Do not touch
  `store/paths.ts` or `processSpawner.ts`** — singletons by design. **Not done, recorded:** merging
  `registerCoreKinds()` into `crudKinds.ts` (a merge, not a deletion); D17; P14 WI 4–8.
- **WI 3 partial DONE 2026-09-21 — the runtime and the companion switch; `runtime/host.ts` still owed.**
  (1) **`EngineOptions.startCompanion?`** (default `true`): a host omits the companion, which `§13.3` found
  `createEngine()` binding unconditionally. Skips the start, the readiness wait **and the preflight port
  check**; `status().companionPort` reads `null`. (2) **`createWorkspaceRuntime()`** in
  `engine/src/runtime/workspaceRuntime.ts` composes the two WI 2 factories, with **two deliberate departures
  from the plan's sketch: no `dataDir`** (a runtime resolves no root and bootstraps nothing — the engine is a
  *library* by D10, and duplicating `setDataRoot()` gives a host two lifecycle authorities) and **ports
  required, not optional** (allocation is D16's rule needing host-wide knowledge; self-allocating would
  reintroduce the TOCTOU race). No `registry` either — a transport is per *host* by D13. (3) **The TLS-CA
  constraint WI 2 handed forward is CLOSED in the factory:** `createProxyServer()` gained **`ownsCA`**
  (default `true` = pre-P14 desktop). A host member passes `false` — it still `loadCA()`s its own configured
  CA (idempotent) but **never `unloadCA()`s**, because that one mkcert root outlives any runtime.
  **Ownership is the fix, not deletion** — removing the call leaks the CA past the stop it bounds. **Check
  the sibling branch too:** the *same* bug sat in `start()`'s `else` (TLS off ⇒ `unloadCA()`), which killed a
  TLS-enabled sibling; now owner-only, with a case. **Also removed:** `proxy/server.ts`'s six module-scope
  mutables, still declared one commit after the refactor that orphaned them — shadowed, unread, named only by
  a comment, so **`tsc` could not catch them**.
- **Open, never answered:** whether to retire the P0 spike code (`spike/protocol/` +
  `tests/spike/protocolPoc.test.ts`) now its fixture is complete (10 of 10 for seven groups) and it no longer
  depends on the shell. Retiring it is a *phase* decision — takes `spike/` plus five plan docs.
- **WI 5 allocator partial DONE** — `engine/src/runtime/portAllocator.ts`, 18 real-socket cases. D16's order
  is **two distinct paths, not one fallback: an override taken THROWS** (`PortUnavailableError`, naming the
  next free port — the message must be actionable or the rule is just a crash), while **a base taken
  SCANS** contiguously upward. A malformed `BIFURC_PROXY_BASE_PORT` throws rather than silently defaulting,
  and it moves the **base**, so the increment still works after it. **`tryBind()` does a REAL `listen()` and
  releases the handle before resolving — never `checkPortInUse`** (bind-then-close ⇒ TOCTOU; D16 names it).
  **The load-bearing assertion is "after `tryBind` says free, a real `listen()` succeeds"**, not "a taken
  port reads as taken" — the latter passes on a mock and would miss a leaked probe. **Still owed:** the
  `preflight()` port-check downgrade (a busy *base* must be informational — `plan/10`'s entrypoint exits on
  any `ok:false`, refusing the very config this phase enables) and the `checkPortAvailable` hint text.
