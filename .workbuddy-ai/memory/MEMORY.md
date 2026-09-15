# Bifurc — Project Memory

Durable notes only; session history lives in `memory/YYYY-MM-DD.md`.
**Detail lives in the repo, not here** — read `plan/README.md` (programme status: single source of
truth), `TESTING.md` (tests), `plan/03-phase-2-engine-extraction.md` (per-item P2 status),
`Cleanup_plan.md`, `plan/00-decisions.md`.

## Layout & programme

- **Flat single-package repo** — the app IS the repository root (it was `bifurc/` before
  2026-09-14). `package.json` is named `bifurc`; `workspaces: ["packages/*"]` links
  `packages/protocol` only. Paths are repo-root relative — **never `cd bifurc`**.
- `../bifurc-extension` is an **external client on its own release cycle**; its four commands
  (`mock:add`, `request:add`, `folder:add`, `config:get`) are a **frozen public API**.
  `../bifurc-studio` is a marketing site — **not** the P7 web UI.
- **P6 (shell seam) is the only milestone**; everything after it is additive. Track C parked (Tauri
  parked, P10 de-prioritised, Electron stays). ~33–46 engineer-weeks; Tier 1 seam 14–20.5 w.
- Hard rules: **no renderer edits between P1–P6**; no `ws` transport before auth; `window.api`
  byte-identical through P5–P6; one broadcast-site conversion per commit; post-freeze protocol
  changes recorded in `plan/protocol-changes.md`.
- **P2: items 1–7 done; only item 8 remains** — the physical `packages/engine` move, dependency
  split, `@/*` alias resolution, tsup build. Every unchecked P2 acceptance criterion is blocked on
  it. Deliberately open: git `dialog.showErrorBox` is not routed through `preflight()` (product
  decision); `importExport:*` stays outside the CommandRegistry until P3.

## Engine architecture (verified by inspection — do not re-derive)

- **The EventBus is `src/eventBus.ts`** — the plan docs say `src/events/bus.ts` and are wrong.
  `bus.emitTyped` / `bus.onTyped`. All 8 broadcast sites are inverted. The only remaining
  `BrowserWindow` / `webContents.send` in `src/`: `src/ipc/eventBridge.ts` (a deliberate temporary
  bridge re-emitting `companion:refresh` for the still-unmodified renderer) and `clientHandlers.ts`
  (zoom / titlebar chrome, CLIENT-classified).
- **`src/startup.ts`** is Electron-free and owns startup: `preflight({dataDir, ports})` (read-only,
  never throws) + `bootstrapWorkspaces(settings)` (workspace dirs, git repos, auto-sync, active-
  workspace repair). `main.ts` does `setDataRoot(app.getPath("userData"))`, then `bootstrapWorkspaces`.
  **Preserved quirk:** the bootstrap loop calls `initWorkspaceDir()` for every *listed* workspace, so
  the "active workspace's dir is on disk?" check can never fail for one — the fallback/create branch
  only fires when `activeWorkspaceId` is absent from `workspaces`. Extraction, not a fix.
- **`src/shutdown.ts`** — memoised, idempotent `shutdownEngine()` (spawner → pollers → companion →
  proxy; children before servers). `processSpawner.stop()` has a per-entry `stopping` guard making
  `stopAll()` repeatable; entries stay in the map so `getState`/`getLogs` survive shutdown.
- **`src/commands/registry.ts`** — CommandRegistry validating payloads against the frozen
  `@bifurc/protocol` Zod schemas; **~112 commands**, every `EntityKind`; `ipcMain.handle` is a thin
  adapter. `src/commands/entityKindMap.ts` bridges the one real rename: engine
  `"rules"`/`"sockets"` ↔ protocol `"proxyRules"`/`"wsConnections"`.
- `src/proxy/` (the actual product) has **zero** Electron imports. `appSettings.ts` /
  `workspaceFs.ts` resolve via `dataDir()` (`src/store/paths.ts`) — loud error if `setDataRoot()`
  never ran.
- `src/companion/companionServer.ts` already speaks `{id,action,payload}` → `{id,ok,data,error}` over
  loopback WS (9271) for the extension — generalise it for P4, don't write a new server. **No auth**;
  the `127.0.0.1` bind is its whole access control.
- `importExport` is string-in / string-out, so 32 of 34 files convert mechanically; only
  `workspace-zip` streams and needs a real staging directory.
- `mkcert@3.2.0` is pure JS. CA trust is **host-scoped** → moves client-side. **Firefox ignores the
  OS trust store** (its own NSS) — must be handled deliberately in the UI.

## Testing & environment

`unit` = `tests/**` minus `tests/integration`; `integration` = real sockets,
`fileParallelism: false`. Prefer real I/O over mocks; use the production hooks
`setDataRootOverride` / `setSettingsPathOverride` / `setDataDirOverride` (the last also clears the
git cache) and always reset them.

- **vitest's mock registry survives `vi.resetModules()`** — re-imported mocked modules reuse the same
  `vi.fn()`s, so call counts accumulate across tests. Add `beforeEach(() => vi.clearAllMocks())`
  before asserting counts, or the failures are spurious.
- **`npx tsc` is NOT the compiler here** — it resolves a placeholder, warns, and still exits 0. Use
  `npm run typecheck` / `typecheck:renderer`.
- Coverage thresholds are a **ratchet**; the `include` glob is load-bearing.
- Sandbox: `vitest --coverage` and `playwright test` fail while bulk-deleting their output dirs — use
  `--coverage.clean=false --coverage.reportsDirectory=coverage-run<N>` with a **fresh** dir.
  Playwright/Electron E2E needs a desktop session and **cannot run here** — the 11 e2e specs are
  unverified locally; don't claim otherwise.
- Known non-regression: `tests/spike/protocolPoc.test.ts` `soap.execute` throws
  `ECONNREFUSED 127.0.0.1:1` (1–2 failures, varies per run); reproduces on the pre-change tree.
- **Don't batch `Edit` calls to one file in one message** — each reads the same original; last wins.

## Recurring bug classes (fixed — the patterns matter)

- `reloadConfig()` must run **after** every on-disk write in a handler, or routing silently ignores it.
- `\${…}` in a template literal renders literally. After fixing one, `grep -n '\\\${' src/`.
- `simple-git status()` reports a new file in BOTH `staged` and `created` — dedupe with a `Set`.
- Node's global HTTP agent is shared — any helper setting `connection: close` must also pin
  `agent: false`, or the next caller on that origin gets a spurious 400 / `HPE_CLOSED_CONNECTION`.
- **The commit subject is a contract**: the Audit Log parses
  `^(create|update|delete) (\w+)(?: \[fields\])? Name$`.

## Known debt

- `typecheck` covers `src/**` only; `typecheck:renderer` has ~152 pre-existing errors — fix, then gate.
- **gRPC is not implemented**; `protocolExecution.integration.test.ts` pins the stub contract
  (Cleanup_plan §1.5 = **wontfix**, 2026-09-15). Implementing it must update those tests alongside.
- 0% coverage: `renderer/panels/**` (E2E-only), `renderer/components/rest`'s
  `useCollectionRunner` / `useRestActions`.
- `mkId()` (`src/proxy/serverUtils.ts`) has a 4-char suffix — widen before durable use.
