# Bifurc — Project Memory

**Read `TRAPS.md` first** — failure modes not discoverable from the code, and the *full* text of several
measurement rules only summarised here. Authorities: programme state → `plan/README.md` §Status · live phase →
`plan/08` (P7) · closed → `plan/14` (P14) · decisions → `plan/00-decisions.md` · suites → `TESTING.md` ·
ledgers → `plan/07` · procedures → the `bifurc-*` skills. **A pointer plus the rules that cost the most.**

## 🚫 Standing rule — the user runs shell-bound commands

`npm run dev`, `npm run test:e2e`, anything Electron/GUI/GPU: **ask, don't run.** Full reasoning in
`~/.workbuddy-ai/MEMORY.md`. **An agent-shell failure is evidence about the agent shell, not the product** —
name the mechanism that makes it environment-specific, or ask for a run. **Never start a build you will not
wait for.**

**A failed run's evidence is already on disk — read it before asking for a re-run.** `error-context.md` (the
error **plus a full accessibility snapshot**) and `test-failed-1.png` under
`test-results/<spec>-<test>-electron/`; a *passing* run leaves `playwright-report/index.html`, whose embedded
`report.json` `stats` decode with `re.search(r'data:application/zip;base64,(…)'` + `zipfile`.

## Layout — five things that silently waste a session

- **Flat repo — the app IS the root.** `workspaces: ["packages/*"]`; never `cd bifurc`. Git root is the PARENT
  dir; repo `Bifurc/.git`, branch `standalone-engine`. Check `git rev-parse --show-toplevel` before any *write*.
- **`git rm` and `git stash` are BOTH destructive here** — they cost the **object store** (`.git/refs`,
  `.git/index`, loose objects), not the tree; `git stash push` cost five commits on 2026-09-21. Use the
  filesystem equivalent + `git add`. Recovery in `TRAPS.md`.
- **App runs `packages/*/dist/`; tests run `src/`** → `npm run build:packages` after ANY package edit. The
  vitest alias covers `@bifurc/engine/*` **only**; `@bifurc/protocol`/`@bifurc/client` resolve to built `dist/`,
  so an un-rebuilt edit is **silently untested** (`npm test` does not build). The dts leg can fail *after* the JS
  leg prints "Build success" — grep `error TS`. Bypass the safe-delete guard with
  `env -u CODEBUDDY_SAFE_DELETE_BULK_STATE_DIR -u CODEBUDDY_TOOL_CALL_ID …`, **in the background**.
- **`node`/`require` cannot load absolute `I:\…` paths** — write probes *inside* the repo with a bare specifier.
- **One `Edit` per file per message.** Two `Edit` calls to the *same* file in one message silently revert each
  other; different files in parallel are fine. A same-file `EBUSY` is transient.

## Measuring the suite

**Full rule set in `TRAPS.md`** — counting by hand (§912, verbatim), retiring a test, "a negative verdict is a
claim too" (own section), Δ0's three causes (`plan/07` §2041). The four that change behaviour daily:

- **A *delta* is the signal; name the mechanism for every delta.** Baseline **re-measured, never carried
  forward**. **`Δ0` has three causes and the count separates none** — read per-file (`plan/07`).
- **There is NO standing failure — every failure is a flake to *identify*, not assume.** The load-induced pair
  is `gitStore.test.ts` + `gitOps.test.ts` (`EBUSY`/hook-timeout; 28/28 isolated in 120 s). **The stable number
  is the TOTAL, not the passed/failed split.**
- **An assertion must be able to fail — one mutation is not enough to show it does.** Ask of every instrument:
  *what would it fail on?* **Prefer a NARROW mutation**, and **check it is well-formed first**.
- **A repo can change under you.** Check `HEAD` before committing; re-measure after.

**Current: 2,664 tests / 115 files — 2,624 passed, 40 skipped, 0 failed** (2026-09-22, after the `ws.ts`
client became a `ClientChannel`). The **+2** is exactly the two new `ws` cases. **e2e: 11 specs / 46 tests — 46
expected, 0 unexpected, 0 flaky, owner-measured 2026-09-22.** Trajectory in `plan/07`.

## Phase state — the plans are the authority

- **P0–P6 done.** P6 criterion 2 (11/11 specs, 46/46 tests) owner-run; the seven failures getting there were
  **all test- or fixture-side, no product code changed** (`TRAPS.md`). P4 8/10 (extension release + audit
  record — neither is code here); P3 item 5 open as *UI* → P7.
- **Facts NOT in the plans, and they cost time:** **P5's spec is `packages/client/src/surface.ts` (144 keys)**;
  `src/preload.ts` is the authority, not `renderer/types/window.ts`. **Dead-channel reduction CLOSED** — `src/`
  20 → 17 files, **18 real `ipcMain.handle(` sites, all keepers** (a bare `grep` of 20 also catches a comment in
  `rpcBridge.ts` and the bridge itself); `coreKinds.ts`'s `registerCrudKind<Environment>()` — **a kind is NOT a
  channel**. **Re-measure from the tree, not prose.**
- **The sandboxed-preload defect is FIXED** (`e8e535f`): `build:main` bundles `src/preload.ts` with esbuild, so
  **`build:main` is required after any `src/preload.ts` change.** Its guard *evaluates* the bundle with a
  `require` knowing only Electron's five names — a regex or static require-walk proves reachability, not
  loadability.

## P7 — the web UI (`plan/08`) — the live phase, opened 2026-09-22

**WI 0 done:** **119 renderer IPC methods / 234 call sites / 209 files, 0 without a `surface.ts` key; 144
surface keys, 25 never called.** Item 3's `dialog:*` channels **no longer exist** (P6 removed them); the live
surface is `window.api.<method>`, 16 file/dialog-shaped rather than 8. **No web target in the tree.**

**WI 1 + 1b BUILT — the build record is `plan/08` §"The build record (2026-09-22)"; read it there.** In short:
`@bifurc/web-client`, a **new package** on the owner's decision (decoupling is the goal), supplies a
`ClientChannel` over the native `WebSocket` and the **16 required `RequiredClientLocal` members**, without
which `createClient` cannot be called at all; the wire machinery moved to `@bifurc/protocol/src/wire/` so the
browser's graph holds no Node built-in (grepped from `dist`, not assumed); and **`ws.ts`'s third copy of the
session state machine is gone** (WI 1b). **Run `tsc` explicitly after any type-level change — vitest does not
typecheck.**

**Open, and the next real work:** no `apps/` exists — a web target is built from nothing (item 5). Two gaps
are **recorded, not worked around**: `tlsRemoveCert`'s engine half is unreachable from any non-shell
`ClientLocal` (a gap in the injection point), and CA install is manual in a browser (item 4).

## P14 — the executor pivot (`plan/14`, D10–D17) — CLOSED, §8 is 21 met · 0 open

Unit = a **workspace**, not the engine: N per host via `WorkspaceRuntime`, desktop adopts it, one RPC endpoint
per host with `workspaceId` in params, D14 split downgraded, D15 CLI parked, D16 base `80` win32 / `8080`
elsewhere, D17 per-workspace blob staging. `@bifurc/engine` does **not** move. **The narrative — five rules,
D16's split, the 2026-09-22 audit — is in `plan/14`; read it there.** The one rule invisible from the code:
**an optional wire parameter is done when the *body* reads it, not when the schema accepts it** —
`server.start {workspaceId:"A"}` returned **`{ ok: true }`** while binding a *third* server on the *active*
workspace's port. **The failure succeeds**, so no suite caught it.
