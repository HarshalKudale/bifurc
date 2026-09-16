# 03 — Phase 2: Engine extraction

**Goal:** turn `src/` into `@bifurc/engine` — a pure Node package with **zero Electron imports** that
starts, serves and shuts down headlessly.

**Effort:** 4–6 weeks. **Depends on:** **D6 cleanup**, P1. **Blocks:** P3, P4, P9.

> This is the largest single workstream in the programme. It is mostly mechanical, but one part —
> inverting the broadcast pattern (item 2) — is genuinely architectural and is where subtle regressions
> will surface.

---

## Preconditions

- **`Cleanup_plan.md` has landed (D6 — hard prerequisite, not a parallel track).** At minimum its
  Phase 1–2 (dead code + shared utils). Rationale:
  - Cleanup splits `handlers.ts` (1,909 lines) into `src/ipc/handlers/*`, which is exactly the shape P2
    needs before moving files into packages.
  - Cleanup extracts `entityCrudFactory`, which P1 item 2 then collapses from 36 channels to 6.
  - Running both concurrently touches ~110 files at once and makes regressions untraceable.
  - Cleanup Phase 4–5 (file splitting, React anti-patterns) may follow later — only 1–2 are required here.
- `plan/handler-classification.md` complete (P1 item 1).
- `@bifurc/protocol` builds and is frozen.

---

## Repo state note (2026-09-14)

The repository is now **flat and single-package**: `package.json` is named `bifurc`, there is no
`bifurc-monorepo` root manifest, and `npm workspaces` is absent. Item 8 below therefore **adds**
`packages/*` workspaces to a single-package repo rather than restructuring an existing monorepo — a
little less work than this document originally assumed.

All paths are repository-root relative. The `bifurc/` prefix no longer exists.

---

## Current state — what actually needs to change

20 files import `electron`. Most only import `ipcMain` (the registration layer, which is replaced
wholesale). The real coupling is narrower:

| Coupling | Files | Sites |
|---|---:|---:|
| `BrowserWindow.getAllWindows()` broadcast | 8 | ~15 |
| `app.getPath("userData")` | 2 | 2 |
| `dialog.*` / `shell.*` | 5 | ~10 |
| `mainWindow` reference held by a service | 1 | 3 |

**`src/proxy/` — the actual product — has exactly one Electron import** (`webhookServer.ts:14`, purely to
broadcast payloads). The proxy core, TLS handling, mock engine, script executor and service discovery
are already pure Node. That is the good news.

---

## Work item 1 — Build the EventBus

`src/proxy/logEmitter.ts` is already a correct `EventEmitter` that the IPC layer subscribes to. **Copy
this pattern; do not invent a new one.** It is the proof that the approach works in this codebase.

```ts
// packages/engine/src/events/bus.ts
export interface EngineEvents {
  "log.entry":         RequestLogEntry;
  "log.chunk":         { logId: string; chunk: string; done: boolean };
  "server.error":      string;
  "server.status":     { running: boolean; port: number };
  "sync.status":       { wsId: string; status: string; error?: string | null; updatedIds?: string[] };
  "sync.entityStatus": { wsId: string; status: Record<string, string> };
  "webhook.payload":   WebhookPayload;
  "entity.changed":    { wsId: string; kind: string; id?: string };   // replaces companion:refresh
  "process.output":    { processId: string; stream: "stdout" | "stderr"; chunk: string };
  "settings.changed":  { key: string; value: unknown };
}
```

Typed, one emitter, no Electron. The transport layer (P4) subscribes and fans out to its clients; the
Electron shell (P6) subscribes and forwards to `webContents`.

### Note on `companion:refresh`

`notifyRendererRefresh()` sends `companion:refresh` — a bare "something changed, re-fetch" hint. That is
a **UI concern leaking into the engine**. Replace it with `entity.changed { wsId, kind, id }`, which
carries actual information and lets each client decide how to invalidate. Strictly better, and it
removes a name (`companion:*`) that will make no sense once the extension is just another client.

---

## Work item 2 — Invert the broadcast pattern

The core architectural change. Every one of these sites must stop reaching into Electron and instead
emit on the bus.

| # | File | Current | Replace with |
|---|---|---|---|
| 1 | `src/ipc/handlers.ts:24–49` | 4 forwarders: `onSyncStatusChange` → `webContents.send("sync:status")`, `logEmitter` → `log:entry` / `log:chunk` / `server:error` | Subscribe in the **transport layer**, not here. Delete these forwarders. |
| 2 | `src/ipc/handlers/coreHandlers.ts:28` | `BrowserWindow.getAllWindows().forEach(w => w.webContents.send("sync:entityStatus", …))` | `bus.emit("sync.entityStatus", { wsId, status })` |
| 3 | `src/ipc/handlers/entityCrudFactory.ts:29–42` | `notifyRendererRefresh()` + `broadcastEntityStatus()` | `bus.emit("entity.changed", …)` + `bus.emit("sync.entityStatus", …)` |
| 4 | `src/ipc/handlers/folderHandlers.ts:10` | `notifyRendererRefresh()` | `bus.emit("entity.changed", …)` |
| 5 | `src/ipc/handlers/syncHandlers.ts:18` | `broadcastEntityStatus()` | `bus.emit("sync.entityStatus", …)` |
| 6 | `src/ipc/handlers/systemHandlers.ts:28` | `BrowserWindow.getAllWindows()` loop | `bus.emit(...)` |
| 7 | `src/proxy/webhookServer.ts:137` | `webContents.send("webhook:payload", …)` | `bus.emit("webhook.payload", …)` |
| 8 | `src/companion/companionServer.ts:52–83` | `broadcastEntityStatus()` + `notifyRendererRefresh()` | `bus.emit(...)`; the WS server becomes a **transport**, not a broadcaster |

**Also:** `coreHandlers.ts:20` imports `updateTrayMenu` from `@/main` — an engine→shell import and a
circular dependency. Invert it: the shell subscribes to `settings.changed` and updates its own tray.

### The regression risk

This is where bugs will hide. Watch for:
- **Missed events** — a site you converted but a listener you forgot to wire.
- **Duplicate delivery** — a site that both emits *and* still calls `webContents.send`.
- **Ordering** — `sync.entityStatus` and `entity.changed` for the same mutation must arrive in a
  consistent order or the UI can read stale status for a fresh entity.

**Test for it:** the existing 35 unit suites plus the 11 e2e specs are your safety net. Do not weaken
them. If an e2e spec is flaky, fix the flake — do not skip.

---

## Work item 3 — `processSpawner`: remove the window reference

`src/applications/processSpawner.ts` holds `private mainWindow: BrowserWindow | null` and a
`setMainWindow(win)` method, used to stream child-process output to the UI. A headless engine has no
window.

```
// before
setMainWindow(win: BrowserWindow) { this.mainWindow = win; }
... this.mainWindow.webContents.send(channel, data)

// after
bus.emit("process.output", { processId, stream, chunk })
```

Also revisit lifecycle: today the spawner stops all children on `app.on("before-quit")`. In a headless
engine that becomes a `shutdown()` call on the engine, and it must be **idempotent** — the shell, a
supervisor restart, and a SIGTERM handler may all call it.

> **Status (2026-09-15): done.** The window reference itself was removed in an earlier session (the
> spawner now emits `process.output` / `process.statusChange` on the bus). This session closed the
> lifecycle half:
>
> - **`src/shutdown.ts`** is new and Electron-free. It exports `shutdownEngine(): Promise<void>` and
>   `isShuttingDown(): boolean`. The teardown sequence (spawner → pollers → companion server → proxy
>   server) is memoised behind a single in-flight promise, so the shell quitting, a supervisor
>   SIGTERM and a crash handler can all call it in the same tick and exactly one teardown runs.
>   `main.ts`'s `before-quit` is now `void shutdownEngine()`.
> - **Ordering is deliberate and pinned by a test**: children and pollers stop *before* the servers,
>   otherwise a server still accepting traffic can spawn work after the spawner was torn down.
> - **`processSpawner.stopAll()` is now genuinely idempotent**, via a `stopping` flag on each
>   `RunningProcess` entry checked at the top of `stop()`. Without it a second shutdown pass would
>   re-`taskkill` the same pid and re-emit "stopping" status/log events — and a late `stop()` on an
>   already-exited process would flip its status back from `exited` to `stopping`. Entries are
>   **not** removed from the map, so `getState()` / `getLogs()` keep working for a client reading
>   them during shutdown.
> - Unit-tested in `tests/shutdown.test.ts` (6/6): exactly-once teardown, ordering, repeated calls,
>   concurrent callers, shared promise identity, and the `isShuttingDown()` state machine.
>
> Note for the package move: the four individual stop functions were already null-guarded and safe
> to call when their subsystem is not running — what was missing was a guard on the *sequence*.
> `shutdownEngine()` must not become a thin re-export of them without keeping that guard.

---

## Work item 4 — Inject the data directory

Two sites, but they gate CLI and Docker:

| File | Line | Current |
|---|---|---|
| `src/store/appSettings.ts` | 79 | `path.join(app.getPath("userData"), "app.json")` |
| `src/store/workspaceFs.ts` | 40 | `path.join(app.getPath("userData"), "data")` |

**The seam already exists — in test form.** The store exposes two production hooks used by the
integration harness:

- `setDataRootOverride()` — `src/store/workspaceFs.ts`
- `setSettingsPathOverride()` — `src/store/appSettings.ts`

This is good news: **you are promoting an existing override mechanism to the primary path**, not
designing a new one. The integration tests already prove it works, and they already exercise the
"reset to `null` in cleanup" discipline.

```ts
// packages/engine/src/paths.ts
let dataRoot: string | null = null;
export function setDataRoot(dir: string): void { dataRoot = path.resolve(dir); }
export function dataDir(): string {
  if (!dataRoot) throw new Error("Data root not initialised — call setDataRoot() at startup");
  return dataRoot;
}
```

Resolution order at engine startup:
1. `--data-dir <path>` CLI flag
2. `BIFURC_DATA_DIR` environment variable
3. Platform default: `%LOCALAPPDATA%\Bifurc` (Windows), `~/Library/Application Support/Bifurc` (macOS),
   `$XDG_CONFIG_HOME/bifurc` or `~/.config/bifurc` (Linux)

The Electron shell passes option 1 with `app.getPath("userData")`, preserving current behaviour exactly.

**Also add:** `setDataRoot()` must create the directory and its subdirectories if missing, and must be
called before any store module is used. Make the "not initialised" error loud — a silent fallback to
cwd is how you end up with data in a container's ephemeral layer.

**Watch the test override:** once `setDataRoot()` is the production path, decide whether the two
`*Override` hooks remain or fold into it. Do not end up with two mechanisms doing the same job — that is
how the e2e data-dir isolation breaks silently (see P6 item 5).

**Verify `src/store/gitStore.ts`** — it imports `electron` but does not appear in the `app.getPath` grep.
Find out what it actually needs and remove it.

> **Status (2026-09-15): done.** `src/store/appSettings.ts` and `src/store/workspaceFs.ts` no longer
> import `electron` — their fallback branch (after the win32 `LOCALAPPDATA` special case and the
> `*Override` hooks, both checked first, in that order) now calls `dataDir()` from
> `src/store/paths.ts`. `src/main.ts` calls `setDataRoot(app.getPath("userData"))` once, at the very
> top of `app.whenReady()`, before any store module is touched — this preserves the exact directory
> Electron previously resolved, so existing installs see no path change.
>
> **Decision taken on the two `*Override` hooks:** they stay separate, not folded into
> `paths.ts`/`setDataRoot()`. They are proven by the full integration suite (`proxyHarness.ts` et
> al.) and folding them in was judged a distinct, riskier change than this pass should attempt
> without the 11 e2e specs available to catch a data-dir isolation regression (still true — see
> `plan/baseline.md` "Environment caveats"). Revisit this decision once e2e can run.
>
> `gitStore.ts` was verified in a prior session: it does not import `electron` at all any more (it
> only needed `workspaceFs`'s `wsDir`/`setDataRootOverride`, both already Electron-free).
>
> `electron`-importing files in `src/` are now down to 16 (from 18 at the start of this session).

---

## Work item 5 — Split out the shell-only handlers

Per `plan/handler-classification.md`, these never cross the wire and must be **removed from the engine**:

| File | Handlers | Destination |
|---|---|---|
| `src/ipc/handlers/systemHandlers.ts` | `zoom:get`, `zoom:set`, `theme:get`, `theme:set`, `shell:setTitleBarOverlay`, `shell:openExternal`, `dialog:*`, `app:isFirstLaunch`, `app:completeFirstLaunch` | Electron shell (P6) |
| `src/ipc/handlers/systemHandlers.ts` | `app:checkUpdate` | Split — shell version + engine version |
| `src/ipc/handlers/tlsHandlers.ts` | `tls:installCA` | Client (P6/P7/P11) — see `File_Ops_Protocol.md` §6.2 |
| `src/main.ts` | tray, menu, window, zoom shortcuts, titlebar overlay, single-instance | Electron shell (P6) |

`app:isFirstLaunch` / `app:completeFirstLaunch` read `settings.hasSeenWelcome` — that is shell launch
state, not engine state. Move it.

> **Status (2026-09-15): the first row is done, ahead of P6.** `zoom:get`/`zoom:set`,
> `theme:get`/`theme:set`, `shell:openExternal`, `shell:setTitleBarOverlay`, `dialog:pickFilePath`,
> `dialog:pickFolderPath`, `dialog:openFile`, `tls:installCA`, `app:isFirstLaunch`, and
> `app:completeFirstLaunch` all moved into a new `src/ipc/handlers/clientHandlers.ts` with its own
> `registerClientHandlers()`, called directly by `src/main.ts` alongside — not from within —
> `registerIpcHandlers()`. No channel names changed and no renderer code was touched; this is a
> registration-time reorganization, not a P6 shell-seam change, which is why it was safe to do now
> rather than waiting for P6. `tls:installCA` moved out of `tlsHandlers.ts` specifically (the rest
> of that file stays — it reads/writes the CA files themselves, which is engine-side data).
> Unit-tested in `tests/ipc/clientHandlers.test.ts` (21/21 passing) — these handlers had **no**
> direct unit coverage before this move.
>
> **Not done:** `app:checkUpdate` (SPLIT — stays as-is per its own note in
> `handler-classification.md`, deferred to P12), `capture:shareJson` (SPLIT, untouched), and
> `src/main.ts`'s tray/menu/window/zoom-shortcut/titlebar/single-instance code (that code doesn't
> move until P6 actually stands up a separate engine process — moving it now would be premature
> without `packages/engine` existing to move it *out of*).

---

## Work item 6 — Headless startup path

`src/main.ts:255` currently:

```ts
const hasGit = await checkGitInstalled();
if (!hasGit) {
  dialog.showErrorBox("Git required", "...");     // ← Electron dialog
  app.quit();                                      // ← Electron lifecycle
  return;
}
```

The engine cannot depend on a dialog to report a fatal condition. Replace with a structured startup
result the caller can render however it likes:

```ts
// packages/engine/src/startup.ts
export interface StartupCheck { ok: boolean; code?: ErrorCode; message?: string; hint?: string }
export async function preflight(): Promise<StartupCheck[]>
// checks: git present, data dir writable, ports available, mkcert usable
```

The Electron shell renders failures in a dialog; the CLI prints them; the Docker entrypoint exits
non-zero with the message on stderr. **One check, three presentations.**

> **Status (2026-09-15): half done.** `src/startup.ts` implements exactly this —
> `preflight({ dataDir, ports? })` runs all four checks (`checkGitInstalled()` reused from
> `gitStore.ts`, a data-dir write probe, `checkPortInUse()` reused from `applications/portUtils.ts`
> per configured port, and a cheap `typeof createCA === "function"` mkcert-loaded check) and never
> throws. Unit-tested in `tests/startup.test.ts` (5/5 passing).
>
> The wiring in `main.ts` is deliberately conservative: the pre-existing git-required
> `dialog.showErrorBox` + `app.quit()` block is untouched (same behaviour, same ordering), so
> user-visible behaviour is unchanged. `preflight()` is additionally called once settings are
> loaded (needed for the real configured ports) and any non-ok check is only `console.warn`'d —
> it does not yet block startup for `data-dir-unwritable` / `port-in-use` / `mkcert-unusable`.
> Whether those should become blocking (and whether the git check should be re-routed through
> `preflight()` instead of its own direct call) is a product decision left open, not an oversight.
>
> **Not done:** the git check still is not routed through `preflight()` — see above, that is a
> product decision, not an oversight.
>
> **Status (2026-09-15, this session): the workspace bootstrap is now extracted too.**
> `src/startup.ts` gained `bootstrapWorkspaces(settings)` alongside `preflight()`, so the whole
> engine-startup surface lives in one Electron-free module. It does exactly what `main.ts`'s
> `app.whenReady()` handler used to do inline — create every known workspace's dirs
> (`initWorkspaceDir`) and git repo (`initWorkspaceRepo`), start auto-sync for the workspaces whose
> `syncConfig.autoSync` is set, then repair `activeWorkspaceId` or create a fresh default workspace
> — plus the `setAutoSyncReloadFn(reloadConfig)` wiring, which is engine-internal and the shell
> never needed to know about.
>
> `main.ts` is now a single call, `settings = (await bootstrapWorkspaces(settings)).settings;`, and
> no longer imports `workspaceFs`, `gitStore`, `autoSync`, `syncManager` or `store/config` at all.
> The function returns the effective settings instead of mutating the caller's object; the leftover
> `require("@/proxy/server")` for `reloadConfig` is gone (it was redundant — `main.ts` already had a
> static import of that module).
>
> **A real quirk was found while testing this, and deliberately preserved.** Because the loop calls
> `initWorkspaceDir()` for *every* workspace in settings — creating the directory if missing — the
> subsequent "is the active workspace's dir on disk?" check can never fail for a workspace that is
> *listed*. So the "fall back to another workspace / create a default" branch only ever fires when
> `activeWorkspaceId` is **not** in `workspaces` (settings edited, or a workspace dropped from the
> list while still active). The "its dir was deleted" reading of that code is unreachable. This is
> pre-existing behaviour, and P2 is an extraction, not a behaviour change — so it is preserved
> verbatim and annotated in `src/startup.ts` with a "do not fix during the package move" note.
>
> Integration-tested in `tests/integration/workspaceBootstrap.integration.test.ts` (7/7) with a real
> temp data root, real directories and real `git init` — including that a second bootstrap run adds
> no second `chore: init workspace repo` commit, and that a healthy active workspace leaves the
> settings file byte-identical (no pointless rewrite on every launch). Only `startAutoSync` is
> stubbed, so the suite does not leave a 30-second poller timer running.

---

## Work item 7 — De-register `ipcMain`

Once items 1–6 are done, `registerIpcHandlers()` has nothing left to do. Replace it with a
**CommandRegistry**: the same handler functions, registered by command name against the protocol schema
rather than against `ipcMain`.

```ts
// before
ipcMain.handle("entity:setEnabled", async (_e, wsId, kind, id, enabled) => { ... });

// after
registry.register("entity.setEnabled", EntitySetEnabled, async (payload, ctx) => { ... });
```

The bodies barely change — `(_e, a, b, c)` becomes `({ a, b, c }, ctx)` and `return` becomes a typed
result. The `ctx` carries the session (for auth in P4) and the bus.

**Do this last within P2.** It is the mechanical payoff of everything above, and doing it early just
creates churn.

> **Status (2026-09-15): started.** `src/commands/registry.ts` implements the `CommandRegistry`
> class described above — `register(action, handler)` / `invoke(action, payload, ctx)` / `list()`
> / `isRegistered()`, validating every payload against the real, frozen
> `@bifurc/protocol` Zod schema before the handler runs. Unit-tested in
> `tests/commands/registry.test.ts` (9/9 passing).
>
> Three of `coreHandlers.ts`'s commands are wired through it end-to-end as a proof of the
> pattern — `config.get` (no params), `env.setActive` and `workspace.setActive` (both a single
> named param that maps 1:1 onto the existing positional IPC argument). `ipcMain.handle(...)` is
> now a thin adapter (`(_e, id) => commandRegistry.invoke("env.setActive", { id }, ctx)`) rather
> than the handler body itself — exactly the shape this item describes. No channel name, argument
> shape, or return value changed; `tests/ipc/handlers.test.ts`'s existing assertions on these
> three handlers pass unmodified, which is the regression signal for "wire behaviour unchanged."
>
> **Getting `@bifurc/protocol` consumable from `src/` required a slice of work item 8**, done
> alongside this: root `package.json` now declares `"workspaces": ["packages/*"]` and depends on
> `"@bifurc/protocol": "*"`; `npm install` links it via a real `node_modules` symlink, and
> `packages/protocol` is built with its own `npm run build` (tsup → ESM+CJS+`.d.ts`) so `tsc`
> resolves it through the compiled `.d.ts` rather than pulling raw `.ts` source into the `src/`
> program (the latter tripped `rootDir` violations — TS6059 — when first attempted with a plain
> path alias). Verified end-to-end: `npm run typecheck` is clean, `npm run build:main` compiles,
> and `node -e "require('./dist/commands/registry.js')"` resolves `@bifurc/protocol` from the
> compiled output without error. Type-only consumers (`coreHandlers.ts`'s
> `import type { ConfigGetParams, ... }`) are erased at compile time and never touch the runtime
> module graph at all — only `registry.ts` itself requires `@bifurc/protocol` at runtime.
>
> **Second batch (this session): `syncHandlers.ts`.**
> `sync:setRemote/disconnect/push/pull/getState/setAutoSync/getEntityStatus`,
> `git:diff/discard/sync/history`, `entity:publish/restore`, `folder:publish`, `audit:diff`, and
> `history:list/diff` (16 of 19) all convert cleanly — clean 1:1 channels, no `entityCrudFactory`
> involvement, exactly the "next slice" called for above. **`audit:list` was deliberately
> reverted to its original direct handler** after converting it exposed a real, pre-existing gap:
> the frozen `AuditListParams` schema is `.strict()` and does not declare `fromTs`/`toTs`, but
> `AuditLogPanel.tsx` actively sends both when a date-range filter is set. Routing it through
> `registry.invoke()` as specified would have made every date-filtered audit query throw — a
> genuine regression, not a registration-only change, so it was left as-is with a comment
> explaining why (a `protocol-changes.md` entry is the right fix, not a workaround here).
> `audit:export` remains untouched for the SPLIT reason already documented (engine renders
> content; shell owns the save dialog). This is the first real evidence for why work item 7's own
> status note insists on converting "one file at a time" rather than batching — the mechanical
> cases and the one-schema-gap case were sitting in the same file.
>
> **Third batch (this session): `folderHandlers.ts`.** All 4 folder commands (`folder.add`,
> `folder.rename`, `folder.move`, `folder.delete`) convert cleanly — `FolderKind` in
> `packages/protocol/src/commands/folder.ts` is the exact same 11-value enum as the local
> `FolderKind` type here, and every param name/shape matches the real `preload.ts` call sites
> with no gap like `audit.list`'s.
>
> **Fourth batch (this session): `tlsHandlers.ts`.** Only `tls.generate`, `tls.certStatus`, and
> `tls.removeCert` convert — all three take no params (`z.object({}).strict()`). The other three
> channels in the file (`tls:exportCert`, `tls:importCert`, `tls:importKey`) are SPLIT (engine
> should read/return cert *content*; client owns the open/save dialog — see
> `packages/protocol/src/commands/tls.ts`'s own comments) but today's handlers still call
> `dialog.showSaveDialog`/`showOpenDialog` directly, so converting them needs the same real
> behavioural split `tls:installCA` got in work item 5, not a registration-only move. Left
> untouched with a comment.
>
> **Fifth batch (this session): `runnerHandlers.ts`.** Only `runner.getHistory` and
> `runner.listFolderIds` convert (both take just `workspaceId`/`folderId`). Two more were
> **tried and reverted** — a second real schema-gap finding, the same class as `audit.list`:
> - `runner:saveReport`/`runner:exportReport` take the real `CollectionRunReport` object
>   (`renderer/lib/collectionRunner.ts`), which carries `requestId`, `url`, `testLogs`,
>   `preScriptError`, `postScriptError` per result — none of which exist in the protocol's
>   `RunResult`/`RunReport` schemas.
> - `runner:saveConfig`/`runner:loadConfig` were actually converted and then **failed a real
>   test**: `tests/integration/runnerStorage.integration.test.ts` saves configs like
>   `{delayMs, stopOnFailure, iterations}` with no `requestOrder` at all, but the protocol's
>   `RunnerConfigSchema` requires `{requestOrder, delayMs}` exactly. The on-disk config is
>   > genuinely free-form, not the schema's fixed shape — running the targeted integration suite
>   before committing caught this immediately, which is exactly why work item 7's own status note
>   says to convert one file at a time and check real usage rather than trust the schema blindly.
>
> **Sixth batch (this session): `graphqlHandlers.ts` and `soapHandlers.ts` (network-calling
> halves only).** `graphql.introspect`/`graphql.execute` and `soap.fetchWsdl`/`soap.execute`
> convert cleanly — all four match the frozen schema and the real `preload.ts` call sites
> exactly (`soap:fetchWsdl` needed wrapping its bare positional `url` string into `{url}` at the
> `ipcMain.handle` adapter, same pattern as `sync:disconnect` etc. in the second batch). Each
> file's CRUD-factory channels (`graphql:addRequest/Mock`, `soap:addRequest/Mock`, etc.) and
> schema/WSDL-CRUD channels (`graphql:addSchema/deleteSchema/listSchemas`,
> `soap:addWsdl/deleteWsdl/listWsdls` — no protocol command exists for these yet; the protocol's
> own comment on `graphql.ts` says this CRUD is meant to collapse into `entity.*` too) wait for
> the CRUD collapse.
>
> **Seventh batch (this session): `grpcHandlers.ts` (network-calling half).** All five —
> `grpc.execute`, `grpc.reflect`, `grpc.mockServerStatus`, `grpc.startMockServer`,
> `grpc.stopMockServer` — convert. These are pure stubs today (pinned by
> `tests/integration/protocolExecution.integration.test.ts`, Cleanup_plan.md §1.5's D6
> disposition: wontfix — see `packages/protocol/src/commands/grpc.ts`'s own comment), so there
> was no real-usage risk to check beyond the schema match, which is exact.
>
> **Eighth batch (this session): `applicationHandlers.ts` — all 10 channels.** This file has no
> CRUD-factory involvement at all (it's hand-written, not `entityCrudFactory`-generated) and,
> notably, **isn't renderer-exposed today** — there is no `application*` entry anywhere in
> `preload.ts`; only `tests/integration/applications.integration.test.ts` exercises it. Every
> param matches the frozen schema exactly (`application.save`'s `application` field is an opaque
> `z.record(...)`, so — unlike `runner.saveConfig` — there's no fixed-shape gap to trip over).
> All 10 channels (`list`, `save`, `delete`, `start`, `stop`, `getState`, `getAllStates`,
> `getLogs`, `checkPort`, `killPort`) convert.
>
> **Ninth batch (this session): `src/ipc/importExport/index.ts` — only `export.formats`.**
> The other three channels in this file (`importExport:export/preflight/import`) were checked
> and rejected — **two separate real gaps, not one**:
> - `export.create`/`import.preflight` are SPLIT (this file's own header comment already says
>   "Full blob-layer rewiring is P3's job" — engine should render/read content or a `blobId`;
>   client owns the save/open dialog), but today's handlers still call
>   `dialog.showSaveDialog`/`showOpenDialog` themselves and pass the resulting path straight to
>   the exporter/importer.
> - `import.commit`'s `collisionStrategy` enum is `["skip", "overwrite", "rename"]` in the frozen
>   protocol schema, but the actual local `CollisionStrategy` type — and every real renderer call
>   site — uses `["keep", "override", "new"]`. Different string values, not just missing fields:
>   converting this one as specified would reject every real import outright. This is the third
>   schema-gap finding this session, and the most severe (a hard enum mismatch, not a missing
>   optional field).
>
> **Not done — and deliberately not attempted this pass:** converting the remaining ~55
> handlers — all `entityCrudFactory.ts`-generated CRUD channels (`mock:add`, `rule:update`,
> `ws:delete`, and ~33 more), plus `importExport:export/preflight/import`. The CRUD
> channels are one factory generating N per-kind channels
> today, while the protocol collapses all of them into six generic `entity.*` commands (P1 item
> 2) — wiring those through the registry means collapsing the factory first, a data-modelling
> change in its own right, not a mechanical find-and-replace. The import/export channels are P3
> (`File_Ops_Protocol.md` blob layer) territory per their own file's comment, not this pass's.
> Recommended next slice for a future
> session: tackle the CRUD collapse as its own dedicated, separately-verified step — with this
> session's three schema-gap findings (`audit.list`, `runner.saveConfig`, `import.commit`) as a
> concrete reminder to check every collapsed command against real renderer payloads and targeted
> tests, not just the schema shape.
>
> **Tenth batch (this session): the CRUD collapse itself — `entityCrudFactory.ts`'s 36 channels
> (12 kinds x add/update/delete) plus `entity:load`/`entity:setEnabled`.** This is the "recommended
> next slice" from the previous session, done as its own dedicated pass:
>
> - `entityCrudFactory.ts`'s three `ipcMain.handle` bodies were extracted into
>   `createEntityCore`/`updateEntityCore`/`deleteEntityCore` — byte-for-byte the same logic,
>   just callable from more than one place. Every kind's `opts` (previously only closed over by
>   its own three handlers) is now also stored in a module-level `entityCrudRegistry: Map<string,
>   CrudFactoryOpts<any>>`, keyed by the engine-internal storage `kind` string, looked up lazily
>   at invoke time (not at registration time), so it does not matter which of
>   `crudHandlers.ts`/`graphqlHandlers.ts`/`soapHandlers.ts`/`grpcHandlers.ts` registers first.
> - Three new commands — `entity.create`, `entity.update`, `entity.delete` — are registered once,
>   at module load, dispatching to whichever kind's core function `entityCrudRegistry` resolves.
>   `registerEntityCrudHandlers()`'s legacy per-kind `ipcMain.handle` bodies are now thin
>   adapters calling `commandRegistry.invoke("entity.create/update/delete", …)`, exactly work
>   item 7's own before/after shape. **One genuine return-shape mismatch surfaced and was
>   bridged, not left as a gap:** `entity.create`'s frozen result is `{id, entity}`, but the
>   legacy `mock:add`/`rule:add`/etc. channels have always returned the raw created entity —
>   the adapter unwraps `.entity` before returning, so the wire response stays byte-identical.
>   `entity.update`/`entity.delete` already returned `{ok: true}` on both sides, so those pass
>   straight through with no unwrapping.
> - **A real, load-bearing kind-naming mismatch was found and bridged with a translation table
>   (`src/commands/entityKindMap.ts`), not left undone.** The frozen `EntityKind` enum (used by
>   `entity.create`/`update`/`delete`/`load`) renames two of the twelve CRUD kinds relative to
>   their engine-internal storage `kind` string: `"rules"` (the real value used by `writeEntity`,
>   `readEnabledSet`, every `ProxyRulesPanel.tsx` call site, etc.) is `"proxyRules"` on the wire;
>   `"sockets"` is `"wsConnections"`. Unlike this session's earlier schema-gap findings
>   (`audit.list`, `runner.saveConfig`, `import.commit` — all left unconverted because the gap
>   was a structural incompatibility with no safe 1:1 mapping), this one *is* a safe, consistent
>   rename with no structural difference, so it was bridged with `toProtocolKind()`/
>   `toEngineKind()` rather than left as a gap. The other ten CRUD kinds, plus the four
>   non-CRUD-factory kinds already carried by `EntityKind` (`environments`, `graphqlSchemas`,
>   `protoFiles`, `wsdls`), are identical strings on both sides and pass through unchanged.
>   `EntitySetEnabledParams.kind`, by contrast, was already restricted to the six literal
>   engine-internal strings that carry enabled-state (`mocks`, `mappings`, `rules`,
>   `graphqlMocks`, `soapMocks`, `grpcMocks`) — it needs no translation at all, a design
>   inconsistency with the generic `EntityKind` worth knowing about but not worth fixing here.
> - `entity:load` and `entity:setEnabled` (previously two hand-written `coreHandlers.ts` handlers,
>   not `entityCrudFactory`-generated) were converted the same way: their bodies moved into
>   `commandRegistry.register("entity.load"/"entity.setEnabled", …)` at module scope (same
>   convention as `config.get`/`env.setActive`), and the `ipcMain.handle` bodies became thin
>   adapters. `entity:setEnabled`'s pre-existing `{ok:false, error:"invalid_kind"}` guard for an
>   unrecognised kind is kept **ahead of** `commandRegistry.invoke()`, not inside the registered
>   handler — `EntitySetEnabledParams.kind` is a strict 6-value enum, so an invalid kind would
>   otherwise fail Zod validation and throw instead of resolving gracefully, a real behaviour
>   change the guard exists specifically to prevent.
> - **Scope boundary, deliberately not crossed:** `entity.create`/`entity.update`/`entity.delete`
>   only dispatch kinds present in `entityCrudRegistry` — the twelve `entityCrudFactory.ts`
>   kinds. `environments` (gated by `subscription/entityCount.ts`'s create limit, flat storage,
>   no folder concept) and `graphqlSchemas`/`protoFiles`/`wsdls` (add + delete only, no update,
>   via `graphql:addSchema`/`grpc:addProto`/`soap:addWsdl` — none of them `entityCrudFactory`-
>   generated) keep their own bespoke handlers untouched; calling the generic commands with one
>   of those four kinds throws a clear "not routed through the CommandRegistry yet" error rather
>   than silently doing the wrong thing. Unifying those four is a separate, smaller follow-up,
>   not attempted this pass.
> - Verified: `npm run typecheck` and `npm run build:main` are clean; the full suite is
>   1597/1599 (the 2 failures are a pre-existing, unrelated `127.0.0.1:1` connectivity quirk in
>   this sandbox — confirmed by reproducing them against the pre-change `git stash` tree too).
>   Five new tests in `tests/ipc/handlers.test.ts` (`entity.* CommandRegistry collapse`) pin the
>   two kind-translation cases specifically (`rule:add`/`ws:add` still call `writeEntity` with
>   the untranslated `"rules"`/`"sockets"` storage kind; `entity:load` with those two kinds
>   resolves instead of throwing).
>
> **Remaining, explicitly out of scope for this pass:** unifying `environments` onto the same
> generic commands (gated create, flat-only storage, its own quirks); and the ~4
> `importExport:*` / SPLIT channels already documented above as P3 territory. Note
> `graphqlHandlers.ts`/`soapHandlers.ts`/`grpcHandlers.ts`'s own `graphql:addRequest/Mock`,
> `soap:addRequest/Mock`, `grpc:addRequest/Mock` etc. channels are **not** in this remaining
> list — they already called the same shared `registerEntityCrudHandlers()` this pass rewired
> (see `crudHandlers.ts` for the other 6 of the 12 covered kinds), so converting the factory
> once converted all 12 kinds' 36 channels simultaneously, across all 4 call-site files.
>
> **Eleventh batch (this session): `graphqlSchemas`/`protoFiles`/`wsdls` — the last three
> non-`environments` kinds `EntityKind` carries.** These don't fit `CrudFactoryOpts` at all (no
> `AppConfig` array, no "update" concept — only add/delete/list, straight to disk via
> `writeEntity`/`deleteEntityFile`/`readAllEntities`), so rather than stretch the factory to cover
> a shape it wasn't designed for, a second, smaller registry was added alongside it:
> `simpleEntityKinds` (a `Set<string>`) plus `createSimpleEntityCore`/`deleteSimpleEntityCore`/
> `listSimpleEntitiesCore`, consulted as a fallback by `entity.create`/`entity.delete` when a
> kind isn't in `entityCrudRegistry`, and exclusively by the new `entity.list` command (which
> until now had zero registered handler — the previous batches' status notes only reserved its
> name). `registerSimpleEntityHandlers()` mirrors `registerEntityCrudHandlers()`'s shape:
> `graphql:addSchema/deleteSchema/listSchemas`, `soap:addWsdl/deleteWsdl/listWsdls`, and
> `grpc:addProto/deleteProto/listProtos` (9 channels, 3 kinds) all now route through
> `entity.create`/`entity.delete`/`entity.list`, with the same `.entity`/`.entities` unwrapping
> `registerEntityCrudHandlers` uses to keep the wire response byte-identical. All three kinds are
> identical strings on both the engine and protocol side, so `entityKindMap.ts` needed no changes.
> Verified: typecheck, `build:main`, and the full suite (still 1597/1599, same 2 pre-existing
> `127.0.0.1:1` failures) all clean — `tests/integration/protocolExecution.integration.test.ts`'s
> existing "graphql schemas round-trip through add → list → delete" / "soap WSDLs round-trip
> through add → list → delete" / "grpc protos round-trip through add → list → delete" tests are
> the regression signal here and needed no changes to keep passing.
>
> **What's left after this batch:** only `environments` (gated by `subscription/entityCount.ts`'s
> create limit; also has an "update" unlike the three above) remains outside the CommandRegistry
> among `EntityKind`'s 16 values, plus the P3-bound `importExport:*` channels. Unifying
> `environments` is a smaller, well-scoped follow-up — it would need the generic `entity.create`
> path to carry the create-gate check, which none of the other kinds do today.
>
> **Twelfth batch (this session): `environments` — the last non-`importExport:*` gap.** It fits
> `CrudFactoryOpts` exactly like `mappings` does (`isFlat: true`, no folders, no enabled state,
> has an update), so no new registry shape was needed — just one addition to the existing one:
> `CrudFactoryOpts.gateKind?: string`, the friendly kind name (`"environment"`) `gateCreate()`
> (`@/subscription/entityCount`) expects. `entity.create`'s registered handler checks it — ahead
> of `createEntityCore()`, so a blocked create never touches config/disk — and returns
> `{error: "limit_reached", ...gate}` verbatim when `gateCreate()` disallows it, exactly what
> `env:add` returned before the collapse. `EntityCreateResult`'s frozen `{id, entity}` shape has
> no room for that error shape, but `registry.ts`'s `invoke()` only validates *params*, not
> return values (the same trade-off `entity:setEnabled`'s `invalid_kind` guard already relies
> on), so this is safe. **One real bug surfaced and was fixed, not just worked around:** the
> `ipcAdd` thin adapter unconditionally did `result.entity` to unwrap `entity.create`'s return —
> which silently turned a gate-blocked create into `undefined` on the wire instead of the error
> object, a regression a new test caught immediately. Fixed with an `"entity" in result` check
> that passes the error shape through unchanged. `environments`' two delete-time quirks — the
> synthetic `"__global__"` environment can never be deleted, and deleting the active environment
> must clear `activeEnvironmentId` — are handled the same way the pre-existing mapping->proxyRule
> cascade delete is: a `kind === "environments"` special case inside the shared, generic
> `deleteEntityCore()`, not a new hook. `coreHandlers.ts`'s hand-written `env:add`/`env:update`/
> `env:delete` `ipcMain.handle` bodies are gone, replaced by one `registerEntityCrudHandlers<Environment>({ipcPrefix: "env", kind: "environments", configKey: "environments", isFlat: true, gateKind: "environment"})`
> call — `env:setActive` (a different command, `env.setActive`) is untouched. `entityKindMap.ts`
> needed no changes: `environments` is identical on both the engine and protocol side already.
> Verified: `npm run typecheck` and `npm run build:main` are clean; the full suite is 1601/1603
> (the 2 failures are the same pre-existing, unrelated `127.0.0.1:1` connectivity quirk —
> reconfirmed by reproducing them against the pre-change `git stash` tree too). Three new tests
> in `tests/ipc/handlers.test.ts` (`entity.* CommandRegistry collapse`) pin `env:add`'s flat
> storage, the gate-block error shape (and that it creates nothing), and the `"__global__"`
> delete guard; the pre-existing `env:add`/`env:update`/`env:delete` describe blocks earlier in
> that file were left in place unchanged and still pass, now exercising the collapsed path.
>
> **What's left in work item 7 after this batch: only the P3-bound `importExport:*` SPLIT
> channels.** Every `EntityKind` value — all 16 — is now routed through the CommandRegistry.
> Total: **~112 commands** routed through it (see `registry.ts`'s own status note).

---

## Work item 8 — Package structure and build

```
packages/
  protocol/          # from P1
  engine/
    src/
      commands/      # registry + command implementations
      eventBus.ts    # NOT events/bus.ts — see the note below
      proxy/         # moved from src/proxy
      store/         # moved from src/store
      sync/          # moved from src/sync
      applications/  # moved from src/applications
      startup.ts
      paths.ts
      index.ts       # public API: createEngine(), EngineOptions
    package.json
    tsup.config.ts
```

> **Corrected 2026-09-15:** this tree originally listed `events/bus.ts`. The EventBus has always
> been a single file, `src/eventBus.ts`, and it moved to `packages/engine/src/eventBus.ts` in layer 2
> — there is no `events/` directory and never was. The tree above is now the *target*; as of layer 3
> the moved entries are `store/`, `lib/`, `subscription/`, `proxy/`, `sync/`, `eventBus.ts`,
> `applications/`, `companion/` and `commands/`. Only `startup.ts` (and `shutdown.ts`, which the
> target tree does not list) remain to move.

### Build config

- `tsup` → ESM + CJS + `.d.ts`.
- **Resolve the `@/*` aliases.** Currently `tsc` + `tsc-alias` maps `@/*` → `src/*`. At package
  boundaries that must become real imports (`@bifurc/engine/...`) or per-package aliases. Expect this to
  be fiddly — it touches every import line in the moved files.
- Public API surface: `createEngine(opts) → { start(), stop(), registry, bus, status() }`. Nothing else
  is exported. Keep it small.

### Workspace restructure

Root `package.json` currently has `"workspaces": ["bifurc"]`. Becomes:

```json
{ "workspaces": ["packages/*", "apps/*"] }
```

Note `package.json` currently declares **all** dependencies (React, CodeMirror, electron,
electron-builder, tailwind) in one flat list. These must split:
- engine deps → `packages/engine` (`ws`, `js-yaml`, `simple-git`, `mkcert`, `archiver`, `unzipper`)
- renderer deps → `apps/desktop` or a shared `packages/ui`
- build tooling → root devDependencies

**Getting this split wrong is how you end up shipping CodeMirror inside a Docker image.**

### Status (2026-09-16): started — package created, three layers moved

The package, its build and the resolution story are **done and verified**. The move is being done
in layers rather than one commit, per this doc's own mitigation ("do one package first, prove the
pattern, then move the rest").

**Layer 3 — `applications/`, `companion/`, `commands/` — has also moved.** 8 files, 48 statements
rewritten across 26 files. The cleanest layer yet: all 7 files that were in the pre-move coverage
report came back *identical* on all four metrics including raw covered/total counts (the 8th,
`applications/types.ts`, had been wrongly excluded from the report — see the note in `TESTING.md`
§5). `ws` joined the engine as its third runtime dependency (it is `companion/` that needs it, as
predicted when `companion/` was still in `src/`).

**Layer 2 — `proxy/`, `sync/` and `eventBus.ts` — has moved too.** These three had to go together,
and the reason is worth recording: `eventBus.ts` imports `@/proxy/logEmitter` and
`@/proxy/webhookServer` (types) and `@/sync/statusTracker` (a value), while `proxy/**` imports
`@/eventBus` for `bus.emitTyped`. Moving any one of them alone would have left the engine importing
*out* of the package. 28 files moved, 177 import statements plus 12 `vi.mock` specifiers rewritten.

```
packages/engine/
  package.json        # @bifurc/engine, deps: mkcert, simple-git, ws; no electron, no renderer
  tsconfig.json       # moduleResolution: bundler, types: [node], noEmit
  tsup.config.ts      # ESM + CJS + .d.ts, structure-preserving multi-entry
  src/
    store/            # moved from src/store   (11 files)   — layer 1
    lib/              # moved from src/lib     (2 files)    — layer 1
    subscription/     # moved from src/subscription (1)     — layer 1
    proxy/            # moved from src/proxy   (19 files)   — layer 2
    sync/             # moved from src/sync    (8 files)    — layer 2
    eventBus.ts       # moved from src/eventBus.ts          — layer 2
    applications/     # moved from src/applications (4)     — layer 3
    companion/        # moved from src/companion (2)        — layer 3
    commands/         # moved from src/commands (2)         — layer 3
    index.ts          # provisional barrel — see below
```

Layer 2 needed one new runtime dependency, `mkcert` (used by `proxy/tlsCert.ts`); everything else in
`proxy/` and `sync/` is Node builtins (`child_process`, `http`, `https`, `net`, `tls`, `vm`,
`zlib`, `fs`, `path`, `os`, `events`) plus the already-present `simple-git`. Layer 3 added `ws`
(`companion/companionServer.ts`).

**Why the bottom layer went first:** it is the only part of the engine with no outgoing `@/` dependency
on the rest of the app (`store/` imports only itself; `lib/` and `subscription/` import nothing
internal at all). That made it the cheapest place to prove the package boundary end to end. Layer 2
was the first layer that *did* have internal couplings to untangle, which is why the `eventBus` ↔
`proxy`/`sync` cycle had to be handled as one unit rather than three. Layer 3 was measured clean
before it started — its only `@/` import was internal to the layer — which is why it needed no
special handling and came back 7/7 exact.

**The build must not bundle — this is load-bearing, not a style choice.** `store/paths.ts` holds
the resolved data root as module-level state, and `store/config.ts` / `store/gitStore.ts` hold
caches the same way. Bundling would inline a private copy of each into every entry point, so a
`setDataRoot()` call made through one entry would not be seen by the modules reached through
another — and because the failure mode is a fallback rather than a throw, it would be silent.
`bundle: false` emits one output file per input file and keeps the module graph — and therefore
every singleton — exactly as written. Verified after the build: `dist/store/config.js` does
`require("./appSettings")` / `require("./workspaceFs")`, and `dist/store/paths.js` owns the
singleton.

**Three resolution paths, deliberately different:**

| Consumer | Resolves `@bifurc/engine/*` via | Points at |
|---|---|---|
| `tsc` (typecheck + emit) | root `tsconfig.json` `paths` | `packages/engine/dist/*.d.ts` |
| vitest | `resolve.alias` in `vitest.config.ts` (**redeclared inside each `test.projects` entry**) | `packages/engine/src/*.ts` |
| Node at runtime | the package's `exports` map | `packages/engine/dist/*.js` |

Tests point at **source**, not `dist`, on purpose: they must exercise the real code, and
`coverage.include` has to be able to instrument it — resolving to `dist/` would report the engine
as 0% covered while silently testing compiled JavaScript. Production points at the build output.

**Getting the vitest path right took real work, and the failure mode was nasty.** Because
`@bifurc/engine` is a genuine npm workspace package, Node can resolve it unaided — so Vitest
classified it as *external* and loaded the **built** `packages/engine/dist/*.mjs` through native
`import()`, bypassing every Vite resolver and transformer. Consequences: all 14 engine files
reported **0%** coverage, and — far worse — a source edit that was never rebuilt would have been
**silently untested**, because the suite passed green against stale output.

Three candidate mechanisms were tested and only one works:

| Mechanism | Result |
|---|---|
| `resolveId` plugin (as `dualAliasPlugin` does for `@/`) | **Never invoked.** Vitest externalizes the specifier before any plugin resolver runs. Verified with a temporary `console.error` probe that never printed. |
| `deps.inline: [/^@bifurc\/engine(\/|$)/]` (and the legacy `server.deps.inline`) | **No effect.** Not needed once the alias works. |
| `resolve.alias` | **Works** — but only when redeclared inside each `test.projects` entry. |

The last point is the one that cost the most time. `resolve.alias` placed at the root
`defineConfig` level is **silently ignored** by project runs — exactly like root-level `plugins`,
which is why `dualAliasPlugin` had always been duplicated into each project. The proof was
deliberately corrupting the alias target to a non-existent `packages/engine/src/__BOGUS__/`
directory and observing that the suite still passed and still loaded `dist/`. The config now
shares a single `viteOptions` object across the root and both projects.

`coverage.include` also gained `packages/engine/src/**/*.ts`, and that entry is load-bearing:
`src/**/*.ts` does **not** match `packages/*/src/**`, so without it the moved files would vanish
from the report entirely.

**Layer 2 added two rewrite passes that layer 1 did not need, and both are silent-failure traps:**

- **Self-imports must become relative inside the package.** After layer 1 the moved files already
  contained 23 `@bifurc/engine/store/*` self-imports. Left alone they *appear* to work, but they
  resolve through the `exports` map to `dist/` at **runtime** while the rest of the engine loads from
  source — i.e. **two live copies of module state**, which is precisely the silent-singleton failure
  `bundle: false` exists to prevent. Any file moved into `packages/engine/src` must use relative
  paths for engine-internal imports.
- **`vi.mock()` is not matched by an import-shaped regex.** Layer 2's first four passes rewrote 177
  specifiers and still left **12 stale mock specifiers across 7 files**, because
  `vi.mock("@/sync/autoSync", …)` is a plain function call, not a `from "…"`. A mock whose specifier
  no longer matches the code under test is worse than no mock: the real module loads and the test
  silently stops being hermetic. The rewriter must match
  `/\bvi\.(mock|doMock|unmock|importActual|importMock)\(\s*(["'])([^"']+)\2/g` — while **not**
  touching `tests/renderer/**`, whose `@/lib/*`, `@/hooks/*` and `@/components/*` mocks belong to the
  renderer's own alias space.

Worth recording: `tsc-alias` does **not** rewrite `@bifurc/engine/*` to a relative path the way it
rewrites `@/*`. It leaves it bare, exactly as it already leaves `@bifurc/protocol` bare, and Node
resolves it through the workspace symlink and the `exports` map. This was verified by inspecting
the emitted `dist/startup.js` (`require("@bifurc/engine/store/gitStore")`) and by resolving the
specifier from `dist/` — it lands on `packages/engine/dist/store/config.js`.

**A latent CI bug was found and fixed while doing this.** `dist/` is gitignored repo-wide, so
neither package's build output is committed, and CI only ran `npm ci` — meaning `npm run
typecheck` failed on a fresh clone with ten `TS2307: Cannot find module '@bifurc/protocol'`
errors. Adding a second package would have doubled that. Fixed by adding `build:packages`
(`npm run build --workspace …` for each package) and wiring it into `prepare` — which `npm ci`
runs — plus `build:main` and `typecheck`. Installs are now self-sufficient.

**`index.ts` is provisional and says so.** The plan's end state for this file is
`createEngine(opts) -> { start(), stop(), registry, bus, status() }` with nothing else exported.
That is not reachable while only the storage layer has moved, so the barrel exposes the moved layer
as *namespaces* rather than flattened star-exports — `store/config.ts` re-exports from
`store/workspaceFs.ts` and `store/types.ts`, and `WorkspaceSyncConfig` / `WorkspaceSyncMeta` are
declared in both `appSettings.ts` and `types.ts`, so flattening would make those names ambiguous.
Namespacing also makes the interim surface obviously provisional, so nobody starts depending on it
before P6 freezes the real API.

**Cleanup done in passing:** two `vi.mock("@/subscription/gate")` blocks were deleted from
`tests/ipc/handlers.test.ts` and `tests/spike/protocolPoc.test.ts`. They mocked a module that does
not exist anywhere in this repository — `git log --all` shows no `subscription/gate` file has ever
existed, and the factories exposed an older `canCreate`/`canEnable`/`canUseFeature` API while the
real module (`subscription/entityCount.ts`) exports `gateCreate`/`gateEnable`. They were leftovers
from a pre-history refactor. Nothing imported the module, so they were inert; leaving them would
have produced a bogus `@bifurc/engine/subscription/gate` specifier during the rewrite.

### Still to do in work item 8

The remaining engine modules move next, in dependency order, each verified against the full suite:

1. ~~`proxy/` (the actual product) and `sync/`~~ — **done (layer 2)**, together with `eventBus.ts`,
   which they are mutually coupled to. 28 files, 189 statements rewritten across 95 files; `mkcert`
   added as the engine's second runtime dependency.
2. ~~`applications/`, `companion/`, `commands/`~~ — **done (layer 3)**. 8 files, 48 statements
   across 26 files, 7/7 of the pre-move files exact on all four coverage metrics. `ws` moved with
   `companion/`, as predicted.
3. `startup.ts` and `shutdown.ts` — already Electron-free, they just need to move. These are the
   last two engine files in `src/`.
4. `createEngine(opts)` — the real public API, which is what finally satisfies the
   "engine starts from a bare Node script with `--data-dir`" acceptance criterion
5. Dependency split completion: `js-yaml`, `archiver`, `unzipper` and `@bifurc/protocol` move out
   of the root flat list as their modules move (`mkcert`, `simple-git` and `ws` have already moved).
   `@bifurc/protocol` is a package dependency rather than a bare one, so it is a separate decision.

After step 3, `src/` contains only `ipc/` (the registration layer this phase replaces), `main.ts`
and `preload.ts`.

**Not part of item 8, but found while doing it:** the packaged app cannot resolve `@bifurc/protocol`
or `@bifurc/engine`, because `build.files` covers only `dist/**/*` + `package.json`. Dev mode and the
test suite are unaffected. See the measured note in `plan/12`.

---

## How to start — the first three things

1. **Build the EventBus and convert site #1** (`src/ipc/handlers.ts:24–49`). It is the simplest and it
   proves the pattern end-to-end. Run the test suite — it should stay green.
2. **Do item 4 (data dir) next.** It is 2 files, ~30 lines, and it unblocks Docker and CLI entirely.
   Cheap, high-leverage, low-risk.
3. **Then item 2's remaining 7 sites, one commit each.** One file per commit, full test run per commit.
   Do not batch them — when an event goes missing you need to bisect to a single file.

> **Status (2026-09-15): all three done** — see the "Honest status" note under Acceptance criteria.
> Items 5 (split shell-only handlers) and 6 (headless startup preflight) were also started this
> session — item 5's first row is done.
>
> **Status (this session): item 7 started.** `src/commands/registry.ts` — the `CommandRegistry` —
> now exists, unit-tested, and proven end-to-end on 48 commands across nine files:
> `coreHandlers.ts` (`config.get`, `env.setActive`, `workspace.setActive`), `syncHandlers.ts`
> (16 of 19), `folderHandlers.ts` (all 4), `tlsHandlers.ts` (3 of 6), `runnerHandlers.ts`
> (2 of 6), `graphqlHandlers.ts` (2 of 8), `soapHandlers.ts` (2 of 8), `grpcHandlers.ts` (5 of 8),
> `applicationHandlers.ts` (all 10), and `src/ipc/importExport/index.ts` (1 of 4). This also
> required landing a small, scoped slice of item 8 early: `packages/protocol` is now a real
> linked npm workspace dependency (`"@bifurc/protocol": "*"`), which is what makes its Zod
> schemas importable from `src/` at all. See work item 7's own status note for the full detail,
> the three real schema-gap findings (`audit.list`, `runner.saveConfig`, `import.commit`), and
> why the remaining ~55 handlers are next but not yet done.
>
> **Status (this session, continued): the CRUD collapse landed.** `entityCrudFactory.ts` — the
> factory behind all 12 CRUD kinds across `crudHandlers.ts`/`graphqlHandlers.ts`/
> `soapHandlers.ts`/`grpcHandlers.ts` — now routes its 36 add/update/delete channels through
> three new generic commands (`entity.create`/`entity.update`/`entity.delete`), and
> `coreHandlers.ts`'s `entity:load`/`entity:setEnabled` route through `entity.load`/
> `entity.setEnabled` the same way. `src/commands/entityKindMap.ts` bridges the one real
> naming mismatch this required (engine `"rules"`/`"sockets"` vs. protocol
> `"proxyRules"`/`"wsConnections"`). Total now: **~100 commands** routed through the
> `CommandRegistry`. See work item 7's own status note (tenth batch) for the full detail —
> what's bridged, what's deliberately still out (`environments`, `graphqlSchemas`, `protoFiles`,
> `wsdls`, and the P3-bound `importExport:*` channels), and the new regression tests that pin
> the kind translation.
>
> **Status (this session, continued further): `graphqlSchemas`/`protoFiles`/`wsdls` (eleventh
> batch) and `environments` (twelfth batch) both landed.** Every `EntityKind` value is now
> routed through the CommandRegistry — the only thing still outside it is the P3-bound
> `importExport:*` SPLIT channels. See work item 7's own status note (eleventh/twelfth batches)
> for the full detail: the new `simpleEntityKinds`/`entity.list` registry for the three
> no-`AppConfig`-array kinds, and `CrudFactoryOpts.gateKind` plus the `environments`-specific
> delete guards for the create-gated kind. Total now: **~112 commands** routed through the
> `CommandRegistry`.

---

## Acceptance criteria

- [x] `grep -rn "from \"electron\"" packages/engine/src` returns **zero** results. *(Verified
      2026-09-15 after the bottom-layer move: **zero**. The package's only non-relative imports are
      `fs`, `os`, `path` and `simple-git`. Note this is now true of the *package*, not of `src/**`
      as a whole — `src/**` still has **17** electron-importing files, 11 of which are
      `src/ipc/handlers/*` importing only `ipcMain` (the registration layer this phase replaces
      wholesale). The genuinely shell/client-scoped ones are `main.ts`, `preload.ts`,
      `eventBridge.ts` (the deliberate temporary bridge) and `clientHandlers.ts` (never moves to
      `packages/engine`, see work item 5). Everything this phase converted is Electron-free:
      `src/proxy/` — "the actual product" per this doc's own note — has **zero** Electron imports,
      and `gitStore.ts`, `companionServer.ts`, `webhookServer.ts`, `processSpawner.ts`,
      `appSettings.ts`, `workspaceFs.ts`, `startup.ts`, `shutdown.ts` and `eventBus.ts` are all
      clean. They become package-level guarantees as each module moves.)*
- [x] `grep -rnE "BrowserWindow|app\.getPath|dialog\.|\bshell\." packages/engine/src` returns zero.
      *(Verified 2026-09-15: **zero**. Two classes of false positive were removed rather than
      tolerated, because a criterion that greps clean is worth more than one with documented
      exceptions. (1) Three comments in `store/{paths,appSettings,workspaceFs}.ts` and one in
      `eventBus.ts` quoted `setDataRoot(app.getPath("userData"))` / `BrowserWindow.getAllWindows()`
      while explaining what the shell does; they were reworded to keep the same information without
      the token. (2) `proxy/service-discovery.ts` holds a literal Windows path ending in
      `powershell.exe`, which the unanchored pattern `shell\.` matched — so the pattern is now
      `\bshell\.`, which is the *correct* semantic (Electron's `shell` API is always a standalone
      identifier) and still catches real usage: it matches `shell.openExternal(url)` in
      `src/ipc/handlers/clientHandlers.ts`. The only remaining `BrowserWindow.getAllWindows()` /
      `webContents.send` sites in `src/` are `eventBridge.ts` (deliberate) and `clientHandlers.ts`
      (zoom/titlebar chrome, CLIENT-classified).)*
- [ ] Engine starts from a bare Node script with `--data-dir`, serves, and shuts down cleanly.
      *(Not done — the package now exists and builds, but only its storage layer has moved, so there
      is still nothing to run. The pieces are individually tested: `store/paths.ts` resolves the
      data root with a loud failure when unset, `src/startup.ts` provides `preflight()` **and**
      `bootstrapWorkspaces()`, and `src/shutdown.ts` provides an idempotent `shutdownEngine()`. What
      is missing is `createEngine()` plus the proxy/sync/command modules that would sit behind it —
      the remaining steps of work item 8.)*
- [x] `setDataRoot()` not called → loud error, not a silent cwd fallback. Implemented in
      `src/store/paths.ts` (`DataRootNotInitialisedError`), unit-tested
      (`tests/store/paths.test.ts`, 8/8 passing), **and now wired as the primary path** in
      `appSettings.ts`/`workspaceFs.ts` — see the work item 4 status note above.
- [x] All unit + integration suites pass. *(**69 files / 1616 tests, 1615 passing** as of
      2026-09-15; zero regressions.)* The 11 e2e suites remain **unverified** — they cannot run in
      this sandbox (no desktop session, per `plan/baseline.md` "Environment caveats"). The single
      failure is the documented sandbox network-interceptor caveat
      (`ECONNREFUSED 127.0.0.1:1` in `tests/spike/protocolPoc.test.ts`, P0 spike code untouched by
      P2), reproduced identically on the pre-change tree. Earlier sessions recorded 2 such failures;
      the count varies run to run, which is consistent with it being an environment artifact rather
      than a regression.
- [x] No `companion:refresh`; replaced by `entity.changed`. **Internally** — every engine-side
      emission site now emits `bus.emitTyped("entity.changed", ...)`. The wire name
      `companion:refresh` still exists, deliberately, in the temporary shell bridge
      (`src/ipc/eventBridge.ts`) that translates it back for the current, unmodified renderer —
      removing the wire name entirely is P5/P6 work (updating the renderer's listener).
- [x] `coreHandlers` no longer imports from `main`. Verified — the `updateTrayMenu()` call became
      `bus.emitTyped("settings.changed", {})`; `src/main.ts` itself subscribes
      (`bus.onTyped("settings.changed", () => updateTrayMenu())`), which keeps the dependency
      pointing the correct direction (shell depends on engine bus, not the reverse).
- [x] `packages/engine` has no renderer or Electron dependency in its `package.json`. *(Verified
      2026-09-15: `dependencies` is exactly `{"simple-git": "^3.36.0"}`; `devDependencies` is
      `@types/node`, `tsup`, `typescript`. No Electron, no React, no CodeMirror. The remaining
      engine deps (`ws`, `js-yaml`, `mkcert`, `archiver`, `unzipper`, `@bifurc/protocol`) are still
      in the root flat list because the modules that use them have not moved yet — removing them
      now would break the shell. They move with their modules in the remaining steps of work
      item 8.)*
- [x] Engine package builds to ESM + CJS + types. *(Verified 2026-09-15: `npm run build` in
      `packages/engine` emits `dist/store/config.js` (CJS), `dist/store/config.mjs` (ESM) and
      `dist/store/config.d.ts` — the directory structure is preserved, so consumers can deep-import
      `@bifurc/engine/store/config`. Declarations are emitted with `bundle: false` for the same
      reason the JavaScript is: bundling would duplicate module-level state. `packages/protocol`
      builds the same way, and is the thing `src/commands/registry.ts` depends on at runtime.)*

**Honest status:** work items 1 (EventBus), 2 (broadcast inversion, all 8 sites) and 3
(`processSpawner` — the window reference **and** the idempotent lifecycle) are **done and
verified** — full unit + integration suite green, zero regressions. Work item 4 (data dir) is
**done**: `setDataRoot()` is the primary
path in `appSettings.ts`/`workspaceFs.ts`, wired from `main.ts`'s `app.whenReady()`; the two
`*Override` test hooks were deliberately kept separate rather than folded in (see the work item 4
status note above for the reasoning). Work item 6 (headless startup) is **done apart from one
deliberate gap**: `preflight()` and `bootstrapWorkspaces()` both live in the Electron-free
`src/startup.ts` and are tested, and `main.ts` no longer touches
`workspaceFs`/`gitStore`/`autoSync`/`syncManager` directly — but the pre-existing git-check block
was left as its own direct call rather than routed through `preflight()`, and whether the other
checks should become *blocking* is a product decision left open. Work item 5 (split shell-only
handlers) is **started**: the 10 pure-CLIENT channels
now live in `src/ipc/handlers/clientHandlers.ts`, registered separately from
`registerIpcHandlers()`; the two SPLIT channels in `systemHandlers.ts` (`app:checkUpdate`,
`capture:shareJson`) and `main.ts`'s tray/window/menu code are untouched, per that item's own
status note. Work item 7 (CommandRegistry) is **started**: `src/commands/registry.ts` exists,
unit-tested, and proven on 48 commands across `coreHandlers.ts` (`config.get`, `env.setActive`,
`workspace.setActive`), `syncHandlers.ts` (16 of 19), `folderHandlers.ts` (all 4),
`tlsHandlers.ts` (3 of 6), `runnerHandlers.ts` (2 of 6), `graphqlHandlers.ts` (2 of 8),
`soapHandlers.ts` (2 of 8), `grpcHandlers.ts` (5 of 8), `applicationHandlers.ts` (all 10), and
`src/ipc/importExport/index.ts` (1 of 4); converting `audit.list`,
`runner.saveConfig`, and `import.commit` each exposed a real gap between the frozen protocol schema and actual
handler/renderer usage, so all three were deliberately left unconverted rather than silently breaking
real functionality — see work item 7's status note. **Since then, three further passes closed
almost everything else:** the CRUD collapse (all 12 `entityCrudFactory.ts` kinds' 36 channels
plus `entity:load`/`entity:setEnabled`, onto `entity.create`/`entity.update`/`entity.delete`/
`entity.load`/`entity.setEnabled`), the `graphqlSchemas`/`protoFiles`/`wsdls` simple-entity pass
(onto `entity.create`/`entity.delete`/`entity.list`), and the `environments` pass (the last
`EntityKind`, via a new `CrudFactoryOpts.gateKind` for its create-gate check plus
`deleteEntityCore`-level guards for its two delete-time quirks). Work item 7 is now **done
except for the P3-bound `importExport:*` SPLIT channels** — every `EntityKind` value routes
through the CommandRegistry, ~112 commands total. See work item 7's own status note (tenth
through twelfth batches) for the full detail. Work item 8 (the physical `packages/*`
restructuring + dependency split) is now **started, with its infrastructure done and three of four
layers moved**. `packages/engine`
exists as a real linked npm workspace with its own `tsup` pipeline (structure-preserving ESM + CJS +
`.d.ts`), its own `tsconfig.json`, and a `package.json` whose runtime dependencies are
`mkcert`, `simple-git` and `ws`; it has **zero** Electron imports and **zero**
`BrowserWindow`/`dialog.`/`\bshell.` references. **Layer 1** moved
`src/{store,lib,subscription}` → `packages/engine/src/` (14 files, via `git mv` so blame survives)
and rewrote 254 referencing statements across 97 files. **Layer 2** moved
`src/{proxy,sync}` + `src/eventBus.ts` (28 files) and rewrote 189 statements across 95 files —
those three had to go together because `eventBus` and `proxy`/`sync` import each other.
**Layer 3** moved `src/{applications,companion,commands}` (8 files, 48 statements across 26 files).
Renderer: **zero** files in every layer, because its `@/` alias points at `renderer/`. `packages/protocol`
remains a real linked workspace and is the other inter-package dependency. A latent CI bug was
found and fixed along the way: neither package's `dist` is committed, CI only ran `npm ci`, and a
fresh clone therefore failed `npm run typecheck` with `TS2307` for `@bifurc/protocol` — a new
`build:packages` script wired to `prepare` (which `npm ci` runs) plus `build:main` and `typecheck`
makes installs self-sufficient. What remains in item 8: moving `startup.ts` and `shutdown.ts`, then
building the real `createEngine()` public API — which is what finally satisfies the "engine starts
from a bare Node script with `--data-dir`" criterion. The dependency split is correspondingly
partial: the engine declares its own deps and no Electron/renderer deps, but the deps of the
not-yet-moved modules necessarily stay in the root list. See the "Cleanup_plan.md status" section of
`plan/README.md` for the still-open D6 sub-items that also block a complete P2.

**The move is now verified as behaviour-preserving, by measurement rather than by assertion.** The
full suite runs **1615/1616 over 69 files** — identical to the pre-move baseline at every layer —
and the one failure is the documented sandbox `ECONNREFUSED` caveat, not a regression. The stronger
evidence is per-file coverage, layer by layer:

| Layer | Files compared | Identical on all 4 metrics | Regressions | Missing |
|---|---:|---:|---:|---:|
| 1 — `store/`, `lib/`, `subscription/` | 12 | 11 | 0 | 0 |
| 2 — `proxy/`, `sync/`, `eventBus.ts` | 27 | 26 | 0 | 0 |
| 3 — `applications/`, `companion/`, `commands/` | 7 | **7** | 0 | 0 |

"Identical" includes the raw covered/total counts, not just the percentages. The two files that
moved did so in the **up** direction, with identical totals: `store/config.ts` because P2 items 5–7
added tests that reach more of it, and `sync/gitOps.ts` because a real-git fallback branch happened
to execute more in that run (line-level lcov data confirms which lines). Identical denominators are
the load-bearing part — they prove no statement was added, removed or restructured. Headline
coverage is **48.00% / 31.99% / 36.30% / 50.39%** (statements / branches / functions / lines), up
from 46.14 / 31.30 / 34.34 / 48.39, against a denominator that *grew* from 11,129 to 11,446
statements — so this is a genuine improvement, not a scope artefact. See `TESTING.md` §4.8.

That verification only became possible after fixing the test-resolution bug described above: before
it, the engine reported **all 14 files at 0%** while the suite passed, because tests were running
against the built `dist/`. The one file still at 0% is the new `index.ts` barrel, deliberately not
excluded — it becomes the real API when `createEngine()` lands.

---

## Rollback

Extraction is done in place with the app still working at every commit (P6 is what switches the shell
over). If the extraction goes wrong, `git revert` to the last green commit. **Because nothing is wired
up until P6, this phase is fully reversible** — that is deliberate.

---

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| Event inversion drops or duplicates events | **High** | One file per commit; full e2e run per commit; never batch sites |
| `@/*` alias resolution at package boundaries becomes a time sink | High | Do one package first, prove the pattern, then move the rest |
| **Materialised (2026-09-15).** The alias risk was real, and worse than expected — but it is now closed. Root cause: `@bifurc/engine` is a genuine npm workspace package, so Vitest externalized it and loaded the built `dist/*.mjs` through native `import()`. `resolveId` plugins never fire for an externalized specifier, and root-level `resolve.alias` is not inherited by `test.projects` runs (the same reason `plugins` was already duplicated per project). Symptom: 14 engine files at 0% coverage while the suite passed green — meaning an unrebuilt source edit would have been **silently untested**. Fixed with `resolve.alias` redeclared inside each project; a deliberately bogus alias target proved the diagnosis. Cost: roughly half a session. **Lesson for the remaining layers: verify that a moved module is actually instrumented by coverage before trusting a green suite.** | — | Closed. Covered by `TESTING.md` §4.8 and §5, and by the `bifurc-engine-layer-move` skill |
| Dependency split is done lazily, leaking renderer deps into the engine | Medium | Verify `packages/engine/package.json` by inspection at the gate |
| `processSpawner` shutdown becomes non-idempotent, orphaning child processes | Medium | Test SIGTERM twice; assert no orphaned children |
| Moving files breaks git history for the moved modules | Low | Use `git mv` so blame survives |
| Someone wires the shell up early "to test it" | Medium | Hard rule: the shell switch happens in P6 only |
