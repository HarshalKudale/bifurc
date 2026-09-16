# Bifurc — Project Memory

**Detail lives in the repo, not here** — `plan/README.md` (programme status, source of truth),
`plan/00-decisions.md`, `plan/03-phase-2-engine-extraction.md`, `TESTING.md`, `Cleanup_plan.md`.
Test gotchas + engine mechanics: the `bifurc-test-coverage` and `bifurc-engine-layer-move` skills.
Session history: `memory/YYYY-MM-DD.md`.

## Layout

- **Flat single-package repo** — the app IS the root (was `bifurc/` before 2026-09-14). `workspaces:
  ["packages/*"]`. Paths root-relative — **never `cd bifurc`**.
- **`packages/engine`** (P2 item 8, started 2026-09-15): **three layers moved** (49 files) —
  `src/{store,lib,subscription}`, then `src/{proxy,sync}` + `src/eventBus.ts` (mutually coupled, so
  they could not be split), then `src/{applications,companion,commands}`. Runtime deps: `mkcert`,
  `simple-git`, `ws`. Still in `src/`: `startup.ts` and `shutdown.ts` (plus `ipc/`, `main.ts`,
  `preload.ts` by design) — then the real `createEngine()`, which is what blocks the "engine starts
  from a bare Node script with `--data-dir`" criterion. Deliberately open: git `dialog.showErrorBox`
  not routed via `preflight()`; `importExport:*` out of the registry until P3.
- **Packaging does NOT follow the package split** (owned by `plan/12`). `build.files` covers only
  `dist/**/*` + `package.json`, and electron-builder does not dereference the
  `node_modules/@bifurc/*` workspace symlinks — so a **packaged** build cannot resolve
  `@bifurc/engine/*` or `@bifurc/protocol`. Dev mode and tests are fine. Never claim "the app still
  works" without this caveat.
- **P6 (shell seam) is the only milestone**; everything after is additive. Track C parked. Hard
  rules: **no renderer edits P1–P6**; no `ws` transport before auth; `window.api` byte-identical
  through P5–P6. `../bifurc-extension` is an external client; its 4 commands are frozen API.

## Engine facts verified by inspection — do not re-derive

- **The EventBus is `packages/engine/src/eventBus.ts`** (moved in layer 2) — the plan docs say
  `src/events/bus.ts` and are wrong; no `events/` directory has ever existed. Only remaining
  `BrowserWindow` in `src/`: `ipc/eventBridge.ts` (temporary bridge) and `clientHandlers.ts`
  (zoom/titlebar, CLIENT-classified).
- **`src/startup.ts`** — `preflight()` (read-only, never throws) + `bootstrapWorkspaces()`.
  **Preserved quirk** (see `plan/03`): it calls `initWorkspaceDir()` for every *listed* workspace,
  so the "active dir on disk?" check can never fail for one. Extraction, not a fix.
- **`src/shutdown.ts`** — memoised idempotent `shutdownEngine()` (spawner → pollers → companion →
  proxy). `processSpawner.stop()` has a per-entry `stopping` guard; entries stay in the map so
  `getState`/`getLogs` survive shutdown.
- **`packages/engine/src/commands/registry.ts`** — ~112 commands validating against frozen
  `@bifurc/protocol` Zod schemas; `entityKindMap.ts` bridges `"rules"`/`"sockets"` ↔
  `"proxyRules"`/`"wsConnections"`. `companion/allowedActions.ts` is the **frozen** API for the
  external `bifurc-extension` — it moved, but its contents must not change.
- `packages/engine/src/proxy/` has **zero** Electron imports. `companionServer.ts` (still in
  `src/companion/`) already speaks `{id,action,payload}` → `{id,ok,data,error}` over loopback WS
  9271 — generalise for P4. **No auth**; the `127.0.0.1` bind is its whole access control.

## Environment gotchas

- **`npx tsc` is NOT the compiler here** — resolves a placeholder, warns, **exits 0**. Use
  `npm run typecheck`. Same trap: piping vitest through `grep` returns grep's status, not vitest's.
- Sandbox, three traps: (1) `vitest --coverage` fails bulk-deleting its output dir — use
  `--coverage.clean=false --coverage.reportsDirectory=coverage-run<N>` with a **fresh** dir; a
  *failing* test aborts report generation entirely, so deselect `soap.execute` via
  `--testNamePattern='^(?!.*soap\.execute)'`. (2) After ~50+ deletions in a turn, `tsup`'s
  `bundle-require` cannot unlink its temp config, so `build:packages`/`typecheck`/`build:main`
  fail — not a code bug; re-run with the sandbox bypassed. (3) Playwright/Electron E2E needs a
  desktop session and **cannot run here**; the 11 e2e specs are unverified.
- Known non-regression: `tests/spike/protocolPoc.test.ts` `soap.execute` throws
  `ECONNREFUSED 127.0.0.1:1` (1–2 failures, varies); reproduces on the pre-change tree.
- **Don't batch `Edit` calls to one file in one message** — each reads the same original; last wins.
- Recurring bug classes (all fixed; patterns documented in `TESTING.md` §7): `reloadConfig()` must
  run **after** every on-disk write in a handler; `\${…}` in a template literal renders literally;
  `simple-git status()` reports a new file in BOTH `staged` and `created`; Node's global HTTP agent
  is shared, so a helper setting `connection: close` must also pin `agent: false`.
