# Bifurc — Project Memory

**Detail lives in the repo, not here** — `plan/README.md` (programme status, source of truth),
`plan/00-decisions.md`, `plan/03-phase-2-engine-extraction.md`, `TESTING.md`, `Cleanup_plan.md`.
Test gotchas + engine mechanics: the `bifurc-test-coverage` and `bifurc-engine-layer-move` skills.
Session history: `memory/YYYY-MM-DD.md`.

## Layout

- **Flat single-package repo** — the app IS the root (was `bifurc/` before 2026-09-14). `workspaces:
  ["packages/*"]`. Paths root-relative — **never `cd bifurc`**.
- **`packages/engine` (P2 item 8): done.** 52 files moved in four layers
  (`src/{store,lib,subscription}`, `src/{proxy,sync}` + `eventBus.ts`, `src/{applications,companion,
  commands}`, `src/{startup,shutdown}.ts`). `src/` holds
  only `ipc/`, `main.ts` and `preload.ts` by design. **`createEngine({dataDir})` →
  `{start,stop,status,registry,bus}`** is implemented and proven end to end by
  `tests/integration/engineSmoke.integration.test.ts`. Deliberately open: git `dialog.showErrorBox`
  not routed via `preflight()`; `importExport:*` out of the registry until P3; and **on Windows
  `--data-dir` is not authoritative** — `workspaceFs`/`appSettings` check `%LOCALAPPDATA%` before the
  data root (pre-existing; fixing it would move existing users' data).
- **Packaging does NOT follow the package split** (`plan/12`; also a P9 blocker). `build.files` covers
  only `dist/**/*` + `package.json`, and electron-builder does not dereference the
  `node_modules/@bifurc/*` symlinks — so a **packaged** build cannot resolve `@bifurc/engine/*` or
  `@bifurc/protocol`. `plan/10`'s Dockerfile shares the gap: only `dist` + the root `package.json`
  are copied, with **no install step**. Dev mode and tests are fine. Engine runtime deps are exactly
  `@bifurc/protocol`, `mkcert`, `simple-git`, `ws`; `js-yaml`/`archiver`/`unzipper` belong to
  `src/ipc/importExport/**`, not the engine.
- **P6 (shell seam) is the only milestone**; everything after is additive. Track C parked. Hard
  rules: **no renderer edits P1–P6**; no `ws` transport before auth; `window.api` byte-identical
  through P5–P6. `../bifurc-extension` is an external client; its 4 commands are frozen API.

## Engine facts verified by inspection — do not re-derive

- **The EventBus is `packages/engine/src/eventBus.ts`** — the plan docs say `src/events/bus.ts` and
  are wrong; no `events/` directory has ever existed. Only remaining `BrowserWindow` in `src/`:
  `ipc/eventBridge.ts` (temporary bridge) and `clientHandlers.ts` (zoom/titlebar, CLIENT-classified).
- **`startup.ts`** — `preflight()` (read-only, never throws) + `bootstrapWorkspaces()`. **Preserved
  quirk** (`plan/03`): it calls `initWorkspaceDir()` for every *listed* workspace, so the "active dir
  on disk?" check can never fail for one. **`shutdown.ts`** — memoised idempotent `shutdownEngine()`
  (spawner → pollers → companion → proxy); `processSpawner.stop()` has a per-entry `stopping` guard.
- **`packages/engine/src/commands/registry.ts`** — ~112 commands validating against frozen
  `@bifurc/protocol` Zod schemas; `entityKindMap.ts` bridges `"rules"`/`"sockets"` ↔
  `"proxyRules"`/`"wsConnections"`. `createEngine()` **registers none** — the *consumer* does (shell
  today, RPC client at P6); assert registry identity, not population.
  `companion/allowedActions.ts` is the **frozen** API for the external `bifurc-extension` — its
  contents must not change.
- `packages/engine/src/proxy/` has **zero** Electron imports. `companion/companionServer.ts` speaks
  `{id,action,payload}` → `{id,ok,data,error}` over loopback WS 9271 — generalise for P4. **No auth**;
  the `127.0.0.1` bind is its whole access control.

## Environment gotchas

- **`npx tsc` is NOT the compiler here** — resolves a placeholder, warns, **exits 0**. Use
  `npm run typecheck`. Same trap: piping vitest through `grep` returns grep's status, not vitest's.
- Sandbox, three traps: (1) `vitest --coverage` fails bulk-deleting its output dir — use
  `--coverage.clean=false --coverage.reportsDirectory=coverage-run<N>` with a **fresh** dir; a
  *failing* test aborts the report, so deselect `soap.execute` via
  `--testNamePattern='^(?!.*soap\.execute)'`. (2) After ~50+ deletions in a turn, `tsup`'s
  `bundle-require` cannot unlink its temp config, so builds fail — not a code bug. (3) Electron E2E
  needs a desktop session — **cannot run here**.
- Known non-regression: `tests/spike/protocolPoc.test.ts` `soap.execute` throws
  `ECONNREFUSED 127.0.0.1:1`; reproduces on the pre-change tree.
- **Don't batch `Edit` calls to one file in one message** — each reads the same original; last wins.
- Recurring bug classes: `TESTING.md` §7 (all fixed; `reloadConfig()` must run after every on-disk
  write, `\${…}` in a template literal renders literally, `simple-git status()` double-lists new files).