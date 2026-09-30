# Bifurc — Project Memory

**Pointers, not records.** State → `plan/README.md` §Status · decisions → `plan/00-decisions.md` · suites →
`TESTING.md` · ports → `plan/17` §1 · edge/mapping → `plan/15` · **traps → `.workbuddy-ai/memory/TRAPS.md`** ·
procedures → the `bifurc-*` skills · per-day → `memory/YYYY-MM-DD.md`. **Grep TRAPS.md before diagnosing
anything.**

> **⚠ ONE BUILD TARGET: the Electron desktop app.** `web/` + the container image are **deleted**.
> **`npm run dev` / `test:e2e` are the user's to run** — an agent-shell failure is evidence about the *shell*,
> not the product.

## Build & layout — the rules that bite immediately

- **Flat repo — the app IS the root.** **`git rm`, `git stash`, `git checkout -- <file>` are ALL destructive**;
  `cp` to `/tmp` first. **⚠ `rm` is a shim, and `git rm` under `scripts/` wiped the whole directory** → use
  `python -c "import os; os.remove(...)"`, small batches, verify each, **commit first**.
- **App runs `packages/*/dist/`; tests run `src/`** → `npm run build:packages` after a package edit (needs
  `CODEBUDDY_SAFE_DELETE_ENABLED=0`). Only `@bifurc/engine/*` aliases to `src`. **Directory renames blocked**;
  a *file* `git mv` works. **⚠ One `Edit` per file per message.**
- **⚠ `build:packages` omits `@bifurc/gui`**; **Vitest resolves `@/…` through its own alias plugin** — a green
  test is not proof the bundle resolves.
- **⚠ No tsconfig covers `tests/`** → `typecheck` green while `tests/` broken. **`packages/gui` is not in
  `npm run typecheck`** — sole runner `npm run typecheck --workspace @bifurc/gui` (**~33 pre-existing errors;
  the instrument is *which files*, not the count**).
- **⚠ `package-lock.json`'s workspace set is a coupling** — check with `npm ci --dry-run`.

## Measuring the suite

- **A *delta* is the signal; name its mechanism.** Baseline **re-measured, never carried forward**. Judge a
  mutation by **exit code**. **Freeze the tree while a run is in flight.** **⚠ A single-project run is NOT the
  full run** — unit + integration = the full total. `getFreePort()` is a **reservation, not a guarantee**
  (TRAPS 112).
- **⚠ Standing ENVIRONMENT failures — deterministic, NOT to be fixed.** `gitRemote.integration.test.ts` **fails
  every case** (10 tests; sandbox `git push` reports success **without writing the remote's refs**). The
  safe-delete shim (`EBUSY … rmdir` on `%TEMP%`) makes `gitStore.test.ts`'s `beforeEach` time out (**24/24
  alone**) and leaves **four teardown-only** suite `FAIL`s — `fileOps`, `importExport`, `importExportBlob`,
  `importExportFormats` (`Hook timed out in 20000ms` in `afterAll`; all their tests pass). **⚠ Never blame a
  change for a count in the git/temp family — check alone first.**
- **Last full measurement 2026-09-29: 2,742 tests / 128 files** (unit 98 / 2,281 incl. 40 skipped; integration
  30 / 461). 11 failures, **all environment**. Engine-layout phase 8 added exactly +1 file / +5 tests by design.
  **e2e: 11 specs rewritten 2026-09-28 — the old 46/46 figure is stale and wants an owner run.**

## Adding a protocol command — SEVEN places, four of which THROW

(1) `COMMANDS` (`protocol/src/commands/index.ts`); (2) `COMMAND_FIXTURES` (`fixtures.ts`, equality-asserted);
(3) `SCOPE_BY_COMMAND` (`engine/src/transport/auth/scopes.ts` — `assertScopesAreTotal()` **throws at import**);
(4) `client/src/retry.ts`'s idempotency partition — `assertRetryPolicyIsTotal()` **throws at import**;
(5) `tests/ipc/handlers.test.ts` asserts **every** command is registered → the **shell** (`src/ipc/handlers.ts`)
must register it too; (6) `tests/integration/workspaceHost.integration.test.ts`'s **hand-maintained**
`loadRegisterFn` table (key = the call-site regex's short name); (7) `tests/engine/layout.test.ts`. Plus
`plan/protocol-changes.md` (*write the entry first*). **`@bifurc/protocol` resolves from `dist/` in tests** →
`build:packages` **before** any test sees the new list.

**The saved-entity family (2026-09-29):** `request.replaySaved` · `graphql.executeSaved` · `soap.executeSaved` ·
`grpc.executeSaved` — each `{workspaceId, id}`, load the entity and `registry.invoke()` the original (protocol
89 → **93**). Engine side is **one** module, `workspace/entities/savedRequestCommands.ts`. **⚠ `SavedRequest.body`
is stored as TEXT but `request.replay` wants BASE64** — the encode must reproduce `textToB64`'s blank-means-empty
rule. → `2026-09-29.md`.

## The client surface (no longer frozen) + the Send gate

**`window.api` grew for the first time** — 132 → **136** (`transport` 62 → 66). The byte-identical freeze existed
to make P5→P6 a wiring change, and P6 is done; it is replaced by the **pinned counts** in
`packages/client/tests/surface.test.ts` (total + per-kind) and `client.test.ts`. **`src/preload.ts` needs no edit
for a new key**; `shape.test-d.ts` needs none *if* `types/window.ts` matches the client exactly.

**The Send gate: `packages/gui/src/lib/savedEntitySend.ts`** — id-vs-payload, a conjunction: not a draft · no
local changes · no `{{…}}` (**env vars AND `{{random.*}}`**) · no executable scripts (**REST only**). **⚠ A draft
reports `hasLocalChanges === false` by construction.**

**⚠ The four protocol editors hang jsdom** (CodeMirror + layout) and **no e2e spec drives Send** → tab *wiring*
is verified by typecheck + review only; only the pure gate is unit-tested.

## ⚠ The UI is a CLIENT — the boundary rule, and `plan/19`

**Owner's rule (2026-09-29): the renderer holds no domain logic and no domain state.** Test for any line in
`packages/gui/src`: *would this differ if the UI were a terminal, a web page, or a test harness?* If yes, it is
engine work. Engine-owned: `{{env}}` **and** `{{random.*}}` resolution, pre/post/test scripts, the collection-run
loop **and its run state**, cURL parsing, report rendering. UI state that legitimately stays: open tabs, drafts,
layout, theme, **"is this tab saved / does the editor differ from disk"**. New protocol modules (MQTT et al.)
land **engine-first**.

**`plan/19-engine-owned-processing.md` (P19, D22–D28).** Finding: the renderer holds **five
re-implementations** of engine logic. **WI 1 LANDED** — `tests/gui/boundary.test.ts`, the renderer boundary
ratchet (`layout.test.ts`'s idiom on a second boundary; **allowlist = 9 entries in 9 files, measured — §6
predicted 6**; `scriptRunner.ts` / `testRunner.ts` are the two §1's table misses). **WI 2's engine half
LANDED** — `utils/resolve.ts` holds `resolveVars` + `resolveHeaders`, `mockHandler.ts` imports them, and
`resolveVars`'s cases moved to `tests/proxy/resolve.test.ts`. **WI 3 LANDED FOR REST, both halves** — `pipeline/runPipeline.ts` is D24's one pipeline (`send` injected, so the
file knows no protocol); `workspace/envState.ts` supplies the merged env (`__global__` over active) and
**deliberately avoids `loadWorkspaceDispatchSlice`**, which *writes* `enabled.json`; `utils/resolve.ts`
gained `mergeEnvVars` (the merge travels with the CALL). On 2026-09-30 the wire and the tab followed:
**`request.replaySaved` → `request.send`** — a `z.union` of `{workspaceId, id}` and `{workspaceId, request,
scripts}`, body as TEXT, response base64 + the pipeline's bookkeeping; **count stays 93** (the replaced name
was never released, so it is a rename, not a migration). `useRestActions` stopped resolving/orchestrating.
**⚠ The ratchet forced a PLACEMENT, not a permission:** `request.send` cannot sit in `workspace/entities/`
(it calls `pipeline/`, and `workspace/`'s row is `kernel/` + `utils/`) → it lives in
**`pipeline/commands.ts`**, and `pipeline`'s contract gained `kernel/`. **Allowlist 9 → 8** (first entry this
ratchet has ever retired). **⚠ `hasExecutableScripts` left the Send gate entirely** — `true` for REST alone,
so REST's migration made it a constant; `usesVariables` stays for the other three.
**GraphQL followed 2026-09-30: `graphql.executeSaved` → `graphql.send`** — a union of `{id}` and `{request}`,
where `request` is the **stored shape** (`endpointUrl`), and **no `scripts` member**, because the sandbox's
context is a `{method, url, headers, body}` request and a GraphQL request is not one. It landed in
`graphql/commands.ts`, **not** `pipeline/`, and does **not** call `runRequestPipeline` — that view exists for
the three script steps, and fabricating one to satisfy a signature is inventing a shape. **⚠ The pattern for
the remaining two: their own feature module, direct resolve + dispatch.**
**⚠⚠ THE FINDING: every non-REST tab resolves FEWER fields than its own gate scans** — GraphQL resolved
`endpointUrl`/`headers`/`variables` but not `query`/`operationName`; SOAP not `soapAction`; gRPC not
`serviceName`/`methodName`. So a `{{token}}` in any of those has **never resolved**, in *either* path (the gate
only ever chose between two ways of sending the same unresolved field). **The gate is the better spec** —
written from intent, the tab from practice — and the engine must resolve **every** field, because resolving the
narrower set would keep `usesVariables` in the gate forever. `GraphQLTab`'s `activeEnv` prop was deleted as the
visible evidence.
**SOAP: `soap.executeSaved` → `soap.send`** — union `{id}` | `{request}` with `{endpointUrl, soapAction,
headers, body}`; **the one protocol where the stored shape and the command's params are IDENTICAL** (no rename);
the tab never resolved `soapAction`. **gRPC: `grpc.executeSaved` → `grpc.send`** — same shape;
`protoFileId`/`useReflection` required; **`protoFileId` deliberately NOT resolved** (a stored file, not template
text); the tab never resolved `serviceName`/`methodName`. **⚠ gRPC's resolution is UNOBSERVABLE** — the
`grpc.execute` stub ignores every parameter, and replacing it needs a monkey-patched registry
(`register()` throws on duplicates) — recorded as a gap; `NOT_FOUND` proves the load half.
**⚠ WI 3 COMPLETE: `savedRequestCommands.ts` is DELETED** (all four `*.send` live in their own feature
modules), and **`savedEntitySend.ts` is now `!isDraft && !hasLocalChanges`** — both domain terms deleted, with
`containsVariableToken`. **Allowlist still 8: the resolver/randomizer entries belong to WI 6** (`useWebSocket`
+ `HealthBarPanel` still resolve), and the four `pipeline`/`pipelineStep` entries to **WI 4**. §6's "the last
tab" was wrong.

**WI 4's engine half LANDED (2026-09-30): `runner/runCollection.ts` + `runner/runRegistry.ts`.** Two policies
were **decided by the owner** (§8 traps 5/6 ask for exactly that): **D29 — a run's lifetime is the ENGINE's**
(a disconnect and a workspace switch do not cancel it; a host restart loses it, because resuming would have to
decide whether to re-send what it already sent) and **D30 — one run per `{workspaceId, folderId}`** (same
folder refused, different folders concurrent, a *finished* run does not block a new one). The loop is **thin
because WI 3 exists**: one `registry.invoke("request.send", {workspaceId, id})` per request. `runRegistry.ts`'s
map is module-level **by design** and nothing in it can observe a disconnect. **⚠ `stopOnFailure` and
`iterations` in `runner.json` stay UN-honoured** — the renderer read neither, and implementing them would be a
behaviour change smuggled in under a move. **⚠ The engine half is safe before the renderer's, unlike WI 3's** —
two run loops do not double-apply anything the way two resolvers do.
**WI 4's WIRE LANDED (2026-09-30): `runner.start`/`cancel`/`status` + `event.runner.progress`/`finished`.**
Protocol **93 → 96** commands, **9 → 11** events; client surface **136 → 141** (transport 66 → 69, subscribe
7 → 9). **⚠ Two scopes deliberately: `start`/`cancel` = `execute`, `status` = `read`** — that is what lets a
reconnecting client ask what happened without the right to start or stop. **⚠ `total` rides on every progress
event** (a mid-run attacher never saw the start call). **⚠ The conformance matrices widened AUTOMATICALLY** —
they are derived from `COMMANDS`/`EVENT_NAMES`, so the wire cost three list entries, not three hand-kept tables.
**⚠ `retry.ts`'s partition needed all three**: `status`/`cancel` idempotent, **`start` non-idempotent** (a retry
is a run the caller did not ask for).
**WI 4 COMPLETE (2026-09-30): the renderer half landed too.** `useCollectionRunner` is a **subscription**
(`runnerStatus` on mount so a tab opened mid-run attaches; `runnerStart`/`cancel`; the two events), and it is
a **mirror, not the authority** — nothing in it decides anything. **⚠ A flaw the tests caught:** the first
version only *updated* a snapshot it had, so a `runner.finished` arriving with **no** snapshot was **dropped** —
the exact disconnected-client case D29 exists for. Both handlers now **establish** state, and that is what
`total` on every progress event was for. **⚠ A progress event carries no start time**, so that branch's
`startedAt` is a guess (recorded in the hook).
**Four deletions landed: `lib/collectionRunner.ts`, `lib/scriptRunner.ts`, `lib/testRunner.ts`, and
`runnerReport.ts`'s HTML renderer — allowlist 8 → 4.** `runnerReport.ts` survives with its two *storage*
wrappers, which is the `file#construct` keying working as designed. `requestsRunnerTab.test.tsx` was
**rewritten** to assert the new contract, and that rewrite is what caught the flaw.
**Remaining: WI 5** (cURL parsing, capture→entity construction, path helpers, `HealthBarPanel`'s resolution)
and **WI 6** (WebSocket — carries the resolver/randomizer allowlist entries and is the item that finally
empties it), then **WI 7** (the extensibility contract).

**D28 LANDED** — the engine's
`utils/randomizer.ts` owns `RANDOMIZER_TOKENS`; the UI keeps a **copy** pinned by
`tests/gui/randomizerCatalogue.test.ts`, and **the pin is what makes the copy legitimate** (without it the
copy *is* §1 row 2). **⚠ WI 2's renderer half is BLOCKED on WI 3 ⇒ they are ONE atomic change: the renderer
has no `@bifurc/engine` dependency by design, so it cannot import the engine's resolver and cannot stop
resolving until the engine does.** **D27 (decided, both halves): the pipeline surface is a NEW `*.send` per
protocol — §5 option B, NOT the plan's recommended A — `{workspaceId, id}` | `{workspaceId, request,
scripts}`, body as TEXT, response base64; and the four `*Saved` commands are **REPLACED** by it (a rename,
not a migration — that family was never committed).** **⚠ `mergeEnvVars` moves with the CALL (WI 3), not with
`resolveVars`** — a refinement of §8 trap 2. **⚠ `savedEntitySend.ts` is TRANSITIONAL → must shrink to
`!isDraft && !hasLocalChanges`.** Details → `plan/19` §3 + §11 · `2026-09-29.md`.

**⚠ Two traps invisible in a green suite:** do **not** resolve in both places even briefly (a resolved value can
itself contain `{{…}}`, so a *shadowed* case double-resolves); and **`mergeEnvVars` (global+active) lives only in
`App.tsx:288`** — the engine's `mockHandler` has no equivalent.

## Phase state

**Live: P18** — enterprise SSO / org workspaces / permission gating (**WI 0 done**). **P17**: only item 4's
Electron half open. **P0–P6, P14 done. P7, P9, P16 decommissioned.** `packages/gui` exports `<App />`;
`renderer/` is its only consumer. **⚠ P18 adds a SECOND authorization axis — per workspace
(`PermissionContext`), not per session (`transport/auth/scopes.ts`). Do not merge them** (D19–D21).

## Engine module layout — DONE, and the layering ratchet is CLEAN (branch `engine_modules`)

`Engine_Module_Plan.md` reorganised `packages/engine/src` into feature modules behind a layering rule.
**Phases 0–8 committed** (`c531c7c` → `fbb6fc7`, + `7639458`, `74bb223`, `206d34f`). **The plan's move counts
were 2 right / 5 wrong — measure, never trust the row.**

**⚠ The rule is §6's per-module table, NOT §5.1's blanket ban** (they contradict; §6 governs, decided
2026-09-29). **`tests/engine/layout.test.ts` enforces it** — 5 assertions, teeth verified by mutation — and
**`ALLOWED_VIOLATIONS` is now `{}`**: all 24 violations fixed, each deletion forced by the staleness assertion.
Exemptions: `import type` (erased) and **the two SEAM modules** (`runtime/instances`, `runtime/entityStatus`).

**Placement rule: what a module READS decides its home, not how many callers share it.** Stateless + shared →
extract **down** into `utils/`; reads a file/config/sibling → it belongs in the layer it reads from. **Count the
CALLERS' modules before choosing a destination.** **§5.2's ESLint half is impossible — no ESLint in this repo.**
Tests **do not** mirror `src/` (TESTING.md §3.5). Procedures → skill `bifurc-engine-layout-phase`.

- **⚠ The map's `from` fields keep the OLD paths** (codemod input, not a description of the tree); `--check`
  drift is expected.
- **The GUI has its own unrelated `reloadConfig`** (UI callbacks) — never touch it in this migration.
- **`packages/**/*.test.ts` IS in the `unit` project; `packages/engine/tsconfig.json` covers
  `packages/engine/tests/`.**
- **⚠ The layout ratchet has SEVEN assertions now (was five)** — two silent exits closed on 2026-09-29/30,
  both found while `plan/19` WI 3 added `pipeline/`. (1) *"every real directory is named in `CONTRACT`"* —
  before it, **creating a directory was the one silent way out of the layering rule**: `contractViolations()`
  returns `false` for an unknown module, so its imports were never checked, and the orphan test only iterates
  `CONTRACT`'s keys. (2) *"`ORPHANS_ALLOWED` has no stale entries"* — nothing else reads that map, so an
  entry for a module that **is** imported is inert, and it silently pre-excuses the next orphan of that name.
- **⚠ A test that PINS a split must move with it** (splitting `fileops/` broke 10 tests that name *which module
  registers which command*).
