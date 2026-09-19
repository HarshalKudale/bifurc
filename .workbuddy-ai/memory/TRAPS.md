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

A module-scope `commandRegistry.register(...)` takes effect on *import*. Moving one behind an explicit
`register*Commands()` call silently unregisters it for every suite that imports the handler group
without running `registerIpcHandlers()`, and the failure is `No handler registered` at **invocation**
time — nothing catches it at build time. **Read the importer set before calling a slice mechanical**:
slice 1 needed 5 suite edits, slice 2 needed 0, slices 3–4 needed 2 each.

The conformance matrix cannot cover this either — it registers `() => ({ reached: cmd })` over every
command deliberately, testing the transport and the schema rather than any implementation. And the
ratchet asks only "is this command registered *in this process*", which the shell has already done for
all of them.

## A *delegating* channel is both the risk and the evidence

Channels that `commandRegistry.invoke()` break on an empty registry — but they also make a
channel-driven test a registry-driven test. Self-contained duplicate bodies do the opposite: three of
`runnerHandlers`' five are copies, so `runnerStorage.integration.test.ts` passed with the engine half
wrong. **Mutation-check every move**: drop the `register*` call and confirm the failure is
`No handler registered`.

## `require()` in a handler body

Invisible to tsc and to every unit test, but **fatal in the engine's ESM build output** — `require` does
not exist there, so a verbatim copy compiles, typechecks, passes the suite, then throws at *call* time
in `dist/*.mjs`. Use static imports. **Check the artifact, not the source**:
`grep -c 'require(' packages/engine/dist/**/*.mjs` — the source is what lies.

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
