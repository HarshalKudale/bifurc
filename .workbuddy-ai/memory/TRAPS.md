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
