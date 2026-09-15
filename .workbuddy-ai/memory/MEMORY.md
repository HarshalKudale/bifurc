# Bifurc — Project Memory

Curated, durable notes. Full session history lives in `memory/YYYY-MM-DD.md` (see `2026-09-14.md`).

## Repo layout (changed 2026-09-14)

**Flat, single-package.** The app was `bifurc/`; it is now the repository root.

- `package.json` is named **`bifurc`**. No `bifurc-monorepo` root manifest, **no `npm workspaces`**
  (P2 item "restructure to `packages/*`" adds them back).
- All paths are repository-root relative. **`cd bifurc` is no longer needed anywhere.**
- CI (`.github/workflows/release.yml`) uses `working-directory: .`, `release/*.exe`,
  `require('./package.json')`.
- Sibling repos under `I:\workspace\Bifurc-Worskspace\`:
  - `bifurc-extension` — companion browser extension. **Extracted from this repo**; external client with
    its own release cycle. Its four commands (`mock:add`, `request:add`, `folder:add`, `config:get`)
    are a **frozen public API**.
  - `bifurc-studio` — Lovable-built marketing/landing site. NOT the engine web UI; unrelated to P7.

## Decoupling programme — plan of record

Plan lives in **`plan/`**; start at `plan/README.md`. Its status table is the single source of truth.

- **D1–D9 resolved** in `plan/00-decisions.md`. Execution order: `00` → `01`–`12` → `13-checklist.md`.
- **`07-phase-6-shell-seam.md` is the only milestone** — decoupled engine with the Electron shell still in
  place and zero user-visible change. Everything after is additive.
- **Track C parked** (D1/D4/D5): P11 Tauri parked, P10 de-prioritised. Electron stays. This removed the
  MSIX and WebKitGTK risks from the critical path.
- **P0 needs only spike 4** (protocol PoC). Spikes 1 (MSIX) and 3 (WebKitGTK) skipped; spike 2 (Bun) optional.
- **D6: `Cleanup_plan.md` is a hard prerequisite to P2** (at minimum its Phase 1–2), not a parallel track.
- **D7** narrows the CLI to engine lifecycle + read-only. **D8** sets the web UI to inspection + light editing.
- Hard rules: no renderer edits between P1–P6; no `ws` transport merged before auth; `window.api`
  byte-identical through P5–P6; P2 broadcast-site conversions one file per commit.
- Root design docs: `Decoupling_Assessment.md` (v1 measurements), `Decoupling_Plan_v2.md` (Tauri path —
  **superseded**, Tauri parked), `File_Ops_Protocol.md` (blob layer + cert lifecycle).

**Effort (corrected):** Tier 1 seam (P1–P6) **14–20.5 w**; Tier 2 clients (P7–P9) 7–9 w; cross-cutting
5–7 w; **total ~33–46 engineer-weeks** with 25% contingency. The v2 "10–14 w for the seam" figure was
wrong — it omitted transport and the RPC client.

## Architecture facts (established by code inspection — do not re-derive)

- Renderer is platform-clean: `renderer/types/window.ts:193` declares `platform` but **nothing reads it**.
  No `navigator.userAgent` sniffing. 207 `window.api.*` call sites across 43 files.
- `src/companion/companionServer.ts` already implements `{id, action, payload}` → `{id, ok, data, error}`
  over loopback WS (port 9271), consumed by `bifurc-extension`. Generalise it; do not write a new server.
  It has **no authentication** — binding to `127.0.0.1` is its whole access control.
- `importExport` is **string-in / string-out**: `ExporterFn.run(wsId, filePath)` does
  `fs.writeFileSync(filePath, string)`. `filePath` is a pure transport detail, so 32 of 34
  exporter/importer files convert mechanically. Only `workspace-zip` is stream-based, and
  `unzipper.Open.file()` requires a real path (blob staging must be a real directory).
- `systemHandlers.ts:232` `dialog:openFile` already returns `{name,size,base64,mimeType}` — the correct
  client-side read pattern, already used by `BinaryViewer`, `MultipartEditor`, `ProtoExplorer`.
- `mkcert@3.2.0` is **pure JS** (`node-forge`) — no native binary, containerises cleanly.
  `certManager.ts:41` `installCA` shells out to `certutil` / `security` / `sudo update-ca-certificates`;
  CA trust is **host-scoped** → must move to the client for remote engines.
- **Firefox does not use the OS trust store** (own NSS store). Installing a CA via `certutil`/`security`
  leaves Firefox broken. Must be handled deliberately in the UI.
- Tauri v2's Windows bundler emits **NSIS + MSI (WiX)** only — **no MSIX target**.
- `oklch()` in 7 renderer files, `color-mix()` in 2 — WebKitGTK risk on older Linux (parked).
- `main.ts:255` hard-requires git and reports failure via Electron `dialog.showErrorBox` — blocks
  headless/Docker startup.
- `coreHandlers.ts:20` imports `updateTrayMenu` from `@/main` — engine→shell import cycle, must invert.
- Proxy live state is three module-level values in `src/proxy/server.ts` — `currentConfig`, `fullRules`,
  `enabledSets` — all refreshed together by `reloadConfig()`.

## Testing architecture

Full guide: **`TESTING.md`** at the repo root. Read it before changing tests.

**Two Vitest projects** (declared in `vitest.config.ts`):
- `unit` — `tests/**` minus `tests/integration`. Fast, no sockets. `npm run test:unit`.
- `integration` — `tests/integration/**`. Boots real servers, `fileParallelism: false`. `npm run test:integration`.

**Integration harness pattern** — `tests/integration/proxyHarness.ts`:
- Boots the **real** `startServer()` against a real temp workspace, driven by real TCP/HTTP sockets.
  Do **not** mock `net`/`http`/`https` in integration tests — that was the old `tests/proxy/server.test.ts`
  mistake.
- Three **production** hooks make this possible: `setDataRootOverride()` (`src/store/workspaceFs.ts`),
  `setSettingsPathOverride()` (`src/store/appSettings.ts`), `setDataDirOverride()`
  (`src/store/gitStore.ts`, also clears the git cache). Always reset them to `null` in cleanup.
- Enabled state lives in `enabled.json` per entity kind, **not** on the entity (`enabled` is stripped on
  write). `bootstrapEnabledSet()` treats a **missing** flag as ENABLED, so a faithful seeder must
  `set.delete(id)` for disabled entities, not merely skip them. `readAllEntities()` does not inject
  `enabled` — read it via `loadConfig()`.
- Import/export of rules must use the **full rule files**, never `cfg.proxyRules` (UI stubs that blank the
  target and both scripts). `saveConfig()` deliberately skips rule files, so importers must write them with
  `writeEntity(..., "rules", …)` + `enabled.json` + `upsertNameEntry` explicitly.
- Routing: proxy rules and GraphQL/SOAP mocks apply on the **forward-proxy (absolute-form) path only**;
  `*.localhost` goes straight to mapping/mock lookup. A mock is only "fully mocked" when **every** response
  header key is listed in `mockedResponseHeaders`. Priority: mock → GraphQL mock → SOAP mock → proxy rule →
  passthrough. The hand-rolled proxy reads the body as "everything after `\r\n\r\n`", so tests must send an
  explicit `content-length`.
- **Testing IPC handlers directly:** `vi.hoisted` + `vi.mock("electron")` swapping `ipcMain.handle` for a
  `Map`, then invoke the handler with `({}, ...args)`.
- **Cross-realm `Error`:** errors built inside a `vm` context are not `instanceof` the host realm's `Error`,
  so `scriptExecutor` falls back to `String(e)` and messages arrive prefixed `"Error: "`.
- **Platform-sensitive assertions:** pass `platform` explicitly to `generateResolvedCommand()` — the dev
  host is Windows (`mvn.cmd`, `gradlew.bat`), CI is Linux.

**Conventions**
- E2E helpers live in `e2e/helpers/index.ts` and **assert**. Never wrap steps in
  `if (await locator.isVisible())` — that produces tests that cannot fail.
- Coverage thresholds in `vitest.config.ts` are a **ratchet** (46/31/34/48 as of 2026-09-14; actuals
  46.14/31.30/34.34/48.39 over 61 files / 1360 tests — unit 45/1035, integration 16/325). Raise when gaps
  close; never lower to make CI pass.
- **The coverage `include` glob is load-bearing.** It once listed only `renderer/components/**/*.tsx`,
  hiding 17 `.ts` files (three already tested) and under-reporting that dir as 1.8% instead of 10.8%.
- **`dualAliasPlugin` in `vitest.config.ts` resolves `@/x` aliases** and must fall through to `index.ts`
  when the extensionless candidate is a directory (fixed with an `isDir()` guard).
- Happy-path status per screen lives in **`HAPPY_PATH_COVERAGE.md`** (repo root) — check before claiming a
  feature is covered.
- `mkId()` (`src/proxy/serverUtils.ts`) has a 4-char random suffix — collisions possible in a
  same-millisecond burst. Widen before relying on it for durable ids.

## Production bug classes found (all fixed — the pattern is what matters)

- **`reloadConfig()` ordering.** It must run **after** every on-disk write in an IPC handler.
  `entityCrudFactory` used to `saveConfig(); reloadConfig();` *before* writing the entity file, so
  `workspaceCfg()` filtered the new entity out of routing — an added mock was not served and a deleted mock
  kept being served. Any mutation that changes routing and does not call `reloadConfig()` is invisible to
  the proxy while looking correct in the renderer's config object.
- **`simple-git`'s `status()` reports a newly-added file in BOTH `staged` and `created`.** Never
  concatenate the two arrays without a `Set` — doing so double-counts every new file and silently made the
  single-entity branch in `publishEntities()` unreachable. Fixed with
  `Array.from(new Set([...staged, ...created, ...deleted]))`.
- **`git status --porcelain` without `-z` C-quotes paths containing spaces**, so comparisons against a
  computed path silently fail. Use `-z`, or `unquoteGitPath()` (exported from `src/sync/statusTracker.ts`).
- **Escaped-interpolation class of bug:** `\${…}` inside a template literal renders a literal `${…}`.
  Found twice (`generateRunnerHtml` in `runnerHandlers.ts` — all 15 interpolations, making the exported
  report inert — and the update-checker URL in `systemHandlers.ts`). After fixing one, `grep -n '\\\${' src/`.
- **`broadcastEntityStatus()` must not serialize an un-awaited async call** — it did, so
  `JSON.stringify(getWorkspaceSyncStatus(wsId))` produced `{}` and the renderer was always told "nothing is
  dirty".
- **`createTabReducer`** now implements `LOAD_ENTITY` / `LOAD_DRAFT` / `REFRESH`, guarded on `options.init`.
  REST implements all three itself and passes no `init` — giving `restTabReducer` an `init` would make its
  own cases unreachable. `REFRESH` must **preserve runtime/response fields** (`RUNTIME_FIELDS`); a naive
  "copy only keys that differ from the blank state" merge skips entity values equal to the default (e.g.
  `responseStatus: 200`).
- **Node's global HTTP agent is shared across code paths.** `replayRequest` (`src/proxy/serverReplay.ts`)
  sends `connection: close`; without `agent: false` Node still pooled the socket when the upstream answered
  `keep-alive`, and the next caller on that origin got a spurious 400 / `HPE_CLOSED_CONNECTION`. Any new
  outbound helper that sets `connection: close` must also pin `agent: false`.
- **Diagnosing HTTP flakiness:** add `server.on("clientError", (err, socket) => …)` to get the parser's
  `err.code`. Failures that reproduce only in a full-file run (never in isolation) are cross-caller
  socket-pool interactions, not bad assertions.
- **A bare `require("@/…")` inside a source module only works because `tsc-alias` rewrites the build
  output**, and is unresolvable for Vitest. Prefer static imports.
- **Tray icon packaging regression (commit `2477194`):** icons moved to `build/` but stayed out of
  `build.files`; `nativeImage.createFromPath()` on a missing path returns an **empty NativeImage** with no
  throw. Fixed with `extraResources` + `iconPath()`/`loadIcon()`. CI should assert the asar contains them.

## Git / sync facts

- `entity:publish` **pushes automatically** when a remote is configured (`publishEntities` → `getSyncConfig`
  → `g.push`). A test isolating `sync:push` must make its own commit first.
- **A local bare repo is a valid git remote** — `git init --bare` + `git symbolic-ref HEAD refs/heads/main`,
  then pass the path with **forward slashes**. No network needed for sync tests.
- `initWorkspaceDir()` must run **before** `initWorkspaceRepo()`, or `.gitignore` is missing and the init
  commit is empty (first push fails). `initWorkspaceRepo()` also commits `workspace.json`.
- **Commit subject format is a contract**: the Audit Log parses
  `^(create|update|delete) (\w+)(?: \[fields\])? Name$`. Changing `publishEntities()`'s message format
  silently empties the Audit Log.
- **Dead code in `src/sync`:** `fetchRemoteHead()` and `performGitClone()` (`gitSyncOps.ts`) have zero call
  sites; `restoreEntity()` (`publishService.ts`) is unused. Caps `gitSyncOps.ts` at ~50% coverage.
- Companion tests need a **real git repo** in the workspace (`setDataDirOverride(ws.dataRoot)` to clear the
  git cache, then `simpleGit(dir).init()` + a commit), or the status map is legitimately empty.

## Config / IPC facts

- **`config:save` restarts the server** when the port changed, TLS settings changed, or the server is not
  running (`if (!isRunning() || incoming.port !== prev.port || tlsChanged)`), and it is what restarts the
  companion server when `companionPort` changes. It compares against `loadConfig().port` — the *saved* port,
  not the running one — so a test must boot on the port `app.json` already names or the branch will not fire.
- **`server:restart` rebinds the same port immediately after `stopServer()`** without `EADDRINUSE` (Node
  releases the listening handle synchronously on `close()`).

## Environment gotchas

- In sandboxed/agent environments, `vitest --coverage` and `playwright test` fail while bulk-deleting
  `coverage/` or `test-results/`. Work around with
  `npx vitest run --coverage --coverage.clean=false --coverage.reportsDirectory=coverage-run` and
  `npx playwright test --output=./.e2e-out`. `--coverage.clean=false` does **not** stop Vitest deleting
  `coverage-<x>/.tmp` afterwards, so use a **fresh** `reportsDirectory` each run. `coverage-*/` is gitignored.
- Playwright/Electron E2E needs a desktop session (or `xvfb-run` on Linux). It cannot run in a headless
  agent sandbox.
- **`npx tsc` is NOT the TypeScript compiler here.** It resolves the placeholder npm package, prints "This
  is not the tsc command you are looking for", and **still exits 0** — so `npx tsc | wc -l` reports "0
  errors" and looks clean. Always use `npm run typecheck` / `npm run typecheck:renderer`.
- **Do not batch `Edit` calls in one message** — each reads the same original file and the last write wins.
  One edit per message, then `Grep` to confirm.

## Known debt

- `npm run typecheck` covers `src/**` only. `npm run typecheck:renderer` (`tsconfig.renderer.json`) reports
  ~152 pre-existing errors — fix, then gate it in CI.
- Remaining guarded E2E specs: `protocols`, `environments`, `capture-ws-webhooks`, `settings`, `screenshots`.
- **gRPC is not implemented.** `grpc:execute`/`grpc:reflect` return
  `{ok:false, error:"gRPC runtime not yet configured…"}`; `grpc:startMockServer` is a stub and
  `grpc:mockServerStatus` is hard-coded `{running:false, port:9102}`.
  `tests/integration/protocolExecution.integration.test.ts` pins that stub contract — implementing gRPC
  will fail the suite until those tests are updated alongside the feature.
- Still at 0%: `renderer/panels/**` (E2E-only) and the `renderer/components/rest`
  `useCollectionRunner`/`useRestActions` hooks.
