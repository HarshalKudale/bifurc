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
> **Not done:** extracting the workspace bootstrap from `main.ts` (init dirs, init repos, start
> auto-sync, validate active workspace, create a default one) into the engine — it still lives in
> the shell's `app.whenReady()` handler, unchanged.

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
      events/bus.ts
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

- [ ] `grep -rn "from \"electron\"" packages/engine/src` returns **zero** results. *(Not yet
      applicable — `packages/engine` does not exist yet; the physical restructuring in work item 8
      is not done. Progress made in place: `src/**` electron-importing files reduced from 22 →
      18 → **16** across sessions — `gitStore.ts`, `companionServer.ts`, `webhookServer.ts`,
      `processSpawner.ts`, `appSettings.ts`, and `workspaceFs.ts` are now electron-free. `src/proxy/`
      — "the actual product" per this doc's own note — has **zero** Electron imports. A new
      `src/ipc/handlers/clientHandlers.ts` was also added this session and does import `electron` —
      by design, it never moves to `packages/engine` (see work item 5's status note).)*
- [ ] `grep -rn "BrowserWindow\|app.getPath\|dialog\.\|shell\." packages/engine/src` returns zero.
      *(Same caveat — see the per-item status below for what's actually converted.)*
- [ ] Engine starts from a bare Node script with `--data-dir`, serves, and shuts down cleanly.
      *(Not done — needs item 8's package extraction. Item 6's preflight() now exists in
      `src/startup.ts` and is wired into `main.ts`, but nothing runs it from a bare Node script yet
      since there is no engine entrypoint outside Electron.)*
- [x] `setDataRoot()` not called → loud error, not a silent cwd fallback. Implemented in
      `src/store/paths.ts` (`DataRootNotInitialisedError`), unit-tested
      (`tests/store/paths.test.ts`, 8/8 passing), **and now wired as the primary path** in
      `appSettings.ts`/`workspaceFs.ts` — see the work item 4 status note above.
- [x] All unit + integration suites pass. *(67 files / 1594 tests as of this session; zero
      regressions.)* The 11 e2e suites remain **unverified** — they cannot run in this sandbox (no
      desktop session, per `plan/baseline.md` "Environment caveats"). Integration suite: 65/67
      files green, 1592/1594 tests, matching the documented baseline exactly (the 2 failures are
      the sandbox network-interceptor caveat, not a regression).
- [x] No `companion:refresh`; replaced by `entity.changed`. **Internally** — every engine-side
      emission site now emits `bus.emitTyped("entity.changed", ...)`. The wire name
      `companion:refresh` still exists, deliberately, in the temporary shell bridge
      (`src/ipc/eventBridge.ts`) that translates it back for the current, unmodified renderer —
      removing the wire name entirely is P5/P6 work (updating the renderer's listener).
- [x] `coreHandlers` no longer imports from `main`. Verified — the `updateTrayMenu()` call became
      `bus.emitTyped("settings.changed", {})`; `src/main.ts` itself subscribes
      (`bus.onTyped("settings.changed", () => updateTrayMenu())`), which keeps the dependency
      pointing the correct direction (shell depends on engine bus, not the reverse).
- [ ] `packages/engine` has no renderer or Electron dependency in its `package.json`. *(N/A —
      package doesn't exist yet.)*
- [ ] Engine package builds to ESM + CJS + types. *(N/A — same reason. `packages/protocol` does,
      though — `npm run build` in that package produces ESM+CJS+`.d.ts` via `tsup`, and is now
      the thing `src/commands/registry.ts` depends on at runtime.)*

**Honest status:** work items 1 (EventBus), 2 (broadcast inversion, all 8 sites), and 3
(`processSpawner` mainWindow removal) are **done and verified** — full unit + integration suite
green, zero regressions. Work item 4 (data dir) is now **done**: `setDataRoot()` is the primary
path in `appSettings.ts`/`workspaceFs.ts`, wired from `main.ts`'s `app.whenReady()`; the two
`*Override` test hooks were deliberately kept separate rather than folded in (see the work item 4
status note above for the reasoning). Work item 6 (headless startup) is **half done**:
`src/startup.ts`'s `preflight()` exists, is unit-tested, and is wired into `main.ts` as an
additional, non-blocking diagnostic pass — but the pre-existing git-check block was left as its
own direct call rather than routed through `preflight()`, and the workspace-bootstrap loop (init
dirs/repos, auto-sync, active-workspace validation) has not been extracted out of `main.ts` into
the engine. Work item 5 (split shell-only handlers) is **started**: the 10 pure-CLIENT channels
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
restructuring + dependency split) is **mostly not started**, but is no longer untouched:
`packages/protocol` is now a real linked npm workspace (`"workspaces": ["packages/*"]`,
`"@bifurc/protocol": "*"` as a dependency), built with its own `tsup` pipeline, and consumed by
`src/commands/registry.ts` — the first real inter-package dependency in the programme.
`packages/engine` itself, and the corresponding `src/*` → `packages/engine/src/*` file moves, are
still not started. See the "Cleanup_plan.md status" section of `plan/README.md` for the
still-open D6 sub-items that also block a complete P2.

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
| Dependency split is done lazily, leaking renderer deps into the engine | Medium | Verify `packages/engine/package.json` by inspection at the gate |
| `processSpawner` shutdown becomes non-idempotent, orphaning child processes | Medium | Test SIGTERM twice; assert no orphaned children |
| Moving files breaks git history for the moved modules | Low | Use `git mv` so blame survives |
| Someone wires the shell up early "to test it" | Medium | Hard rule: the shell switch happens in P6 only |
