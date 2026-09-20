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

### 5. Step 3 is not mechanical: 25 of the 93 commands have no registry implementation

Measured 2026-09-18, after step 3a. `plan/07`'s step 3 reads as a mechanical flip — "route all methods
through the bridge, delete `registerIpcHandlers()`" — and warns only to "expect failures in the
shell-only handlers first". The failures are not a handful of shell concerns. They are **25 missing
engine implementations**:

```
app.checkUpdate   audit.list        config.save          healthbar.checkUrl
healthbar.getServices  healthbar.saveServices         proxy.status
request.replay    runner.loadConfig runner.saveConfig   runner.saveReport
script.execute    server.restart    server.start        server.status
server.stop       services.discover webhook.registerActive
webhook.unregisterActive            webhookServer.start  webhookServer.status
webhookServer.stop                  workspace.add        workspace.delete
workspace.rename
```

68 of the 93 are registered and route trivially. The 25 above are served **only** by a shell
`ipcMain.handle` body on their legacy channel, so `registry.invoke("<command>")` answers
`UNKNOWN_COMMAND`. Verified two ways: a test over the real `registerIpcHandlers()` (`tests/ipc/handlers.test.ts`'s
"registry coverage" block) and by grep — `grep '"runner.saveReport"' src/ packages/engine/src/` finds
three references (the protocol table, the scope table, the shell handler) and **zero** registrations.

**Why this matters more than a to-do list.** `@bifurc/client` classifies **all 25** as
`{kind: "transport"}` — a 1:1 mapping onto a protocol command. So `client.serverStatus()` calls
`registry.invoke("server.status")`, which the real shell cannot answer. The client therefore does not
merely lack 25 methods; it **advertises 25 it cannot deliver against the real engine**, and nothing
noticed because P5 tested it against the conformance suite's *stub* registry and step 2 routes only
`config:get`. The surface test cannot see this either — every key is exposed correctly; only the
implementation behind 25 of them is missing.

The one entry the plan and the client disagree about is `app.checkUpdate`: the "Local handlers stay
local" table calls it a shell half, the client classifies it `transport`. One is wrong, and the
decision belongs with whoever implements it.

**Consequences for the plan.**

1. **Step 3 splits.** 3b-1 routes the 68 registered commands (mechanical, and safe while
   `registerIpcHandlers()` still runs). 3b-2 implements the 25 in the engine. Only then can the legacy
   handlers be deleted.
2. **`registerIpcHandlers()` cannot be deleted before 3b-2.** Deleting it while a command has neither a
   registry entry nor a channel would break that method on **both** paths at once, and the surface test
   would still pass — the key stays exposed. This is the phase's one genuinely silent failure mode, so
   the inventory is now a **ratchet** in `tests/ipc/handlers.test.ts`: implementing a command fails the
   test until its name leaves the list, and a new command that lands unregistered fails immediately.
3. **The `plan/07` "Local handlers stay local" table is incomplete.** It lists 13 local members and
   they match `ClientLocal` exactly — but it does not mention that 25 *routable* commands are
   unimplemented, which is the larger half of the work.

**3b-2 is complete.** Five slices landed, **21 of the 25** moved; the other four are `NARROWED` /
`BLOCKED` / `SPLIT` and are not moves at all (§5b). The ratchet went **25 → 19 → 15 → 10 → 5 → 4**.

| Slice | Commands | Module | Ratchet |
| --- | --- | --- | --- |
| 1 | `server.status` / `server.start` / `server.stop` / `server.restart` / `proxy.status` / `services.discover` | `packages/engine/src/proxy/serverCommands.ts` | 25 → 19 |
| 2 | `config.save` + `workspace.add` / `workspace.rename` / `workspace.delete` | `packages/engine/src/store/configCommands.ts` | 19 → 15 |
| 3 | `webhook.registerActive` / `webhook.unregisterActive` / `webhookServer.start` / `webhookServer.status` / `webhookServer.stop` | `packages/engine/src/proxy/webhookCommands.ts` | 15 → 10 |
| 4 | `healthbar.getServices` / `healthbar.saveServices` / `healthbar.checkUrl` / `request.replay` / `script.execute` | `packages/engine/src/miscCommands.ts` | 10 → 5 |
| 5 | `runner.saveReport` | `packages/engine/src/runner/runnerCommands.ts` | 5 → 4 |

Each module is the only registration site for its commands. The engine's own
`transport/auth/scopes.ts` had already assigned **every one of the 21** scopes before the modules
existed — independent evidence, in all five slices, that these were always engine commands.

**Two slices were not literal copies, and both reasons generalise.**

- *Slice 3, `webhookServer.start`.* The shell body read `cfg.webhookPort ?? 9101` and ignored any
  argument, because its channel took none; `WebhookServerStartParams` declares `port` as optional. The
  engine half honours it and falls back to the config chain, so every existing caller — all of which
  pass nothing — gets exactly the old behaviour. Honouring a declared parameter is not a behaviour
  change; ignoring one would leave a schema the implementation contradicts.
- *Slice 4, `healthbar.checkUrl`.* The shell body used `require("https")` / `require("http")`
  **inside** the handler. `require` does not exist in the engine's **ESM** build output (`tsup` emits
  `.mjs` alongside `.cjs`), so a verbatim copy would have thrown at call time on the ESM entry point
  while typechecking and every unit test passed. Both modules are now imported statically, as
  `serverReplay.ts` and `webhookServer.ts` already do. **This is a class to watch for in every
  remaining move**: `require` inside a function body is invisible to `tsc` and to the test runner.

**One gap carried over deliberately.** `RequestReplayResult` declares `durationMs` and `replayRequest`
does not return it, so `request.replay` has never carried a field its own frozen contract advertises.
It is moved as-is, for the same reason `configCommands.ts` left `workspace.add`'s odd failure shape
alone — changing a command's result shape while moving it hides the move behind a contract change. The
renderer does not read it (it times the call itself).

**`runner.saveReport` needed a check that is not obvious.** `RunReport` is a plain `z.object()`, and
zod's `z.object()` **strips unknown keys** — so "the schema accepts the payload" is not the same as
"the schema preserves it". The question was whether parsing loses any field the renderer sends. It does
not: `CollectionRunReport`'s 9 fields and `RunnerRequestResult`'s 11 are all present. Had it stripped,
this would have been a **data-loss bug in saved run history**, not a type error — the exact failure
`RunResult`'s own header describes for P1.

### 3b-1 — the preload flip (2026-09-18)

**`src/preload.ts` is no longer a channel table.** It was 136 hand-written `ipcRenderer.invoke`
calls; it is now `{ ...client }` plus **eight explicit overrides**. **136 of the 144 keys route
through the bridge.**

The reason the table could just be deleted is that `renderer/types/window.ts` was never the contract
— `src/preload.ts` was — and `@bifurc/client` was built in P5 to satisfy that contract exactly,
**positional arguments included** (`loadEntity: (wsId, kind, id) => call("loadEntity", { workspaceId:
wsId, kind, id })`). So the client's methods are drop-in replacements for the old bodies, which is
what P5's `shape.test-d.ts` proves and what makes the flip mechanical rather than a rewrite.

#### The thirteen held back, in two groups that need two different fixes

**Four have no registry implementation** (`§5b`): `checkUpdate` (SPLIT), `listAudit` (NARROWED),
`saveRunnerConfig` / `loadRunnerConfig` (BLOCKED). Routing them converts a working method into an
`UNKNOWN_COMMAND`, so they stay on their channels until the protocol moves. Not a backlog —
`app.checkUpdate` is P12's, and the two `runner.*Config` are blocked by a **passing test**.

#### The two `ClientLocal` hooks, and the five they unlocked

`clientHandlers.ts` gained `client:writeArtifact` (a save dialog plus a write) and the preload now
supplies `readArtifactFile` (a pass-through to `dialog:openFile`, which already returns exactly
`LocalFileContent`). Both are **client** channels — deliberately not protocol commands, because a
remote engine must never choose a path on the user's machine. That is why `@bifurc/client` takes them
as injected hooks at all: P7's browser downloads instead.

They unlocked five of the nine: `exportAudit`, `tlsExportCert`, `shareCaptureJson`, `tlsImportCert`,
`tlsImportKey`.

#### Four artifact methods held back, each for a specific defect in the *client*

Auditing the four that remained turned up three real problems, and **none of them is a shell problem**
— they are gaps in the client's egress path that routing would have exposed as user-visible
regressions:

- **`exportData` would corrupt the binary `workspace-zip`.** `ClientLocal.writeArtifact` takes a
  **decoded string**, and the client base64-decodes into a utf-8 string before handing it over. A ZIP
  routed through the bridge is corrupt on arrival. This needs a binary variant of `writeArtifact` (or
  one that takes base64) before `exportData` can move — a change to `@bifurc/client`, not to the shell.
- **`exportRunnerReport` would drop the HTML choice.** The client hardcodes `format: "json"`, while
  `runnerHandlers.ts` derives the format from the extension the user picked (`.html` is the default).
  Routing it silently removes HTML export.
- **`preflightImport` / `importData` may open a second dialog.** The client opens its *own* picker and
  ignores any path in `req`. That is right for a blob-based client, but it is a second dialog unless
  the renderer never supplied a path. Unverified — it stays on the known-good channel until the e2e
  import spec confirms it.

The general lesson, and the reason this was worth auditing rather than flipping: **"the command is
registered" is not "the client's method is equivalent."** The registry says nothing about what the
client does with the result, and three of these four break *after* the engine has done its job
correctly.

#### Follow-up (2026-09-18): the binary one is fixed, and the other three are one gap, not three

Acting on the three findings turned up two more, both in `artifactToFile` and both affecting the five
egress methods 3b-1 had **already** routed — so this was shipped behaviour, not a prerequisite:

- **`writeArtifact` is now base64 in, bytes out** (`26d2d32`). It took a decoded `string` and the
  shell wrote it back as `"utf-8"`, which corrupts anything that is not text. That was `exportData`'s
  blocker; it is gone.
- **The blob branch was worse than the text branch.** It read `read.content` / `read.base64`, but
  `BlobReadResult` is `{data, eof}` — so it decoded `""` and wrote an **empty file**, with no error.
  It also issued a single un-offset `blob.read`, which caps at `BLOB_READ_CHUNK_BYTES` (512 KB) and
  has no terminator other than `eof`. `BLOB_INLINE_THRESHOLD_BYTES` is 1 MB, so almost every real
  export took that path. Neither was caught because the client's fake transport answers every request
  with `{ok:true, echoed:cmd}` and no test ever reached either branch.

With the binary objection gone, the remaining three collapse into **one** missing primitive: *the
dialog has to run before the command.* `File_Ops_Protocol.md` §3.2 requires it and the reason is not
cosmetic — the shell's `importExport:export` comment says it plainly: fail fast on cancel, so the
engine never renders a 200 MB workspace archive the user then abandons. `ClientLocal.writeArtifact`
bundles "ask where" and "write" into one call, which forces the dialog *after* the render. A
`pickSavePath` hook is what lets `exportData` and `exportRunnerReport` move.

The two imports need a second, separate primitive, and the earlier "may open a second dialog" is now
**confirmed and worse than that**. `ImportExportModal.tsx` calls preflight with no path, takes
`res.filePath` from the result, and feeds that same path back into `importData`:

1. The client's `preflightImport` returns `import.preflight`'s result verbatim, and the engine only
   ever knew a `blobId` — so `res.filePath` is `undefined`, and the collision branch hands
   `undefined` to `applyImport`.
2. The client's `importData` **ignores** `req.filePath` and calls `uploadLocalFile()`, which opens a
   dialog — a second one, for a file the user already picked.

So: `readArtifactFile` needs an optional path (reuse, don't re-ask), and `preflightImport` needs to
return the path it chose. Neither is a shell change.

#### Two preconditions that were checked rather than assumed

1. **`config.save` no longer calls `updateTrayMenu()`.** The registry handler emits
   `settings.changed` on the bus instead. That is only equivalent because `src/main.ts:299`
   subscribes — `bus.onTyped("settings.changed", () => updateTrayMenu())`. Without that
   subscription, routing `saveConfig` would have silently stopped updating the tray.
2. **`event.log.entry` arrives as a batch; the renderer expects one entry.** `eventPump.ts` coalesces
   under the same wire name, so a pass-through would hand the capture panel `{entries}` at exactly
   the moment there was most traffic to show. `onLogEntry` is the one subscription the client does
   **not** pass through — it unwraps the batch and calls back once per entry, matching the legacy
   `log:entry` channel.

#### What this does and does not prove

The suite is **unchanged** — 2,430 passed / 40 skipped / 1 failed (the `soap.execute` flake), byte
for byte the pre-flip figure — and the surface test still reports **144 keys**. But that is weaker
evidence here than it was for 3b-2: the unit suite tests *handlers*, not the preload, and the
preload's only coverage is the key count. A wrong argument mapping would pass everything and break
the app. **The e2e suite is the real gate for this step and it cannot be run from the agent shell.**

The one piece of genuinely new shell code here — `client:writeArtifact` — does have its own tests
(`tests/ipc/clientHandlers.test.ts`), because it is new rather than moved: it asserts the bytes and
path handed to `fs`, that the picker is pre-filled from the engine's suggested name, and that *cancel*
stays distinguishable from *failure*.

### 5b. The remaining 15 are not one category — audited, not assumed

The first version of the ratchet said every remaining command was "engine work that has not been
moved yet". **Auditing all 19 against their frozen schemas showed that is false.** `runnerHandlers.ts`
had already recorded the reason for three of them, and the audit generalised it. The entries split by
*why* they are unimplemented, because the four reasons need four different responses:

| Category | Count | Meaning | Response |
| --- | --- | --- | --- |
| `MOVABLE` | **0 — all moved** | The schema accepts every payload the handler accepts. | Move it. ✅ **done.** |
| `NARROWED` | 1 | The schema accepts a **strict subset**, and `.strict()` makes the difference a `BAD_REQUEST`. | A decision, not a move. |
| `BLOCKED` | 2 | The schema rejects payloads a **currently passing test** uses. | Cannot move until the frozen protocol changes. |
| `SPLIT` | 1 | The protocol declares an engine-half contract that **deliberately differs** from the shell implementation. | Not a move at all. |

**`MOVABLE` is 0 as of 2026-09-18** — all 21 have a registry implementation, so the ratchet's movable
half is deleted from `tests/ipc/handlers.test.ts` rather than left empty. That deletion is deliberate
signalling: an empty `MOVABLE` array invites the next reader to file the next unimplemented command
there by default, whereas deleting it forces the classification to be argued again. What remains is
one `NARROWED`, two `BLOCKED` and one `SPLIT`, each pinned with its own reason.

**`NARROWED` — `audit.list`.** `AuditListParams` has no `filePath` / `fromTs` / `toTs`, but
`QueryLogOptions`, the type the handler actually takes, does. So a caller passing `filePath` gets
`BAD_REQUEST` through the registry where the legacy channel answered it. The honest caveat: nothing
currently passes those — `listAudit` is on `window.api` and no renderer code calls it — so the
narrowing is **latent, not live**. That is why it is neither `MOVABLE` (moving it silently reduces the
command's accepted params) nor `BLOCKED` (nothing is actually broken). P9's whole point is *new*
clients, and a remote client is exactly the caller that would find the missing fields.

**`BLOCKED` — `runner.saveConfig` / `runner.loadConfig`.** Blocked by a **passing test**, which is the
strongest evidence available: `tests/integration/runnerStorage.integration.test.ts:207` saves
`{delayMs: 250, stopOnFailure: true, iterations: 3}` and line 210 asserts an exact round-trip — while
`RunnerConfigSchema` **requires** `requestOrder` and `RunnerLoadConfigResult` *declares* that fixed
shape as what a load returns. So neither the request nor the response can satisfy the frozen schema.
The ratchet now asserts this block directly (parsing the real config through the schema must fail), so
if anyone later widens `RunnerConfigSchema` the test fails and says these two have become movable —
rather than leaving them filed as "unimplemented" forever.

**`SPLIT` — `app.checkUpdate`.** The disagreement this plan recorded ("shell half" per the local-handlers
table, `transport` per the client) is **resolved by the protocol's own comment**: `misc.ts` marks it
SPLIT, with the schema as "the interim engine-half contract" and a note that **P12 moves it fully
client-side**. Neither side was simply wrong — the GitHub fetch is engine-safe outbound network, while
`app.getVersion()` and the `process.platform` asset match are client-local. The concrete consequence is
that `AppCheckUpdateResult` has **no `currentVersion` and no `hasUpdate`** — the two fields the shell
handler's success branch returns — so the engine half cannot be produced by moving that body. It needs
the split written.

**The corollary worth carrying forward:** "unimplemented in the registry" is not a synonym for "engine
work outstanding". Three of the 25 were blocked by the frozen protocol and one was a split, so step 3
cannot end with all 93 registered — it ends with **89 registered + 2 blocked + 1 narrowed + 1 split**,
and the ratchet is what keeps that honest.

### 5c. The mirror problem: five engine commands are registered *from the shell*

**Status: the pass is COMPLETE (2026-09-19), and guarded since 2026-09-20.** It turned out to be **51**
commands, not five, moved across seven slices (`§5c-i` … `§5c-vii`). **Zero module-scope
`commandRegistry.register(` calls remain in `src/`** — and a static test now fails if one reappears
(`§5c-viii`), so the class cannot come back silently. The five below are the ones this section
originally named; the count was the first thing the pass corrected.

Found while writing `store/configCommands.ts`, and deliberately **not** fixed there.

`packages/protocol/src/commands/config.ts` declares `config.get`, `env.setActive` and
`workspace.setActive`; `entity.ts` adds `entity.load` and `entity.setEnabled`. All five are registered
— but at **module scope in `src/ipc/handlers/coreHandlers.ts`**, i.e. in the *shell*, not in
`packages/engine`.

That is finding 5 on the other side of the seam. On a containerised engine (P9),
`createEngine()` + a transport would have **none** of the five registered, so `config.get` would answer
`UNKNOWN_COMMAND` — the same failure, reached from the opposite direction. They are invisible to the
current ratchet because it only asks whether a command is registered *in this process*, and in this
process the shell has registered them.

Not fixed here because it is a different change with a different blast radius: P6 is about commands the
registry cannot serve at all, and moving an already-registered command between packages can break
callers that currently work. It needs its own pass — and the ratchet will not catch it, so it should be
written down rather than rediscovered.

#### 5c-i. Slice 1 — the five §5c commands moved into the engine (2026-09-19)

**§5c's count is wrong: the number is 51, not 5.** A grep for `^commandRegistry.register(` across
`src/` finds **51** module-scope registrations in nine shell files — `syncHandlers.ts` 17,
`applicationHandlers.ts` 10, `coreHandlers.ts` 5, `grpcHandlers.ts` 5, `entityCrudFactory.ts` 4,
`folderHandlers.ts` 4, `graphqlHandlers.ts` 2, `soapHandlers.ts` 2, `runnerHandlers.ts` 2. §5c named
the five in `coreHandlers.ts` because that is the file `configCommands.ts` was written next to; the
other 46 are the same defect, and they are the remaining slices rather than a footnote.

Slice 1 moves the five §5c named — `config.get`, `env.setActive`, `workspace.setActive`,
`entity.load`, `entity.setEnabled` — into `packages/engine/src/store/coreCommands.ts` behind
`registerCoreCommands(registry)`, called from `src/ipc/handlers.ts` beside the other
`register*Commands` modules. Every body moved **verbatim**, and `coreHandlers.ts`'s channel adapters
were already thin `commandRegistry.invoke()` delegations, so the wire behaviour is the same code
reached through a different door. **46 module-scope registrations remain.**

**One dependency had to travel, and it was triplicated.** `syncEnabledSet` existed three times —
`handlers/utils.ts`, `entityCrudFactory.ts`, and a private copy in the engine's
`transport/legacyCompanion.ts` — all semantically identical (the engine's `??` and the shell's
`if (!set)` are the same thing, because `readEnabledSet()` returns `Set<string> | null` and an empty
`Set` is truthy). It now lives in `packages/engine/src/store/enabledSet.ts`. This function decides
whether a mock, mapping or rule is **served by the proxy**, and the enabled-set is written to
`enabled.json` rather than held in memory — so two copies disagreeing by one branch would mean an
entity behaving differently depending on which path toggled it, *persistently*. `handlers/utils.ts`
became dead (zero importers) but is **deliberately still on disk**: `git rm` correlated 2-for-2 with
the whole `src/` tree vanishing from the working tree (see `MEMORY.md`), and an unimported file is
harmless.

**§5a's trap now applies to these five.** §5a records that re-pointing legacy bodies broke
`protocolPoc` and `settingsMutations` because they register handler *groups* without
`registerIpcHandlers()`. Moving the five has the same shape: importing `coreHandlers.ts` no longer
populates them, so **five suites now call `registerCoreCommands(commandRegistry)` themselves** —
`entityConflictScope` (which had relied on a bare side-effect `import "@/ipc/handlers/coreHandlers"`),
`foldersAndWorkspaces`, `protocolExecution`, `settingsMutations` and `protocolPoc`. That is the cost
of an explicit call site, and it is the point: a module side effect cannot be seen by a ratchet, an
explicit call can. None of the five calls `registerIpcHandlers()`, so none of them double-registers —
checked, because `register()` throws on a duplicate.

**Verification — and the honest gap.** The vitest suite **could not be run**. All 101 files failed at
collection with the signature `TESTING.md` §8 already records — `TypeError: Cannot read properties of
undefined (reading 'config')` / *"Vitest failed to find the runner"* — reproduced by an **untouched**
file (`packages/protocol/src/commands/index.test.ts`), so nothing here is implicated. Re-ruled-out
this time: a cold `node_modules/.vite`, `--pool=threads`, `--pool=forks`, `--no-isolate`,
`NODE_OPTIONS` unset, and orphaned workers (there are none). `--pool=vmThreads` "passes" and is **not**
evidence — §8 explains that it silently does not apply `vi.mock`. The mechanism is legible in
`@vitest/runner`: `createDefaultSuite()` reads `runner.config.sequence` off a `runner` that was never
installed, i.e. the test files are seeing a **second instance** of the runner's module state. What was
verified instead:

- `npm run typecheck` → **EXIT=0** (all three packages and the root shell).
- A standalone probe against the **built** engine, no vitest, **7/7**: importing the engine registers
  nothing (the §5c property); all five are `UNKNOWN_COMMAND` before registration and registered after;
  an invalid payload yields `BAD_REQUEST` rather than `UNKNOWN_COMMAND` — which proves *registered and
  schema-checked* while touching no filesystem; a valid `entity.load` resolves `{ok:false}`; a second
  `registerCoreCommands()` throws `already registered`, so a future double call fails loudly; and
  `env.setActive("__global__")` / `workspace.setActive(<unknown>)` keep their documented `{ok:false}`
  returns.
- Static: `coreHandlers.ts` has zero `commandRegistry.register(` calls, `handlers.ts:99` makes the
  explicit call, and the remaining count is **46 = 51 − 5**.

**Slice 1 is implemented, not verified.** The suite must be re-run before it is called done. No test
count change is expected — the five suites were already reaching these commands — so any delta is a
signal rather than noise. Also worth knowing: the engine's `.mjs` output is **not loadable by plain
Node**, because `tsup` is configured `bundle: false` and emits extensionless relative specifiers
(`import "./config"`); the probe had to use the CJS build. That is the path the Electron shell takes,
so it is not a live defect — but any future child process that imports the ESM entry will hit
`ERR_MODULE_NOT_FOUND`, which is worth checking before P9 builds one.

#### 5c-ii. Slice 2 — the `runnerHandlers` pair (2026-09-19)

`runner.getHistory` and `runner.listFolderIds` moved from module scope in
`src/ipc/handlers/runnerHandlers.ts` into `packages/engine/src/runner/runnerCommands.ts`, which already
held `runner.saveReport` / `saveConfig` / `loadConfig` — so `registerRunnerCommands()` now owns the whole
`runner.*` namespace. **44 module-scope registrations remain.**

**This slice was free, and the reason is the one worth carrying forward.** `runnerHandlers.ts`'s own
comment called these two "the reason they cannot move into `packages/engine`", because §5c's trap is
real: a module-scope registration takes effect on *import*, so relocating one behind an explicit
`register*Commands()` call silently unregisters it for every suite that imports the handler group
without running `registerIpcHandlers()` — and the failure is a `No handler registered` at *invocation*
time, invisible to the compiler. Here the affected set was **empty**:
`tests/integration/runnerStorage.integration.test.ts` is the only file that imports `runnerHandlers`,
and it already called `registerRunnerCommands(commandRegistry)` — added in step 3d for the config pair.
So slice 2 needed no suite edits at all, where slice 1 needed five. **Check the importer set before
assuming a slice is mechanical; that check is what distinguishes the two.**

Worth noting too that these two channels *delegate* (`commandRegistry.invoke(...)`) where
`runner:saveReport` / `runner:saveConfig` / `runner:loadConfig` are self-contained duplicates. A
delegating adapter is the one that breaks when the registry is empty — which is exactly why this slice
carried a suite risk that the step-3d config move did not.

**Verified** the same way as slice 1, and more strongly. `npm run typecheck` → EXIT=0, plus a standalone
probe against the built engine that now drives the moved bodies against a **throwaway data root** via
`setDataRoot()` — the documented P8/P9 entry hook — so nothing touched the real profile and the
containerised-engine story was exercised end to end (set a data root → register → invoke). **14/14**,
including the behaviour the shell bodies had: `listFolderIds` returns only folders carrying
`runner.json` and ignores a stray file in `.runs`; `getHistory` sorts newest-first, **drops a corrupt
`report.json` without failing the folder**, and maps `totalTests`/`passedTests`/`failedTests` into the
summary; history is readable for a folder that has no `runner.json` (history ≠ configured); a missing
folder or workspace resolves `[]` rather than throwing; `loadConfig` returns the object for a configured
folder and `null` otherwise — the contract `listFolderIds` depends on. The suite still could not run
(see §5c-i), so this remains **implemented, not suite-verified**.

#### 5c-iii. Slice 3 — the four `folder.*` commands, and the suite comes back (2026-09-19)

`folder.add` / `rename` / `move` / `delete` moved from module scope in
`src/ipc/handlers/folderHandlers.ts` into `packages/engine/src/store/folderCommands.ts` behind
`registerFolderCommands()`. **40 module-scope registrations remain.** Bodies verbatim, and the
now-redundant local `FolderKind` union went with them — `packages/protocol/src/commands/folder.ts`'s
`FolderKindValue` is the same 11-value enum, so the channel parameter is typed as the protocol's
directly and the `as FolderKindValue` casts disappear. That is what P2 work item 7 meant by a "clean
1:1", and it is the only slice so far where a type could be *deleted* rather than moved.

**The channels delegate, so this slice carried the risk slice 2 did not.** `folderHandlers.ts`'s four
`ipcMain.handle` bodies are thin `commandRegistry.invoke()` delegations. A delegating channel is only
ever as alive as the registry behind it, so a suite that registers this handler group by hand *must*
also call `registerFolderCommands()` — otherwise the channel exists with nothing behind it and fails
as `No handler registered` at **invocation** time, which no compiler can see. Two suites register it by
hand, so two needed the call (`foldersAndWorkspaces`, `protocolPoc`); both already had the pattern from
slice 1. This is the same check slice 2 was decided by — **read the importer set first** — and here it
came back non-empty.

**The file is a two-store invariant, and that is why it is documented rather than merely moved.** Every
one of the four has to keep `AppConfig`'s folder array (`FOLDER_CONFIG[kind].configKey` — what the
renderer draws the sidebar from) in step with `index.json` in the workspace
(`readIndex`/`writeIndex` — folders *and* the manual drag `order`, what the on-disk tree is derived
from). `fsKind` is not derivable from `kind` (`ws` → `sockets`, `rule` → `rules`), so the table is
copied rather than computed. Two behaviours are preserved deliberately: `folder.rename` and
`folder.move` resolve `{ok: true}` **even when the id matches no folder** (the `if (f)` guards skip the
work without telling the caller; the renderer's declared result is `{ok: boolean}`, so reporting a
failure would be a contract change smuggled in under a move), and `rename`'s `renameSync` → `cpSync` +
`rmSync` fallback stays verbatim, because on Windows a rename can fail across a lock.

**The suite came back, and it immediately found a real defect in slice 1.** Slice 1 was committed
during the outage and had never been suite-verified; on the first green run, 29 of
`settingsMutations.integration.test.ts`'s 30 tests died with `Command "config.get" is already
registered.` Slice 1 had added `registerCoreCommands(commandRegistry)` to that suite's **`beforeEach`**
— and the registry is a module-level singleton with no `clear()`, while `register()` throws on a
duplicate *on purpose* to catch double-registration. So it passes the first test and fails every one
after. The channel layer is the opposite (the electron mock's `handlers` map is keyed by channel, so a
second `ipcMain.handle` just overwrites), which is what makes the mistake look harmless. Fixed in
`0160188` by moving the call to `beforeAll`, which is what the other four suites slice 1 touched
already did. **This is the concrete cost of shipping a slice unverified, and the reason the step-3e
entries below say "verified" or "not" explicitly.**

**Measured, and it is the cleanest verification in the phase:** `npm run typecheck` → EXIT=0; full
suite **101 files / 2,508 tests — 2,467 passed, 40 skipped, 1 failed** (the documented environmental
`soap.execute`). That is the recorded step-3d baseline **exactly**, so slices 1, 2 and 3 together
produce **no test-count delta** — which is the evidence that three slices of "move the body, keep the
channel" were behaviour-preserving. A delta here would have been a signal, not noise.

#### 5c-iv. Slice 4 — the nine protocol-client commands (2026-09-19)

`graphql.introspect` / `graphql.execute`, `soap.fetchWsdl` / `soap.execute` and the five `grpc.*`
stubs moved out of the three `src/ipc/handlers/{graphql,soap,grpc}Handlers.ts` module scopes into
`packages/engine/src/protocols/` — three new files, one `register*Commands()` each, all called from
`src/ipc/handlers.ts`. **31 module-scope registrations remain** (`syncHandlers` 17,
`applicationHandlers` 10, `entityCrudFactory` 4).

The three files were grouped into one slice because they are one theme — the protocol request editors'
Send buttons — and because the importer set is nearly identical: `protocolExecution` imports all three,
`protocolPoc` imports two. Two suites to edit, both already `beforeAll` + dynamic-import from slices
1 and 3. The shell-side CRUD collapse stays where it is in all three files: `add*`/`update*`/`delete*`
route through `registerEntityCrudHandlers()` and the `add*`/`delete*`/`list*` trio through
`registerSimpleEntityHandlers()`, and §5c's `^commandRegistry.register(` grep never counted those.

**Two of the nine needed more than a cut-and-paste, and the hazard is one the compiler cannot see.**
`graphql.*` and `soap.*` reached for their transport with `require("https")` / `require("http")`
*inside the handler body*. That is fine in the shell, which is CJS — but `require` **does not exist in
this package's ESM build output**, so a verbatim copy would have compiled, typechecked and passed every
unit test, then thrown at *call* time in `dist/*.mjs`. This is exactly the defect `healthbar.checkUrl`
was fixed for in the 3b-2 misc slice, and the same fix applies: static builtin imports. **Checked at
the artifact rather than the source** — `grep -c 'require(' packages/engine/dist/protocols/*.mjs` is
**0**, which is a check the source cannot give you, because the source is what lies.

**This slice is verifiable in a way slice 2's was not, and the reason is the delegating channel.**
`runnerHandlers.ts`'s three self-contained duplicates meant `runnerStorage.integration.test.ts` would
have passed with the engine half wrong. Here every channel is a thin `commandRegistry.invoke()`, so a
channel-driven test **is** a registry-driven test. `protocolExecution.integration.test.ts` drives all
nine by channel against real local HTTP servers (GraphQL, SOAP, a WSDL GET) — 44/44 — and
**mutation-checked**: commenting out `registerGrpcCommands(commandRegistry)` fails 4 tests with
`No handler registered for command "grpc.execute"`. So the green is not vacuous; the suite really is
reaching the registry and not a legacy duplicate. The general rule, now with evidence on both sides:
*a delegating channel makes a move riskier (it can silently unregister) and simultaneously makes it
testable (the channel test reaches the implementation).*

Behaviours preserved deliberately, each documented in its module. `graphql.introspect` and
`soap.fetchWsdl` **catch** and resolve `{ok:false, error}`, while `graphql.execute` and `soap.execute`
**throw** — and because the transport hands a handler's value back *uninspected*, a resolved
`{ok:false}` is an envelope **success** carrying a failed result, while a throw becomes a
transport-level error. That asymmetry is also the whole explanation for the phase's one standing
failure: `soap.execute` against a dead endpoint rejects, surfacing as `[INTERNAL] connect
ECONNREFUSED 127.0.0.1:1`, whereas the sandbox the test was written in answered `127.0.0.1:1` with a
synthetic 404 — so the request *succeeded* and the body carried a numeric `status`, which is what
`tests/spike/protocolPoc.test.ts` asserts. Recorded in `plan/baseline.md` "Environment caveats"; not
"fixed", because changing which of the two shapes `soap.execute` uses is a contract change. In the
gRPC module: `grpc.mockServerStatus` resolves `{running:false, port:9102}` with **no `ok` field at
all** and `9102` a hardcoded placeholder, and `grpc.stopMockServer` reports success for a server that
was never started while `grpc.startMockServer` reports failure — asymmetric, and kept, because the
renderer's Stop button depends on not being told it failed. All five gRPC bodies are deliberate stubs
(`Cleanup_plan.md` §1.5 D6 disposition: wontfix), so moving them is not an invitation to implement
them.

**Measured:** `npm run typecheck` → EXIT=0; full suite **101 files / 2,508 tests — 2,467 passed, 40
skipped, 1 failed**, again the exact baseline. **31 remain.**

#### 5c-v. Slice 5 — the ten `application.*` commands, and the ratchet turns out to guard the *move* (2026-09-19)

All ten (`list`, `save`, `delete`, `start`, `stop`, `getState`, `getAllStates`, `getLogs`, `checkPort`,
`killPort`) moved from module scope in `src/ipc/applicationHandlers.ts` into
`packages/engine/src/applications/applicationCommands.ts` behind `registerApplicationCommands()`.
**21 module-scope registrations remain** — `syncHandlers` 17 and `entityCrudFactory` 4.

**The cleanest slice of the pass, because nothing had to travel.** Every dependency was already in
`packages/engine`: `store/workspaceFs`'s `readAllEntities` / `writeEntity` / `deleteEntityFile`,
`applications/processSpawner`, `commandGenerator`'s `generateResolvedCommand`, and `portUtils`'s
`checkPortInUse` / `killProcessOnPort`. The shell file imported them all from `@bifurc/engine/...` and
added nothing but an `ipcMain.handle` per command. **A file whose entire body is "import from the
engine + register + delegate" is §5c in its purest form** — the shell never had a reason to own it, so
the move is a relocation rather than a decision, and there is no shell behaviour to lose. That makes it
the natural contrast with slices 3 and 4, where the *body* was fine but a dependency (a two-store
invariant, a `require`) had to be dealt with first. **The heuristic that predicts a slice's difficulty
is therefore not the command count but how much of the body is the shell's own.**

One dependency is a **singleton**, and importing it is the point: `applications/processSpawner.ts`
exports a single `ProcessSpawner` instance, so the process started through this module is the same one
`stop()` reaches. That is exactly what `tsup`'s `bundle: false` protects — bundling would inline a
private copy of the module into every entry point, and a spawned child would become unreachable, with
no error to say so. `application.save`'s **mutation of the object it was handed** (assigning `id` and
`createdAt` when absent) is also preserved and documented: an in-process `registry.invoke()` caller
sees its own object mutated, which the integration suite does, even though an IPC caller only ever sees
a structured clone. And `application.delete` stops the process **before** deleting the file, because on
Windows a live child can hold it open.

`generateId` collapsed on the way in — the file's private copy was **byte-identical** to the exported
one in `store/config.ts`, which `store/folderCommands.ts` already imports. Worth noting *why* this is
hygiene rather than a fix, in contrast with slice 1: `syncEnabledSet` mattered because a divergent copy
would persist disagreement to `enabled.json`, whereas `generateId` is **pure** — a divergent copy could
only produce a differently-shaped id. **Collapse a duplicated helper when it is byte-identical and
already exported; escalate to a correctness concern only when it holds or writes state that outlives
the process.** A further private copy survives in `store/appSettings.ts`; left alone, because
collapsing it is not part of a §5c move.

**The ratchet caught a mistake in this slice — which corrects how §5c describes it.** §5c and §5c-i
both say the ratchet "cannot see" the shell-registered commands, and that is true of the *defect*: a
command registered at module scope in the shell looks registered in this process, so the ratchet is
silent. But the first attempt at this slice added the import to `src/ipc/handlers.ts` and lost the call
site (a tooling race — two edits to the same file in one message, where the second write clobbers the
first), and `tests/ipc/handlers.test.ts`'s *"implements every protocol command except the pinned list"*
failed immediately with all ten `application.*` names in the **missing** list. So the ratchet is blind
to the defect and **sharp on the move**: once a command's only registration is an explicit call,
forgetting the call makes it unregistered, and the ratchet says so. That is the same asymmetry §5c-i
noted from the other side — an explicit call site is checkable and a module side effect is not — and it
means the pass has a safety net after all, just not the one §5c assumed.

**Measured:** `npm run typecheck` → EXIT=0; `applications.integration.test.ts` **12/12**, including
real child-process spawns with log streaming; `tests/ipc/handlers.test.ts` **166/166**; full suite
**101 files / 2,508 tests — 2,467 passed, 40 skipped, 1 failed** (the documented environmental
`soap.execute`), the exact baseline again. **Mutation-checked:** commenting out
`registerApplicationCommands()` fails 11 of the 12 suite tests with `No handler registered for command
"application.save"`; the twelfth only asserts the channel map, so it correctly still passes — which is
also a reminder that a "registers every channel" test proves nothing about delivery. **21 remain.**

#### 5c-vi. Slice 6 — the 17 `sync.*` / `git.*` / `history.*` commands, and the self-sufficient suite (2026-09-19, `4c6d8ab`)

The largest single slice by count: all 17 moved from module scope in `src/ipc/handlers/syncHandlers.ts`
into `packages/engine/src/sync/syncCommands.ts` behind `registerSyncCommands()`. They are `sync.*` (7),
`git.*` (4), `entity.publish` / `folder.publish` / `entity.restore` (3), `audit.diff` (1) and
`history.*` (2). **4 module-scope registrations remain** — `entityCrudFactory`'s four `entity.*`.

The shell file keeps **19** channels: the 17 plus `audit:list` and `audit:export`, both of which were
already delegating and neither of which is part of §5c. `audit:list` is the case §5a is about — its
body still calls `queryLog` directly rather than through the registry, and per §5a it is **not**
re-pointed, because the `register*Commands()` modules run from `registerIpcHandlers()` and a
delegating legacy body would acquire a hidden ordering dependency. `audit:export` is the opposite and
worth noting as the *legitimate* shape: the channel owns a `dialog.showSaveDialog` (a genuinely
shell-side concern) and delegates the artifact through `call("audit.export", …)`, which resolves to an
engine registration in `fileOps/commands.ts`. **The distinction is not "delegates vs. duplicates" but
"who owns the part the engine cannot have."**

**Nothing had to travel** — the second slice in a row where every dependency was already in
`packages/engine` (`sync/syncManager`, `sync/autoSync`, `sync/gitOps`, `sync/statusTracker`,
`store/config`, `store/gitStore`, `proxy/server`, `eventBus`). That is the §5c shape in its purest
form, as in slice 5.

**One helper that looks collapsible and must not be.** `folder.publish` declares a local
`sanitizeDirName`, and `store/workspaceFs` exports a function of that exact name which
`store/folderCommands.ts` already imports — so collapsing them is the obvious tidy-up. They are not
the same function:

| | `store/workspaceFs.sanitizeDirName` | the local one in `folder.publish` |
| --- | --- | --- |
| pattern | `[<>:"/\|?*\x00-\x1f]` | `[<>:"/\|?*]+` |
| replacement | `""` — stripped | `"-"` |
| fallback | `\|\| "unnamed"` | none |

`"a:b"` becomes `"ab"` under one and `"a-b"` under the other, and a name made only of illegal
characters becomes `"unnamed"` versus `"-"`. Collapsing would have silently changed the git path a
folder publish writes to, for exactly the names a user is most likely to have typed by accident. **Kept
local and verbatim.** The generalisable rule: **grep the helper's *body*, not its name** — a
same-named function in the same package is not evidence of sameness, and here the two are used for
different things anyway (a filesystem directory versus a git pathspec prefix).

**Three bodies that look like one, deliberately left as three.** `git.sync`, `entity.publish` and
`folder.publish` share a shape — `syncChanges` → `emitEntityStatus` → a best-effort
`updateLastKnownHead`. They are not identical (only `folder.publish` builds a pathspec prefix), and
they are kept as three registrations rather than factored into one helper. **Factoring is a
behaviour-affecting refactor and this is a move**; the same reasoning keeps `git.discard` and
`entity.restore` as two commands with one body, since the shell's own comment calls the `entity.*` /
`folder.*` names "backward-compatible … delegating to generic git ops" and both names are in the
contract the renderer already calls. The silent `catch { }` around `updateLastKnownHead` is preserved
too — it is unbound and unlogged on purpose, because reading the remote head is bookkeeping and must
not turn a publish that already happened into a failed one.

**Three load-bearing conditionals, each of which a "clean-up" would flatten:**
`sync.setRemote` reloads the config only when `result.ok`; `sync.pull` reloads, invalidates the status
cache and emits an entity status only when `result.ok && result.updated` (a pull that changed nothing
must not churn the renderer's entity list, and the `updated` half is what makes that true); and
`sync.setAutoSync` starts or stops the poller from the **requested** `enabled`, not from `result.ok`,
so the toggle leaves the poller matching what the user asked for even if persisting the setting
failed. `audit.diff` resolves `{before: null, after: null}` when the commit changed nothing — a
resolved value, so an envelope **success**, not a throw. `history.list` defaults `limit ?? 100` /
`offset ?? 0` and falls back to the active workspace; `history.diff` does neither. That asymmetry is
the shell's and was kept.

**The mutation check is the most informative one of the pass, because it separates three audiences.**
Commenting out `registerSyncCommands()` produced two distinct failures and one non-failure:

- The ratchet (*"implements every protocol command except the pinned list"*) failed with **exactly the
  17** names in its missing list — the cleanest possible proof that the call site is the only
  registration, since the received array *is* the slice's command list.
- Two `history:list` tests failed with `No handler registered for command "history.list"`, thrown
  through the delegating channel at `syncHandlers.ts:111`. This is §5a's failure mode observed
  directly: a delegating channel is only ever as alive as the registry behind it, and the error
  arrives at **invocation** time, invisible to the compiler.
- **`gitRemote.integration.test.ts` stayed 24/24 green.** It calls `registerSyncCommands()` itself in
  `beforeAll`, so it is *self-sufficient* — which is exactly why it cannot detect a missing call site.
  Before the move this suite was populated by an import side effect (it imports `registerSyncHandlers`
  statically), so the explicit call reproduces a property it never asserted.

That third bullet is the sharpest thing this slice establishes: **a suite that registers the engine
half itself is structurally blind to the wiring, and its green is not evidence about the wiring.**
Slices 3–5 each added such a call to keep suites working; this is the first time the consequence was
measured rather than inferred.

**Measured:** `npm run typecheck` → EXIT=0; `gitRemote` + `gitSync` + `tests/ipc/handlers.test.ts`
**219/219**; full suite **101 files / 2,508 tests — 2,467 passed, 40 skipped, 1 failed**, the exact
baseline (the documented environmental `soap.execute`). Verified after the move that `require(`
appears **only in comments** across `packages/engine/src` — the `graphql.*` / `soap.*` conversions in
slice 4 are the last real ones. **4 remain.**

#### 5c-vii. Slice 7 — the four `entity.*` CRUD commands, and the pass is complete (2026-09-19, `b48c65e`)

The last four — `entity.create` / `update` / `delete` / `list` — moved from module scope in
`src/ipc/handlers/entityCrudFactory.ts` into `packages/engine/src/store/entityCrudCommands.ts` behind
`registerEntityCrudCommands()`. **Zero module-scope `commandRegistry.register(` calls remain in
`src/`**, so §5c's pass is structurally complete: all 51 commands it turned out to be are registered by
an explicit `register*Commands()` call from `registerIpcHandlers()`.

**Structurally unlike slices 1–6, and that is the whole of its difficulty.** Each earlier slice moved a
*file of commands* — registrations on one side, channels on the other, separable by cutting between
them. This file is a **factory split across the seam**. The four commands dispatch at invoke time
through `entityCrudRegistry` / `simpleEntityKinds`, and those containers were populated by the six
**shell** files calling `registerEntityCrudHandlers()` / `registerSimpleEntityHandlers()`. The
dependency ran the wrong way, which makes this the one slice where the naive cut is *actively
dangerous*: moving the four registrations without the containers leaves them consulting an **empty**
map, so every one of them throws

```
entity.create: kind "rule" is not routed through the CommandRegistry yet
```

— the same message a genuinely unregistered kind throws. A plausible error, whose real cause is the
move itself, pointing the reader at the wrong file. **When a slice's commands read state that the shell
populates, the slice is not a move of commands; it is a move of the container plus an inversion of who
writes to it.**

The inversion: the containers went to the engine; the shell calls `registerCrudKind()` /
`registerSimpleKind()` (the engine half) and keeps three `ipcMain.handle` channels per kind (the shell
half). **Both factory signatures are byte-identical, so none of the six caller files needed an edit** —
`coreHandlers`, `crudHandlers`, `graphqlHandlers`, `grpcHandlers`, `soapHandlers`. That is the property
to check first when what has to move is a factory rather than a command list: *can the exported
signature survive unchanged?* If yes, the importer set stops mattering.

`entityCrudRegistry`, `simpleEntityKinds`, `isGitTracked` and the six `*Core` helpers were `export`ed
but had **no consumer outside the file** — verified by grep across `src/`, `tests/` and `packages/`,
not assumed — so they are module-private in the engine behind the two registration functions. Worth
flagging because it is the one thing in this slice that is **not** a pure relocation: an encapsulation
change. It is unobservable, but "unobservable" is a claim that has to be checked rather than asserted,
which is what the grep was for.

**Behaviours preserved verbatim, several of which a clean-up would flatten:**

- **`gateCreate()` ahead of `createEntityCore()`**, so a blocked create never touches config or disk.
  Only `environments` sets `gateKind`, and it returns `{error: "limit_reached", ...gate}` exactly as
  the pre-collapse `env:add` did. `EntityCreateResult`'s frozen `{id, entity}` has no room for that,
  which is safe only because the registry validates **params** and not return values — the same
  trade-off `entity.setEnabled`'s `invalid_kind` guard already made.
- **`onAddConflict`'s sibling-disable is diffed and written through `syncEnabledSet`.** The lambda
  mutates `cfg`, but `saveConfig()` cannot persist that: `writeEntity()` strips `enabled` on the way
  out, because enabled state lives exclusively in `enabled.json`. Before the diff existed the mutation
  was silently lost — the sibling stayed enabled on disk and switched itself back on the moment the new
  entity was deleted. `entity.setEnabled`'s equivalent loop always wrote through `syncEnabledSet`,
  which is why the two sites disagreed about the same rule. Only `mocks` and `rules` set
  `onAddConflict`, so for every other kind the flag changes nothing.
- **`mayResolveConflicts` is a required parameter, not a defaulted one**, so a new caller has to decide
  rather than inherit the privileged path. It is `mayAffectUnnamedEntities(ctx)` at the one call site.
- **`reloadConfig()` runs LAST in all three writers, for a different reason each time.** On create:
  it snapshots the enabled-sets from `enabled.json`, so running it before `syncEnabledSet()` left the
  proxy with a stale set, `workspaceCfg()` filtered the brand-new entity out of routing, and it only
  started working after some unrelated action happened to reload. On delete: `saveConfig()` does not
  unlink files for entities removed from the config, so reloading before the file was deleted (and
  before the enabled-set was updated) kept the deleted entity live in the routing tables.
- **`invalidateCache()` on delete only** — the status cache is keyed by workspace, and only a removal
  can make a previously-reported entity stale.
- **The tracked-vs-untracked name-entry branch.** A git-tracked entity with a name entry becomes an
  `addPendingDeletion` (the deletion has to be *published*, so the name entry must survive until the
  commit lands); an untracked one is `removeNameEntry` immediately. Getting this backwards makes a
  published deletion either unreplayable or resurrectable.
- **`entity.list` serves only simple kinds, and `entity.update` has no simple path at all** —
  `graphqlSchemas` / `protoFiles` / `wsdls` are written straight to disk with nothing mirroring them
  into an `AppConfig` array, and there is no "update" concept for any of them. Both asymmetries are the
  shell's.

**The importer set, and why the skill's prediction was wrong.** §5c-vii's scoping note predicted seven
affected suites, deriving them from the six factory *callers*. The measured set is **five**:
`entityConflictScope`, `foldersAndWorkspaces`, `settingsMutations`, `protocolPoc`, `protocolExecution`.
The two predicted extras (`gitRemote`, `gitSync`) never imported anything reaching the factory — their
`syncHandlers` import was a comment, not a dependency. The prediction came from a grep for the *file
name* across the consumers; the truth came from asking which suites **register a handler group that
reaches the factory**, which is one step further down the import graph. **Derive the affected set from
the registration calls a suite makes, not from the files it mentions.**

One placement detail with a bug behind it: in `settingsMutations` the call had to go in the existing
`beforeAll`, **not** next to `registerCrudHandlers()` in the `beforeEach`. The channel layer is
per-test and re-registering is harmless (the electron mock's map is keyed by channel), but the registry
is a singleton with no `clear()` and `register()` throws on duplicates by design — which is exactly the
bug slice 1 shipped, 29 of this file's 30 tests dead.

**Measured:** `npm run typecheck` → EXIT=0. The six affected suites **285/286** — the single failure is
the documented environmental `soap.execute`, and `settingsMutations` at **30/30** confirms the
`beforeAll` placement. Full suite **101 files / 2,508 tests — 2,467 passed, 40 skipped, 1 failed**, the
exact baseline, so **seven slices produced no test-count delta** — which is the evidence that seven
slices of "move the body, keep the channel" were behaviour-preserving.

**Mutation-checked, and the sharpest result of the whole pass.** Commenting out
`registerEntityCrudCommands()` produced **52 failures, and every one of them was in
`tests/ipc/handlers.test.ts`**:

- the ratchet, with **exactly the four** `entity.*` names in its missing list; and
- 51 more, because this file drives the per-kind channels — `mapping:update`, `mapping:delete`, … — and
  each of those **delegates** into one of the four, so a channel test is a registry test. The failure
  is `EngineError: No handler registered for command "entity.update"`, thrown from
  `CommandRegistry.invoke` through the channel.

Meanwhile `entityConflictScope` stayed **9/9 green**, because it registers the engine half itself.
That is §5c-vi's finding confirmed a third time, and it now has the cleanest possible statement:
**the mutation's blast radius is a map of which suites actually reach the registry, and the suites that
stay green are exactly the ones that cannot tell you anything about the wiring.**

**§5c closes here.** The pass moved 51 commands across seven slices and seven new engine modules
(`store/coreCommands`, `runner/runnerCommands`, `store/folderCommands`, `protocols/{graphql,soap,grpc}Commands`,
`applications/applicationCommands`, `sync/syncCommands`, `store/entityCrudCommands`). What it does
**not** do is delete `registerIpcHandlers()`: `app.checkUpdate` (SPLIT → P12) still needs a channel, and
`src/ipc/importExportHandlers.ts` is dead but dies with it. That is step 3, and it is now unblocked on
the §5c side.

#### 5c-viii. The guard §5c needed, and did not have (2026-09-20)

§5c's own complaint was that the class was invisible: *"they are invisible to the current ratchet
because it only asks whether a command is registered in this process, and in this process the shell has
registered them."* The pass was the manual substitute. This is the mechanism.

`tests/ipc/handlers.test.ts` → *"registers nothing at module scope — the §5c guard"* asserts the
**architectural rule** rather than the historical bug:

> registration goes through `register*Commands(registry)`, so the `commandRegistry` singleton is never
> registered directly.

That is not a style preference. It is what makes the engine containerisable: a consumer that supplies
its own registry — P9's Docker engine, a test's own registry, `createInProcessTransport(registry)` — can
only ever see commands registered **explicitly against it**. `createEngine()` registering no commands by
design is the same statement from the other side.

It reads every `.ts` under `src/` and `packages/engine/src/` through
`import.meta.glob(..., { query: "?raw", import: "default" })` and requires
`/^\s*commandRegistry\.register\(/m` to match nothing.

**It is strictly stronger than the grep the pass itself used, and that was measured rather than
argued.** The pattern matches a *statement* — a line whose first non-whitespace content is the call —
so it catches module scope at column 0 **and** a module-scope call wrapped in a top-level `if`/`try`
block. Mutation-verified: the `grep -rn '^commandRegistry.register(' src/` that every slice's
verification used **misses** the indented variant (`exit 1`, no match), while this guard fails on it and
names the file. Two shapes it still does not catch, stated in the test rather than left implied: a call
preceded by other code on the same line (`if (x) commandRegistry.register(...)`), and an indirect alias
(`const r = commandRegistry; r.register(...)`).

**The anti-vacuity guards are the part worth carrying forward.** A wrong glob returns `{}`, so
`Object.values(...).join("\n")` is `""`, `"".match(...)` is `null`, and every assertion passes — a guard
that reads as coverage while covering nothing, which is worse than no guard because it is trusted. So
the test asserts three things before it asserts the real one:

1. the reader found **>20 files** in each tree;
2. a **positive control**: `/^\s*registry\.register\(/gm` matches **>30** times in the engine — the
   parameterised form is plentiful, so the reader demonstrably sees real registration code;
3. therefore a zero for the *singleton* form means "not written that way", not "content never loaded".

**Mutation-checked both ways:** a module-scope registration in a scratch `src/` file fails the guard and
names that file; the same call indented inside a top-level `if` also fails it; deleting the file
restores green. Full suite **101 files / 2,509 tests — 2,468 passed, 40 skipped, 1 failed**, i.e. **+1
over the baseline, and the +1 is this test and nothing else** — the same "measure the delta" discipline
every slice of the pass used.

### 5a. The trap in 3b-2: do not re-point the legacy bodies yet

The natural follow-up — and it looks like pure DRY — is to replace the shell's `ipcMain.handle` bodies
with `commandRegistry.invoke("<command>", {}, ctx)`, the way `config:get` already does. **It was tried
and reverted. It is a regression, not a style question.**

The difference is *how the command got registered*. `config.get` / `env.setActive` /
`workspace.setActive` are registered at **module scope** in `coreHandlers.ts`, so importing that file
populates them. The `register*Commands(registry)` modules are called **explicitly from
`registerIpcHandlers()`**, so a legacy body delegating to one acquires a hidden ordering dependency on
a function in a different file — and on a function callers are not obliged to run.

They are not obliged, and two suites already didn't:
`tests/spike/protocolPoc.test.ts` and `tests/integration/settingsMutations.integration.test.ts`
register handler *groups* (`registerCoreHandlers()` / `registerSystemHandlers()`) without
`registerIpcHandlers()`. Re-pointing broke **six** of their tests with `UNKNOWN_COMMAND` —
`server.status` among them — because the registry they reached was empty. Nothing catches this at
build time and the shell's own handler file looks untouched; the failure is
`No handler registered` at *invocation* time.

So the duplication is deliberate and bounded: six one-line bodies, behaviourally identical to the
handlers in `serverCommands.ts`. It is removed by **step 3**, which deletes `registerIpcHandlers()`
and the shell's handler groups together — one implementation, no second entry point to keep in sync.
Re-pointing early trades a temporary, visible duplication for a permanent, invisible coupling.

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

> **This table is the smaller half of step 3.** It lists the keys that must *never* be routed — and it
> matches `ClientLocal` exactly. What it does not say is that **25 of the 93 protocol commands have no
> registry implementation at all**, so they cannot be routed yet even though the client classifies them
> as routable. See finding 5 above for the list and the consequences.

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

   **Corrected 2026-09-18 (finding 5):** the failures are not a handful of shell concerns but **25
   unimplemented engine commands**, so this step is two steps:

   - **3b-1** — route the registered commands. ✅ **done 2026-09-18: 131 of 144 keys** go through the
     client; the thirteen held back are listed in *3b-1 — the preload flip* above. (The plan used to
     say "the 68"; it is 89 registered now, minus the four with no implementation, minus the nine
     artifact-egress ones.)
   - **3b-2** — implement the **25** missing commands in the engine. ✅ **done 2026-09-18: 21 moved;**
     the remaining four are `NARROWED` / `BLOCKED` / `SPLIT` and are **not** moves (§5b). Do **not**
     re-point the shell bodies at the registry while doing it — see §5a; that trap applies to all five
     slices.
   - **3b-1 → 3c** — route the now-89 registered commands through the bridge, **then** delete
     `registerIpcHandlers()` + `eventBridge.ts`. ✅ **3c done 2026-09-18** — the event half only.
   - **3b-3** — the four artifact-egress methods that needed the dialog to run *before* the command.
     ✅ **done 2026-09-19: 140 of 144 keys route.** The client gained `pickSavePath` and a
     path-accepting `readArtifactFile`, and `writeArtifact` gained an optional destination. See
     *Step 3b-3* above.
   - **3d** — unblock the pinned keys. ✅ **keys 1–2 done 2026-09-19: 143 of 144 keys route.** Three of
     the four were **schema** problems rather than routing problems and were resolved by widening the
     schema they were pinned on (`listAudit`, then `saveRunnerConfig` / `loadRunnerConfig`), not by
     moving them. See *Step 3d* above.
   - **then** delete `registerIpcHandlers()`. **This is now blocked on one key, not eight** —
     `checkUpdate` (SPLIT), which is a genuine split resolved by P12 (§5b). So step 3 ends at
     **92 registered + 1 split**, not "89 registered + 2 blocked + 1 narrowed + 1 split". Deleting
     `registerIpcHandlers()` earlier would break that key on **both** paths at once, and the surface test
     would still pass because the key stays exposed.

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

## Step 3a — the push channel (2026-09-18)

**Events now have a real path.** `ipcTransport.subscribe()` no longer throws; it attaches one
`ipcRenderer.on(EVENT_CHANNEL)` and dispatches by `envelope.event`, and the main half keeps one engine
subscription per wire name and broadcasts each envelope to every live window.

This is the piece **step 3 cannot be done without**. `registerIpcHandlers()` and `wireEventBridge()` are
deleted together, so the moment the legacy handlers go, all nine event channels go with them. An event
path that has never carried a frame is not something to discover a problem in on the day the fallback
disappears.

| File | Role |
|---|---|
| `src/ipc/rpcContract.ts` | `EVENT_CHANNEL`, `EventFrame`, and the two control-action names |
| `src/ipc/rpcBridge.ts` | per-name refcount, broadcast, `subscribe`/`unsubscribe` control frames |
| `src/ipcTransport.ts` | the dispatcher, the local name check, `close()` releases everything |
| `tests/ipc/eventChannel.test.ts` | 12 cases, main half |
| `tests/ipc/ipcTransport.test.ts` | 9 → **16**; the `UNSUPPORTED`-refusal case is replaced |

### Why a second channel, and why `RPC_CHANNEL` cannot serve

`ipcMain.handle` / `ipcRenderer.invoke` is a request/response pair — one invoke, one resolve. An event
has no request to answer, so there is nothing to resolve and no `invoke` to hang it on. The push
direction is `webContents.send` → `ipcRenderer.on`, a different Electron API and therefore a different
channel. The frame that crosses it is a plain `EventEnvelope`, the same type `EventLog` emits and
`eventPump` writes, so nothing new is invented at this boundary.

### Five decisions

1. **One channel, not one per event.** The legacy side uses nine (`sync:status`, `log:chunk`, …), which
   is why `eventBridge.ts` needs nine subscriptions. Here the event name travels *inside* the envelope,
   exactly as on every other transport, so the preload needs one `ipcRenderer.on`.

2. **A per-name count of *renderers*, not of callbacks.** The preload refcounts too, but the two answer
   different questions: the preload's says "how many callbacks in this renderer want this name", this
   one says "how many renderers want it". Only the second can decide whether the engine should still be
   delivering — and a single window closing must not unsubscribe a second window still listening, which
   is what a `Set` keyed by name would have done.

3. **Broadcast to all windows, and let the preload filter.** This matches `eventBridge.ts`'s existing
   semantics, so the engine side is genuinely process-wide and one subscription per name is the whole
   need. A renderer that did not ask for a name has no callback registered and drops the frame.
   Per-window subscriptions would mean per-window `send` — a different feature (targeted delivery) that
   nothing asks for and the legacy path never had.

4. **Control frames are recognised before commands.** `subscribe`/`unsubscribe` are the protocol's
   `RESERVED_ACTIONS`, and `transport/types.ts` refuses to load if the protocol ever grows a real
   command by either name — so the branch cannot shadow anything. A test asserts the bridge's two
   duplicated strings against `RESERVED_ACTIONS`, because the contract duplicates them deliberately (a
   value import would put a second copy of the wire-name table in the renderer bundle).

5. **A batch of names is all-or-nothing, validated against the name table.** Without a pre-flight,
   `["event.sync.status", "nonsense"]` would subscribe the first, fail on the second, and return a
   rejected request — leaving a live subscription with **no handle to release it**, since the detacher
   is only returned on success. That is a leak, not an inconvenience. The check is against
   `BUS_NAME_BY_WIRE_NAME`, *not* a dry run through the transport: `EventLog`'s retention listeners are
   attached on the first subscribe for a name and deliberately outlive their subscribers, so a dry run
   would permanently retain a name the batch then rejected.

### The one thing `subscribe()` cannot do, and why that is acceptable

The contract says `subscribe()` throws **synchronously**. `ipcRenderer.invoke` is asynchronous, so a
rejection from the main half cannot become a synchronous throw. Two things make this a documented
limitation rather than a hole:

- **The realistic failure is synchronous anyway.** An unknown wire name is refused locally against the
  same table `inProcess.ts` uses, with the same code and the same wording — so the common mistake throws
  where the contract says it should.
- **A dead bridge is not silent.** Every `request()` already fails loudly with `ENGINE_ERROR`, so a
  subscribe that quietly does nothing cannot be the *first* symptom. The alternative — letting the
  rejection escape a `void`ed promise — is an unhandled rejection in the preload, which is worse than a
  documented no-op on an already-broken transport.

### Verification

| Check | Result |
|---|---|
| `tests/ipc/eventChannel.test.ts` | **12 passed** |
| `tests/ipc/ipcTransport.test.ts` | **16 passed** (was 9) |
| `npm run typecheck` | clean — protocol, engine, client |
| full suite | **2,423 passed** / 40 skipped / **1 failed** — the same `soap.execute` flake |
| delta against the finding-1 baseline (2,404) | **exactly +19** = 7 + 12, no other movement |
| `git diff --stat renderer/` | **empty** |

**The case that matters most** is *"carries `log.entry` end to end"*: it spans `logEmitter` →
`wireLogEventsToBus()` → `EventLog` → `EVENT_CHANNEL`, and it is the only test in the repo that
exercises finding 1's fix through a consumer. It was **confirmed non-vacuous** rather than assumed — it
failed with an empty `sent` until the wiring was added, which is precisely the shape of the original
defect. It also asserts the wire projection (`ts` → `timestamp`, capture bodies dropped) and that
`event.log.entry` arrives as a **batch of one**, since that shape belongs to `@bifurc/protocol` rather
than to the coalescing.

**What step 3a does not do, deliberately:** it changes nothing the renderer can see. The preload's seven
`on*` methods still use the legacy channels and `eventBridge.ts` still broadcasts them, so the phase
stays revertable and the new path can be deleted with nothing to unpick. Wiring the renderer onto it is
step 3b.

**Still unproven:** the real Electron round trip for events, exactly as for commands — both ends are
mocked here, so `webContents.send` → `ipcRenderer.on` across a real process boundary remains the e2e
suite's job.

---

## Step 3c — delete the legacy event path (2026-09-18, `eb4c207`)

`eventBridge.ts` was the temporary *middle hop*: it read `logEmitter` directly and broadcast the legacy
`log:entry` / `log:chunk` / `server:error` channels, so the renderer could keep receiving events over
`ipcRenderer.on(...)` unchanged while emission sites moved to the bus one at a time. Step 3a gave
events a real path and 3b-1 flipped the preload onto it, so the middle hop was dead — confirmed rather
than assumed, by grepping each of its seven channels for listeners outside the file itself (all zero).

### Why this had to be its own step

Step 3's description was "route all methods, then delete `registerIpcHandlers()` + `eventBridge.ts`
**together**." They are not the same deletion, and only one of them was ready:

- The **event** path is fully replaced. Nothing listens on the legacy channels any more.
- The **command** path is not: four of the 144 keys have no registry implementation, and four more are
  the artifact-egress methods above. Deleting `registerIpcHandlers()` today would break all eight on
  both paths.

So 3c deletes the half that is genuinely dead and leaves the other half standing.

### The consequence that matters

Deleting it makes `wireLogEventsToBus()` the **only** path for the three log events rather than merely
an additive one. Finding 1's comment said "additive, not a replacement" — that is now wrong, and the
comment in `handlers.ts` was updated with it. This call is what keeps the capture panel, the
request-log panel and the server-error banner alive.

### Two consumers died with it

- `tests/integration/companionServer.integration.test.ts` observed `companion:refresh` /
  `sync:entityStatus` via `webContents.send` — i.e. *through* the bridge. It now observes
  `bus.onTyped("entity.changed")` / `bus.onTyped("sync.entityStatus")` directly, which is what it
  actually meant to assert; the bridge hop was incidental.
- `tests/ipc/handlers.test.ts` had a four-test block asserting `logEmitter` reached `BrowserWindow`.
  All four tested the deleted middle hop, and two were vacuous (`not.toHaveBeenCalled()`). Deleted
  rather than re-pointed: the *traffic* is covered by
  `packages/engine/tests/eventBus.logWiring.test.ts` and the bus → `EVENT_CHANNEL` hop by
  `tests/ipc/eventChannel.test.ts`.

### Verification

| Check | Result |
|---|---|
| `npx vitest run tests/integration/companionServer.integration.test.ts tests/ipc/` | **10 files / 350 passed** |
| `npm run typecheck` | clean |
| full suite | **2,430 passed** / 40 skipped / **1 failed** — the same `soap.execute` flake |
| delta against the 3b-1 baseline (2,475) | **exactly −4** = the four deleted tests, no other movement |

---

## Finding 6 — artifact egress was binary-lossy and the blob pull was broken (2026-09-18, `26d2d32`)

Recorded in full under §3b-1's follow-up above. Summary: `writeArtifact` now takes base64 and writes
bytes; the blob branch of `artifactToFile` now loops over `blob.read` with `offset`, reads the correct
`data`/`eof` fields, accumulates bytes, and releases in a `finally`. Six new tests, two confirmed
non-vacuous by mutation.

| Check | Result |
|---|---|
| `packages/client/tests` + `tests/ipc/clientHandlers.test.ts` | **89 passed** |
| `npm run typecheck` | clean |
| full suite | **2,436 passed** / 40 skipped / **1 failed** — the same `soap.execute` flake |
| delta against the 3c baseline (2,471) | **exactly +6**, no other movement |
| mutation check (`if (read?.eof) break;` → `break;`) | both blob tests fail — they are live |

---

## Step 3b-3 — the last four artifact methods (2026-09-19, `d89ab79`)

**140 of the 144 keys now route.** The four that needed a *third* primitive are off the shell's legacy
channels, and the primitive they needed was the one `File_Ops_Protocol.md` §3.2 has been asking for
since P3: **the dialog has to run before the command.**

`ClientLocal.writeArtifact` bundled "ask where" with "write there", which forces the dialog *after* the
render. That is fine for the five egress methods whose artifact the engine has already produced, and
wrong for the two that make the engine render something expensive the user might abandon. So the ask
was split out, and the ingress mirror was widened.

| Hook | Shape | Why this shape |
| --- | --- | --- |
| `pickSavePath(opts)` | ask, return a path, write nothing | egress is *dialog → command → write*, so the ask must be separable |
| `writeArtifact(…, destPath?)` | bundled when omitted, write-only when present | keeps the five already-routed methods on the form that shows the engine's own `suggestedName` |
| `readArtifactFile(opts?)` | ask **or** reuse `opts.path`, return the path it read | ingress is *dialog → read → upload → command*, so ask and read can share a call |

The dialog's chrome is built from the engine's own `export.formats` table rather than a second
hard-coded list of the 17 formats and their extensions — the same reasoning `blob/publish.ts` uses for
holding the inline-vs-blob decision once. One visible consequence, stated plainly because it is a
behaviour change: **with no engine there is no dialog.** That is correct rather than regrettable —
there is nothing to export either way — but it does mean `exportData` now fails before the user picks a
destination instead of after.

### Two client-side regressions, both of which routing would have exposed

Neither is a shell problem. Both are gaps in the client that only became reachable when the preload
stopped overriding the method — which is the same lesson 3b-1 recorded, in its second form.

1. **`exportRunnerReport` hardcoded `format: "json"`.** The shell's dialog offers **HTML or JSON** and
   derives the format from the extension the user picked. Choosing the extension *is* choosing the
   format, so this could not be repaired after the fact: routing it as it stood would have silently
   deleted HTML export, which is the dialog's **default**. The starkest case of why the dialog cannot
   run after the render.
2. **`importData` ignored `req.filePath` and re-opened a picker.** Confirmed, and worse than the
   "may open a second dialog" 3b-1 recorded: `ImportExportModal.tsx` takes `res.filePath` from
   preflight and feeds it back, and the old client returned `import.preflight`'s result verbatim —
   which the engine built from a `blobId`, so `filePath` was `undefined` and the collision branch
   handed `undefined` to `applyImport`. Now the path is reused, and the staged blob is reused with it
   through a `pendingUploads` map, so the bytes cross the wire **once** (§5, "uploaded ONCE"). The map
   is bounded by construction: one entry per file picked since the last preflight, cleared on commit,
   and a re-run of preflight on the same file releases the entry it displaces rather than leaving it
   for the sweep.

**A third defect, in a hook rather than a method.** `readArtifactFile` was a pass-through to
`dialog:openFile`, which caps at **1 MB** because it feeds binary request bodies — while the legacy
`importExport:preflight` accepted `BLOB_MAX_INGRESS_BYTES` (100 MB). Reusing that channel would have
silently rejected **every import between 1 MB and 100 MB**, which is most real collections. The new
handler caps at the protocol constant and returns the path `dialog:openFile` never had. Two things
follow from that cap being *ours* now: it is the protocol's number rather than a second one, and the
handler is the only place that reads a file for import.

**And a fourth, in `exportData`'s argument mapping.** It read `req.workspaceId`; `ExportRequest` has
`wsId`. Nothing noticed because the preload overrode the method entirely — routed as written, **every
export would have been a `BAD_REQUEST`.** Same shape as finding 5: the surface test cannot see it,
because every key is exposed correctly and only the payload behind one is wrong.

### Verification

| Check | Result |
| --- | --- |
| `npm run typecheck` | clean — protocol, engine, client |
| full suite | **101 files / 2,503** — 2,462 passed, 40 skipped, **1 failed** (the `soap.execute` flake) |
| delta against finding 6 (2,436) | **exactly +26** = 12 + 11 + 3, no other movement |
| `git diff --stat renderer/` | **empty** |
| surface test | 144 keys, both directions clean |
| mutation: hardcode `format: "json"` | the two HTML rows of the `it.each` fail |
| mutation: disable the remembered-blob lookup | "reuses the path and the blob" fails |

The +26 is measured, not inferred, and it decomposes exactly: 12 in `packages/client/tests/client.test.ts`
(9 plain + one `it.each` with 3 rows), 11 in `tests/ipc/clientHandlers.test.ts`, and 3 from the
`for (const channel of expectedChannels)` loop that generates one test per registered channel — the
three new `client:*` channels are the third term. The file count is unchanged.

Both new claims were mutation-checked rather than trusted, because the assertions are about **ordering
and count** and a value-only assertion passes either way.

### What this unlocks, and what it does not

`src/ipc/importExportHandlers.ts` is now **dead**: nothing in `src/`, `renderer/` or `e2e/` references
any of its four channels. It is the `kindLabel` copy in the client that is now the only one. It dies
with `registerIpcHandlers()`, which is the remaining work in step 3 — and that is now blocked on
**one key, not eight** (see *Step 3d* below).

**Still unproven, and unchanged by this step:** the real Electron IPC round trip. Both ends are mocked,
so the wire *between* them remains the e2e suite's job, from a normal terminal.

---

## Step 3d — unblocking the pinned keys (2026-09-19)

**Three of the four pinned keys were not routing problems. They were schema problems**, and that is the
finding worth carrying forward: "the command is not in the registry" was never the same claim as "the
engine cannot serve it". §5b had already split the four by *why* they were unimplemented — `NARROWED`,
`BLOCKED`, `BLOCKED`, `SPLIT` — and only the `SPLIT` turned out to need a split.

| Key | Category | Resolution |
|---|---|---|
| `listAudit` | `NARROWED` | **done 2026-09-19** (`fcd6d0d`) — widen `AuditListParams`, then move. |
| `saveRunnerConfig` / `loadRunnerConfig` | `BLOCKED` | **done 2026-09-19** — widen `RunnerConfigSchema`, then move. |
| `checkUpdate` | `SPLIT` | **still open.** The one genuine split, resolved by P12. |

### Why widening was the right response rather than moving

Both widenings are recorded in `plan/protocol-changes.md` with their justifications, and both follow the
precedent `entity.ts` set: a large, evolving, **caller-owned** shape stays `z.record` at the wire
boundary and the domain layer owns validation. `AppConfig` and `ApplicationConfig` were already that
call. `RunnerConfigSchema` was the outlier — a fixed `{requestOrder, delayMs}` object describing a
payload no caller sends, in a store whose own shell body is `JSON.stringify(config)` with `config: any`.

The `BLOCKED` category's value showed up here. It existed so that a schema mismatch would be *asserted*
rather than left filed as "unimplemented", and its assertion checked the block **directly** — parsing the
real config through the frozen schema had to fail. So widening the schema failed that test and announced
the two commands had become movable. That is the ratchet working as designed, on the one category where
the fix was a protocol change.

### The two defects in a frozen schema, and why only one was visible

`RunnerConfigSchema` was wrong twice: it **rejected** what real callers save (`stopOnFailure`,
`iterations`, no `requestOrder`), and — because `z.object()` drops unknown keys — it would have
**stripped** them from any payload that did pass. The second is the more dangerous and the one no
`BLOCKED` entry could express, because a dropped field is silent data loss rather than a validation
error. The replacement guard asserts `toEqual` on the whole config, which is the assertion the block
never could make: it only ever proved rejection.

### A widening invalidates fixtures, not just callers

Found by running the suite rather than by reading it. `COMMAND_FIXTURES["runner.saveConfig"].invalid`
was `config: {}` — a near-miss only because the old schema *required* `requestOrder`. An empty record is
a valid record under `z.record`, so the every-command matrix in the conformance suite (which asserts each
fixture's `invalid` is rejected, across **all four transports**) would have failed four suites for a
reason with no visible connection to the change. Confirmed by mutation: restoring `config: {}` fails
`run-in-process` with *"promise resolved `{reached: 'runner.saveConfig'}` instead of rejecting"*.

The generalisation is worth stating because it will recur: **`z.record` widens a schema's acceptance, so
every `invalid` fixture targeting that field must be re-derived rather than trusted.**

### The integration suite could not see any of this

`tests/integration/runnerStorage.integration.test.ts` drove `runner:saveConfig` / `runner:loadConfig` —
the **shell channels** — and every assertion in it would have kept passing if the engine implementation
were wrong, because those channels are served by self-contained `ipcMain.handle` bodies that never
consult the registry. The conformance suite cannot cover it either: its every-command matrix registers
`() => ({ reached: cmd })` over each command *deliberately*, so it tests the transport and the schema
rather than any implementation.

So the file gained a block that drives both commands **on the registry** — the path a containerised
engine has — asserting the on-disk round-trip, that the raw file preserves every field (the data-loss
half), that the registry and the channel address the *same bytes* in both directions, and the
`null`-for-missing/corrupt and `{ok:false}`-rather-than-throw branches. This is the integration-suite
twin of the "unit suite is weak evidence for the preload flip" trap recorded earlier in this phase.

**Measured:** full suite **101 files / 2,508 tests — 2,467 passed**, 40 skipped, 1 failed (the same
documented `soap.execute` flake). Delta from the step-3b-3 baseline of 2,503 is **+5**, decomposed as
**+1** for key 1's `audit.list` widening guard and **+4** for this step's registry block. Typecheck
clean across all three packages.

---

## Acceptance criteria

- [ ] **Unit suites pass.** Baseline after **step 3d (keys 1–2 of the four pinned)**:
      **101 files / 2,508 tests** — **2,467 passed**, 40 skipped, and the one documented `soap.execute`
      flake (`ECONNREFUSED 127.0.0.1:1`, `tests/spike/protocolPoc.test.ts`). After **step 3b-3 (the last
      four artifact methods)**: 101 / 2,503 — 2,462. After **finding 6 (egress
      fix)**: 101 / 2,477 — 2,436. After step 3c: 101 / 2,471 — 2,430. After **step 3b-1 (the flip +
      the two hooks)**: 101 / 2,475 — 2,434. After step 3b-2: 101 / 2,471 — 2,430. After the step-3b
      inventory: 101 / 2,467 — 2,426. After step 3a: 101 / 2,464 — 2,423. After finding 1: 100 / 2,445
      — 2,404. After step 2: 99 / 2,436 — 2,395. Before step 2: 97 / 2,418. The "35 unit suites" this
      criterion used to say predated P2–P5 entirely.
      *(3b-3 is **+26**: 12 in the client suite, 11 in the shell-handler suite, 3 from the
      channel-registration loop. 3c is **−4**: the deleted vacuous forwarding tests. Finding 6 is
      **+6**. All measured, not inferred.)*
      *(3b-1 added **exactly +4** — the `client:writeArtifact` cases, the only genuinely new code in
      the step. Everything else moved rather than being written, and the flip itself added none, so
      the flip's own delta is zero. Measured, not inferred.)*
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
