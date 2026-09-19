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

## Git / env

- **NEVER `git stash`** (wiped the object store once). **`git fetch` doesn't persist remote-tracking
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
  `npm run dev` / `test:e2e`.
- **Safe-delete guard refuses past ~50 deletions/turn**, killing `build:packages`; **`git clean -fdX
  <path>` is the way through.** A failed `tsup` leaves entry points deleted — check `dist/index.js`.
  One `rm -rf` of a big cache dir exhausts the whole turn's budget (it reported `count: 9715` against a
  threshold of 50) and every later delete in that turn dies — the bypass is
  `env -u CODEBUDDY_SAFE_DELETE_BULK_STATE_DIR -u CODEBUDDY_TOOL_CALL_ID npm run …`. `bundle: false`
  means every `tsup` run litters `packages/*/tsup.config.bundled_*.mjs` (32 accumulated).
- **Linking a new workspace package needs a manual Windows junction** (`New-Item -ItemType Junction`),
  then `node -e "require.resolve('@bifurc/<pkg>')"`.
- **A suite capturing `ipcMain.handle` needs `register*Commands(registry)` in `beforeAll`**, else
  `ENOENT` reads as an fs bug.
- **Uncommitted edits inside `src/` have been lost once** (the whole `src/` tree vanished mid-turn, cause
  never identified). Commit early when touching `src/`. Recovery is
  `git restore --source=HEAD --staged --worktree src/` — plain `git restore <path>` keeps the file
  deleted when the deletion was staged.
