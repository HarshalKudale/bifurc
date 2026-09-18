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
- A green baseline: **97 unit suites / 2,418 tests**, 11 e2e specs, all passing **before** any change in
  this phase. *(Corrected 2026-09-18 — "35 unit suites" was written before P2–P5 added the engine,
  blob, fileOps and client suites. The e2e figure of 11 was verified: `ls e2e/*.spec.ts | wc -l` = 11 —
  which is 11 spec files / **46 tests** as Playwright counts them.)*

> **The e2e baseline cannot be captured from an agent shell — run it from your own terminal.**
>
> Attempting it produced **46 failed, 0 passed**, every test dying in ~850 ms with
> `electronApplication.firstWindow: Target page, context or browser has been closed`. The cause is that
> Electron cannot start a Chromium GPU process when spawned from the agent shell and **hard-exits**:
>
> ```
> FATAL:content\browser\gpu\gpu_data_manager_impl_private.cc:417]
> GPU process isn't usable. Goodbye.        → exit code 3
> ```
>
> **This is not a defect in Bifurc.** Three controls establish that:
>
> 1. `npm run dev` (`npm run build && electron .` — the same binary, the same `dist/main.js`) launches
>    fine from a normal terminal.
> 2. A **minimal 5-line Electron app** with no Bifurc code fails identically from the agent shell.
> 3. `dangerouslyDisableSandbox: true` changes nothing — the limit is the shell's **process context**,
>    not the sandbox flag.
>
> The machine itself is healthy: two adapters, `AMD Radeon(TM) Graphics` and
> `NVIDIA GeForce RTX 4080 SUPER`, both `Status: OK`.
>
> **So no launch flags are needed — not in dev, not in a packaged build.** The two switches already in
> `main.ts` (`disable-gpu-shader-disk-cache`, `disable-gpu`) predate this and are unrelated hardening.
> Do not add workarounds to `main.ts` or to `e2e/fixtures/electronApp.ts` for this.
>
> Two secondary traps found while probing, both worth keeping:
> - **`timeout <n> ./electron.exe --no-sandbox …` fails with `bad option: --no-sandbox`** — coreutils'
>   `timeout` parses the Chromium switches as its own. Chromium switches must go **after** the app path.
> - **Wrapping the launch in a shell function produces a different, misleading error**
>   (`TypeError: … reading 'commandLine'`, i.e. `require("electron")` returning a path string = Node
>   mode). Run each probe as a **plain sequential command**.
>
> Consequence for the gate: the **unit** baseline is solid and is what step 2 is judged by. The e2e
> criterion (*"11 e2e specs pass, unmodified"*) must be run by the user or on a machine whose shell can
> host a GUI process — record the counts then.

### The TOS gate is the *real* e2e blocker — fixed in the fixture 2026-09-18

Separate from the agent-shell limitation above, the reason the suite fails on a normal machine is
**first-launch Terms of Service**, and it is worth writing down because the fix is not where you would
look for it.

`renderer/App.tsx:23` gates the *entire* shell:

```ts
const [tosAccepted, setTosAccepted] = usePersistedState<boolean>("app:tos-accepted", false);
if (!tosAccepted) return <TermsAcceptanceScreen onAccept={() => setTosAccepted(true)} />;
```

`usePersistedState` is backed by plain **`localStorage`** (`renderer/lib/storage.ts` — `getItem` /
`setItem`, JSON-encoded). So the flag is **per-profile renderer state, not `app.json`**, which means
`writeSampleWorkspace()` cannot reach it however much you add to it. `sampleData.ts` already writes
`hasSeenWelcome: true`, and that is *correct but irrelevant* — it drives `app:isFirstLaunch`, a different
gate. The fixture creates a fresh `--user-data-dir` per test, so `localStorage` starts empty and every
test lands on the TOS screen.

**Fix:** `e2e/fixtures/electronApp.ts`'s `page` fixture now seeds the key with `addInitScript` and then
`reload()`s. The reload is load-bearing — `addInitScript` only applies to the *next* navigation, and the
seed has to be in place before `App.tsx` reads the key.

**Two constraints that shape the fix:**
- It has to live in the **fixture**, not `sampleData.ts`. The specs stay unmodified (work item 5);
  the harness is ours to change.
- It must **not** be solved by editing the renderer (e.g. making the TOS gate read `app.json`). The rule
  at the top of this document forbids renderer edits in P1–P6, and this would be exactly the
  "edit the renderer to make it work" failure the risks table lists as **High** likelihood.

---

## Findings from the code, before the first edit

Four things the plan above does not say, all established by reading the shell and the engine on
2026-09-18. The first two are **blocking** for work item 1 and are not visible from the plan text.

### 1. The three log events never reach the bus — a transport-only P6 goes silently blind ✅ resolved 2026-09-18

> **Resolved.** `wireLogEventsToBus()` now lives in `packages/engine/src/eventBus.ts` and is called
> from **both** `createEngine()`'s `doStart()` and `registerIpcHandlers()`. See
> *Finding 1 — what landed* below for the verification table.

`EngineEvents` (`packages/engine/src/eventBus.ts`) declares `log.entry`, `log.chunk` and `server.error`.
`ENGINE_EVENT_NAMES` lists all three. `BUS_NAME_BY_WIRE_NAME` maps them, `assertBridgeIsTotal()` asserts
them at import time, and the conformance suite has cases for them.

**Nothing ever emits them on the bus.** They travel only through the separate `logEmitter`
EventEmitter (`packages/engine/src/proxy/logEmitter.ts`), and `src/ipc/eventBridge.ts` — the file this
phase deletes — subscribes to `logEmitter` **directly**. `handlers.ts:44–46` says so in a comment:

> `log:entry` / `log:chunk` / `server:error` no longer need a forwarder here — `logEmitter` is already
> Electron-free, so the shell's `eventBridge.ts` subscribes to it directly.

So the moment the renderer's subscriptions move onto the transport, `onLogEntry`, `onLogChunk` and
`onServerError` have nothing to deliver: the capture panel, the request-log panel and the server-error
banner all go dead. Every unit test stays green, because they test the bridge's *mapping*, not its
*traffic*.

**The fix belongs in the engine**, as a `wireLogEventsToBus()` alongside `emitEntityStatus()`'s
precedent, so P7/P8/P9 inherit it rather than each re-solving it. **It must reach the
`registerIpcHandlers()` path, not just `createEngine()`** — the shell uses the module singletons
(`commandRegistry`, `bus`, `logEmitter`) and never calls `createEngine()`, so wiring it into the factory
alone would leave today's only client unserved.

### 2. Electron's IPC loses `err.code`, and the client's retry policy branches on it

A rejection crossing `ipcMain.handle` → `ipcRenderer.invoke` arrives at the renderer as a plain `Error`
with the message flattened into a string. `EngineError.code` does not survive.

That matters because `packages/client/src/retry.ts` decides whether to retry with
`isRetryable(err.code)` — the whole three-term policy. Flatten the code and every failure reads as
unclassified, so the policy either retries everything or nothing.

**The bridge must therefore return a discriminated result** rather than letting the handler reject:

```ts
{ ok: true, value } | { ok: false, error: RpcError }   // RpcError = {code, message, details?}
```

and the renderer side must rebuild the error. `toRpcError` (main side) and `remoteErrorToEngineError`
(renderer side) already exist in `packages/engine/src/transport/types.ts` and are the right halves —
this is the same problem `stdio` and `ws` solved, one hop earlier.

### 3. `TransportKind` has no `"ipc"` member

`TransportKind = "in-process" | "stdio" | "ws" | "socket"`. The renderer-side bridge is a fifth kind.
Nothing switches on the union — the only read is `auth.test.ts:712`'s
`expect(transport.kind).toBe(inner.kind)`, and `session.ts:91` merely stores it — so **adding `"ipc"` is
safe and is a one-line change** to `packages/engine/src/transport/types.ts`. The alternative, labelling
the bridge `"in-process"`, is a lie that will cost someone an afternoon.

### 4. Work item 2 is larger than the phase's framing suggests

The phase reads as a *wiring* change, and work item 1 is. But work item 2 ("spawn the engine binary",
resolve `--data-dir`, pick a socket, handshake, supervise, watchdog the orphan case) replaces the
process model. The plan's own "How to start" step 2 is the smaller, correct first cut: **keep the
engine in-process**, build the IPC bridge over `createInProcessTransport(commandRegistry, {bus})`, and
prove the seam. The process split then becomes a change of transport *behind* a proven seam.

That ordering is also what makes the rollback flag meaningful: `BIFURC_ENGINE_RPC=0` restores
`registerIpcHandlers()` only while both paths exist.

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

## Step 2 — what landed (2026-09-18)

**The seam carries traffic.** `config:get` — and only `config:get` — now goes
`client → ipcTransport → ipcRenderer.invoke("engine:rpc") → rpcBridge → registry.invoke("config.get")`,
while the legacy `config:get` channel stays registered and serving. The other 143 keys are untouched.

| File | Role |
|---|---|
| `packages/engine/src/transport/types.ts` | `TransportKind` gains `"ipc"` |
| `src/ipc/rpcContract.ts` | the channel name + the discriminated frame; imported by **both** halves |
| `src/ipc/rpcBridge.ts` | main half — `createInProcessTransport(commandRegistry, { bus })` behind `ipcMain.handle` |
| `src/ipcTransport.ts` | preload half — `kind: "ipc"`, unwraps the result, rebuilds `EngineError` |
| `src/ipc/handlers.ts` | calls `registerRpcBridge()` last |
| `src/preload.ts` | builds the client; `getConfig` routed |
| `package.json` | the `@bifurc/client` dependency (see below) |

### Two things the plan's own snippet gets wrong

1. **`createClient(createIpcTransport())` does not compile.** `CreateClientOptions.local` is
   **required** — `client/src/local.ts` is explicit that there is no sensible default for a native
   picker or an OS trust store. And the snippet's `{...client, ...local}` is redundant: the client
   *already* delegates all 13 local members to `local` (`client/src/index.ts:481–493`). The correct
   shape is `createClient(createIpcTransport(), { local: { …13 } })`, spread once.

2. **`@bifurc/client` was not linked, so the preload could not have loaded.** `node_modules/@bifurc/`
   held symlinks for `engine` and `protocol` only, and the root `package.json` did not depend on the
   client. `build:main` is `tsc && tsc-alias` — **no bundling** — so the import survives as a bare
   `require("@bifurc/client")` in `dist/preload.js`, and an unresolvable require there means
   `exposeInMainWorld` never runs: `window.api` is `undefined` and *the whole renderer dies*. The
   dependency was added and the link created.

   **`npm install` cannot be used to create the link.** The root `prepare` script runs
   `build:packages`, whose `tsup` cleanup deletes the previous output and trips this environment's
   safe-delete guard (`SAFE_DELETE_BULK_CONFIRM_REQUIRED`). Create the link directly — a Windows
   junction, because Git Bash's `ln -s` silently produces an **empty directory** instead.

### A new finding: `@bifurc/client` has no vitest alias

`vitest.config.ts` aliases `@bifurc/engine/*` to source and nothing else, so a **bare** import of
`@bifurc/client` resolves through `node_modules` to `packages/client/dist/`. The client's own suites
import `../src/…` relatively and therefore do test source; but `src/preload.ts`'s import — and so
`packages/client/tests/surface.test.ts`'s view of it — reads the **built** package. This is the same
silent-staleness class `MEMORY.md` records for `@bifurc/protocol`: **rebuild the client after editing
its source, or the preload is tested against a stale build.** Deliberately not changed here — adding an
alias is a separate decision with its own blast radius.

### What step 2 deliberately does not do

**No events.** `ipcTransport.subscribe()` throws `UNSUPPORTED`, which the transport contract requires
rather than accepting a subscription it can never fire. That is safe only because the client's hub is
**lazy**: nothing calls `subscribe()` until a listener registers, and the preload's seven `on*` methods
still use the legacy channels. `tests/ipc/ipcTransport.test.ts` pins that assumption — if the hub ever
becomes eager, it fails at client-construction time rather than in the field. The **push channel** is
still step 3's problem, not this one's. (Finding 1, which shared this sentence, was pulled forward and
fixed on its own — see below. It was a *correctness* defect that already existed, not a consequence of
this step.)

### How step 2 was verified — and the limit of that verification

| Check | Result |
|---|---|
| `npm run typecheck` | clean — protocol, engine, client |
| root `tsc --noEmit` (covers `src/**`) | clean |
| the two new suites (`rpcBridge`, `ipcTransport`) | **18 passed** |
| full suite | **2,395 passed** / 40 skipped / **1 failed** — the documented `soap.execute` flake, unchanged |
| `dist/preload.js` requires | resolve; the compiled bridge graph loads and `kind === "ipc"` |
| **the exposed surface** | **144 keys**, both-direction diff against `SURFACE_KEYS` clean |

That last row is stronger than it looks. `packages/client/tests/surface.test.ts` imports the **real**
`src/preload.ts` under a mocked Electron and captures the object handed to
`contextBridge.exposeInMainWorld`. So *"the renderer's surface is unchanged"* is asserted against the
actual object rather than against a type — which is the only version of that claim worth having, given
`renderer/types/window.ts` is known to under-declare.

**What none of this proves: that Electron's real IPC round trip works.** `ipcRenderer.invoke` is mocked
on one side and `ipcMain.handle` on the other, so the wire *between* them is the one thing untested
here. That is the e2e suite's job, and it has to be run from a normal terminal (see Preconditions).

---

## Finding 1 — what landed (2026-09-18)

Finding 1 was pulled forward out of step 3 because it is a **pre-existing correctness defect**, not a
consequence of moving the renderer onto the transport: the three log events were declared, mapped and
conformance-tested but never emitted on the bus, so any bus-based subscriber was already blind to them.

| File | Role |
|---|---|
| `packages/engine/src/eventBus.ts` | `wireLogEventsToBus(target?, source?)` — the shared implementation |
| `packages/engine/src/index.ts` | wired in `doStart()` (before `startServer`), detached in `stop()` |
| `src/ipc/handlers.ts` | wired on the shell path, which never calls `createEngine()` |
| `packages/engine/tests/eventBus.logWiring.test.ts` | 9 cases, all about **traffic** rather than mapping |

### Four decisions worth stating

1. **The name mapping is hand-written and cannot be derived.** `logEmitter` emits `"request"` /
   `"chunk"` / `"server-error"`; the bus calls them `"log.entry"` / `"log.chunk"` / `"server.error"`.
   Unlike `transport/types.ts`'s bus↔wire bridge — where the two sets differ by a uniform `event.`
   prefix and the map is therefore checked by `assertBridgeIsTotal()` — `"request"` and `"log.entry"`
   share nothing to strip. The bus names win because they are the ones the transport already knows;
   `logEmitter` is engine-internal and has emitted these three names since before the bus existed.

2. **`log.entry` carries ONE `RequestLogEntry`, not a `{entries: […]}` batch.** Batching is a *wire*
   concern owned by `eventPump.ts`'s `toClientEvent`, which every serialising transport applies. Putting
   the batch shape on the bus would hand every in-process subscriber an envelope no engine code
   produces, and the conformance suite's batch case would be asserting a shape the engine never emits.

3. **Idempotent per bus, via a `WeakSet`, because two callers are expected.** `createEngine()` and
   `registerIpcHandlers()` both wire it. Today only one runs, but a consumer that does both must not
   receive every entry twice — and the symptom would be duplicated rows in the capture panel, which
   reads as a UI bug rather than a wiring one. A module-level boolean instead of a `WeakSet` would have
   wrongly suppressed wiring a *second, distinct* bus. Detaching clears the flag, so `stop()` followed
   by a fresh engine in the same process works; a one-shot flag would silently stop all logging forever.

4. **`source` is injectable, for the reason `createInProcessTransport`'s `bus` is.** `logEmitter` is a
   process-wide singleton and Vitest shares a module registry across every file in a worker, so a test
   using the default would leave three listeners attached for whatever ran next. The last case asserts
   the **default** path by traffic on the real singletons, and detaches in `finally`.

### The call site ordering is load-bearing

`wireLogEventsToBus(bus)` is placed **before** `startServer(settings.port)` in `doStart()`. The proxy is
the only emitter of these three events, and a **bind failure** — the `listen()` error branch — emits
`server.error`. Wiring after the start would make that the one event that got away, i.e. the failure
mode would be "the port is busy and the UI says nothing".

### Additive, not a replacement

`eventBridge.ts` subscribes to the **bus** for six events (`sync.status`, `sync.entityStatus`,
`entity.changed`, `webhook.payload`, `process.output`, `process.statusChange`) but to `logEmitter`
**directly** for these three. So wiring the bus cannot double-deliver to the renderer: the legacy
channels keep coming from `logEmitter`, and the bus now *additionally* carries them for the transport.
Both paths run side by side until step 3 deletes the legacy one — and this call is what makes that
deletion possible instead of silently fatal.

### Verification

| Check | Result |
|---|---|
| the new suite | **9 passed** |
| `npm run typecheck` | clean — protocol, engine, client |
| full suite | **2,404 passed** / 40 skipped / **1 failed** — the same `soap.execute` flake |
| delta against the step-2 baseline (2,395) | **exactly +9**, no other movement |
| `packages/*/dist` entry points | present — engine 720 files, client 32, protocol 6 |
| `wireLogEventsToBus` in the build | present in `dist/index.js` and `dist/eventBus.js` |
| `git diff --stat renderer/` | **empty** |

The two things this still does **not** prove are the same two step 2 left open: that Electron's real
IPC round trip works (both ends are mocked), and that a **bus-based** subscriber now receives the three
events end to end — the wiring is unit-verified, but nothing consumes the bus for them yet. That is
step 3, when `eventBridge.ts` is deleted and the transport becomes the only path.

---

## Acceptance criteria

- [ ] **Unit suites pass.** Baseline after finding 1: **100 files / 2,445 tests** — 2,404 passed, 40
      skipped, and the one documented `soap.execute` flake. (After step 2: 99 / 2,436 — 2,395 passed.
      Before step 2: 97 / 2,418. The "35 unit suites" this criterion used to say predated P2–P5
      entirely.)
- [ ] **11 e2e specs pass, unmodified.** Same count as baseline. **Not yet captured** — requires a
      normal terminal, see Preconditions.
- [ ] The renderer is **unchanged** (`git diff --stat renderer/` shows only the P3 blob files).
- [ ] `window.api` is identical to baseline. The instrument is
      **`packages/client/tests/surface.test.ts`**, which imports the real `src/preload.ts` under a
      mocked Electron and diffs the exposed keys against `SURFACE_KEYS` in both directions. It is a
      test, not the standalone script this criterion used to name.
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
