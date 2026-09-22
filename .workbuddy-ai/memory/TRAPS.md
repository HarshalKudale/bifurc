# Bifurc — failure modes that cost real time

Referenced from `MEMORY.md` in this directory. **Read this before touching anything** — none of it is
discoverable from the code, and each item was paid for at least once.

## Registering a command twice throws

`commandRegistry` is a module-level singleton with **no `clear()`**, and `register()` throws on a
duplicate *on purpose*, to catch double-registration. So an engine-half registration belongs in a
suite's **`beforeAll`, never `beforeEach`** — per-test it passes the first test and kills every one
after it with `Command "config.get" is already registered.` (29 of `settingsMutations`' 30 tests,
fixed in `0160188`).

The *channel* layer is the opposite: the electron mock's `handlers` map is keyed by channel, so a
second `ipcMain.handle` for the same channel simply overwrites and is harmless. **That asymmetry is
what makes the mistake look safe.**

## A module-scope registration is invisible to the compiler and to the ratchet

**Status: this class is extinct in `src/` as of 2026-09-19** — `grep -rn '^commandRegistry.register('
src/` returns nothing. The §5c pass moved all 51 across seven slices. Keep the trap: the *shape* is what
to watch for in new shell code.

A module-scope `commandRegistry.register(...)` takes effect on *import*. Moving one behind an explicit
`register*Commands()` call silently unregisters it for every suite that imports the handler group
without running `registerIpcHandlers()`, and the failure is `No handler registered` at **invocation**
time — nothing catches it at build time. **Read the importer set before calling a slice mechanical**:
slice 1 needed 5 suite edits, slice 2 needed 0, slices 3–4 and 6 needed 2 each, slice 5 needed 1, slice
7 needed 5.

The conformance matrix cannot cover this either — it registers `() => ({ reached: cmd })` over every
command deliberately, testing the transport and the schema rather than any implementation. And the
ratchet asks only "is this command registered *in this process*", which the shell has already done for
all of them.

**But the ratchet is sharp on the *move*, which is the half worth remembering.** Once a command's only
registration is an explicit `register*Commands()` call, forgetting that call makes it unregistered and
the ratchet says so immediately — it caught exactly that in slice 5 (all ten `application.*`, `cd7afad`),
slice 6 (exactly the 17 `sync.*`/`git.*`/`history.*`) and slice 7 (exactly the four `entity.*`). So the
ratchet is blind to the §5c *defect* and a real guard on the §5c *fix*: an explicit call site is
checkable, a module side effect is not.

## A *delegating* channel is both the risk and the evidence

Channels that `commandRegistry.invoke()` break on an empty registry — but they also make a
channel-driven test a registry-driven test. Self-contained duplicate bodies do the opposite: three of
`runnerHandlers`' five are copies, so `runnerStorage.integration.test.ts` passed with the engine half
wrong. **Mutation-check every move**: drop the `register*` call and confirm the failure is
`No handler registered`.

**Corollary, measured in §5c slice 6: a suite that registers the engine half *itself* is structurally
blind to the wiring, and its green is not evidence about it.** `gitRemote.integration.test.ts` calls
`registerSyncCommands()` in its own `beforeAll`, so it stayed **24/24 green** with the call site in
`handlers.ts` commented out. Slices 3–6 each *added* such a call to keep suites alive — necessary, but
it means the suite can never catch a missing call site. **The call sites that can fail loudly are only
the ratchet and the delegating channels**, so the mutation check must be run against *those*, and a
hand-registering suite's green must never be quoted as verification of the wiring. Also: before the
move, such a suite was populated by an *import side effect* (static `import { registerXHandlers }`), so
adding the explicit call reproduces a property the suite never asserted.

## `require()` in a handler body

Invisible to tsc and to every unit test, but **fatal in the engine's ESM build output** — `require` does
not exist there, so a verbatim copy compiles, typechecks, passes the suite, then throws at *call* time
in `dist/*.mjs`. Use static imports. **Check the artifact, not the source**:
`grep -c 'require(' packages/engine/dist/**/*.mjs` — the source is what lies. After §5c slice 4 the
only remaining `require(` strings in `packages/engine/src` are in **comments**; re-grep before
believing that.

## A resolved `{ok:false}` is an envelope success; only a throw fails

The transport hands a handler's value back **uninspected**, so a handler that *resolves*
`{ok:false, error}` is a successful envelope carrying a failed result, while a *throw* becomes a
transport-level error. `graphql.introspect` / `soap.fetchWsdl` catch; `graphql.execute` /
`soap.execute` throw. Not interchangeable, and "the command reports failure" describes only one of them.

## Schemas: accepts ≠ preserves, and widening invalidates fixtures

- **"Schema accepts the payload" ≠ "it preserves it"** — a plain `z.object()` **strips** unknown keys.
  A dropped field is data loss, not a validation error, and nothing reports it.
- **Widening a schema invalidates its `invalid` fixture.** `COMMAND_FIXTURES[...].invalid` is asserted
  *rejected* on all four transports; `z.record` turned `config: {}` from a near-miss into a valid
  record, so the matrix would have failed for no visible reason. **Re-derive the near-miss** (a
  non-record), never trust it.
- `z.record` is the house call for a caller-owned evolving shape (`entity.ts`, `AppConfig`,
  `ApplicationConfig`, `RunnerConfigSchema`).

## Commands that read state the *shell* populates

`entity.*`'s four commands dispatch at invoke time through `entityCrudRegistry` / `simpleEntityKinds`,
and those containers were filled by the six **shell** files calling `registerEntityCrudHandlers()` /
`registerSimpleEntityHandlers()`. Moving the registrations without the containers leaves the commands
consulting an **empty** map, so each throws `"<kind>" is not routed through the CommandRegistry yet` —
**the same message a genuinely unregistered kind throws**. A plausible error whose real cause is the
move, pointing the reader at the wrong file. **When a slice's commands read shell-populated state, it is
not a move of commands: it is a move of the container plus an inversion of who writes to it.** Fix by
moving the container into the engine and giving the shell a `register<Thing>Kind()` to call. If the
factory's exported signature can stay byte-identical, the importer set stops mattering — slice 7's six
caller files needed **zero** edits.

**Derive the affected suite set from the registration calls a suite makes, not from the files it
mentions.** Slice 7's set was predicted as 7 suites (from the factory's callers) and measured at **5**:
`gitRemote` / `gitSync` matched a grep for the file name only because of a *comment*. One step further
down the import graph: `grep -rn 'register.*Handlers()' tests/` and ask which of those reach the
container.

## A same-named helper in the same package is not evidence of sameness

**Grep the helper's *body*, not its name.** `folder.publish` declares a local `sanitizeDirName` while
`store/workspaceFs` exports one of that exact name — different pattern
(`[<>:"/\|?*]+` → `"-"` vs. `[<>:"/\|?*\x00-\x1f]` → stripped), and only the engine's has an
`|| "unnamed"` fallback. Collapsing them would have silently changed the published git path for
`"a:b"` (`"ab"` vs `"a-b"`). Two same-named functions in one package doing different things is normal;
the danger is that the tidy-up *looks* like hygiene. Slice 6 kept it local and verbatim.

The flip side is worth knowing too: collapse a duplicated helper **only** when it is byte-identical
*and* already exported, and escalate to a correctness concern **only** when it holds or writes state
that outlives the process (`syncEnabledSet` → `enabled.json` was a correctness fix; `generateId` is
pure, so collapsing it was hygiene).

## A routed key can deliver *less* than the shell channel it replaced — and the cast hides it

**Compare the shell channel's return shape against what the routed command can actually produce, for
every key that composes anything.** `@bifurc/client` bridged the two with
`as ReturnType<BifurcApi["<key>"]>`, which is a cast in the direction that **fails silently**: the
missing fields arrive as `undefined`, which is falsy and not an error. Twice now:

- `checkUpdate` — `AppCheckUpdateResult` has `tagName`; the renderer reads `hasUpdate`. Routing it would
  have left `hasUpdate: undefined` → the About panel reports "you're up to date" forever. Fixed by
  deleting the command and making the key client-local (2026-09-20).
- `tlsCertStatus` — the shell channel composes **eight** fields by joining the engine's fingerprint with
  `certLifecycle.readTrustSummary()`, which reads the *client's* trust store. The client routes to
  `tls.certStatus`, which returns the engine half only, so `trustState` / `firefoxDetected` /
  `firefoxProfiles` are `undefined`: `undefined` is neither `"trusted"` nor `"stale"`, so the drift
  warning cannot render, and it is falsy, so the Firefox note cannot either. `tlsRemoveCert` is the same
  shape. Found 2026-09-20 by the §5b-bis audit; **latent**, because P7 is what renders those fields.

**Why it hides:** `BifurcApi` is a *declared* type, and the cast makes the compiler agree with the
declaration rather than with the runtime. Every unit test that drives the shell channel still passes,
because the shell channel is correct — only the routed path is short. So the test that finds this is
neither: it is **reading both implementations side by side**.

**The tell:** a key whose shell body calls something the engine cannot (`app.getVersion()`,
`dialog.*`, the client trust store). If the client routes it and casts, the composition is lost. The
fix is not to widen the command — it is to make the client compose the client-side half, the way it
already does for the artifact flows via `pickSavePath` / `writeArtifact`.

## The docblock that sanctions the bug

Found 2026-09-20, in the same audit as the entry above, and it is the reason that audit found *two*
defects where it expected one. `packages/client/src/index.ts`'s `exportAudit` took `artifactToFile`'s
artifact-first default, which undid P3's deliberate dialog-first ordering for `audit:export` — and
`packages/client/src/local.ts`'s `writeArtifact` docblock, the authority any reader consults before
writing one of these callers, **named `exportAudit` in the cheap group**: *"the other egress methods
(`exportAudit`, `tlsExportCert`, `shareCaptureJson`) hand over bytes the engine has already produced, so
reordering them buys nothing."* False for `exportAudit` alone — its artifact is the one egress payload
with no size ceiling, because `audit.export` runs `queryLog({ limit: 0 })`, the whole `git log`.

**Three other places in the tree already said the opposite**, and they were not enough:

| source | says |
|---|---|
| `src/ipc/handlers/syncHandlers.ts` | "Dialog FIRST. The pre-P3 handler queried the whole log and *then* asked for a path…" |
| `packages/engine/src/fileOps/commands.ts` | "the one egress artifact with no natural size ceiling" |
| `plan/07` §5b-bis | the audit's own conclusion |

So the regression did not read as a mistake — it read as **the documented design**, and a reviewer
checking the call against the nearest doc would have confirmed it. Two lessons:

- **A wrong docblock is a defect with a wider blast radius than the bug it causes**, because it
  regenerates the bug every time someone follows it. When a fix is a *reversal* of something a doc
  asserts, fix the doc in the same pass or the next author reverts you. Both were corrected together.
- **Agreement between a call site and its local doc proves nothing** — they were written from the same
  wrong premise. The evidence that settles it is a third source: the *other* implementation, or the
  engine's own cost note. In this case the shell channel's body and the engine's docblock both existed,
  both said dialog-first, and neither was consulted, because the client-side doc was closer to hand.

**Corollary — "latent" is a claim about the *caller*, not about the defect, so go and find the caller.**
The audit first filed `tls:removeCert` as latent alongside `tls:certStatus`, on the reasoning that P7 is
what renders the tls fields. That is true of `tls:certStatus` and false of `tls:removeCert`, which loses
an *action* rather than a field: `renderer/panels/settings/TlsSettingsSection.tsx:57` calls it today, so
the "remove certificate" button deletes the engine's keypair and leaves the OS trust store trusting the
CA — exactly what `certLifecycle.ts`'s docblock says the two-sided design exists to prevent. One grep for
the key in `renderer/` moved it from "P7's problem" to "live". **File severity only after following the
key to its caller.**

## Artifacts, blobs, dialogs

- **Artifacts are bytes.** `writeArtifact` takes **base64**; the channel writes a Buffer. Any UTF-8
  round-trip corrupts `workspace-zip`.
- **`blob.read` is paged**: 512 KB cap, `eof` is the only terminator, result is `{data, eof}`. Loop with
  `offset`. Inline threshold is 1 MB, so almost every real export is a blob.
- **The dialog runs before the command** (`File_Ops_Protocol.md` §3.2) — fail fast on cancel, so the
  engine never renders a 200 MB archive the user abandons. `writeArtifact` bundles "ask where" with
  "write", which is why `pickSavePath` exists.
- **A fake transport answering every request with `{ok:true}` hides whole branches** — no client test
  reached either branch of `artifactToFile`. Assert requested `offset`s, not just results.

## Evidence quality

- **The unit suite is weak evidence for the preload flip** — it tests handlers, not the preload. **e2e
  is the real gate** and cannot run from the agent shell.
- **e2e is the renderer's only coverage** (11 specs / 46 tests), and **11 of 46 cannot fail**.
- **"The command is registered" ≠ "the client's method is equivalent."** Audit before routing; three of
  the four held-back egress keys broke *after* the engine did its job correctly.

## "Nothing else reads it" needs a grep of `packages/`, not just `src/`

The shell is `src/`, the engine is `packages/engine/`, the surface is `packages/{protocol,client}/`. A
claim of the form *"nothing else reads this field — verified"* is only true if the grep covered **all
three**, and the instinct is to grep the file you are looking at, which lives in `src/`.

Measured 2026-09-20, on `CrudFactoryOpts`: `ipcPrefix` was recorded as vestigial alongside `ipcAdd` /
`ipcUpdate` / `ipcDelete` / `ipcList`. Four of the five are. `ipcPrefix` is read **three times inside
`packages/engine/src/store/entityCrudCommands.ts`** — as a behavioural discriminator (prepend vs append,
emit `entity.changed`, cascade-delete the targeting `proxyRules`) — because it had been standing in for
the *kind*. The grep that "verified" it had run over `src/` only, where the sole reader is the factory's
channel-name builder.

So **a field named for the shell can still be load-bearing in the engine.** Before deleting one, grep
`packages/` for it, and prefer grepping for the *behaviour* (what branches on it) over the bare name. When
the answer is "the engine branches on a shell name", the fix is usually a **translation into the engine's
own vocabulary** (`opts.kind`) rather than a deletion: deleting it compiles cleanly and silently kills the
behaviour, and no test may notice.

## A test whose two inputs are indistinguishable passes on either branch

Found 2026-09-20 in `handlers.test.ts`, on the pin for the prepend branch:

```
currentConfig.mocks = [{ ...baseMock, id: "existing", createdAt: 1 }];  // name: "Test Mock"
await createEntity("mocks", baseMock);                                  // name: "Test Mock"
expect(currentConfig.mocks[0].name).toBe("Test Mock");                  // ← true under unshift OR push
```

The seeded entity was spread from the same object the test then created, so both carried the same `name`
and index 0 read `"Test Mock"` either way. **The case passed on a `push`** — i.e. it pinned nothing, while
reading as the pin for that exact branch. The fix is to make the two distinguishable (`id`, or a different
name) and to assert **both** indices.

Two rules that generalise:

- **Assert on identity, not on a value both candidates share.** When seeding a "before" state to test an
  ordering or a filter, give the seed a value the new entity cannot have.
- **Pin the boundary, not just the positive.** `mocks` prepends and `requests` appends is one test each;
  only the pair proves the predicate has an extent. A rewrite that prepends *everything* satisfies the
  positive half alone.

This is the same failure as the `transports.test.ts` fixture that asserted the very bug it existed to
catch (`plan/07` §5b-quater) — two instances in two days, both found by asking "would this still pass if
the behaviour were wrong?" rather than by a red test.

## A guard that can silently match nothing is worse than no guard

Written after building §5c-viii's static test. It reads every `.ts` under `src/` and
`packages/engine/src/` via `import.meta.glob(..., { query: "?raw", import: "default" })` and asserts a
forbidden pattern matches nothing. **A wrong glob returns `{}`**, so `Object.values(x).join("\n")` is
`""`, `"".match(re)` is `null`, and the assertion passes — a guard that *reads as* coverage while
covering nothing, which is worse than none because it is trusted and quoted.

So a reader-based guard needs three things before it asserts the real property:

1. **A non-vacuity assertion** that the reader found the tree (`Object.keys(...).length > 20`).
2. **A positive control** — assert the same reader *does* match the benign form (>30 hits of
   `registry.register(`), proving it sees real content. Without this, a zero is indistinguishable from
   "content never loaded".
3. **A mutation check in both directions** — the guard must fail on a deliberately offending file, and
   the offending file must then be deleted and green restored.

Also worth knowing: **a statement-form regex beats a column-0 grep.** `/^\s*pattern/m` catches a
module-scope call at column 0 *and* one indented inside a top-level `if`/`try`; `grep -rn '^pattern'`
misses the second (measured — it returned exit 1 on the indented probe while the guard failed on it).
State in the test what it does *not* catch rather than implying completeness.

## A guard built around a *pinned list* empties along with the list

Found 2026-09-20, while deleting the last entry of a pinned list. Two ratchet tests in
`tests/ipc/handlers.test.ts` were written as *"for each command in `NOT_IN_REGISTRY`, assert X"*. That is
a correct guard while the list is non-empty and a **silent no-op** the moment it empties — `[].filter()`
is `[]`, and `[]` equals `[]`, so it is green. Removing `app.checkUpdate` (the last pin) would have
turned both green while checking nothing.

**The rule: a guard over a *shrinking* list must be rewritten around its invariant, not its list.** Both
were, and both came out **stronger**:

- *"still serves every unregistered command on a legacy channel"* → **"no command is both unregistered
  and unreachable"**, derived from `Object.keys(COMMANDS)` — a set that cannot shrink to zero.
- *"confirms the client claims every pinned command is routable"* → **"every `transport`/`shim` surface
  entry names a registered command"**. The list form could only catch a command *already on the list*;
  the invariant form catches a newly-added routed key whose command nobody implemented — the actual bug
  class.

Pair it with a non-vacuity floor, and expect the floor to be wrong the first time: written as `> 80`, it
failed immediately at the real figure **75**. That is the guard working on its author, before it ever
guarded the code.

## Assert containment of the inputs, not a re-derived copy

Written while doing P6 work item 5 (the e2e data-dir isolation assertion). Two things to carry forward.

**The engine's data dir has *two* inputs on the Electron path, and on Windows the second one wins.**
`src/main.ts:256` calls `setDataRoot(app.getPath("userData"))`, so the plan's phrasing — the engine
"derives its data dir from `app.getPath("userData")`, which that override already controls" — reads as if
there were one input. There are two: `store/workspaceFs.ts#dataRoot()` and
`store/appSettings.ts#settingsPath()` both **short-circuit on `%LOCALAPPDATA%` on win32 before consulting
the data root.** Measured with the e2e fixture's env:

| resolver | resolves to |
|---|---|
| `workspaceFs.dataRoot()` (entity data) | `<temp>\Bifurc\data` |
| `paths.dataDir()` (settings / `app.json`) | `<temp>\electron-userdata` |

Different directories. Both inside the temp dir, so isolation holds — but if you are reasoning about
"where the app's data is", there is no single answer on Windows. This is the defect documented in
`packages/engine/src/index.ts`'s `EngineOptions.dataDir` docblock and tracked as **`plan/03` work item 8**;
do **not** "fix" it casually, because correcting the order moves existing Windows users' data. It also
explains why the e2e fixture's seeded sample data is found at all: `writeSampleWorkspace(dataRoot)` writes
`<dataRoot>/<wsId>/…` and puts `app.json` in the *parent*, i.e. it targets `dataRoot()` and
`settingsPath()` respectively, not `dataDir()`.

**When the thing you want to assert is a path the code derives, assert containment of its *inputs*, not a
re-derivation of the formula.** Both consumers only ever `path.join` relative segments (`"Bifurc"`,
`"data"`, `"app.json"`, `<wsId>`, `<kind>`) onto one of the two inputs — so if both inputs are inside the
temp dir, every derived path is too. Re-deriving the formula in the fixture would have duplicated it, and a
copy drifts from the original silently. Generalised: **a test that re-implements the code under test
verifies the copy, not the code.**

Two corollaries from the same job:

- **An unset env var that the code falls back from is a failure, not a skip.** On win32 a missing
  `%LOCALAPPDATA%` makes the engine read the *developer's real profile* — the exact contamination the guard
  exists to catch — so the fixture pushes an explicit error rather than dropping the candidate from the list.
- **When the suite cannot run (no desktop session), verify the property against the *real built* resolver
  plus a negative case.** A throwaway script driving `packages/engine/dist/store/workspaceFs.js` and
  `.../paths.js` under the fixture's env confirmed all four resolvers land in temp; the negative case —
  override removed → `dataRoot()` escapes to `C:\Users\wrath\AppData\Local\Bifurc\data` and the predicate
  rejects it — is what proves the guard is non-vacuous. A guard whose failure mode you have never observed
  is a guard you are guessing about.

## The engine's command surface has no home outside the Electron shell

**Found 2026-09-21 building P14's workspace host, and it is the P6 §5c bug class one layer up.**

`createEngine()` registers **no** commands, by design (P3: *"the consumer declares what it is willing to
serve"*). Every one of the eighteen `register*Commands()` calls lives in **`src/ipc/handlers.ts`**, inside
`registerIpcHandlers()`. So any consumer that is not the shell gets an **empty registry** and answers
`UNKNOWN_COMMAND` for commands the protocol declares and `@bifurc/client` advertises.

**Why the ratchet could not see it:** the ratchet asks "is this command registered *in this process*", and in
the shell's process the answer is yes — the shell registered it. The same blind spot §5c records, and the
same shape as the corollary in "A *delegating* channel is both the risk and the evidence": a suite that
registers the engine half itself cannot catch a missing call site.

**The fix is `packages/engine/src/registerAll.ts#registerAllEngineCommands(registry)`** — the same eighteen
calls, engine-side. **Do not delete the shell's calls in favour of it**: each carries a comment naming which
P6 reduction group made it the only registration, and that is the ledger. Two lists of one thing drift, so
`tests/integration/workspaceHost.integration.test.ts` asserts the engine-side list is a **superset** of the
shell's — the direction that bites is *shell-serves-it, host-does-not*, because only the desktop runs in CI's
default path. **If you add a `register*Commands()` call to the shell, add it to `registerAll.ts` too or that
guard fails.**

## A schema with `.strict()` makes an unimplemented parameter a REJECTION, not a default

`ConfigGetParams` is `z.object({}).strict()`, so sending `config.get` with `{ workspaceId }` — as D13 says
workspace-scoped commands will — is a **`BAD_REQUEST`**, not a silent extra field. **Consequence for
planning:** adding `workspaceId` to a command that is `.strict()` is a *schema change* whose fixtures must be
re-derived, not a widening you can slip in. Read the params schema before assuming "just pass the id".

The same discovery carries the other half: `config.get` resolves `settings.activeWorkspaceId` **internally**
(`store/config.ts#loadConfig`), which is the coupling `plan/14` WI 4 exists to sever. A host can hold N
runtimes while every workspace-scoped *command* still answers for the active one — the servers were split
before the commands were.

## An assertion that needs a clean machine is testing the machine

Real case, 2026-09-21: a host test opened with `expect(await portIsFree(9271)).toBe(true)` for the companion
port, and **failed on the developer's machine** — a running `Bifurc.exe` (PID 58708) held it. That is not a
bug in the host; the port is *supposed* to be held by a running app.

**The fix is to compare a fingerprint, not a state.** Probe with `connect()` and report what happened
(`ECONNREFUSED` → `"free"`, otherwise a report), then assert **before === after**. On a clean machine that is
`free → free`; on a busy one `accepted → accepted`. It is **strictly stronger** than a status field either
way, because both readings are real binds — and it fails correctly if the host *does* bind the address
(a `listen()` on a held port fails rather than stealing it, so a host that tried would flip `free → accepted`
on a clean machine).

**The general rule:** before asserting "X is free/absent/false", ask what else on a real machine routinely
makes it so. If the answer is "the developer's own running app", the assertion is environmental. `netstat -ano
| grep <port>` then `tasklist /FI "PID eq <pid>"` identifies the holder; **do not kill it** — it may be the
user's live application.

## The vitest suite goes down for hours at a time, and it is never your change

Signature: *every* file fails at **collection** with `TypeError: Cannot read properties of undefined
(reading 'config')` / "Vitest failed to find the runner" — the runner is never installed, because
`createDefaultSuite()` reads `runner.config.sequence`. Hit 2026-09-17 and again 2026-09-19; **cleared on
its own both times.**

**Diagnose by running an untouched file** (`tests/proxy/decompressUtils.test.ts` or
`packages/protocol/src/commands/index.test.ts`); if that fails too, stop — nothing else you test is
meaningful. Ruled out both times: cold `node_modules/.vite`, all three pools, `--no-isolate`,
`NODE_OPTIONS`, orphaned workers, sandbox, node version, `TEMP`. **`--pool=vmThreads` "passes" and is NOT
evidence** — it silently does not apply `vi.mock`, so files that mock nothing look green (`TESTING.md`
§8). **Fall back to `npm run typecheck` + the standalone registry probe** (see the
`bifurc-engine-command-move` skill) and say plainly that the suite did not run.

## The engine's `.mjs` output is not loadable by plain Node

`tsup` is `bundle: false` (deliberate — module-level singletons) and emits extensionless relative
specifiers (`import "./config"`), so `import("@bifurc/engine")` is `ERR_MODULE_NOT_FOUND`. Probe the
**CJS** build; that is the shell's path. Any P9 child process importing the ESM entry will hit this.

## The app's `dist/` keeps every module a refactor moved away

**`tsc` never deletes its output.** Every file that left `src/` during P2 left a compiled copy in
`dist/`, and `files: ["dist/**/*", "package.json"]` ships all of them. Measured 2026-09-20, before the
fix: **92 of 116** `dist/*.js` had no counterpart in `src/` — **447 KB of a second, frozen engine inside
the asar.**

Three things make this a trap rather than a cleanup note:

- **It is invisible.** `dist/` is gitignored, the suite runs `src/`, and the stale files are
  unreachable — walking relative `require`s from `dist/main.js` and `dist/preload.js` reaches 23 files,
  **0** of them stale. Nothing fails, ever.
- **It is the hazard `bundle: false` exists to prevent.** The engine does not bundle because
  `store/paths.ts` holds the data root as module-level state; a second copy is a second
  `_dataRootOverride`. One relative `require` added tomorrow binds to it silently.
- **It drifts, and the drift reaches the artifact.** `dist/shutdown.js` — the pre-move copy of
  `packages/engine/src/shutdown.ts` — required `@bifurc/engine/companion/companionServer`, a module that
  no longer exists under any name. It was the *only* specifier that failed to resolve in a packaged
  bundle, and it is why `scripts/assert-packaged.mjs` exists.

`npm run clean:main` (→ `scripts/clean-main-dist.mjs`) removes everything in `dist/` except
`dist/renderer/` before `tsc` re-emits. **The renderer exclusion is load-bearing:** Vite emits
content-hashed bundles there and `dist/renderer` is not tsc output, so a "delete what has no source"
heuristic classifies the whole UI bundle as stale. Instrument for whether it is back:
`node scripts/pkg-check-reachability.mjs`.

**A measurement taken against an artifact that predates the thing being measured is not a measurement of
it.** `plan/12` recorded "a packaged build cannot resolve either specifier" from the 2026-09-10 asar —
built *before* the `@bifurc` dependencies existed, so `node_modules/@bifurc` was absent for a trivial
reason. Against the current tree electron-builder 26.8.1 resolves the workspaces fine (44 of 44
specifiers, `package.json` included). Re-measure before building the fix a stale number implies.

**`electron-builder --dir` runs from the agent shell.** Unlike `npm run dev` it does not launch
Electron — it copies files — so the GUI limitation does not apply. That is how the numbers above were
obtained, and it is the cheap way to check a packaging claim.

## A `*/` inside a JSDoc block ends the comment

`/** … `dist/**/*.js` … */` does not compile: the `*/` in the glob closes the docblock, and the error
points at a line far below it (`SyntaxError: Unexpected token '*'`). Hit twice on 2026-09-20 while
writing `scripts/assert-packaged.mjs`, which documents paths like `dist/**/*.js` and
`node_modules/@bifurc/*/package.json`. Write `dist` tree, or escape it — in this repo's comment-heavy
style the glob is common enough to keep hitting.

## `soap.execute` is the one standing suite failure, and it is environmental

A dead endpoint makes it **throw**, surfacing as `[INTERNAL] connect ECONNREFUSED 127.0.0.1:1`, where
the sandbox the test was written in answered `127.0.0.1:1` with a synthetic 404 — so the request
succeeded and the body carried a numeric `status`, which is what `tests/spike/protocolPoc.test.ts`
asserts. Recorded in `plan/baseline.md` "Environment caveats"; **not** "fixed", because changing which of
the two shapes `soap.execute` uses is a contract change.

## Tooling: two `Edit` calls to the same file in one message clobber each other

Both report success; the **second write wins** and silently drops the first change. Hit while wiring
slice 5: the import landed, the call site did not, and the file looked untouched where the first edit
had been. Nothing warns you.

**Edit one file at a time, in separate messages.** When a change spans two places in the same file,
either chain the edits sequentially or put both changes in one `old_string`/`new_string` pair. Verify
with a grep for the new call site before running anything — the failure mode is a *missing* line, so it
is invisible in a diff you are skimming. `EBUSY: resource busy or locked` is a different, benign
transient; just retry.

**Refined 2026-09-20 — it is a race, not "last wins", so "put the important one last" is not a
workaround.** Re-measured while correcting five stale claims across two plan docs: a two-edit message
lost one of two; a three-edit message **lost one and kept two** — and the survivor was the *middle* edit,
not the last. The number that land is nondeterministic and the tool reports success for every one,
including the ones it dropped. Two consequences: (1) the only reliable check is a grep for **every**
marker string you wrote, not just the last — a per-edit success report is worthless here; (2) after
re-applying, re-verify the edits that *did* land, because a later batch can clobber them in turn. Hit
again in this session despite this entry already existing, because `TRAPS.md` was not read before the
first write — which is the actual lesson: read this file *first*, not when something looks wrong.

## Tooling: a count-asserting edit script is not idempotent when `new` contains `old`

The entry above is why this repo's bulk edits go through a Python script that asserts a match **count** per
pair (see `scripts/p6-reduction-*.py`). That script has a failure mode of its own, hit on the reduction's
fourth group and caught only by re-running it.

The guard looked reasonable:

```python
found_old, found_new = text.count(old), text.count(new)
if found_old == 0 and found_new == n:   # "already applied" — WRONG
    continue
```

It is wrong for every **append-shaped** edit, i.e. `new = old + extra` — a supersede pointer appended to a
paragraph, a new section prepended to a heading, a progress note appended to a bullet. Applying such an edit
leaves `old` **still present**, as a substring of `new`. So `found_old == 0` never holds, the skip never
fires, and a re-run applies the edit a **second time**. On this group it duplicated `plan/07`'s new
§5b-quindecies (71 lines) and `plan/13`'s progress paragraph — both of which the first run had reported as
`ok`. Nothing warned; the second run printed `ok` again, which reads exactly like progress.

**The guard must test `new` only:** if `found_new == n`, the edit is applied — whether or not `old` survives
inside it. Whether a given pair is append-shaped is not something the caller should have to declare, and
"append-shaped" is the *common* case for a doc ledger (every supersede note and every inserted section).

Two habits catch it:

- **Make the script idempotent and re-run it as a check.** A correct script reports all-skips the second
  time. A duplicate is the signal — and it is the only signal, because the tool reports success either way.
- **After any bulk edit, grep the *count* of a marker that should appear exactly once**
  (`grep -c '^#### 5b-quindecies'`), not just its presence. Presence is what a duplicate has two of.

Related, and worth knowing when hand-editing the same files: the scripts read and write with
`newline=""`, so patterns must be translated to the file's own line ending. `src/**`, most of `tests/**` and
`plan/07` / `plan/13` are CRLF; `packages/**` and `plan/README.md` are LF. A multi-line `old` written with
bare `\n` simply **does not match** a CRLF file (found 0 — a clean abort), but a bare-`\n` *replacement*
written into a CRLF file leaves one mixed-ending line, which is silent.

## Git / env

- **NEVER `git stash`** (wiped the object store once). **Hit again 2026-09-21, and this time the
  mechanism is known.** Two separate hazards compound:
  1. **The repo root is the PARENT directory, not `Bifurc/`.** `git rev-parse --show-toplevel` returns
     `I:/workspace/Bifurc-Worskspace`, an **empty, commit-less repository**; the real repo is
     `Bifurc/.git` on branch `standalone-engine`. A command whose paths are relative to `Bifurc/` is
     therefore malformed for the repo git actually picks, and the operation is interrupted mid-write.
  2. **An interrupted `git stash` deletes `.git/refs`.** That is not cosmetic: git's repo-validity check
     requires `HEAD`, `config`, `objects` **and `refs`**, so with `refs` gone git declares
     `Bifurc/.git` "not a repository" and **discovery silently climbs to the empty parent** — after
     which *every* command operates on the wrong repo and `git status` lists the workspace *siblings*
     (`Bifurc-temp/`, `bifurc-studio/`, …) as untracked. It also leaves a stale `multi-pack-index`
     referencing packs that no longer exist (`failed to load pack in position 0/2/3/4` from `fsck`).
     **Recovery, and it is total:** recreate `.git/refs/{heads,tags}`; write the tip from
     `.git/logs/refs/heads/<branch>` (the **reflog is never touched** — it ends at the real HEAD hash);
     `rm` the stale `multi-pack-index` (verify the one real pack with `git verify-pack -v <idx>` →
     `… .pack: ok`); `rm .git/index && git reset` (the index referenced a never-written object).
     `git fsck` then reports clean. **Nothing was lost either time.**
  **The tell, and it must be checked before any write:** if `git rev-parse --show-toplevel` does not
  return the app directory, **stop**. To read an old version of a file, use
  `git show HEAD:<path> > /tmp/x` — never stash, never checkout. **`git fetch` doesn't persist remote-tracking
  refs** → never trust `origin/*`; use `git ls-remote`. **Verify old trees read-only** (`git grep
  <rev>`), never check out.
- **A push pops a GCM GUI window and dies** — `GIT_TERMINAL_PROMPT=0 GIT_ASKPASS=echo
  GCM_INTERACTIVE=never git push …`, then confirm with `git ls-remote`.
- **`npx tsc` is not the compiler** → `npm run typecheck` (it runs `build:packages` first).
- **`vitest run --project <name> <path>` is BROKEN (Vitest 4.1.5)** — drop `--project` when filtering.
  A failure uniform across unrelated files is an *invocation* bug — but **check an untouched file
  first**: during the outage above, plain `vitest run <path>`, a directory path and `--project` *all*
  failed, so a path-filtered failure is not by itself evidence about the filter.
- **NEVER launch Electron from the agent shell** (GPU process fails, exit 3); the **user** runs
  `npm run dev` / `test:e2e`. Re-measured 2026-09-20: `npx playwright test` gives **46 failed, 0 passed**,
  every test dying in ~850 ms at `firstWindow()` with `Target page, context or browser has been closed` —
  the exact numbers `plan/07` Preconditions already records. **A matching count is the confirmation, not a
  regression**; the three controls there still hold.
- **A repo-wide grep drowns in `coverage*/`.** Eleven generated coverage trees (`coverage/`,
  `coverage-baseline/`, `coverage-run3/` … `coverage-run17/`) hold ~600 HTML files each, and each is a
  **rendered copy of a source file** — so grepping for a renderer symbol returns 120 hits from coverage
  before the one real hit. Measured 2026-09-20 on `__APP_VERSION__`: 12 useful hits buried under ~110.
  **Scope every search to `src/ renderer/ packages/*/src tests/ e2e/`, or pass
  `glob: "!**/coverage*/**"`.** The same applies to `dist/` (compiled copies) and `release/` (extracted
  asars) — all three are generated, all three are large, none is a source of truth.
- **`npx playwright test` from the agent shell dies *before* any spec runs**, and the error is the
  safe-delete guard, not Playwright: Playwright wipes `test-results/` at startup, which is **139 entries**
  (past the 50/turn threshold) → `[safe-delete][SAFE_DELETE_BULK_CONFIRM_REQUIRED] … "targets":
  ["…\\test-results"]`. **This masks the real GPU failure**, so it reads as a new bug. Prefix the run with
  `CODEBUDDY_SAFE_DELETE_ENABLED=0` and the documented 46-failure result is what you get. Playwright also
  only deletes its own output dirs, so disabling the shim for that one command is safe.
- **`npx playwright test` needs no rebuild when `dist/` is current** — `test:e2e` is
  `npm run build && playwright test`, and the build is the slow half. Check first with
  `find src renderer packages/*/src -type f -newer dist/main.js` (empty = skip the build).
- **Safe-delete guard refuses past ~50 deletions/turn**, killing `build:packages`; **`git clean -fdX
  <path>` is the way through.** A failed `tsup` leaves entry points deleted — check `dist/index.js`.
  One `rm -rf` of a big cache dir exhausts the whole turn's budget (it reported `count: 9715` against a
  threshold of 50) and every later delete in that turn dies — the bypass is
  `env -u CODEBUDDY_SAFE_DELETE_BULK_STATE_DIR -u CODEBUDDY_TOOL_CALL_ID npm run …`. **That bypass
  silences the bulk guard but not the trash shim** — deletes still route to the Recycle Bin, and a bulk
  one can fail outright (`spawnSync … genie-trash\win32-x64.exe ETIMEDOUT`, hit 2026-09-20 while landing
  `clean:main`). **The two mechanisms are independent — you need BOTH for a build that deletes its own
  output, and neither one alone is "the hatch".** `CODEBUDDY_SAFE_DELETE_ENABLED=0` disables the shim's
  *trash routing* but **not** the bulk guard: measured 2026-09-20, a `build:packages` run with that flag
  set still died on `SAFE_DELETE_BULK_CONFIRM_REQUIRED {"count":4264,"threshold":50,"scope":"turn"}`.
  Only the `env -u` pair skips the guard, because `checkBulkDeleteGuard()` returns early when
  `CODEBUDDY_SAFE_DELETE_BULK_STATE_DIR` or `CODEBUDDY_TOOL_CALL_ID` is unset (shim
  `node-safe-delete-shim.cjs:190-210`). **The counter is per-turn and cumulative**, so the *first*
  `build:packages` of a turn usually succeeds and a *second* one in the same turn is what trips — do not
  run `npm run typecheck` (which re-runs `build:packages` internally) after a manual `build:packages`.
  When the rebuild is redundant, call the inner steps directly (`npm run typecheck:packages && npx tsc
  --noEmit`). The whole recipe is for *this shell only*; the scripts are fine for CI and for the user.
  `bundle: false`
  means every `tsup` run litters `packages/*/tsup.config.bundled_*.mjs` (32 accumulated).
- **Linking a new workspace package needs a manual Windows junction** (`New-Item -ItemType Junction`),
  then `node -e "require.resolve('@bifurc/<pkg>')"`.
- **A suite capturing `ipcMain.handle` needs `register*Commands(registry)` in `beforeAll`**, else
  `ENOENT` reads as an fs bug.
- **Uncommitted edits inside `src/` have been lost once** (the whole `src/` tree vanished mid-turn, cause
  never identified). Commit early when touching `src/`. Recovery is
  `git restore --source=HEAD --staged --worktree src/` — plain `git restore <path>` keeps the file
  deleted when the deletion was staged.

## A deletion pass is atomic with its test migration

Hit 2026-09-20 resuming P6 stage 3b. The nine protocol-client channels had been deleted and the
`invokeCommand` helper added to `protocolExecution.integration.test.ts` — but the 20 call sites still drove
the deleted channels, so **19 cases died with `No handler registered`**. Nothing in that output says
"unfinished edit"; it reads as a code defect, and the first instinct is to look for a broken registration
rather than at `git status`. **A half-landed deletion pass is not a smaller deletion pass.** The deletion
and its migration are one change; if you cannot finish it, revert it rather than leaving it staged.

**The half that is easy to get wrong silently is the argument shape**, because a one-line adapter has no
behaviour of its own to lose *except* how it calls the command. The nine were **not** uniform: five were
literal passthroughs, `soap:fetchWsdl` **wrapped** its bare `url` as `{ url }`, and three gRPC channels
passed `{}` while ignoring whatever they were called with. So a re-point written as a rest-args forwarder
(`invokeCommand(cmd, ...args)`) is right for five and wrong for four — and **three of the four would still
pass**. Pass the command's real params object explicitly, and say in the helper's docblock *why* it is not
generic.

## De-singletonizing: a facade over ONE lazily-created default instance, and the test that proves it

Established 2026-09-21 opening P14 (`plan/14` WI 2, the webhook receiver — §6 requires the factory shape be
rehearsed there before `proxy/server.ts`). When a module-level singleton becomes a factory, the caller set
must not change in the same step: `plan/14` §9's rollback is "keep the facade alongside the factory until WI 7
flips the shell." So the shape is **`createXxx()` returning an instance, plus the old exported functions
rewritten as a facade over one lazily-created default instance.** The webhook receiver's 10 exported names
stayed byte-identical and `tests/ipc/handlers.test.ts` (which `vi.mock`s the module) stayed 106/106.

Four things that are easy to get wrong, all hit or nearly hit:

- **Lazy, not eager.** `let defaultInstance = null; function defaultServer() { if (!defaultInstance) … }` —
  an eager `const defaultInstance = createXxx()` at module scope makes *importing* the module construct a
  server, the side effect the old `let server = null` deliberately did not have. It also computes
  `DEFAULT_WEBHOOK_PORT`'s value at import, so a `const` declared *below* the factory is a temporal-dead-zone
  throw rather than a compile error. **Declare the default constant above the factory** (TS won't catch the
  ordering, because `createWebhookServer` is a function body).
- **The shared `EventEmitter` must NOT become per instance.** It is a constant the shell's forward and the
  suite subscribe to; a fresh emitter per instance silently detaches both. Per-instance state is the
  *mutable* state only — the server, the port, the error, and the maps.
- **A cap that "becomes per workspace" stays a module constant.** Widening the *scope* of the thing being
  counted (each instance enforces its own map) is not the same as making the *number* configurable. Say which
  one you mean, or the next reader adds a constructor argument nobody passes.
- **Test instance isolation on IDENTITY, not on a value both instances could share** (`TRAPS.md`, "A test
  whose two inputs are indistinguishable passes on either branch"). Two instances on two ports, one suffix
  active in A and 404 in B, a bind failure landing on the loser only. The criterion `plan/14` §8 names is
  "starting runtime B does not observe runtime A's readiness" — the global-`isRunning()` trap — and only an
  identity assertion catches it.

**And the timing half, which cost a red run:** `start()` is fire-and-forget (`index.ts:140-144` — the servers
publish no readiness signal but the predicate), so `start(p); expect(isRunning()).toBe(true)` fails. The
original suite hid this behind a `setTimeout(r, 100)`. Poll a predicate with a deadline instead, and when a
case deliberately triggers a bind failure, **attach an `error` listener to the shared emitter first** — Node
throws `ERR_UNHANDLED_ERROR` otherwise and vitest reports it as an unhandled error separate from the tests.

### The larger instance — `proxy/server.ts`, and the three things it added

Done 2026-09-21 (`plan/14` WI 2 step 2). Same shape, ~1,600 lines instead of ~160, and it produced three
additional rules:

- **When the factory *body* is derived from the module-scope original, the original survives — and deleting
  the wrong copy compiles.** `dispatch` and `dispatchHttps` existed twice after the edit: as factory closures,
  and as the untouched module-scope functions they were copied from. TypeScript is happy either way **as long as
  the two have not drifted**, so the duplication is silent. `diff` the two bodies (normalise indent with `sed`
  first) and confirm the only differences are the ones you intend, *then* delete. Here the only intended one was
  the type-import form — the old file's `import type { TLSSocket } from "tls"` versus the factory's inline
  `import("tls").TLSSocket`.
- **Truncating a region can take an unrelated `export … from` with it, and only a build/typecheck catches it.**
  `export { replayRequest } from "./serverReplay"` sat one line under a section comment *inside* the deleted
  block. No test failed — its consumer is `routingUtils.ts`'s own import — but `tsc --noEmit` failed instantly
  with `TS2305: Module '"./server"' has no exported member 'replayRequest'`. **Treat a re-export block as part
  of the facade and re-attach it on the kept side of the cut; never assume tests cover an export's existence.**
- **A module-level mutable that is *also* another module's state does not become per-instance by wrapping it.**
  The TLS CA (`tlsCert.ts`'s `caCertPem` / `caKeyPem` / `certCache`) is loaded by `start()` and unloaded by
  `stop()` — so with N proxy instances, **B's `start()` overwrites A's CA and either one's `stop()` unloads it
  for both**, silently degrading the other workspace's HTTPS interception to tunnelling. The factory makes this
  *visible*; it does not fix it. Check every `loadX()`/`unloadX()` pair reachable from the constructor body for
  this shape, and hand the ones that are genuinely app-wide to the composition root (here: WI 3's host) rather
  than absorbing them into the instance.

**And one correction-not-code rule:** `currentPort` is not reset by `stop()`, only by the next `start()`, so a
`getPort()` assertion taken after a stop reads the *previous* port. The new case failed; the case was fixed.
**When pre-existing behaviour surfaced by a refactor contradicts a new test's assumption, change the test — the
alternative silently ships a behaviour change inside a "pure refactor" commit.**

### The CA fix, and where the ownership flag belongs (WI 3, 2026-09-21)

The trap above says "hand the ones that are genuinely app-wide to the composition root". **For the TLS CA the
right place turned out to be the factory, not the root** — because the root cannot express it. `tlsCert.ts`
has **no API for "I loaded it, someone else unloads it"**: the only lever is *whether `stop()` calls
`unloadCA()`*, and only the instance knows if it is alone. So `createProxyServer()` takes `ownsCA` (default
`true` = the pre-P14 desktop), a host member passes `false`, and the shared anchor survives its siblings.

Two rules this produced:

- **"Fix the ownership, do not delete the release."** Removing the `unloadCA()` call outright makes the
  hazard vanish and leaks the CA past the `stop()` it was written to bound — the desktop's single-proxy
  lifecycle genuinely needs it. **When a shared resource breaks under N instances, the question is "who owns
  the lifecycle", never "which line do I delete".**
- **Check the *other* branch of the same conditional.** The hazard was in `stop()`; the *same* bug sat in
  `start()`'s `else` — a non-owning instance whose own config has TLS off called `unloadCA()` and silently
  killed a TLS-enabled sibling's interception. Grep every call to the shared release function, not just the
  tear-down path.

### A memoised teardown means one test FILE holds one lifecycle

Hit 2026-09-21 adding `startCompanion: false`'s case to `engineSmoke.integration.test.ts`: the file **failed**
on `portIsFree(proxyPort)` after `stop()` and the same test **passed alone**. Cause: `shutdownEngine()`
memoises **process-wide** (`shutdown.ts:26`), and the engine is single-use — so the *second* engine in a
process awaits an already-settled promise, its `stopServer()` never runs, and the proxy port stays bound. The
file's own header documents the constraint; nothing enforced it.

**A lifecycle whose teardown is memoised process-wide gets one test file per engine shape.** Vitest gives each
*file* its own worker, which is what makes this work. Do **not** reach for `vi.resetModules()` to fit a second
lifecycle into one file — it hands the test a fresh copy of the store's module-level state and stops it
testing what ships. Assert the release (`portIsFree`) too, or the leak is invisible: the status flags said
"stopped" while the socket was still bound.

**Same class of fixture trap, twice more in `tests/proxy/server.test.ts`:** its file-level `beforeEach` gives
**every** `net.createServer()` call the *same* mock object, so two "independent" instances share `listening`
and one's `stop()` flips the other — any isolation-shaped case must re-mock with
`mockImplementation(() => makeMockServer())` in its own `beforeEach`. And its `vi.mock("fs")` makes
`fs.mkdtempSync` a silent no-op, so a real temp dir needs `vi.importActual("fs")`. **A test file's global mocks
and its shared fixtures are part of every case's environment — including the cases added a week later.**

### A module-scope `let` that a factory now shadows is dead code `tsc` cannot see

Found 2026-09-21, one commit *after* the refactor that orphaned them: `proxy/server.ts` still declared
`server`, `currentPort`, `currentConfig`, `fullRules`, `lastError`, `enabledSets` at module scope, while the
factory held its own copies of all six. Nothing read the module ones — **only the header comment still named
them** — so `tsc --noEmit` passed, every test passed, and the file looked de-singletonized while carrying six
misleading declarations. This is the same failure mode as the duplicated `dispatch` bodies above, in its
quieter form: there the copy *was* live and drift would compile; here the copy is dead and nothing at all
complains. **When a factory takes over module state, delete the module declarations in the same commit** —
grep each name for any reference outside the factory body, and treat "only a comment mentions it" as a hit.

## Deleting a phase's evidence artifact is a phase decision, not a pass decision

Found 2026-09-20: `tests/spike/protocolPoc.test.ts` (330 lines, 12 tests) had been deleted in the same edit
batch as the channel deletion, with **no note in `plan/` and none in the memory**. It is the runtime half of
P0 spike 4, and `plan/spike-results.md:192` records it as *"kept in version control as the record of this
de-risking work"* — so the deletion contradicted a written decision, silently.

**The cost that is easy to miss is the baseline.** It is where the documented 1 standing failure lives
(`soap.execute`), so deleting it would have turned the suite's "1 failed" into "0 failed" and quietly moved
the number every later reconciliation is measured against. A test file that is *the* known-failing one is
load-bearing in a way that is invisible from its size.

**The fix that keeps both halves:** re-attach the deleted channels in `beforeAll` with the exact bodies they
had. That is honest when the suite's subject is the *envelope* rather than the routing — the commands still
answer identically, two of them by a shorter path — and it doubles as the check that the deletion lost
nothing. Retiring throwaway spike code is a legitimate later decision, but it takes the `spike/` tree with it
plus every plan doc that cites it, so it belongs to a phase.

## A padded markdown table row mis-attributes its own cells

The `plan/README.md` status rows are single ~6k-char lines padded with spaces, and the Notes cell contains
`|` characters inside code spans — so `line.split("|")` does **not** return the table's columns, and
`cells[4]`/`cells[5]` land in the middle of the notes. Worse, the row's *shape* is the only way to notice a
missing cell: measured 2026-09-20, **P7's and P14's rows had 5 pipes where the table's other rows have 6**,
so each row's note sat in the **Gate** column with no Notes cell — the programme's source of truth
misattributing which phase a note described, for two phases, invisibly in a rendered table.

Two rules: **check a row's pipe count against its siblings before trusting a cell index**, and edit these
rows with a script that cuts after the *n*-th pipe (preserving the first *n* cells byte-for-byte) rather than
with a literal match — the earlier session's script got two rows wrong for exactly this reason, and a
rendered table gives no signal.

**Corrected 2026-09-20 (P6 stage 3b slice 2), because the `|` half of the entry above is overstated.** The
pipes inside those Notes cells are **escaped** — `[<>:"/\|?*\x00-\x1f]`, `\|\| "unnamed"` — and `\|` renders
as a literal pipe *without* ending the cell. So the escape is not the defect; the **unescaped** pipe is. P6's
row carried exactly two (`graphql:introspect|execute`, `soap:fetchWsdl|execute`), which pushed its notes into
extra columns; both are now `\|`. The corollary is a false-alarm source: a naive `row.count("|")` counts
escaped pipes too, so P6 measured **12** when only **2** were wrong, and P3/P4 measured 10/17 with **zero**
real defects. Count with `(?<!\\)\|` instead — a well-formed row is exactly the table's column count, 6 here.

## One `it` per array entry makes a deletion look like lost coverage

Found 2026-09-20 finishing P6 stage 3b. `tests/ipc/handlers.test.ts` builds `expectedChannels` and then
`for (const channel of expectedChannels) it(\`registers a handler for "${channel}"\`, …)` — at `describe`
level, so **each array entry is its own test**. Deleting 18 channel names therefore removed **18 tests**, and
the suite delta read `−18 tests / −18 passed`, which looks exactly like a deletion pass that threw away
coverage.

> **Sharpened 2026-09-21 by a mistake in the other direction.** The rule above is right, but it was then
> over-applied: in the sixth group the array went 7 → 2 entries, so the generated contribution was **−5**,
> and the *rest* of the file's movement (−20, hand-written blocks) was invisible to that reasoning. Two
> errors followed. **First, the array's length is not a file's test count** — quoting "−14 generated" from
> 16 → 4 entries counted entries as tests, when the entries that *remained* still generate their `it`s.
> **Second, counting `it(` lines in a `git diff` is not a count either**: 18 removed / 4 added suggested
> −14, while the file's real movement was **135 → 110**, because reformatting hides lines and generated
> cases have no source line at all. **Only a run reports a count.** Measure per-file with vitest
> (`Tests  N passed`), compare against the file's *own* previous run, and treat the total as a sum of
> measured per-file numbers rather than as a diff statistic.

> **And the baseline itself can move before the deletion starts.** This group opened with a fix-first commit
> (`174a01d`) that added **two `it()`s to `packages/client/tests/client.test.ts`**. The previous group's
> recorded total (2,484) predated it, so the true baseline was **2,486** — and a reconciliation against
> 2,484 came out **2 high**, which reads as "the arithmetic is *better* than reality" and is easy to dismiss.
> **A group that opens with a fix-first commit invalidates the inherited baseline; re-measure it.**

It had not. Those channels no longer exist, so a test asserting a handler for them would assert the *absence*
of the thing the list exists to check. **The rule: before reading a suite delta as lost coverage, find out
whether the removed cases were *generated from a data list*.** The signature is a delta that is an exact
multiple of the list you edited, with `failed` unchanged. Contrast the same pass's slice 1, where the delta
was **0** because those 19 cases were *re-pointed* at the surviving path rather than deleted — a re-point
preserves the count, a generated list shrinks with it.

And pair it with the inverse check: the deletion *did* break two cases, in a different file, and the count
went **up** (`1 failed` → `3 failed`). Both halves are needed — neither a delta of 0 nor one of −18 is
evidence on its own; the mechanism is.

## A suite can be getting its registrations as a side effect of a handler file

Found 2026-09-21, in the sixth reduction group's verification gate — and **the deletion itself gave no hint
it was coming.** Four suites failed with `No handler registered for command "request.replay"` /
`"workspace.add"`, errors that are *routed-surface* (the envelope reached the registry; the registry had
nothing to dispatch to) rather than transport errors, and every failure was in a file the pass had edited.

The mechanism: a suite that calls `registerSomeHandlers()` or a `*Kinds()` group **runs that file's module
scope**, and for four suites the handler files' module scope was quietly the thing that registered the
commands they drive. `server:status` is the cleanest instance — `systemHandlers.ts` called
`registerServerCommands()` at module scope, so the spike inherited it transitively, and it survived the fifth
group's deletion of `server:restart/stop/start` and only died with `server:status` in the sixth. Deleting the
channels removed a registration nothing else provided.

**The rule: a command whose only registration came from a handler file the reduction deleted must be wired
explicitly by every suite that drives it.** Concretely — **compare each suite's `register*Commands()` set
against `src/ipc/handlers.ts`, never against the suite's own history.** The suite's history looks fine; it is
the *shell* that changed.

**What makes this a trap rather than a chore is the error message.** `No handler registered for "config:save"`
names a channel that has been dead since §5c and points the reader at a file that no longer contains it, so
the instinct is to look for a deleted channel rather than a missing registration line.

Two adjacent misdirections found in the same gate, both mine and not the deletion's:
**`theme:*` / `zoom:*` are LIVE channels** in `clientHandlers.ts` (17 live / 0 dead — untouchable), shell-local
behaviour touching `webContents` and the title-bar overlay, so the reduction has no claim on them; inventing
`theme.get` / `zoom.set` commands for them produced `No handler registered for command "theme.set"`.
**And read a channel's positional signature off its call sites, never off the command's parameter list** —
`workspace:add` takes one argument (the name; the command generates the id), and mapping `(id, name)` gave
`Invalid payload for command "workspace.add": expected string, received undefined` *on `name`*. That is the
fifth group's argument-shift trap recurring in a new place.

## A count written into prose is never re-derived — and it propagates

Three separate instances in this plan, all the same shape. `plan/07` said **five** engine commands were
registered from the shell; the §5c pass measured **51**. `handler-classification.md` said **112** legacy
channels; the §5b-bis audit measured **132**, because the 48 that `entityCrudFactory` generated are
invisible to `grep ipcMain.handle("`. And **"six caller files"** — repeated four times across `plan/07`,
`plan/13` and the engine's docblock — was never true: it is **five**, and the sentence claiming six then
listed exactly those five names, so it disproved itself on its own line.

The mechanism is always the same: the number is written once when it is measured, then *quoted* by later
sections, and quoting is cheaper than re-measuring. A doc that quotes a count inherits its staleness
silently, because nothing renders an error.

Two rules. **When a section needs a count, re-measure it — or write "measured 2026-09-XX: N" so the reader
knows it is a reading and not an invariant.** And when a count turns out wrong, **correct it at the site and
say it was wrong**: the older docs still say six, so a silent fix leaves the next reader able to "restore"
it from a source that looks authoritative. Verify with history rather than arithmetic — `git log -S <symbol>`
settled this one in a single command and showed that six was never true, not even before some file was
merged away.

**A fourth instance, 2026-09-21, and it is the one to read if you only read one** — because the wrong number
was *derived* rather than merely quoted, and the derivation was arithmetic over the wrong base. The
dialog-owner risk subset was measured at **7** in §5b-decies; §5b-undecies widened the criterion and added
`tls:generate`, making it **8**; the first reduction group then deleted three of the eight, taking it to
**5**. But §5b-duodecies never restated the subset, so §5b-terdecies computed from *§5b-decies' 7* — two
sections back, past the widening *and* past the deletion — and wrote "shrinks **7 → 6**". Groups 3, 4 and 5
then inherited "6" by quoting, and it stood for four sections.

Two things distinguish this instance. **The error entered through an *omission*:** a section that does not
mention a number cannot be caught by re-reading it, so the next section reaches past it to the last place the
number appears. And **the count was not just stale, it was arithmetically wrong at the moment it was
written** — 7 → 6 was never a valid step, since one deletion of one channel from a set of eight cannot leave
six. A stale *reading* is undetectable from the page; a wrong *delta* is not, if you check that the delta and
the change agree.

The cheap defence, and it is the one that actually fired here: **re-derive from the artefact, not from the
prose.** `grep -n "dialog\." <the four remaining files>` returned exactly three sites in three files, which
settled the subset at **3** in one command. The ledger's four sections had all agreed with each other and
were all wrong.

**A fifth instance, 2026-09-21, for a suite whose cases are *generated* — and this one caused the `git stash`
disaster above.** P14 WI 1 added a scoped block to `packages/protocol/src/commands/index.test.ts`, whose body
is a `for (const action of actions)` loop emitting two `it`s per command. The new block added three
`it.each(SCOPED)` of ten plus two standalone assertions. The recorded delta was "190 → 216 (+26)", with **+5
left flagged as unaccounted** — and the reconciliation was being chased when the stash destroyed the store.

The true numbers, obtained by enumerating the run's own names (`--reporter=json`, then
`assertionResults[].title`, then classify by regex):

```
generated per-command pairs (92 commands × 2)   184
coverage invariant                                1
services.discover NOT scoped                      1
scoped block (3 × 10)                            30
                                      total     216      prior = 216 − 31 = 185,  delta = +31
```

**Both halves of the earlier figure were wrong, and the mechanism is the interesting part.** The prior count
was never 190: the 26 was obtained by subtracting only the 30 `it.each` cases from the post-change total and
attributing the residual to the change — so the *baseline absorbed* the coverage invariant and the non-scoping
assertion, two cases that existed on both sides. And the delta was then read as *smaller* than the block
added, which is what created the phantom "+5". **A count that is too low by exactly the number of
pre-existing cases looks like a missing test rather than an arithmetic slip**, and it sent me looking for a
test that did not exist.

**The rule: for a suite with loops or `it.each`, never count by hand and never subtract portions of an
edit — enumerate the run and classify by name.** The enumerable form is one command and is authoritative:

```
npx vitest run <file> --reporter=json --outputFile=/tmp/x.json
node -e 'const j=require("/tmp/x.json");
  const ns=j.testResults.flatMap(r=>r.assertionResults).map(a=>a.title);
  console.log(ns.length); console.log(ns.filter(n=>/ — accepts a valid payload$/.test(n)).length);'
```

This also removes the entire *reason* the stash was run: the prior count of a file can be had from the file's
own generator, without touching git. Reaching for `git stash` to answer a question about test code cost the
session's commit history.

## A table that stands in for a wrapper must not reproduce what the wrapper supplied

Found 2026-09-21 on the P6 reduction's fifth group, and **caught by the suite, not by review** — the failing
assertion was a `BAD_REQUEST` about a payload field, which looks nothing like a signature problem.

The pattern is the reduction's test migration: a deleted channel's body is reproduced by a table entry, and
the suite's `invoke` helper branches on the table before falling through to the real `handlers` map. The
deleted body's signature included the IPC event — `ipcMain.handle("capture:shareJson", async (_e, entries,
suggestedName) => …)` — so the table entry was written the same way. **But the branch it lives in *replaces*
the `fn({}, ...args)` call that used to inject the event**, so `_e` swallowed `entries` and `entries` received
the suggested filename:

```
BAD_REQUEST: Invalid payload for command "capture.shareJson": expected array, received string   ← path: ["entries"]
```

**A positional call is split across two places** — the wrapper injects the event and forwards the rest; the
table builds the payload from the rest. A table that stands in for the wrapper's *whole* body must therefore
take the caller's **positional** arguments and no event. The convention is visible in the two existing
tables (`tests/integration/deletedChannelHarness.ts`, `foldersAndWorkspaces`' `DELETED_FOLDER_CHANNELS`):
both are `(args) => payload`, neither declares an event.

**The generalisable part is the failure mode, not the fix.** Getting this seam wrong is a silent *shift* of
every argument by one, not a missing value — so no error names a signature, and the assertion that fires is
about whatever the misaligned argument happened to land in. When reproducing a deleted handler body, ask
which half of the call the *harness* supplies before copying the signature.

## A cast to the declared return type turns a contract into an assertion — and `transport` is a claim

Found 2026-09-21 starting the P6 reduction on `tlsHandlers.ts`. Two mechanisms, one bug.

**The bug.** `window.api` is `{ ...client }`, so `window.api.tlsGenerate()` is the *client's* method. It was
`() => call("tlsGenerate", {}) as ReturnType<BifurcApi["tlsGenerate"]>`. The engine answers `{ok, fingerprint}`
and deliberately reports no path (`File_Ops_Protocol.md` §8), while
`renderer/panels/settings/TlsSettingsSection.tsx:67` requires one:
`if (result.ok && result.certPath && result.keyPath)`. So the guard failed, `handleGlobalChange` never ran,
`config.tlsCaCertPath` stayed null, and the panel kept saying "not configured" — **the Generate button was a
no-op from the user's side, with the CA sitting on disk.** `tlsImportCert` / `tlsImportKey` had the same hole
on `result.path`. Introduced by `8d0b81a` (P6 step 3b-1); `git show 8d0b81a^:src/preload.ts` proves the
pre-routing form was `ipcRenderer.invoke("tls:generate")`, the channel that computed the paths.

**Mechanism 1 — the cast.** `call()` returns `unknown`, so a cast is unavoidable *at that call site*. The
mistake is casting the **whole returned object** to the declared type: `as ReturnType<BifurcApi["tlsGenerate"]>`
is not a check, it is an assertion that silences the check. `renderer/types/window.ts:185` declares
`certPath` / `keyPath`; the compiler was willing to enforce that; the cast told it not to. **Cast the `unknown`
from the boundary, never the object you build on top of it** — then the declared type does the work.

**Mechanism 2 — the classification.** The three keys were filed `transport`, defined in `surface.ts` as "a 1:1
mapping onto a protocol command". That filing *is* the claim "nothing happens beyond the call", and it was
false. `shim` is the kind that says otherwise. A misclassification is therefore not a bookkeeping error: it
can license a routing decision that loses behaviour, and then keep licensing it.

**Why nothing caught it.** `surface.test.ts` asserts **keys** and per-kind **counts** (144, and the totals);
`handlers.test.ts` asserts that every routed key names a *registered command*. Neither looks at a **result
shape**, and a reclassification that preserves the total is invisible to both. The suite was green through a
broken primary flow.

**The rule for the reduction, and it generalizes past it.** Before deleting a dead channel, the question is
not *does a twin exist* — it is **what does this body do that the twin does not**, asked of the *fields* as
well as the behaviour. And the risk criterion is **"owns a step the engine does not perform"**, of which a
`dialog.*` is only the loud instance; **synthesising a field the engine refuses to report** is the quiet one.
Reachability and behaviour-freedom are orthogonal: `tls:generate` was unreachable *and* was the last correct
implementation of a live behaviour. When the dead code is the only correct copy, deleting it does not lose a
behaviour — it destroys the record of one the live path had already lost.

**The audit that finds these without reading every channel:** for each `window.api` result the renderer
destructures, find the client method that must supply it and check the method's *actual* return shape against
`renderer/types/window.ts`. Grep the renderer for `result.<field>`, then trace the field to its producer.

## `git rm` deletes the whole `src/` tree in this repo — use `rm` + `git add`

Found 2026-09-21, **twice**, on the first attempt to delete a file as part of the P6 reduction. The command
was `git rm src/ipc/handlers/tlsHandlers.ts`. It reported success — `rm 'src/ipc/handlers/tlsHandlers.ts'` —
and `git status` then showed **all 22 tracked files under `src/` as deleted**: `src/main.ts`, `src/preload.ts`,
every handler, everything. `find src -name "*.ts"` returned 0. Nothing was lost (git had it all), but the
working tree was destroyed, and the second attempt reproduced it exactly.

The first hypothesis was the sandbox's safe-delete guard (`CODEBUDDY_SAFE_DELETE_BULK_STATE_DIR`), because the
commands that ran with `env -u` on those two variables were unaffected. **That was wrong**: running
`env -u CODEBUDDY_SAFE_DELETE_BULK_STATE_DIR -u CODEBUDDY_TOOL_CALL_ID git rm …` wiped it a third time.

The isolating experiment is the part worth remembering, because it took one command and settled it — restore,
then delete the same file with **plain `rm`**, then re-count:

```
git checkout HEAD -- src/                        → 23 *.ts under src/
rm src/ipc/handlers/tlsHandlers.ts
find src -name "*.ts" | wc -l                    → 22   ← the file is gone, the tree is intact
```

**So the rule is: never `git rm` in this repository — `rm <file>` then `git add <file>`** (or `git add -A`
after a batch). No hook and no script is involved: `.git/hooks` holds only samples, and the one script that
deletes anything (`scripts/clean-main-dist.mjs`) is hardcoded to `path.resolve("dist")` and never ran. The
cause is probably the `I:` drive rather than git itself — but the workaround holds either way, and the
diagnostic is what turns a mystery into a one-command check.

Two habits this should leave behind. **Restore before diagnosing** — `git checkout HEAD -- <path>` is
non-destructive when the work was never committed, and diagnosing on a damaged tree wastes the effort. And
**verify the tree immediately after any bulk-deleting command**, so the damage is attributed to the command
that caused it rather than to whatever ran last.

## A reserved port that includes its own base is a silent off-by-one

`allocatePorts()` gained a `reserved` set so the scan would not hand out ports the host was about to bind
itself (the RPC endpoint, a not-yet-started workspace). The first version also put both *bases* in it — and
a base is the starting point of its own range, so `findFreePortFrom(base)` skipped `base` and returned
`base + 1`. Every base-path allocation was off by one.

**Why it survived a full-suite run and an integration test.** A runtime on `base + 1` still comes up and
still serves. `tests/integration/workspaceHost.integration.test.ts` observed ports `81` and `82` and the plan
text wrote that up as "D16's `80`-base plus contiguous scan" — which is what it *looked* like. The assertion
that would have caught it is on the **identity of the first port**, and the host test did not make it. Only
`tests/runtime/portAllocator.test.ts` did, and it failed immediately: `expected 52008 to be 52006`.

**The second attempt reserved only the other range's base** — which fixed the proxy half and broke the
webhook half identically, because a base is a base whichever range it belongs to. Reserving **neither** is
the rule; cross-range safety comes from `reserved.add(proxyPort)` *after* the proxy is chosen, which is
strictly stronger than avoiding one hypothetical collision point. The diagnostic that settled it was
instrumenting the real test file to print `base`, `proxyPort` and `tryBind(base)` — the proxy half was
already correct and the failure had moved to the webhook.

**Two process lessons.** A probe file written by hand is *not* equivalent to the failing test: my first three
probes all passed while the real file failed, because the divergence was inside the real test's own
assertions, not in the module. Copy the failing file and instrument *it*, rather than reconstructing it.
And `packages/engine/dist/` is what some suites actually load — `vitest.config.ts` aliases
`@bifurc/engine/*` to `packages/engine/src/`, but I chased a stale `dist/` for several runs on the strength
of that comment alone. Verify which file is loaded (`grep` the built output for the line you just changed)
before theorising about behaviour.

## `git stash` is destructive on this drive too — it can take the object store with it

**The second member of the `git rm` family, found 2026-09-21, and worse.** The command was an attempt to
measure a suite's *prior* test count by hiding one file:

```
git stash push --keep-index -q -- packages/protocol/src/commands/index.test.ts
```

It left the repository with `fatal: bad object HEAD`. `git log`, `git status` and `git diff` all failed.
The damage was not to the working tree — **every file was intact** — but to the store:

- `.git/refs/heads/standalone-engine` still held the right sha (`df3cdaf…`), and `.git/logs/HEAD` held a
  complete reflog naming every commit of the session.
- **Not one of those shas resolved.** `git cat-file -t` failed for `df3cdaf`, `11ee39f`, `63e8a40`,
  `9ef3f3e`, `6b8ac10` — all five commits, plus every tree and blob they referenced.
- The pack verified `ok` but held only **40 pre-session commits**; the session's commits had lived as
  **loose objects**, and `.git/objects/df/` no longer existed.
- `git fsck` reported only `refs/heads/standalone-engine: invalid sha1 pointer …` and reflog errors — the
  pack itself was healthy, which is what makes this look like ref corruption rather than data loss.

**Recovery, and it is worth having the procedure written down because nothing in `git` suggests it works.**
The trick is that the *ref* was broken while an *older* commit was still perfect. Find the newest readable
commit, repoint the branch by writing the file directly (no porcelain command will run while `HEAD` is
unresolvable), then rebuild the index:

```
# 1. Find the newest commit that still resolves. Do NOT loop with a `while read` over the reflog —
#    it hit the 120 s foreground timeout; use `git cat-file --batch-check` over the shas instead.
tail -60 /tmp/reflog_pairs.txt | awk '{print $2}' | sort -u \
  | git cat-file --batch-check          # "commit" vs "missing" per sha

# 2. Verify the candidate is not just readable but COMPLETE — a commit object can survive
#    while its trees do not:
git ls-tree -r <candidate> | awk '{print $3}' > /tmp/blobs.txt
git cat-file --batch-check < /tmp/blobs.txt | awk '{print $2}' | sort | uniq -c   # want: N blob, 0 missing

# 3. Repoint the branch. `git update-ref` and `git reset` both fail here; write the ref file.
cp .git/logs/HEAD /tmp/reflog_HEAD_backup.txt
printf '%s\n' <candidate> > .git/refs/heads/standalone-engine
git rev-parse HEAD                            # now resolves

# 4. Clear the poisoned reflog, then rebuild the index. `git reset --mixed` FAILS: it reads the old
#    index, which still points at the missing blobs. Remove the index and re-read the tree.
git reflog expire --expire=now --all
rm -f .git/index && git read-tree HEAD
git status --short                            # the whole working tree shows as changes; nothing lost
```

**What this cost and what it left.** The five missing commits are **unrecoverable** — `git fsck`'s dangling
commits were all dated 2026-09-06 … 2026-09-15, i.e. everything the session produced was inside the
destroyed objects, and no reflog or `ORIG_HEAD` points at an intact one. What survived is the **working
tree**, which is the real work; the commit *history* for P14 WI 2–5 plus P6 groups 6–9 was lost. Had the
tree also been lost, this would have been a multi-day session gone.

**So: never `git stash` in this repository.** The task that motivated it — measuring a suite's prior test
count — has a non-destructive answer and should never have needed git at all: enumerate the current run and
classify by name. `git stash` was reaching for `git` to answer a question about *test code*, and the same
information was available from `npx vitest run <file> --reporter=json` (see the next section, where the
count that motivated the stash was also wrong). Note also that **`git rm` and `git stash` are both in this
family** — the common factor is a git command that rewrites the index or the ref on a data drive that
appears to be doing something unusual with hardlinks. Treat all of them as destructive here and prefer the
plain-filesystem equivalent plus `git add`.

## Namespacing a store surfaces every *half-threaded* id, and only the integration suite sees it

Found 2026-09-21 during P14 D17 (blob staging becomes `<dataDir>/blobs/<wsId>/`). The change is
mechanical — add an optional `workspaceId` to eleven functions and four commands — and the unit suites
stayed green throughout. What failed was different in kind:

`tests/integration/importExportBlob.integration.test.ts` went from **0 → 30 failures**, every one a
`blob-not-found` on a round trip that had passed minutes earlier. The cause was not the store: it was
that `export.create` had been given `TEST_WS` while its own client's `blob.put` / `blob.read` /
`blob.release` had not, so the engine staged bytes into one namespace and the client read from another.
**The engine's own export was unreadable by the client that requested it** — a real latent bug that a
host-wide root had been hiding, because with one namespace every id resolved regardless of who asked.

**Three things this establishes, and the third is the expensive one.**

1. **A namespace change is not additive even when every signature is optional.** The old behaviour
   (omitted = the host-wide root) is exactly what makes the half-threaded call sites *look* correct in
   isolation. Test coverage that passes for both the scoped and unscoped path is not coverage of the
   transition.

2. **Fix by scoping the caller, never by relaxing the store.** The tempting repair — make `blobRoot()`
   fall back to the shared root when the blob is not found under the given workspace — would have made
   all 30 failures go away and destroyed the property D17 exists to create. The store's job is to be
   strict; the caller's job is to say who it is.

3. **Grep for *call sites*, in the shape the code actually has, and expect the first batch to be
   incomplete.** My first substitution pass matched the obvious `statBlob(params.blobId)` and missed
   **three** more sites in the same file because they sat inside a different wrapper
   (`await entry.pathImporter.run(...)` vs the preflight twin), and it never matched
   `publishArtifact(...)` at all because that call simply had no fourth argument to substitute. A
   mechanical find-replace over a threaded parameter is only as good as the regex, and the regex was
   written against the shape I *remembered*. The audit that finally worked asks the opposite question —
   *"which calls pass NO id?"* — because that is the set the substitution cannot reach:

   ```
   grep -rn "statBlob(\|readBlob(\|blobContentPath(\|releaseBlob(\|putBlob(\|putBlobBytes(\|createStaging(" \
     --include=*.ts packages/engine/src/ src/ \
     | grep -v "blob/store.ts\|blob/sweep.ts" | grep -vE "workspaceId|function |import |\* "
   ```

   The two remaining hits it produced were both real and both **deliberate omissions** (see the next
   section) — an audit that returns a short, *explicable* list is the useful outcome, not a clean one.

**Corollary for the plan's own hazard notes:** `plan/14` warned that the `blob.*` `invalid` fixtures
would need re-deriving because a fixture whose invalid payload is the newly allowed key silently stops
testing rejection. That hazard was **live but not hit** — the fixtures happened to be fine, and the
exposure was entirely in the call sites the note did not mention. A hazard note names where the author
was looking, not where the bug is.

## An optional param that is *accepted and ignored* reads as implemented

Found 2026-09-21, P14 WI 1 remainder, by probe rather than by reading. After the protocol half added an
optional `workspaceId` to `server.start` / `.stop` / `.restart` / `.status` and `proxy.status`, the
schemas accepted it — but `registerServerCommands()` still called the module **facade**
(`startServer(port)`, `isRunning()`, `getPort()`), whose lazy default instance is created with
`workspaceId: null` and therefore means *the active workspace*. Measured on a host running two workspaces:

```
host started:  probe-ws-a: proxy=81 webhook=9101     probe-ws-b: proxy=82 webhook=9102
server.status                → { running: false, port: 0 }      ← two members are listening
server.start {workspaceId: A} → { ok: true }                     ← accepted, ignored
server.status                → { running: false, port: 8791 }    ← a THIRD server, on the config port
```

**The failure mode is the one that costs the most to find: it succeeds.** `server.start` returned
`{ ok: true }` after binding a server nobody asked for. Nothing threw, nothing logged, and no suite
failed — because the module facade is *correct* for the desktop (one proxy, meaning "whatever the user
selected"), so every existing test passes. Only asking the question of a two-workspace host exposes it.

**The rule: an optional parameter is not `done` when the schema accepts it — it is done when the body
reads it.** For a wire contract the two are independently verifiable and only the first is visible from
the protocol package. So a protocol change that adds a field must be paired with either (a) a call-site
change, or (b) an explicit record that the field is accepted-and-pending, written where the *handler*
lives. Leaving it implicit makes the protocol advertise a capability the engine does not have, which is
worse than not having the field: a host member's client will send the id, get `{ ok: true }`, and manage
a runtime it never reached.

**And the same applies to a facade meant to *not* be used.** `proxy/server.ts`'s module functions are
documented as "removed in WI 7", but until then they are what every command calls, so the facade is the
live implementation for the RPC surface. A header that says a path is temporary does not stop it being
the path.

**RESOLVED 2026-09-21 (WI 7) — and the fix's shape is the reusable part.** A module-level
**runtime registry** (`runtime/runtimeRegistry.ts`) that `createWorkspaceHost()` populates as it creates
each runtime and clears on `stop()`; the commands resolve through it. Three outcomes, and **the third is the
one a naive fix omits**: a registered runtime for the named workspace → that runtime; no runtime but the
target *is* the active workspace → the **facade** (which genuinely *is* the desktop's server, so the
desktop is unchanged); anything else → **`absent`**, reported as such. Letting the third case fall through
to the facade would re-create the original bug — acting on the active workspace while the caller named
another — with a success code.

**Mutation-check both branches, not the suite as a whole.** Disabling the runtime branch killed **7 of 8**
cases; making `absent` fall through to the facade killed exactly the **2** that should die. That second
mutation is the valuable one: it is the branch whose absence *looks* like it works, and a suite that only
mutation-tested the first branch would have shown 7 reds and looked thorough.

**One trap inside the trap, found by writing the fix.** `proxy/server.ts` seeds `currentPort = 0` but
`webhookServer.ts` seeds it with `DEFAULT_WEBHOOK_PORT` (**9101**). So a "has it ever started?" guard
written for the proxy does **not** transfer: a never-started webhook reports a *plausible* port, and
re-binding it could take **another workspace's**. The rule that does transfer is sentinel-independent —
`restart` re-binds only a port it has **observed** running; `start` binds only a port the **caller named**;
otherwise report rather than guess.

## A sandboxed preload that requires anything but `electron` cannot load — and the failure looks like flakiness

Found 2026-09-21 while trying to run `npm run dev` — **not** P14 work, despite P14 being the phase in
progress. The window opens and the UI renders, but every `window.api` call is undefined. CEF devtools:

```
Unable to load preload script: I:\...\dist\preload.js
Error: module not found: @bifurc/client
    at preloadRequire (VM4 sandbox_bundle:2:144196)
    at runPreloadScript (VM4 sandbox_bundle:2:144465)
    at executeSandboxedPreloadScripts (VM4 sandbox_bundle:2:143738)
```

**The mechanism, read out of Electron 42's own bundle.** `src/main.ts` sets `contextIsolation: true` and
`nodeIntegration: false` and never mentions `sandbox`, so the default (`true` since Electron 20) applies.
A sandboxed preload does not run under Node's module system: the browser process reads the file and hands
`{ filePath, contents }` to `executeSandboxedPreloadScripts`, which evals it with a `require` bound to
`preloadRequire`. That function is, in full:

```js
function preloadRequire(e, t) {
  if (e.loadedModules.has(t)) return e.loadedModules.get(t);
  if (e.loadableModules.has(t)) { const r = e.loadableModules.get(t)(); return e.loadedModules.set(t, r), r }
  throw new Error(`module not found: ${t}`)
}
```

`loadableModules` is the module table of Electron's *own* webpack bundle — `electron`, `events`, `url`,
`timers`, punycode. **There is no `node_modules` resolution and no filesystem resolution at all**, so a
bare specifier and a relative `./sibling` fail identically; the error just names whichever appears first
in the file. `grep -a -o 'preloadRequire.\{0,600\}' node_modules/electron/dist/electron.exe` is how to
read this rather than guess at it.

**Why it worked for the project's whole life, then stopped.** `build:main` is `tsc` + `tsc-alias` with no
bundling, so a bare import survives into `dist/preload.js` verbatim. Until P6 that was harmless:
`git show bae3732:src/preload.ts` is a self-contained 196-line table whose **only** import is `electron`.
Commit `1268bfd` (2026-09-18 17:36, "route config:get through the RPC bridge") added
`import { createClient } from "@bifurc/client"` plus `./ipcTransport` — and `./ipcTransport` pulls in
`@bifurc/protocol` and `@bifurc/engine/transport/types` (real runtime values: `toRpcError`,
`remoteErrorToEngineError`, `BUS_NAME_BY_WIRE_NAME`). The last playwright report on disk is 16:32 the
same day. So the breakage is P6's, not P14's, and the archaeology that proves it is three `git log -S`
calls plus `git show <commit>:src/preload.ts`.

**The rule: for a sandboxed preload, "it compiled" says nothing — the only valid preload is a bundle.**
`tsc` emits a module graph, and a sandboxed preload has no resolver. The one-line check:
`grep 'require("' dist/preload.js` must return exactly one line, `electron`. Corollaries: the `@bifurc/*`
specifiers that `tsconfig.json` deliberately leaves bare are fine for `main.js` (Node resolves them) and
fatal for `preload.js`; the same unbundled file ships in the asar, so packaging does not hide it; and
`tsc` will not report anything, so no typecheck or unit suite can catch it.

**The trap inside the trap: it reads as environmental flakiness.** A non-booting app fails *every* e2e
spec, so a result like "46 failed / 0 passed" gets carried as a known-bad baseline — here, as P6
criterion 2 "CARRIED" — instead of being investigated as one defect with one cause. **When a whole suite
fails rather than a slice of it, suspect the boot path before the tests.** A green unit suite is not
evidence against it: `window.api` is only exercised in a real Electron renderer.

**And the packaging guard already walks `dist/preload.js` — while checking a property the preload does
not use.** `scripts/assert-packaged.mjs` queues **both** entry points (`dist/main.js` *and*
`dist/preload.js`, line 203) and then resolves every specifier with **Node's** algorithm,
`createRequire(<the requiring file>).resolve`. Inside the asar that *succeeds*: electron-builder 26.8.1
dereferences the npm workspace, so `node_modules/@bifurc/{engine,protocol,client}` are in the archive and
"44 of 44" `@bifurc/*` specifiers resolve (measured 2026-09-20, `plan/12`). So the guard **passes on an app
that cannot boot**, and `release.yml` runs it before upload. The blind spot is structural, not a bug in the
script: *resolvable by Node* is the wrong property for a file the sandboxed renderer loads. **The
assertion that would catch it is the opposite shape** — `dist/preload.js` must contain exactly one
`require(...)` and it must be `electron`; anything else means the file is a module graph and needs a
bundler. Worth generalising: when a guard walks a graph, check *which* resolver the guard uses against
*which* resolver the runtime will use. Two entry points in one walk can need two different properties.

> **FIXED 2026-09-21 (`e8e535f`).** See *"A sandboxed preload has NO module resolver"* below — that is the
> canonical entry for this defect and carries the fix, the build-time guard, and the two findings that came
> out of verifying it. This entry is kept for the archaeology (`git log -S` chain to `1268bfd`) and for the
> packaging-guard blind spot, neither of which the other one covers. **Two entries for one defect is one too
> many; if you are reading this to fix something, read that one first.**


## Deleting a test is a coverage claim — and "is X tested elsewhere?" cannot be answered by counting files

Found 2026-09-21, retiring the P0 spike (`spike/protocol/` + `tests/spike/protocolPoc.test.ts`, 12 cases).
The deletion was correct, but **two of the twelve cases would have been silently lost by a naive delete**, and
the check that found them is not the obvious one.

**What "the obvious check" gets wrong.** The tempting test is *grep for the command name across `tests/`, count
the files that mention it*. It failed in **both** directions on the first pass:

- **False negative — `mock.delete` looked uncovered** (0 files outside the spike). It *is* covered, in
  `tests/integration/settingsMutations.integration.test.ts`, under its engine name **`entity.delete`**. The
  spike used the legacy channel name; the real suite uses the command name; both are the same behaviour.
- **False positive — `INVALID_PARAMS` looked covered by implication.** `grep -rn INVALID_PARAMS` over all source
  resolves to **only** `spike/protocol/*` and the spike test. The real system answers **`BAD_REQUEST`**
  (`tests/ipc/handlers.test.ts` pins it extensively). So that case was the spike testing **its own throwaway
  transport** — the one case where deletion genuinely removes an assertion, and the assertion was about code
  that no longer exists. Counting files that mention a string cannot tell "this contract is pinned" from
  "this string only ever existed here".

**Read the assertion, not the name.** The two that mattered were only visible by opening the tests:
`soap.execute`'s dead-endpoint contract is pinned in `tests/integration/protocolExecution.integration.test.ts`,
and `tls.generate`'s renderer-facing `certPath`/`keyPath` shape is pinned in
`packages/client/tests/client.test.ts:1176`. Both were **better** than the spike's version, which is the point:

**The retirement was safe because each contract had a better home, and that had to be established per case.**
Not because the code was old, and not because the file was marked "throwaway, do not productionise" — a header
saying that is a statement of intent, not a coverage audit.

**Two sub-lessons worth keeping.**

1. **A test can be the *only* pin on a contract and still be the wrong test to keep.** `soap.execute` was the
   suite's one standing failure *because* it asserted against `127.0.0.1:1` — the address this sandbox
   intercepts — so its result depended on which answer the host gave. The replacement asserts the same two
   shapes (throw vs resolve `{ok:false}`) against `deadPort()`, which starts a real server, takes its
   **OS-assigned** port, closes it, and returns that port. **Same contract, firmer fixture.** When retiring a
   flaky test, look for the version that does not depend on the environment, and prefer replacing it over
   either keeping the flake or dropping the contract.

2. **A citation that makes a claim about where coverage lives must be updated when coverage moves; a citation
   that records provenance should not be.** The spike had ~16 citing documents and only **one** was the first
   kind (`soapCommands.ts`'s "which is exactly what `tests/spike/protocolPoc.test.ts` pins"). The rest
   ("validated in the P0 spike", "the productionised form of `spike/protocol/shape.ts`") are *historical and
   still true* — rewriting them would erase the fact that the shape had a predecessor worth naming. So the
   edit set is: **update the coverage claims, annotate the provenance, delete neither.** Getting this backwards
   produces either a tree full of dead references or a history that has been quietly rewritten.

**And the count, because this repository counts.** The plan predicted the retirement would take "`spike/` plus
**five** plan docs". Re-derived 2026-09-21, it was **nine** documents citing the spike's *code*
(`spike/protocol`, `protocolPoc`, `spike:typecheck`, `spike/tsconfig`) — **sixteen** mention the word "spike" in
some sense, and the difference is not pedantry: the other seven are naming *other* spikes (1/2/3) or using the
word generically, and treating them as citations would have meant editing documents that were already correct.
Six source files cite it too. The prediction was never re-derived — the same failure mode the "a count written
into prose is never re-derived" section records, now with a sixth instance, and the *shape* was right while the
*magnitude* was ~2× off. **A count that is roughly right is the most dangerous kind: it is close enough to be
believed and wrong enough to be useless as a work estimate.**

## An *additive* response field is still a user-visible change when a test pins the payload

Found 2026-09-21, P14 WI 7. The work item's acceptance criterion is P6's rule — **zero user-visible
change** — and the natural reading of "additive" is that it cannot violate that: adding a field breaks no
caller, so it is safe. **The suite disagreed, correctly.**

`server.status` gained an unconditional `workspaceId` echo. It failed in `tests/ipc/handlers.test.ts:764`:

```
expect(commandRegistry.invoke("server.status", {}, ctx)).toEqual({
  running: true, port: 8080, error: null,
})
```

`toEqual` on an object literal is an **exact** assertion, so the extra key is a failure. That test's own
comment is the reason it exists — *"two views of the same `isRunning()`, with deliberately different
payloads — collapsing them would silently change one of `serverStatus` / `proxyStatus`"* — so it is a
deliberate pin on the desktop's payload, not incidental strictness.

**The code was wrong, not the test.** The field's documented purpose is *"echoed so a multiplexed client
can match response to request"* — and **a client that is multiplexing is exactly a client that sends an
id**. So the echo became **conditional**: present when the caller supplied a `workspaceId`, absent
otherwise. The desktop sends `{}`, gets a byte-identical payload, and a multiplexed client gets the
disambiguation it needs.

**Two transferable rules.**

1. **When an existing test fails on your additive change, the first question is whether the test is
   pinning a contract you were not aware of — not how to update it.** Here the pin was deliberate and
   documented in a comment; "additive so it must be safe" was an assumption about *callers*, and the test
   was making a claim about *payload shape*. Both can be true and the test still wins.
2. **"Additive" and "no user-visible change" are different claims, and only the second was the
   requirement.** An extra key is invisible to a caller that destructures; it is not invisible to one that
   compares, serializes, hashes, or asserts. When the criterion is *zero* change, the safe default is to
   make the addition **conditional on the caller asking for it** rather than to argue it is harmless.

**And the corollary for the test suite: a `toEqual` on a whole response is a shape contract, and it should
say so in a comment.** This one did, which is why the failure was diagnosable in one read instead of
looking like an over-strict assertion someone should loosen.

## A test's own fixture can carry the race the code under test was written to avoid

Found 2026-09-21, full-suite run after P14 WI 7. `tests/runtime/portAllocator.test.ts` failed on
`still takes each base itself when a reservation is supplied (regression)` — **not** the load-induced
`gitStore`/`gitOps` pair, so it had to be identified rather than assumed. Isolated: **21/21 passed.**

**The mechanism, and it is the irony.** That suite exists to pin D16's rule that the allocator must be driven
by a **real `listen()`** and never by a probe-and-close, because a probe is a TOCTOU race. Its fixture does
this:

```
const base = await freePort();        // bind(0) → read the OS-assigned port → close
const out = await allocatePorts({ proxyBase: base, ... });
expect(out.proxyPort).toBe(base);
```

`freePort()` is **bind-then-close — the exact shape D16 forbids the allocator from using.** It is fine in a
fixture only while nothing races it, and under a full parallel run something can: the OS may hand the same
ephemeral port to another worker's `freePort()`, or to a server another suite is starting. When that happens
`allocatePorts` **correctly scans past the base**, and the assertion fails for a reason unrelated to the
regression it guards.

**What made it surface was an unrelated change.** WI 7 added `workspaceCommandScope.integration.test.ts`,
which binds two more ports per run. The case had passed every full run before that. **So "it was green
yesterday" is not evidence about a test whose fixture races — added load is enough to expose it.**

**The fix, and why it is not a retry that hides bugs.** The two causes are distinguishable:
**the regression leaves the base free** (the allocator skipped it); **a race leaves it held** (someone else
has it). So the retry runs *only* while some missed base is genuinely occupied, and a base that is
free-but-skipped is returned unchanged so the assertion still fails. Verified both directions:

- re-introducing the original bug (`reserved.add(proxyBase)`) → **3 tests fail**, including the two modified
  ones, so the guard is intact;
- the fixture fix alone → 21/21.

**The general rule: when you make a test's fixture tolerant, prove the tolerance is conditional.** A retry
whose condition cannot distinguish "the environment interfered" from "the code is wrong" converts a
regression detector into a flake absorber — and it does so silently, which is worse than the flake.

## A sandboxed preload has NO module resolver — `require` is a five-name lookup table

Diagnosed and fixed 2026-09-21. **The app never booted**, and the mechanism had been recorded for several
sessions as a *symptom* ("P6 criterion 2 CARRIED: 46 failed / 0 passed") without a cause.

```
Unable to load preload script: I:/.../dist/preload.js
Error: module not found: @bifurc/client
    at preloadRequire (VM4 sandbox_bundle:2:144196)
    at runPreloadScript (VM4 sandbox_bundle:2:144465)
```

**The mechanism.** `src/main.ts` sets `contextIsolation: true` / `nodeIntegration: false` and never mentions
`sandbox`, so Electron's default applies — **`true` since v20** (this tree is on 42.3.0). A sandboxed preload
does not run under Node's module system: the browser process reads the file, hands `{filePath, contents}` to
`executeSandboxedPreloadScripts`, and evals it with a `require` bound to `preloadRequire`. That function's
entire body is a lookup in Electron's own webpack module table, ending in
`throw new Error("module not found: " + id)`. The table contains **`electron`, `events`, `url`, `timers`,
`punycode`** and nothing else.

**Consequences worth internalising:**

- **There is no `node_modules` resolution and no filesystem resolution**, so a bare specifier and a relative
  `./sibling` fail *identically*. `dist/preload.js` had `require("@bifurc/client")` **and**
  `require("./ipcTransport")`; fixing only the first would have hit the second. **Fix the closure, not the
  first error you see.**
- **`window.api` is the entire renderer surface**, so every `window.api.*` call is `undefined`. The failure
  is not "one feature is broken" — the app is a non-booting shell. That is what made 46 e2e specs fail as a
  block, which reads like flakiness or a broken harness rather than one defect with one cause.

**The blind spot that let it persist: a static require-walk proves REACHABILITY, not LOADABILITY.**
`scripts/clean-main-dist.mjs`'s header records that it verified 92 stale `dist/` files were unreachable "by
walking relative `require`s from `dist/main.js` and `dist/preload.js`". The walk was correct — those files
genuinely were not reached — but walking `require`s out of `dist/preload.js` presumes the walker's resolver is
the one that will run. In the sandbox it is not. **When you reason about a file loaded by another runtime,
confirm which resolver that runtime uses before trusting a resolution argument.**

**The fix: bundle it, and make the build prove it loads.** `scripts/build-preload.mjs` bundles
`src/preload.ts` → `dist/preload.js` with esbuild (`bundle: true`, `format: "cjs"`, `external: ["electron"]`),
and `build:main` runs it after `tsc` so it overwrites the unbundled output. Result: **one surviving
`require("electron")`**, and 1.9 MB of self-contained preload.

**The guard is the point of the script, not the bundling.** After building it evaluates the finished bundle
with a `require` that knows **only** the five sandbox names and throws otherwise — i.e. it reproduces the
sandbox's own failure mode at build time. Mutation-checked by marking `@bifurc/client` external: the guard
fails the build with `module not found: @bifurc/client`, the exact runtime error, exit 1. Without the guard the
condition returns the moment someone adds an import; a regex over `require("…")` would also be fooled by the
string appearing inside a string literal, which is why it evaluates rather than greps.

**Rejected alternatives, recorded so they are not re-proposed:** `sandbox: false` restores `require` and is a
security downgrade (it hands the renderer Node's module system); hand-writing the 144-key surface again is the
pre-P6 state that P5/P6 exist to remove.

**Two incidental findings from the same investigation.**

1. **`packages/engine/dist/` was left holding 161 declaration files and only 13 implementations** — so
   `@bifurc/engine/transport/types` had no runtime file at all. A clean rebuild produced 135/135. The likely
   cause is tsup's `clean: true` interacting with the safe-delete guard on this drive: a *blocked* clean
   leaves `dist` half-deleted, and the dts leg then re-adds declarations over the remains, which is exactly
   the observed shape (declarations present, JS missing). **A partial `dist` is silent until something
   resolves into it** — the preload bundle surfaced it as a build error, which is the good direction.
2. **The protocol's runtime barrel exports its fixtures** (`commands/index.ts:55` `export * from "./fixtures"`),
   so the 93-entry fixture table ships inside the preload bundle. Harmless but wrong; noted, not fixed, since
   changing the protocol's export surface is its own change with its own compatibility question.

**A third finding, from verifying the fix a session later (2026-09-21): the build script's own dependency was
undeclared.** `scripts/build-preload.mjs` does `require("esbuild")`, and `esbuild` was **not** in
`package.json` — it was present only *transitively*, via `tsup` (`^0.27.0`) and `vite` (`^0.27.0 || ^0.28.0`).
So the build worked by **hoisting**, not by declaration: correct today, and it would have broken confusingly if
either package dropped or changed its esbuild range — with the failure appearing in a *build script* that
`package.json` did not say needed esbuild. Now a declared devDependency pinned to the already-resolved
`^0.27.0` rather than introducing a second copy.

**The generalisation, which is the same shape as the preload defect one layer up:** a script that resolves
something at runtime must *declare* what it resolves. `require` succeeds or fails against `node_modules` as it
happens to be laid out, and hoisting makes an undeclared dependency work exactly long enough to be forgotten.
The check is one line — for every `require`/`import` in `scripts/`, is the name in `package.json`?

**And the `*/`-in-JSDoc trap bit again while writing the fix** — the header line
"`@bifurc/*` … `packages/*/src`" closed the comment early and produced `SyntaxError: Unexpected identifier`.
The existing `TRAPS.md` entry on this is accurate; the specific sting is that the offending token was a
**glob**, which is exactly the thing you write without noticing in a path-heavy document.

## `setDataRoot()` redirects *some* roots and not others — and only on Windows

`store/paths.ts#setDataRoot()` looks like the one switch that moves a run's files somewhere else. It is not.
Two path resolvers consult a **different** answer:

| Resolver | Windows | Linux / macOS |
|---|---|---|
| `dataDir()` (`store/paths.ts`) | honours `setDataRoot()` | honours `setDataRoot()` |
| `settingsPath()` → `appDataDir()` (`store/appSettings.ts`) | **`%LOCALAPPDATA%/Bifurc/app.json`**, ignoring `setDataRoot()` | falls through to `dataDir()/app.json` |

So on Windows a `--data-dir` run keeps its **workspaces and blobs** at the given root and reads its
**settings** from the user's real profile. `proxy/certManager.ts` has recorded this for the CA since P3
("that divergence is pre-existing … fixing it is a product decision for P8/P9"); what is new is that it now has
a second, independent victim.

**Second victim, found 2026-09-21 while verifying the P14 WI 6 host artifact.** `node dist/runtime/hostCli.js
--data-dir <temp> --workspace ws-e2e` came up:

```
[host] serving 0 workspace(s); rpc \\.\pipe\bifurc-<pid>       ← and exit code 0
```

The cause was not the CLI and not the data root: `loadSettings()` read `%LOCALAPPDATA%\Bifurc\app.json`, whose
only workspace is `mtsamun7rer3p`, so `--workspace ws-e2e` filtered that list to empty. Confirmed by reading
the real file rather than reasoning about it.

**Why this is a trap and not just a quirk.** Three properties compound:

1. **It is Windows-only**, so a Linux CI — the environment the container deployment actually targets — is
   green, and the divergence is invisible to every suite here.
2. **The symptom is silence, not an error.** "Serving 0 workspaces" reads as an empty data root. Nothing
   throws, and the exit code is 0.
3. **The fix is not local.** The obvious patch — have `settingsPath()` fall through to `dataDir()` always —
   changes where the *desktop* keeps its settings, which is a product decision with a migration. Hence
   `certManager.ts`'s "introducing a *third* answer here would not be [the fix]".

**What to do instead: make the divergence say itself.** WI 6 added the notice that names the directory, and it
is the shape worth copying for any "you asked for X and got nothing" case:

```
[host] requested workspace "ws-e2e" is not in the settings (looked in C:/Users/…/AppData/Local/Bifurc)
       — it will not be served
```

**And the general rule: when a flag claims to redirect "the data", enumerate the resolvers before believing
it.** `grep` for every function that builds a path from an env var or a root override and check each one
consults the override. A redirect that covers 3 of 4 resolvers is worse than none, because the failure is
partial and looks like data loss.

**Third victim, 2026-09-22 — and this time it produced a *false diagnosis*, which is the worst shape yet.**
Writing the guard for the e2e-fixture bug, I called `setDataRoot(<temp>)` and then `loadConfig()`, and got
**0 entities of every kind** — which reads exactly like "the fixture writes nothing", i.e. like the bug I was
there to pin. It was not: `setDataRoot()` is *silently ignored* on win32 once `%LOCALAPPDATA%` is set, so the
engine read the developer's real profile. The fix is to set `process.env.LOCALAPPDATA` to the temp root
**before** anything resolves a path — which is what the e2e fixture itself does — and to restore it in
`afterAll`, because a leaked `%LOCALAPPDATA%` would redirect every later integration test in the same worker.

**The pattern across all three victims: the failure is always attributed to the thing you were working on.**
`certManager` blamed the CA's location, the WI 6 CLI blamed an empty data root, and this one blamed the
fixture. In each case the real cause was one resolver consulting a *different* override — and in each case the
wrong attribution was plausible enough to act on. **When a path-dependent check finds nothing, verify the
resolution inputs before verifying the thing you are checking.**

## A locator can match the right *name* and still be the wrong *role* — and a label can live inside a dropdown

Three different defects produce the **identical** error. `TimeoutError: locator.click` says only that the
locator never resolved; it cannot tell you *why*. The `error-context.md` that `screenshot: "only-on-failure"`
writes alongside it contains a full accessibility snapshot of the page, and **that is what separates them**:

| What the snapshot shows | What it means | The fix |
|---|---|---|
| `generic "Update User" [cursor=pointer]` | the element is there and named right, but its **role** is not `button` | change the locator, not the target |
| the toggle (`button "Development"`) and **no** `Manage Environments` anywhere | the label is rendered only **while a menu is open** | make it a two-step action |
| nothing resembling the target at all | the label does not exist | find out what the real label is |

**Case 1 — `e2e/app.spec.ts:68`, the Requests tree.** `getByRole("button", { name: /^Update User$/i })`
timed out while the snapshot showed the panel open and the row present as `generic "Update User"`.
`FolderTreeItemNode.tsx:28` renders `<div title={item.name} onClick={…}>` — **a plain div with no role**, so
Playwright reports it as `generic`; its accessible name matches only because the accname algorithm falls
back to the `title` attribute. **A `getByRole` call that matches the right name is not evidence that the
role is right**, and it is not evidence the locator is near-correct — it is evidence about the *name* only.
Read the component, or locate by the attribute the name came from (`getByTitle`).

**Case 2 — `e2e/screenshots.spec.ts:117`, the environments tour.** `getByRole("button", { name: /Manage
Environments/i })` timed out because `EnvSelector.tsx:46` renders that item inside `{open && …}` — it exists
**only while the titlebar dropdown is open**, and the snapshot confirms the dropdown was closed (the toggle,
`button "Development"`, is there and the item is not). A locator for it is not a lookup; it is a *sequence*.
The repo already had the sequence — `openEnvironmentsManager` in `e2e/helpers/index.ts` — and the tour
never used it.

**The pattern behind both: a fix that lands in one spec and not its siblings.** `Manage Environments` was
already fixed in `app.spec.ts` and left broken in `happy-path-workflows.spec.ts` (2026-09-22) and in the
screenshots tour (same day). When you fix a stale locator, **grep the whole `e2e/` tree for the pattern**,
not the file you found it in.

**And the corollary that outlives both: an assertion that cannot fail is worse than no assertion.** The same
test asserted `getByText("New Request")` to prove `Ctrl+T` worked — but the panel's own "New request" button
matches that text and is always on screen, so the assertion passed with the keybind doing nothing. When you
repair a test, **check that the replacement can still fail**; the repo's earlier `if (await
locator.isVisible())` guards are the same defect in a different costume.

## Two `Edit` calls to the same file in one message silently revert each other

Both report `Successfully edited file`. The second read a snapshot taken **before** the first edit, applied
its own change to that snapshot, and wrote the whole file back — so the first edit is gone.

Caught by **measuring instead of trusting the success message**: the two edits should have removed ~430
bytes and the file shrank by **2**. Reading the region back confirmed the first edit's text was still the
original.

**Rule: one `Edit` per file per message.** If several are unavoidable, re-read or re-measure the file
afterwards — the tool's success message is about the write, not about the file's final state. This applies
to any tool that rewrites a whole file from a read-then-write cycle.

## Synthetic clicks hide occlusion, so a suite using them fails only where it uses a real one

`locator.click()` refuses to click an element that is covered — it retries and then times out with
`<div class="…"></div> … intercepts pointer events`. `element.click()` inside `page.evaluate()` does not:
it dispatches straight to the node and **never hit-tests**. A suite that navigates with the second form is
therefore blind to anything overlaying the page, and will fail at exactly the one place it uses the first —
**so the failure's location is a property of the test's click style, not of the bug's location.**

That is how the Bifurc screenshot tour failed on 2026-09-22. Panels 1–10 all navigate with
`evaluate(el.click())`. Panel 10's titlebar step matched the wrong button and left a full-screen backdrop
open; nothing noticed, because a synthetic click does not care. Panel 11 is the tour's **only** real click
(`openEnvironmentsManager`), and it timed out for 30 s. **The bug was introduced at panel 10 and surfaced at
panel 11**, which is why reading the failing step as "the environments helper is broken" was wrong — and why
the user's observation that `app.spec.ts`'s environments test *passes* was the decisive clue.

**Reading the error.** The two halves of the message are what identify this:

```
- waiting for getByTitle('Switch environment').first()
  - locator resolved to <button … aria-expanded="false" title="Switch environment" …>
  - <div class="fixed inset-0 z-40"></div> from <div class="relative flex-shrink-0">…</div> subtree intercepts pointer events
```

`locator resolved to` means the target exists, is visible, is enabled, and is stable — **the locator is
fine**. `intercepts pointer events` means something else is on top. So: do not debug the locator; find the
topmost element. Note also that `aria-expanded="false"` on the resolved toggle proved the backdrop was *not*
its own dropdown's — an attribute on the target can rule out the obvious suspect. The page snapshot then
named the real owner directly: `button "SW Sample Workspace" [expanded]` with its `Search workspaces…` box
open.

**How the backdrop got there.** The titlebar branch built `new RegExp(escapeRegex(panel.label), "i")` —
**unanchored** — so `/Workspace/i` matched both `button "SW Sample Workspace"` (the workspace *switcher*)
and `button "Workspace"` (the Workspace *panel*), and `.nth(0)` took the switcher. Clicking it opened a
dropdown whose backdrop is `<div class="fixed inset-0 z-40">` (`WorkspaceDropdown.tsx:76`; `EnvSelector.tsx:48`
is byte-identical), and the tour never closed it. The `bottom` branch of the same function already anchored
its regex (`^Settings$`) — **one branch of a nav helper was anchored and the other was not, which is what made
this a one-line bug with a 30-second symptom.**

**Rules.**
1. **Anchor nav-label regexes** (`^…$`) and treat `.nth(k)` / `.last()` over an unanchored label as a silent
   mis-click, not a tie-breaker. A partial-name match will eventually hit a *different* control that happens
   to contain the word.
2. **When a suite navigates synthetically, any step that opens an overlay must close it**, or the next real
   click dies far from the cause. Prefer dismissing before each step (Escape, or a backdrop click) over
   remembering to clean up after each one — and check which mechanism actually closes each overlay, since
   they differ: `WorkspaceSelector.tsx:75` handles Escape, `EnvSelector` has no Escape handler and relies on
   `onManage` closing it (`TitleBar.tsx:163-166`).
3. **"The helper works" and "the test fails" are compatible.** A shared helper can be correct in isolation
   and still fail in a sequence, because its preconditions were established by earlier steps.

## A proxy reads one workspace's rules and then filters them by a *different* one

**Symptom.** A `WorkspaceRuntime` created for workspace B serves nothing, while one created for the active
workspace serves normally. No error, no log — the request simply falls through to passthrough.

**Cause.** `proxy/server.ts` has two notions of "which workspace" and only one of them was severed in WI 4:

- `servedWorkspaceId()` → `workspaceId ?? cfg.activeWorkspaceId` — **the instance's**, correct. `start()` /
  `reloadConfig()` use it to load `fullRules` and `enabledSets`.
- `serverUtils.ts:35-44`'s `workspaceCfg()` → `const wsId = cfg.activeWorkspaceId` — **the config's
  pointer**, never the instance's. Both dispatch call sites (`server.ts:242,354`) pass
  `currentConfig ?? loadConfig()`.

So for an instance with `workspaceId: "B"` while `activeWorkspaceId` is `"A"`: `fullRules` holds B's rules
and `workspaceCfg` filters them by `"A"`, keeping none. **The state read is per-instance; the dispatch
filter is per-config.**

**And the two halves cancel rather than cross-talk** — the detail that makes the symptom look like something
else. `cfg.mocks` / `cfg.mappings` come from **A** (`loadConfig()` reads the active workspace by
construction, `config.ts:68,80-82`) while `enabledSets` comes from **B**
(`loadEnabledSets(servedWorkspaceId())`). So A's entities are filtered out by *B's* enabled set and B's are
missing from `cfg` entirely. Measured, only the active pointer differing: with the pointer on A, B's own
mock is **404** and B's own rule is **502** with zero upstream hits; with the pointer on B, both are **200**.
**Nothing leaks and nothing works** — so "it must be cross-talk" is the wrong first guess, and a probe that
only looked for the other workspace's data would have found nothing wrong.

**Why the suite is blind to it.** `tests/integration/workspaceRuntime.integration.test.ts`'s "both
genuinely serve" step asserts `getHomePage(port)` — the proxy's own UI page, which is served regardless of
rules, mocks or mappings. §8's criterion says *"each serving its own rules"* and the instrument chosen
cannot see a rule. **The instrument was right for ports and readiness, and for content it was chosen for
convenience.**

**Fixed 2026-09-22 — and the fix is a loader, not a filter.** The obvious repair ("pass the instance's
workspace id into `workspaceCfg()`") **cannot work**, because `cfg.mocks` / `cfg.mappings` *are* the active
workspace's entities — no id recovers data that was never loaded. `store/config.ts` gained
`loadWorkspaceDispatchSlice(wsId)`, `workspaceCfg()` takes that slice and reads the id from it, and the proxy
loads it beside `fullRules` / `enabledSets`. Guards:
`tests/integration/workspaceDispatchScope.integration.test.ts` (4 cases: both directions plus a control) and
the rewritten `workspaceCfg()` unit cases. Reverting the id source fails exactly **3** — and **the control
correctly survives**, because for the active workspace the two ids coincide. That asymmetry is what makes it
a control rather than a guard.

**Rules.**
1. **When a subsystem is "de-globalised", grep for the global's name afterwards** — not just at the
   construction site. WI 4 changed the reads and missed the filter, and the two are 100 lines apart.
2. **Assert the criterion's own noun.** A criterion about *content* ("each serving its own rules") is not
   satisfied by an assertion about *liveness* (a page loads). "Both are up" and "both serve what they
   should" are different claims, and the first is much easier to write.
3. **A test that has to be made *discriminating* is where this class surfaces.** The gap was invisible
   until a test was rewritten to make "which instance?" observable — the same way `plan/07`'s fixture defect
   was invisible until something read the fixture back through the real loader.
4. **When the diagnosis is "the filter uses the wrong id", check whether the *data was ever loaded* before
   writing the fix.** This was written down as a one-id change and was not: the entities came from a loader
   that only ever read one workspace. **A filter cannot recover an absent input.**
5. **Adding an export to a module breaks every `vi.mock` of it** — 82 failures, one file, all reading
   `No "loadWorkspaceDispatchSlice" export is defined on the mock`. Widening a module's surface is the class
   `Schemas: accepts ≠ preserves, and widening invalidates fixtures` records one layer up. **And write the
   *derived* mock:** `tests/proxy/server.test.ts` computes the slice from `loadConfig.mock.results` rather
   than calling `loadConfig()`, because several cases assert `toHaveBeenCalledOnce()` on it — an instrument
   that adds a call breaks assertions about a *different* mechanism, which is this file's recurring shape.

## Auditing a gate: the verdict is an instrument too, and a criterion can be wrong

Recorded 2026-09-22, closing `plan/14` §8's twenty-one acceptance criteria. The audit that started it was
right about eighteen rows and wrong about one, and two of the criteria turned out to be defective rather than
unmet. All three failures share a cause: **the audit treated its own output as an observation rather than as a
claim needing an instrument.**

### A negative verdict is a claim too

Criterion 8 — "`preflight()` does not fail on a busy *base* port and does fail on a busy *per-workspace
override*" — was recorded **unmet**: *"No busy-base-vs-busy-override pair. `engineSmoke.integration.test.ts:212`
only asserts a clean temp dir yields no findings."* Both halves of that sentence are true and the conclusion is
false. `tests/startup.test.ts:87` is a five-case suite covering exactly the criterion, including both cases in
a single run (`:131`). The audit searched `tests/integration/` and stopped.

This is the **inverse** of the entry above about retired tests, and the same error: there, "does anything else
test X?" was answered by counting files that mention X; here, "is X tested?" was answered by searching one
directory. **"I did not find it" is not "it is not there."** A negative row in an audit needs the same
instrument-naming discipline as a positive one — name where you looked, and make the search cover the whole
tree, or the verdict is a statement about your search rather than about the code. The cheapest fix is the one
that would have caught it: `grep -rn` over `tests/` and `packages/`, never a single subdirectory.

### A criterion can be defective, and a test written to satisfy it locks the defect in

Two of the twenty-one could not be satisfied by any honest test:

- **6** said *"stop the host, free the base port, start again, and confirm the workspace that had incremented
  is still on its incremented port."* But `workspaceHost.integration.test.ts:331` asserts `host.start()` after
  `stop()` **rejects** (`already been stopped`) — restarting a *stopped* host is impossible **by design**. The
  criterion contradicted a contract another test pins. Reworded to *a **new** host*, which is the claim that
  matters.
- **5** said *"set an override below 1024 … and confirm the runtime still comes up."* D16's table has **two**
  sources and the word "override" names both: a per-workspace override, which must **throw** when taken, and
  an override of the *environment base*, where the scan advances. The sentence describes the second and the
  parenthetical points at the first. Reworded to say which.

**The rule: when a criterion cannot be satisfied, suspect the criterion before writing a test that agrees with
it.** A test bent to fit a defective criterion is worse than no test — it converts the defect into a green
check, and green checks are what later audits trust. Reword first, instrument second.

### A count is the right instrument when the claim *is* a count

`MEMORY.md` says *"for any generated / looped / `it.each` suite, NEVER count by hand"* — the point being that a
hand count of a generated set is unreliable. Criterion 3's invariant half is the exception that proves it:
*"nothing else in the allocation path branches on platform"* **is a statement about how many platform reads
exist**, so the honest instrument is to read the source and assert the count — one `process.platform`, one
`IS_WINDOWS` use. No behavioural test can express it without enumerating every output of the module and
proving each platform-invariant, which is unbounded and silently stops covering new outputs.

The distinction is *what the claim is about*. A count of a **generated set** is a claim about the runner and
belongs to the runner (`--reporter=json`). A count of **occurrences in a source file** is a claim about the
code, and reading the file is the direct instrument. What must not happen is counting the second kind by eye.

**And the count must be taken over the module's *code*, not over its text — learned the hard way on
2026-09-22.** The criterion-3 case counted the raw source, and it failed on a **documentation** edit: a
docblock added the same evening, while implementing 6b, merely *names* `IS_WINDOWS`, which took the count from
2 to 3. A comment is not a branch on platform, so the instrument had been made sensitive to prose — and the
tempting "fix" would have been to delete a useful explanation to satisfy a test, which is the worst possible
outcome for an instrument. Comment-only lines are now dropped before counting, while a trailing comment on a
*code* line is deliberately kept, so a real second use cannot hide behind one.

**Ask of any text-count: does what it counts include prose?** If yes, it will one day fail on an edit that
changed nothing about the behaviour it guards — and the pressure will be to weaken the prose, not the count.

### A mutation you cannot attribute is not evidence

Mutation-testing the new guards produced one run where the mutation itself was malformed — the replacement
`Promise.resolve().stop()` throws, so the failure measured the mutation rather than the product, and it
happened to look like a clean two-test failure. It was caught only because the *second* test failed too, in a
case that had nothing to do with the change.

**Check that the mutation is well-formed before reading anything into its result**, and prefer a **narrow**
mutation to a broad one: reverting a whole mechanism fails twenty tests and tells you the tests are sensitive,
not that they are sensitive to the right thing. The mutations that carried information here were the ones that
moved *exactly* the intended assertion — `EADDRINUSE`-only error handling failing two cases out of thirty; a
second `process.platform` read failing one out of thirty; `resolvedProxyPort` fed back as a preference failing
one case out of two while correctly leaving its sibling passing.

## Rewording a defective criterion is how a defect gets locked in green

**The failure:** a criterion is found defective, is reworded, and the *rewording quietly replaces its claim*
with a weaker one the code already satisfies. The instrument written for it then passes — and it passes
**because** the code is wrong. Nothing in the suite can see it, because the only document that said what was
required has been edited to agree with the code.

**The instance — P14 §8 criterion 6, 2026-09-22.** The criterion read: *"stop the host, free the base port,
start again, and confirm the workspace that had incremented is **still** on its incremented port."* That is
D16's stickiness rule **verbatim**. It was marked unsatisfiable — correctly, for the **mechanism** it named
(`workspaceHost.integration.test.ts:331` pins that a stopped host refuses to restart). But the rewording
replaced the mechanism **and the claim**, landing on *"a new host lands on the same ports"* — which the scan
satisfies and which is a **different requirement**. The code violates D16, the criterion was green, and the
instrument built for it pinned the deviation.

**How to tell you are doing it:**
- The new wording is *easier to satisfy* than the old one, and nothing else changed.
- The decision the criterion came from is not quoted **inside** the criterion. Quote it — the substitution
  becomes visible on the page.
- You are about to retire the old claim rather than re-test it. A mechanism can be unsatisfiable while the
  claim is perfectly testable; here the *rewording itself* established the instrument that would have tested
  the original claim (a **new** host, not a restarted one).

**The related trap: a discriminating test is evidence for whichever side it points at.** The test in question
was genuinely good — it separated "remembered" from "re-derived" cleanly. It was filed as proof the criterion
held, when what it proves is that the mechanism the decision specifies is **absent**. The test never changed;
only the authority it was measured against did. **Before filing a green test as evidence, name the document
that says the behaviour is required — and check the test points at that document, not at the code.**

**Corollary — a field that is written and never read is a decision that was not implemented.**
`resolvedProxyPort` is written by `recordResolvedPorts()` into `runtime.json` and read by **no product code
path** (two tests read it). Grep for a field's **readers**, not its writers, and treat "written, never read" as
an open decision rather than as a dead field: D16 says that value must decide the next start, so the omission
is the *feature* that is missing, not cruft to delete.

*(Resolved the same evening: the owner chose to implement D16, and `allocatePorts` gained a **soft**
`remembered*` source — tried when free, skipped when taken, never thrown over — with `proxyPortWasRemembered`
on the result, because the *outcome* cannot distinguish that path from the scan. The corollary stands
regardless: reading the **writers** and not the readers is how the omission was found.)*

**And the meta-lesson, because it is the second time in two batches:** the audit was right to distrust the
checkboxes, and then trusted its own rewording. An instrument can answer a neighbouring question — **so can a
criterion.**

## A repo can change under you — check `HEAD` before you commit, and re-measure after

Found 2026-09-22, and it is the one hazard in this repo that no amount of reading the tree will reveal. A
batch finished its work and left the tree uncommitted — as the previous batches had — and by the time it
returned, **a second session had been active in the same repo**: the tree was clean at a new commit, the
programme status table had been rewritten, and the daily log contained a batch entry written by someone else.

**Nothing was lost that time**, but every failure mode here is an expensive one: a commit that sweeps up work
nobody has run; a document edited from two directions so a sentence lands twice; and the reverse — your own
edits landing inside someone else's commit under a message that describes *their* work.

**The rule:** before committing, run `git log --oneline -1` and `git status` — **do not assume the tree is the
one you left.** If `HEAD` moved, **re-measure before reporting anything as verified**: a test count is a
statement about *a revision*, and the revision on disk may no longer be the one you measured. (Here the count
had moved by exactly the six tests the last batch added, and two documents that state the suite size were
still carrying the old number — re-measuring and **re-writing every place that states the result** are two
steps, and only the first had been done.)

**And when a document disagrees with itself after such a merge** — a sentence twice, a row holding two
narratives — treat it as a merge artefact and **find its provenance before fixing it**
(`git show HEAD~1:<file>`): the duplicate may be the *older* text, in which case deleting the wrong copy
throws away the newer account.

## A guard no test can observe is not a defect — but a comment calling it load-bearing is

Found 2026-09-22, mutation-checking the new browser transport's eight cases. Three of four mutations were
caught by a *named* case: silencing `onclose` failed two, making `isReady()` always true failed two, silencing
`onerror` failed one — and that last one only after a case was **added**, because the connect-failure path had
been untested and the wiring was therefore decoration. The fourth was not caught, and the interesting part is
why.

`dispose()` detaches the socket's four handlers before closing it, and the comment said the reason was that a
handler still attached "would report a caller-initiated shutdown as a fatal fault". **That is false.**
`session.ts` sets its own `closed` flag *before* it calls `dispose()`, so a `close` arriving with the handlers
still attached finds the teardown already done and reports nothing. The detach is **defensive** — it stops this
file depending on that ordering, and it stops a queued `message` being handed to a finished session — but it is
not observable, and no test can be written that guards it.

**The rule:** when a mutation is not caught, the question is not "how do I write a test for this" but **"is
this load-bearing at all?"** Three answers are possible and they need three different responses: the test is
missing (add one), the guard is genuinely unobservable (say so *in the code* and stop claiming otherwise), or
the guard is dead (delete it). **A comment that overstates a guard's role is a defect even when the guard is
fine** — it is what will mislead the next reader into hunting for the test that cannot exist.

## The third copy was already there — moving a shared module is what made it visible

Found 2026-09-22, the same afternoon. `session.ts` exists because `stdio.ts` and `ws.ts` had each grown a copy
of the same ~150-line client state machine, and its own header says a third copy is "where *similar* becomes
*drifted*". The extraction was done and `socket.ts` was converted to use it — and **`ws.ts` was not.** Its
client still carries its own `pending` / `control` / `listeners` / `teardown` / `deliver`, and nothing said so,
because nothing was looking: the file compiles, its tests pass, and a duplicated state machine is invisible
from the outside.

It became visible only when `session.ts` was **moved** to another package for P7, so "who imports this?" had
to be answered. `socket.ts` and one test appeared; `ws.ts` did not — and its absence from that list *was* the
finding.

**The rule:** an extraction is not finished when the new module works, it is finished when the old copies are
**gone**, and the way to check is to ask who imports the shared module rather than to assume the refactor
swept. **A file that should import something and does not is invisible to every instrument that only reads
what is there** — the same shape as `"nothing else reads it"` needing a grep of `packages/` and not only
`src/`. Recorded as a **known open item** rather than a suspicion: whoever next touches `ws.ts` should convert
its client to `createClientSession`.
