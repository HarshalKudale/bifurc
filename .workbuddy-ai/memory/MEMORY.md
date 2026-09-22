# Bifurc — Project Memory

**Read `TRAPS.md` first** (failure modes not discoverable from the code). Authorities: state →
`plan/README.md` §Status · live phase → `plan/08` · decisions → `plan/00-decisions.md` · suites →
`TESTING.md` · ledgers → `plan/07` · procedures → the `bifurc-*` skills. **A pointer, not a record.**

> **Shell-bound commands are the user's to run** (`npm run dev`, `test:e2e`, anything Electron/GUI/GPU).
> An agent-shell failure is evidence about the shell, not the product. `~/.workbuddy-ai/MEMORY.md`.

## Layout — what silently wastes a session

- **Flat repo — the app IS the root.** `workspaces: ["packages/*"]`; never `cd bifurc`. Git root is the
  PARENT dir; branch `standalone-engine`. **`git rm` and `git stash` are BOTH destructive** — they cost
  the **object store**, not the tree.
- **App runs `packages/*/dist/`; tests run `src/`** → `npm run build:packages` after ANY package edit. Only
  `@bifurc/engine/*` aliases to `src`; the others resolve to `dist/`, so an un-rebuilt edit is **silently
  untested**. The dts leg can fail *after* "Build success" — grep `error TS`. Guard bypass and the coverage
  fix are both `CODEBUDDY_SAFE_DELETE_ENABLED=0`. **Directory renames are blocked in this sandbox** —
  `git mv` a *file* works, a *directory* gives "Permission denied"; move files individually.
- **One `Edit` per file per message** — two same-file edits silently revert each other.

## Measuring the suite

- **A *delta* is the signal; name its mechanism.** Baseline **re-measured, never carried forward**; **`Δ0`
  has three causes and the count separates none** — read per-file (`plan/07`).
- **There is NO standing failure — every failure is a flake to identify.** **The stable number is the TOTAL.**
- **An assertion must be able to fail — one mutation is not enough.** Prefer a NARROW mutation, check it is
  well-formed first, and **judge it by exit code, not by failure count.** **Check `HEAD` before committing.**

**Current: 2,674 tests / 117 files — 2,634 passed, 40 skipped, 0 failed.** e2e: 11 specs, 46/46 owner-run.

## Phase state

- **P0–P6 done** (P6 criterion 2 owner-run 46/46); P4 8/10; P3 item 5 open as *UI* → P7. **P14 CLOSED.**
- **P7 is live** (`plan/08`): `@bifurc/protocol/src/wire/`; **`@bifurc/web-client`**; **`packages/gui`** —
  the shared UI, exporting `<App />` — with hosts **`renderer/`** (Electron) and **`web/`** (browser), both
  importing it. **Run `tsc` after any type-level change — vitest does not typecheck.**
- **D18 (2026-09-22, LANDED): the renderer became `packages/gui`; only the communication module varies.**
  `window.api` stays the seam and each host fills it. **The UI carries 152 pre-existing type errors**, so no
  typecheck covers a host green — CI's web step is `continue-on-error`.
- **Not in the plans:** **P5's spec is `packages/client/src/surface.ts` (144 keys)**; `src/preload.ts` is the
  authority, not `packages/gui/src/types/window.ts`.
- **`build:main` is required after any `src/preload.ts` change** (`e8e535f`). Open gaps live in `plan/08`.
