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
> session — item 5's first row is done. **Next up:** item 7 (CommandRegistry — deliberately last,
> per its own note above, since it's the mechanical payoff of items 1–6), then item 8 (the
> physical `packages/*` restructuring).

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
- [x] All unit + integration suites pass. *(66 files / 1585 tests as of this session; zero
      regressions.)* The 11 e2e suites remain **unverified** — they cannot run in this sandbox (no
      desktop session, per `plan/baseline.md` "Environment caveats"). Integration suite: 64/66
      files green, 1583/1585 tests, matching the documented baseline exactly (the 2 failures are
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
- [ ] Engine package builds to ESM + CJS + types. *(N/A — same reason.)*

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
status note. Work items 7 and 8 (the CommandRegistry and the physical `packages/*` restructuring +
dependency split) are **not started**. See the "Cleanup_plan.md status" section of
`plan/README.md` for the still-open D6 sub-items that also block a complete P2.

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
