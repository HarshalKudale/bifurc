# Bifurc — Project Memory

**Read `TRAPS.md` first** (failure modes not discoverable from the code). Authorities: state →
`plan/README.md` §Status · live phase → `plan/08` · decisions → `plan/00-decisions.md` · suites →
`TESTING.md` · ledgers → `plan/07` · procedures → the `bifurc-*` skills. **A pointer, not a record.**

> **Electron/GUI commands are the user's to run** (`npm run dev`, `test:e2e`) — an agent-shell failure is
> evidence about the shell, not the product. `~/.workbuddy-ai/MEMORY.md`.

## Layout

- **Flat repo — the app IS the root.** `workspaces: ["packages/*"]`; never `cd bifurc`. Git root is the
  PARENT dir; branch `standalone-engine`. **`git rm` and `git stash` are BOTH destructive.**
- **App runs `packages/*/dist/`; tests run `src/`** → `npm run build:packages` after ANY package edit. Only
  `@bifurc/engine/*` aliases to `src`; the others resolve to `dist/`, so an un-rebuilt edit is **silently
  untested**. Guard bypass and the coverage fix are both `CODEBUDDY_SAFE_DELETE_ENABLED=0`. **Directory
  renames are blocked here** — a *file* `git mv` works, a *directory* does not.
- **One `Edit` per file per message** — two same-file edits silently revert each other.

## Measuring the suite

- **A *delta* is the signal; name its mechanism.** Baseline **re-measured, never carried forward**; **`Δ0`
  has three causes and the count separates none** — read per-file (`plan/07`).
- **There is NO standing failure — every failure is a flake to identify.** **The stable number is the TOTAL.**
- **An assertion must be able to fail — one mutation is not enough.** Prefer a NARROW mutation and **judge it
  by exit code, not by failure count.** **Check `HEAD` before committing.**

**Current: 2,693 tests / 118 files — 2,653 passed, 40 skipped, 0 failed.** e2e: 11 specs, 46/46 owner-run.
**A running headless host holds 80/8080/9101/9271** — port-dependent suites then fail, and say so.

## Phase state

- **P0–P6 done**; P4 8/10; P3 item 5 open as *UI* → P7. **P14 CLOSED.** **P5's spec is
  `packages/client/src/surface.ts` (144 keys).**
- **P7 is live** (`plan/08`): **`packages/gui`** — the shared UI, exporting `<App />` — with hosts
  **`renderer/`** (Electron) and **`web/`**, a **container**: form → handshake → `<App host="web" />`.
  **Run `tsc` after any type-level change — vitest does not typecheck.**
- **D18 (2026-09-22, LANDED)**: the renderer became `packages/gui`; `window.api` stays the seam and each host
  fills it. **The UI carries 152 pre-existing type errors**, so no typecheck covers a host green.
- **The browser's transport is auth-wrapped; Electron's is not.** `hello` goes on the **raw** transport
  *before* `createClient` (`plan/06`), then a probe. **`HELLO_ACTION` is in `@bifurc/protocol`.**
- **A headless browser DOES run from the agent shell** — the project's Playwright plus the cached Chromium at
  `chromium-1243` via `executablePath` (1.60 expects 1223). The web UI is verifiable here; recipe in
  `plan/08` WI 6. **Do not repeat "the owner runs it" without re-measuring.**
- **`build:main` is required after any `src/preload.ts` change** (`e8e535f`). Open gaps live in `plan/08`.
