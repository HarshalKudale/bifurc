# 07 — Phase 6: Shell seam ✅ SHIP MILESTONE

**Goal:** switch the Electron shell from in-process IPC to the RPC client, ship it as a normal release,
and change nothing the user can see.

**Effort:** 2–3 weeks. **Depends on:** P5. **Blocks:** P7, P8, P9, P11, P12.

> **This is the milestone the entire programme exists to reach.** Everything before it is on the
> critical path; everything after it is additive and independently schedulable. Once this ships, the
> shell is disposable and the Tauri migration becomes a low-risk, deferrable project.

---

## Preconditions

- P5 gate green: client satisfies `WindowApi`; key-diff script reports zero differences.
- Conformance suite green on `stdio` and `ws`.
- A green baseline: 35 unit suites, 11 e2e specs, all passing **before** any change in this phase.

---

## The rule for this phase

> **`window.api` stays byte-identical. The renderer is not edited.**

If a renderer edit becomes necessary, stop. The cause is upstream (P1–P5) and belongs there. Editing the
renderer here destroys the regression signal you spent five phases building, and it hides the very
problems this phase is supposed to expose.

**Only three files may change in the shell:** `src/main.ts`, `src/preload.ts`, and the shell-only handler
module. Nothing under `renderer/` — except the two blob-related files already changed in P3.

---

## Work item 1 — Reimplement `preload.ts` as a client bridge

`src/preload.ts` currently makes 136 `ipcRenderer.invoke` calls. It becomes a bridge: the renderer still
calls `window.api.*`, the bridge forwards to the RPC client.

```ts
// src/preload.ts — after
import { createClient } from "@bifurc/client";
import { createIpcTransport } from "./ipcTransport";   // talks to main via ipcRenderer

const client = createClient(createIpcTransport());
const local  = createLocalApi();                        // dialogs, zoom, theme, titlebar, platform

contextBridge.exposeInMainWorld("api", { ...client, ...local });
```

Two transports stacked:
```
renderer  →  (Electron IPC, unchanged)  →  main  →  (RPC)  →  engine
```

**Why keep the Electron IPC hop rather than connecting the renderer directly to the engine?** Because
`contextIsolation: true` and `nodeIntegration: false` are set (`main.ts:170–174`), so the renderer cannot
open a socket. The IPC hop stays. It is also what makes this phase a wiring change rather than a
renderer rewrite.

### Local handlers stay local

Per the P5 shim table, these are implemented in the shell and never cross the RPC boundary:

| Group | Handlers |
|---|---|
| Dialogs | `dialog:openFile`, `dialog:pickFilePath`, `dialog:pickFolderPath` |
| Zoom | `zoom:get`, `zoom:set` (+ the `Ctrl+=/-/0` shortcuts in `main.ts:200`) |
| Theme | `theme:get`, `theme:set` |
| Titlebar | `shell:setTitleBarOverlay` (+ `syncTitleBarOverlay`) |
| External | `shell:openExternal` |
| Launch state | `app:isFirstLaunch`, `app:completeFirstLaunch` |
| Platform | `window.api.platform` |
| CA install | `tls:installCA` (per `File_Ops_Protocol.md` §6.2) |
| Update check | `app:checkUpdate` — shell half |

---

## Work item 2 — Engine lifecycle

```ts
// src/main.ts
async function startEngine(): Promise<EngineHandle>
```

```
1. Resolve the data dir:  app.getPath("userData")   → pass as --data-dir
2. Pick a transport:
     - socket path (D3 = b)  e.g. \\.\pipe\bifurc-<pid>  |  $XDG_RUNTIME_DIR/bifurc-<pid>.sock
     - or ephemeral port     (D3 = a)
3. Spawn the engine binary with --data-dir, --transport, --token
4. Wait for the ready handshake (stdout line, or poll the socket) with a timeout
5. Connect the client, send `hello` with the token
6. Register the client with the IPC bridge
```

### Supervision

Tauri's sidecar does not auto-restart, and neither does Electron. You need this now, not in P11.

| Condition | Behaviour |
|---|---|
| Engine exits non-zero | Restart with exponential backoff (1s, 2s, 4s, capped at 30s) |
| 3 failures within 60s | Stop retrying; show a UI error with the engine's stderr |
| Shell quits | `SIGTERM`, wait up to 5s, then `SIGKILL`. Never leave an orphan. |
| Shell crashes | Engine must self-terminate — pass the shell's PID and have the engine watch it, or use a socket-close watchdog. |

**The orphan case matters.** If the shell crashes and the engine survives, the next launch hits a
port/socket collision and the user sees a broken app with no explanation. The socket-close watchdog is
the robust answer: when the last client disconnects and a grace period expires, the engine exits.

### Single instance

`app.requestSingleInstanceLock()` (`main.ts:239`) already ensures one shell. One shell → one engine.
Keep that invariant; do not let a second engine start.

---

## Work item 3 — Preserve the `ready-to-show` and startup ordering

`main.ts:185` shows the window on `ready-to-show`, then applies persisted zoom and syncs the titlebar
overlay. The engine is now a dependency of a working UI, so decide the ordering explicitly:

**Recommended:** show the window on `ready-to-show` as today, but render a "connecting to engine"
state until the handshake completes. Do **not** block window creation on the engine — a slow or failing
engine would then present as a hung app with no window at all, which is the worst possible failure mode.

---

## Work item 4 — Packaging change for the seam

`package.json` currently bundles only `dist/**/*`:

```json
"files": ["dist/**/*", "package.json"]
```

The engine binary must ship alongside. Add it to `extraResources`:

```json
"extraResources": [
  { "from": "build/tray-icon.png", "to": "tray-icon.png" },
  { "from": "build/icon.png",      "to": "icon.png" },
  { "from": "../packages/engine/dist/engine", "to": "engine" }
]
```

**Note the existing precedent:** `main.ts:36–39` already resolves `extraResources` correctly for the
tray and app icons, with a dev-mode fallback:

```ts
return app.isPackaged
  ? path.join(process.resourcesPath, file)
  : path.join(__dirname, "..", "build", file);
```

Reuse `iconPath()` for the engine binary — same dev-vs-packaged problem, same solution. **Do not
introduce a second path-resolution convention.**

---

## Work item 5 — Update the e2e harness

`e2e/fixtures/electronApp.ts` launches Electron with `LOCALAPPDATA` overridden to a temp dir. The engine
now derives its data dir from `app.getPath("userData")`, which that override already controls — so the
isolation should hold.

**Verify it explicitly.** If the engine resolves its data dir before Electron's `userData` override
applies, every e2e test will share one data dir and produce cross-test contamination that looks like
flakiness. Assert in the fixture that the engine's data dir is inside the temp dir.

The 11 specs must pass **unmodified**. If a spec needs changing to pass, that is a finding — investigate
before editing.

---

## Work item 6 — Rollback plan

Ship this behind a flag:

```ts
// src/main.ts
const useRpc = process.env.BIFURC_ENGINE_RPC !== "0";
```

The old `registerIpcHandlers()` path stays intact for one release. If the seam misbehaves in the field,
users can be told to set `BIFURC_ENGINE_RPC=0` and get the previous behaviour immediately, without a
rollback release.

Delete the flag in the next release, once the seam has survived real usage.

---

## How to start — the first three things

1. **Capture a baseline.** Run the full suite and the e2e suite, and record the exact pass counts. You
   need a known-good number to compare against, and you need it before the first edit.
2. **Build the IPC transport and connect it to the client, but keep `registerIpcHandlers()` running.**
   Route one method (`config:get`) through the RPC path and everything else through the old path. Run the
   suite. If it is green, the bridge works.
3. **Route all methods through the bridge, delete `registerIpcHandlers()`, run everything.** Expect
   failures in the shell-only handlers first — that is the expected shape of the work.

---

## Acceptance criteria

- [ ] **35 unit suites pass.** Same count as baseline.
- [ ] **11 e2e specs pass, unmodified.** Same count as baseline.
- [ ] The renderer is **unchanged** (`git diff --stat renderer/` shows only the P3 blob files).
- [ ] Key-diff script reports `window.api` identical to baseline.
- [ ] App launches, engine spawns, handshake completes, UI is functional.
- [ ] Engine crash → automatic restart with backoff; UI shows an error after 3 failures.
- [ ] Shell quit → engine exits; **no orphaned process** (`ps`/Task Manager verified).
- [ ] Shell killed forcibly → engine self-terminates within the grace period.
- [ ] Two launches → exactly one engine (single-instance preserved).
- [ ] Packaged build (NSIS at minimum) contains the engine binary and runs.
- [ ] Rollback flag verified: `BIFURC_ENGINE_RPC=0` restores the old path.
- [ ] **No user-visible change.** Same UI, same behaviour, same performance.

---

## Rollback

Two layers, deliberately:
1. `BIFURC_ENGINE_RPC=0` — instant, no release needed.
2. `git revert` the phase — the old IPC path is intact until the flag is deleted.

**Do not delete the flag in this phase.** It is the safety net for the highest-risk change in the
programme.

---

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| Someone edits the renderer "just to make it work" | **High** | Hard rule stated at the top of this document; review `git diff --stat renderer/` at the gate |
| e2e data-dir isolation silently breaks → phantom flakiness | **High** | Assert the engine's data dir is inside the fixture's temp dir |
| Orphaned engine after a shell crash | Medium | Socket-close watchdog + explicit acceptance criterion |
| Startup race: UI shows before the handshake, errors are opaque | Medium | Explicit "connecting" state; never block window creation on the engine |
| Shell-only handlers are forgotten and routed over RPC | Medium | The P5 shim table is the checklist; verify each is served locally |
| Perf regression from the added IPC hop | Low | Measure startup time before/after; the hop is one in-process call |
| Engine binary not included in the packaged build | Medium | Reuse `iconPath()`; verify the NSIS artifact explicitly |
