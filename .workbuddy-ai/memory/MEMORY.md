# Bifurc — Project Memory

**Detail lives in the repo, not here** — `plan/README.md` (programme status), `plan/00-decisions.md`,
`plan/03`, `TESTING.md`. Test gotchas + engine mechanics: the `bifurc-test-coverage` and
`bifurc-engine-layer-move` skills. History: `memory/YYYY-MM-DD.md`.

## Layout

- **Flat single-package repo** — the app IS the root. `workspaces: ["packages/*"]`. Paths
  root-relative — **never `cd bifurc`**.
- **`packages/engine` (P2 item 8): done.** 52 files moved in four layers
  (`src/{store,lib,subscription}`, `src/{proxy,sync}` + `eventBus.ts`,
  `src/{applications,companion,commands}`, `src/{startup,shutdown}.ts`). `src/` holds only `ipc/`,
  `main.ts`, `preload.ts` by design. **`createEngine({dataDir})` → `{start,stop,status,registry,bus}`**
  is implemented and proven by `tests/integration/engineSmoke.integration.test.ts`. Runtime deps are
  exactly `@bifurc/protocol`, `mkcert`, `simple-git`, `ws`; `js-yaml`/`archiver`/`unzipper` belong to
  `src/ipc/importExport/**`, not the engine.   Open: git `dialog.showErrorBox` not via `preflight()`; `importExport:*` out of the registry until
  P3; **Windows ignores `--data-dir`** (`%LOCALAPPDATA%` wins — pre-existing).
- **Packaging does NOT follow the package split** (`plan/12`; also blocks P9). `build.files` covers
  only `dist/**/*` + `package.json`, and electron-builder does not dereference the
  `node_modules/@bifurc/*` symlinks — so a packaged build cannot resolve `@bifurc/engine/*` or
  `@bifurc/protocol`. `plan/10`'s Dockerfile shares the gap (copies `dist` with no install step).
- **P6 (shell seam) is the only milestone**; everything after is additive. Hard rules: **no renderer
  edits P1–P6**; no `ws` transport before auth; `window.api` byte-identical through P5–P6.
  `../bifurc-extension` is an external client — its 4 commands are frozen API.
- **The app runs `packages/engine/dist/`; tests run `src/`** (via `resolve.alias`) — different
  artifacts. **`npm run build:packages` after any engine source edit.**

## Engine facts (verified — do not re-derive)

- **The EventBus is `packages/engine/src/eventBus.ts`** — the plan docs say `src/events/bus.ts` and
  are wrong; no `events/` directory has ever existed. Only remaining `BrowserWindow` in `src/`:
  `ipc/eventBridge.ts` and `clientHandlers.ts` (zoom/titlebar, CLIENT-classified).
- **`startup.ts`** — `preflight()` (read-only, never throws) + `bootstrapWorkspaces()`. **Preserved
  quirk** (`plan/03`): it calls `initWorkspaceDir()` for every *listed* workspace, so the "active dir
  on disk?" check can never fail for one. **`shutdown.ts`** — memoised `shutdownEngine()`.
- **`commands/registry.ts`** — ~112 commands validating against frozen `@bifurc/protocol` Zod
  schemas; `entityKindMap.ts` bridges `"rules"`/`"sockets"` ↔ `"proxyRules"`/`"wsConnections"`.
  `createEngine()` **registers none** — the *consumer* does; assert registry identity, not population.
  `companion/allowedActions.ts` is the **frozen** API for `bifurc-extension`.
- `proxy/` has **zero** Electron imports. `companionServer.ts` speaks `{id,action,payload}` →
  `{id,ok,data,error}` over loopback WS 9271 — generalise for P4. **No auth**; the `127.0.0.1` bind
  is its whole access control.

## Environment gotchas

- **NEVER `git stash` here.** On 2026-09-16 a `git stash` + tool-timeout SIGTERM landed during git's
  auto-repack and wiped `.git/objects/pack/*.pack` — the whole object database. Windows diverts git's
  deleted packs to the **I: drive Recycle Bin**; `_recovered-packs/` holds the recovery, and the full
  recipe is in `memory/2026-09-16.md`. Tells: git silently resolves to the *parent* directory, and
  `git status` fails with "unable to read <sha>". Prefer a temp commit or `git worktree`.
- **`npx tsc` is NOT the compiler** — resolves a placeholder, warns, **exits 0**. Use
  `npm run typecheck`. Piping vitest through `grep` returns grep's status, not vitest's.
- Sandbox, three traps: (1) `vitest --coverage` fails bulk-deleting its output dir — use
  `--coverage.clean=false --coverage.reportsDirectory=coverage-run<N>` with a **fresh** dir; a
  *failing* test aborts the report, so deselect `soap.execute` via
  `--testNamePattern='^(?!.*soap\.execute)'`. (2) After ~50+ deletions in a turn, `tsup`'s
  `bundle-require` cannot unlink its temp config, so builds fail — not a code bug. (3) Electron E2E
  needs a desktop session — **cannot run here**.
- Known flaky family: tests asserting a connection to a dead endpoint *fails* (`soap.execute`,
  `upstreamFetch`, `protocolExecution`'s WSDL) — the sandbox intercepts them, so which trip varies.
  Pre-existing.
- **Don't batch `Edit` calls to one file in one message** — each reads the same original; last wins.
- Recurring bug classes: `TESTING.md` §7 (all fixed; `reloadConfig()` must run after every on-disk
  write, `\${…}` in a template literal renders literally, `simple-git status()` double-lists new files).
