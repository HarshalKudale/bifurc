# Bifurc — Project Memory

**Detail lives in the repo, not here** — `plan/README.md` (programme status, source of truth),
`plan/00-decisions.md`, `plan/03-phase-2-engine-extraction.md`, `TESTING.md`, `Cleanup_plan.md`.
Test gotchas + engine mechanics: the `bifurc-test-coverage` and `bifurc-engine-layer-move` skills.
Session history: `memory/YYYY-MM-DD.md`.

## Layout

- **Flat single-package repo** — the app IS the root (was `bifurc/` before 2026-09-14). `workspaces:
  ["packages/*"]`. Paths root-relative — **never `cd bifurc`**.
- **`packages/engine`** (P2 item 8, started 2026-09-15): `src/{store,lib,subscription}` moved in.
  Still in `src/`: `proxy/`, `sync/`, `applications/`, `companion/`, `commands/`, `eventBus.ts`,
  `startup.ts`, `shutdown.ts` — then the real `createEngine()`, which is what blocks the "engine
  starts from a bare Node script with `--data-dir`" criterion. Deliberately open: git
  `dialog.showErrorBox` not routed via `preflight()`; `importExport:*` out of the registry until P3.
- **P6 (shell seam) is the only milestone**; everything after is additive. Track C parked (Tauri
  parked, Electron stays). Hard rules: **no renderer edits P1–P6**; no `ws` transport before auth;
  `window.api` byte-identical through P5–P6. `../bifurc-extension` is an external client; its 4
  commands are frozen API.

## Engine facts verified by inspection — do not re-derive

- **The EventBus is `src/eventBus.ts`** — the plan docs say `src/events/bus.ts` and are wrong. Only
  remaining `BrowserWindow` in `src/`: `ipc/eventBridge.ts` (temporary bridge) and `clientHandlers.ts`
  (zoom/titlebar, CLIENT-classified).
- **`src/startup.ts`** — `preflight()` (read-only, never throws) + `bootstrapWorkspaces()`.
  **Preserved quirk:** it calls `initWorkspaceDir()` for every *listed* workspace, so the "active dir
  on disk?" check can never fail for one; the fallback branch fires only when `activeWorkspaceId` is
  absent from `workspaces`. Extraction, not a fix.
- **`src/shutdown.ts`** — memoised idempotent `shutdownEngine()` (spawner → pollers → companion →
  proxy). `processSpawner.stop()` has a per-entry `stopping` guard; entries stay in the map so
  `getState`/`getLogs` survive shutdown.
- **`src/commands/registry.ts`** — ~112 commands validating against frozen `@bifurc/protocol` Zod
  schemas; `entityKindMap.ts` bridges the rename `"rules"`/`"sockets"` ↔ `"proxyRules"`/`"wsConnections"`.
- `src/proxy/` has **zero** Electron imports. `companionServer.ts` already speaks `{id,action,payload}`
  → `{id,ok,data,error}` over loopback WS 9271 — generalise for P4. **No auth**; the `127.0.0.1` bind
  is its whole access control.

## Environment gotchas

- **`npx tsc` is NOT the compiler here** — resolves a placeholder, warns, **exits 0**. Use
  `npm run typecheck`. Same trap: piping vitest through `grep` returns grep's status, not vitest's.
- Sandbox, three traps: (1) `vitest --coverage` fails bulk-deleting its output dir — use
  `--coverage.clean=false --coverage.reportsDirectory=coverage-run<N>` with a **fresh** dir; a
  *failing* test aborts report generation entirely, so deselect the known-failing `soap.execute` with
  `--testNamePattern='^(?!.*soap\.execute)'`. (2) After ~50+ deletions in one turn, `tsup`'s
  `bundle-require` cannot unlink its temp config, so `build:packages`/`typecheck`/`build:main` fail —
  not a code bug, re-run with the sandbox bypassed. (3) Playwright/Electron E2E needs a desktop
  session and **cannot run here**; the 11 e2e specs are unverified locally.
- Known non-regression: `tests/spike/protocolPoc.test.ts` `soap.execute` throws
  `ECONNREFUSED 127.0.0.1:1` (1–2 failures, varies); reproduces on the pre-change tree.
- **Don't batch `Edit` calls to one file in one message** — each reads the same original; last wins.
- Recurring bug classes (all fixed — the patterns matter): `reloadConfig()` must run **after** every
  on-disk write in a handler; `\${…}` in a template literal renders literally; `simple-git status()`
  reports a new file in BOTH `staged` and `created`; Node's global HTTP agent is shared, so a helper
  setting `connection: close` must also pin `agent: false`; the commit subject is a contract the Audit
  Log parses (`^(create|update|delete) (\w+)(?: \[fields\])? Name$`).
