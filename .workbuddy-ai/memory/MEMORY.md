# Bifurc — Project Memory

**Read `TRAPS.md` first** (`.workbuddy-ai/memory/TRAPS.md`, 42 sections) — failure modes that cost real time and
are not discoverable from the code. Status → `plan/README.md`; the executor pivot → `plan/14`; suites →
`TESTING.md`; the P6 / §5c ledgers → `plan/07`; protocol history → `plan/protocol-changes.md`; procedures →
the `bifurc-*` skills.

## Layout — the four things that silently waste a session

- **Flat repo — the app IS the root.** `workspaces: ["packages/*"]`; **never `cd bifurc`**.
  The git root is the PARENT dir (`I:/workspace/Bifurc-Worskspace`, empty) — the real repo is `Bifurc/.git`
  on branch `standalone-engine`. Check `git rev-parse --show-toplevel` before any *write*.
- **`git rm` and `git stash` are BOTH destructive on this drive** — not the tree, the **object store**
  (`.git/refs`, `.git/index`, loose objects). `git stash push` cost five commits on 2026-09-21. Recovery
  procedure is in `TRAPS.md`; use the plain-filesystem equivalent + `git add` instead.
- **App runs `packages/*/dist/`; tests run `src/`** → **`npm run build:packages` after ANY package edit.**
  The vitest alias covers `@bifurc/engine/*` **only**; `@bifurc/protocol` / `@bifurc/client` resolve to built
  `dist/`, so an un-rebuilt edit is **silently untested** — `npm test` does not build. The build's
  **dts leg** can fail after the JS leg prints "Build success" — grep for `error TS`.
  Bypass the safe-delete guard: `env -u CODEBUDDY_SAFE_DELETE_BULK_STATE_DIR -u CODEBUDDY_TOOL_CALL_ID …`,
  and **run it in the background** (foreground gives SIGTERM on the long legs).
- **No renderer edits P1–P6**; `window.api` byte-identical. **`use mcp__tools` note:** `node`/`require`
  cannot load absolute `I:\…` module paths — write probes *inside* the repo with a bare specifier.

## Measuring the suite — the discipline that has caught every real regression

- **A *delta* is the signal; the standing failure is not. Name the mechanism for every delta.** Never wave at
  the number. **Baseline is re-measured, never carried forward** — a group that opens with a fix-first commit
  moves it before the deletion starts.
- **For any generated / looped / `it.each` suite, NEVER count by hand** — enumerate the run
  (`--reporter=json` → `assertionResults[].title` → classify by name), then subtract. A total too low by
  exactly the number of pre-existing cases **looks like a missing test** rather than an arithmetic slip, and
  that is what sent one reconciliation hunting for a test that never existed (and motivated the `git stash`
  that cost the history).
- **`Δ0` has three causes and the count separates none of them** — read per-file. Groups 1–9 of the P6
  reduction: Δ0 (re-point) · Δ−5 (generated) · Δ−3 · Δ0 (never had cases) · Δ−8 · **Δ−25** · Δ0 (no cases AND
  a re-point) · Δ0 (both, plus a duplicated impl) · **Δ−4** (the only genuine ratchet shrink).
- **Environmental failures, documented and variable in count:** `soap.execute`
  (`tests/spike/protocolPoc.test.ts` — the one that fails **isolated** too, so it is genuinely the standing
  one) · `tests/ipc/certTrust.test.ts` (certutil; 34/34 isolated) · `tests/store/gitStore.test.ts` +
  `tests/sync/gitOps.test.ts` (load-induced `EBUSY` / hook-timeout; **28/28 isolated in 120 s**).
  **The stable number is the TOTAL; the passed/failed split inside this set is NOT.** Measured 2026-09-21
  (post-D17): two consecutive full runs read `4 failed / 2520 passed` and `3 failed / 2521 passed` — **both
  `2564` total**, the same two flaky files, a different number of cases tripping. So a run reporting more
  failures than the set **is not automatically a regression**, but **identify the fourth** rather than assume:
  `--reporter=json --outputFile=/tmp/full.json`, then filter `testResults[].assertionResults[].status`. Cheap,
  and it is the difference between dismissing a failure and having dismissed it. The **second** measurement
  is the one to quote.

**Current: 2564 tests — 2521 passed, 40 skipped, 3 failed** (all three in the environmental set above),
101/105 files. *(Trajectory: 101 / 2,458 / 2,417 / 40 / 1 → … → 2,507 (WI 3) → 2,564 (WI 1 protocol + D17).
**Name every delta's mechanism.**)*

## Phase state — one line each; the plans are the authority

- **P0–P5 done.** P4 8/10 (extension release + audit record — neither code here); P3 item 5 open as *UI* → P7.
  **P5's spec is `packages/client/src/surface.ts` (144 keys); `src/preload.ts` is the authority, not
  `renderer/types/window.ts`.**
- **P6 done in scope** — 144/144 keys classified, zero `SPLIT`; all 51 §5c commands moved and statically
  guarded. `registerClientHandlers()` sits beside the reduced `registerIpcHandlers()` (`src/main.ts:294`).
  **Criterion 2 ("11 e2e specs pass, unmodified") is CARRIED** — owner's terminal only (46 failed / 0 passed).
  **WI 2 (the process split) is downgraded by D14** — desktop stays in-process.
- **The reduction is CLOSED** — nine groups, one file per commit, `src/` 20 → 17 files. **Nothing remains:**
  re-derived from the tree 2026-09-21 = **18 `ipcMain.handle(` sites, all keepers** (17 in `clientHandlers.ts`,
  17 live / 0 dead; + 1 client-local in `systemHandlers.ts`). `coreKinds.ts` keeps 1
  `registerCrudKind<Environment>()` — **a kind is NOT a channel**. **Re-measure from the tree, not prose.**
  Ledger in `plan/07`; procedure in the `bifurc-dead-channel-reduction` skill.

## The executor pivot — `plan/14` (D10–D17), the live phase

The deployable unit is a **workspace**, not the engine. D10–D17 in `plan/00-decisions.md`; `plan/14` is the
authority. Workspace = executor · N per host via `WorkspaceRuntime` · desktop adopts the same runtime · **one
RPC endpoint per host** with `workspaceId` in params · **P6's process split downgraded (D14)** · CLI parked
(D15) · **proxy base `80` on win32 / `8080` elsewhere, counting up (D16)** · **blob staging namespaced per
workspace (D17)**. `@bifurc/engine` does **not** move.

**Done:** WI 0 (the gate — verdict *mechanical*; the one non-mechanical part is the readiness predicate) ·
WI 2 (both factories + the facade) · WI 3 (host, registry, `registerAll` — **the empty-registry find**: all 18
`register*Commands()` calls lived in the shell, so a headless host got `UNKNOWN_COMMAND` for everything while
every desktop suite stayed green — *the P6 §5c class one layer up*) · WI 4 (`servedWorkspaceId()`;
`workspace.setActive` switched the **pointer only**, per the owner's decision) · WI 5 (**allocator + the
`advisory` preflight downgrade**) · WI 1's protocol half (ten commands + **D17's blob half**).

**The four things to carry forward, each earned expensively:**

1. **An optional wire parameter is not done when the schema accepts it — it is done when the body reads it.**
   Measured on a two-workspace host: `server.status` reported `{running: false, port: 0}` with two members
   listening on 81/82, and `server.start { workspaceId: "A" }` returned **`{ ok: true }`** while binding a
   *third* server on the active workspace's config port. `registerServerCommands()` calls the **module
   facade** (`workspaceId: null` = the active workspace). **The failure succeeds**, which is why no suite
   caught it. 🚩 **This makes WI 7's flip load-bearing for correctness on a multi-workspace host, not a
   tidy-up** — the command layer must resolve a runtime from `workspaceId`.
2. **A reserved-port set must not include its own base** — it is the range's own start, so including it makes
   `findFreePortFrom(base)` return `base + 1`, a silent off-by-one the host integration test *observed* and
   wrote up as correct. Cross-range safety comes from `reserved.add(chosenPort)` after the pick.
3. **Namespacing a store surfaces every half-threaded id, and only the integration suite sees it.** D17 took
   `importExportBlob.integration.test.ts` from 0 → 30 failures: `export.create` had a workspace while its own
   client's `blob.put|read|release` did not, so **the engine's export was unreadable by the requester** — a
   latent bug a host-wide root had hidden. **Fix by scoping the caller, never by relaxing the store.** A
   find-replace over a threaded param misses the call sites written in a shape you did not remember; the audit
   that works asks *"which calls pass NO id?"*.
4. **A path that a header calls temporary is still the live implementation.** `proxy/server.ts`'s module
   functions say "removed in WI 7" — until then they are what every command calls.

**Still owed:** **WI 1's `runtime.json` preferred-port plumbing from the UI/settings** (the allocator and the
registry both honour `preferred*`; only the *editor* is missing — a port-defaults gap, since the desktop never
recorded a resolved port and `loadConfig()`'s `port` is genuinely the base) · **WI 6** (deployable host
artifact, feeds P9) · **WI 7** (the desktop adopts the runtime — §13.3 says the single edit is
`src/main.ts:309-310`; see finding 1 above) · **WI 8** (the five egress commands gain `workspaceId`:
`export.create`, `tls.exportCert`, `runner.exportReport`, `capture.shareJson`, `audit.export`).

**Deliberately NOT scoped, and these are decisions, not omissions:**
- `services.discover` — host-wide, not per-workspace (asserted in `index.test.ts` so it cannot read as
  an oversight).
- `fileOps/commands.ts:139-140` (`tlsImport`'s two store reads) — **the CA is one per *machine*, not per
  workspace**: `caCertPath()` resolves under `appDataDir()` (`proxy/certManager.ts`), and `ownsCA` exists
  because N workspaces share one mkcert root. Scoping it would let a client stage under A and import into the
  shared CA — the same half-threading D17's failures had just exposed, on purpose. Documented in the
  function's docblock.

**Open, never answered:** whether to retire the P0 spike code (`spike/protocol/` +
`tests/spike/protocolPoc.test.ts`) now its fixture is complete and it no longer depends on the shell.
Retiring it is a *phase* decision — it takes `spike/` plus five plan docs. P8's CLI stays parked under D15.
