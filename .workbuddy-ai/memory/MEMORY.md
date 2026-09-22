# Bifurc — Project Memory

**Read `TRAPS.md` first** (`.workbuddy-ai/memory/TRAPS.md` — failure modes not discoverable from the code).
Authorities: programme state → `plan/README.md` §Status · live phase → `plan/08` (P7) · closed phases →
`plan/14` (P14) · decisions → `plan/00-decisions.md` · suites → `TESTING.md` · ledgers → `plan/07` ·
procedures → the `bifurc-*` skills. **This file is a pointer plus the rules that cost the most.**

## 🚫 Standing rule — ask the user to run shell-bound commands

Anything needing a real desktop session / Electron / GPU (`npm run dev`, `npm run test:e2e`, any GUI) runs **by
the user**. Full reasoning in `~/.workbuddy-ai/MEMORY.md`. **An agent-shell failure is evidence about the agent
shell, not the product** — name the mechanism that makes it environment-specific, or ask for a run. **Never
start a build you will not wait for.**

**The evidence for a failed run is already on disk — read it before asking for a re-run.** `only-on-failure`
screenshots + `trace: on-first-retry` leave `test-results/<spec>-<test>-electron/error-context.md` (the error
**plus a full accessibility snapshot**) and `test-failed-1.png`; a *passing* run leaves
`playwright-report/index.html`, whose embedded `report.json` `stats` decode with
`re.search(r'data:application/zip;base64,(…)'` + `zipfile`.

## Layout — five things that silently waste a session

- **Flat repo — the app IS the root.** `workspaces: ["packages/*"]`; never `cd bifurc`. The repo is
  `Bifurc/.git` (the git root is the PARENT dir), branch `standalone-engine`. Check
  `git rev-parse --show-toplevel` before any *write*.
- **`git rm` and `git stash` are BOTH destructive here** — they cost the **object store** (`.git/refs`,
  `.git/index`, loose objects), not the tree; `git stash push` cost five commits on 2026-09-21. Use the
  plain-filesystem equivalent + `git add`. Recovery in `TRAPS.md`.
- **App runs `packages/*/dist/`; tests run `src/`** → `npm run build:packages` after ANY package edit. The
  vitest alias covers `@bifurc/engine/*` **only**; `@bifurc/protocol`/`@bifurc/client` resolve to built
  `dist/`, so an un-rebuilt edit is **silently untested** (`npm test` does not build). The dts leg can fail
  *after* the JS leg prints "Build success" — grep `error TS`. Bypass the safe-delete guard with
  `env -u CODEBUDDY_SAFE_DELETE_BULK_STATE_DIR -u CODEBUDDY_TOOL_CALL_ID …`, **in the background**.
- **`node`/`require` cannot load absolute `I:\…` module paths** — write probes *inside* the repo with a bare
  specifier.
- **Tooling: one `Edit` per file per message.** Two `Edit` calls to the *same* file in one message silently
  revert each other. Parallel edits to *different* files are fine. A same-file `EBUSY` is transient.

## Measuring the suite

- **A *delta* is the signal; the standing failure is not. Name the mechanism for every delta.** Baseline is
  **re-measured, never carried forward**; **`Δ0` has three causes and the count separates none** — read
  per-file (`plan/07`).
- **For any generated / looped / `it.each` suite, NEVER count by hand** — enumerate the run
  (`--reporter=json` → `assertionResults[].title`), then subtract. **Exception: when the claim *is* a count of
  occurrences in a source file, reading the file and asserting the count is the direct instrument.**
- **There is NO standing failure — every failure is a flake to *identify*, not assume.** The load-induced pair
  is `gitStore.test.ts` + `gitOps.test.ts` (`EBUSY`/hook-timeout; 28/28 isolated in 120 s). **The stable
  number is the TOTAL, not the passed/failed split.**
- **A retired test's contract must be shown to have a better home, per case — never assumed.** "Does anything
  else test X?" is not answerable by counting files that mention X.
- **An assertion must be able to fail — and one mutation is not enough to show it does.** Ask of every
  instrument: *what would it fail on?* **Prefer a NARROW mutation**, and **check it is well-formed first**.
- **A negative verdict is a claim too.** "I did not find it" is not "it is not there"; searching one directory
  is the same error as counting the files that mention a thing.
- **A checkbox and a table cell are two copies of one claim, and they drift.** Counting a document's own
  markers is a one-line cross-check on whether two places that state the same thing agree.
- **A repo can change under you.** Check `HEAD` before committing; re-measure after. An uncommitted body of
  work is the largest single risk on the board.

**Current: 2,646 tests / 114 files — 2,606 passed, 40 skipped, 0 failed** (2026-09-22, after P7's browser
transport adapter landed). The refactor *alone* was measured separately first — 2,638, **zero delta** — and the
**+8** is exactly the new browser cases. **e2e: 11 specs / 46 tests — 46 expected, 0 unexpected, 0 flaky,
owner-measured 2026-09-22.** Trajectory in `plan/07`.

## Phase state — the plans are the authority

- **P0–P6 done** (P6 criterion 2: 11/11 specs, 46/46 tests, owner-run; the seven failures getting there were
  **all test- or fixture-side, no product code changed** — table in `TRAPS.md`). P4 8/10 (extension release +
  audit record — neither is code here); P3 item 5 open as *UI* → P7. **P5's spec is
  `packages/client/src/surface.ts` (144 keys); `src/preload.ts` is the authority, not
  `renderer/types/window.ts`.**
- **Dead-channel reduction CLOSED** — `src/` 20 → 17 files. Re-derived from the tree = **18 real
  `ipcMain.handle(` sites, all keepers** (a bare `grep` of 20 also catches one comment in `rpcBridge.ts` and
  the bridge itself). `coreKinds.ts` keeps one `registerCrudKind<Environment>()` — **a kind is NOT a
  channel**. **Re-measure from the tree, not prose.**
- **The sandboxed-preload defect is FIXED** (`e8e535f`): `build:main` bundles `src/preload.ts` with esbuild.
  **`build:main` is required after any `src/preload.ts` change.** The guard *evaluates* the bundle with a
  `require` that knows only Electron's five names — a regex or static require-walk proves reachability, not
  loadability.

## P7 — the web UI (`plan/08`) — the live phase, opened 2026-09-22

**WI 0 (the opening measurement) done.** **119 renderer IPC methods / 234 call sites / 209 files, 0 without a
`surface.ts` key; 144 surface keys, 25 never called** — and those 25 are almost exactly item 4's target. Item
3's `dialog:*` channels **no longer exist** (P6 removed them; grep = 0); the live surface is
`window.api.<method>`, 16 of them file/dialog-shaped rather than 8. **No web target in the tree.**

**WI 1's adapter is BUILT** — `@bifurc/web-client` (`packages/web-client`): 8 cases against a **real engine over
a real socket**, mutation-checked. It implements `ClientChannel` and lets `createClientSession` own the rest; a
second session would be the third copy `session.ts` exists to prevent. **The wire machinery (`types.ts` +
`session.ts`) now lives in `@bifurc/protocol/src/wire/`** — protocol is the package that already enforces
`platform: "neutral"`, so the browser's graph contains no Node built-in (grepped from `dist`, not assumed).
**`framing.ts` stayed in engine** (byte-stream framing; a WebSocket needs none of it). **The browser sends
*text* frames**, matching the engine's server and its own `ws` client. Engine keeps thin re-exports, so the
shell is untouched. `@bifurc/client` was *already* `platform: "neutral"` — only the transport was ever missing.

**Still open, and the next real work:** no web target exists (`apps/web`), and **`ws.ts`'s client still does
not use `createClientSession`** — it carries its own copy of the state machine (`TRAPS.md`).

## P14 — the executor pivot (`plan/14`, D10–D17) — CLOSED, §8 is 21 met · 0 open

Unit = a **workspace**, not the engine (N per host via `WorkspaceRuntime`; desktop adopts the same runtime; one
RPC endpoint per host with `workspaceId` in params; D14 split downgraded; D15 CLI parked; D16 base `80` win32 /
`8080` elsewhere; D17 per-workspace blob staging). `@bifurc/engine` does **not** move. **Narrative in
`plan/14`; five rules worth carrying:**

1. **An optional wire parameter is done when the *body* reads it, not when the schema accepts it** —
   `server.start {workspaceId:"A"}` returned **`{ ok: true }`** while binding a third server on the *active*
   workspace's port. **The failure succeeds**, so no suite caught it.
2. **A reserved-port set must not include its own base** — `findFreePortFrom(base)` returns `base + 1`, and the
   host test *observed* that off-by-one and called it correct.
3. **Namespacing a store surfaces every half-threaded id, and only the integration suite sees it** (D17: 0 → 30
   failures). **Scope the caller, never relax the store**; audit by asking *"which calls pass NO id?"*.
4. **A path a header calls temporary is still the live implementation** (`proxy/server.ts`, "removed in WI 7").
5. **Rewording a defective criterion can weaken its claim** — criterion 6 went green on a *substitute* claim
   (reproducibility) while D16's stickiness rule stayed unimplemented. **Quote the decision in the criterion,
   and re-test the old claim rather than retiring it.**

**D16's split: an override must always fail loudly; a *remembered* port must never be a hard failure** (it
falls through to the scan). **A write-only field is an unimplemented decision** — `resolvedProxyPort` was
written and read by no product path; **grep the *readers*, not the writers.** Also: **a filter cannot recover an
absent input** (WI 4's dispatch gap needed a loader); **a module's surface is a fixture too** (a new export
breaks every `vi.mock` of it); **resolution has three outcomes and the third is load-bearing** (runtime →
facade → **`absent`**, since falling through re-creates the bug with a success code). **A command never
allocates a port.**
