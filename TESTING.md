# Bifurc — Testing Guide & Coverage Report

_Last reviewed: 2026-09-28 · suite: **2,666 tests / 119 files** — 2,621 passed, 40 skipped. Unit
**2,206 / 89 files**, integration **460 / 30 files** (`2,206 + 460 = 2,666` ✓, both halves measured
separately, not derived). **0 product failures.** The two failure groups are both environment, and both are
named below._

**The last move was the tab-page unification, 2026-09-28** — `TabPage.tsx` now owns the frame of three of
the four bands (title bar / toolbar / split / footer) for REST, GraphQL, gRPC, SOAP *and* the collection
runner. Ten GUI files changed, one added, and **nothing outside `packages/gui/src/`** — no engine, protocol,
client, reducer, `window.api` call or save payload. `2,658 → 2,666` is `+8` tests and `+2` files:
`tests/renderer/tabPageLayout.test.tsx` (5 cases, this move) and `tests/renderer/requestsRunnerTab.test.tsx`
(3 cases, the runner-tab call-site fix earlier the same day). Integration stayed at `460 / 30`, which is what
says no existing case changed behaviour.

**The five cases in `tabPageLayout.test.tsx` pin a regression that shipped and was caught by the user, not by
the suite.** `UrlBar` moved from "block-level child of a column" to "flex item inside a `TabToolbarRow`"; the
new `bare` variant dropped the padding *and the width*, so the bar collapsed to its content width — a short
URL input and dead space to the right of Send, on every protocol. jsdom has no layout engine, so the
assertion is on the **mechanism** (`flex-1 min-w-0`) rather than a measured width, and the file says so.
**Checked by mutation: reverting the class fails exactly one case** and leaves the other four green.

**The move before that was the autosave conversion — the Save / Update button removed from every entity
type, 2026-09-28.** `2,647 → 2,658` was `+11` tests and `+1` file, and the new file was the whole delta: unit
moved `2,187 → 2,198` total (`2,158` passed + `40` skipped) while integration stayed at `460 / 30`. The
eleven cases are `tests/renderer/debouncedAutosave.test.tsx`, driving the new `useDebouncedAutosave` hook
with fake timers.

**The hook has three properties that are load-bearing, and each is a case rather than a comment.** *Silent
arming* — the first snapshot seen while `enabled` becomes the baseline without writing, so a draft whose id is
swapped for the saved one on create cannot write straight back. *Baseline advances only on success* — a failed
write leaves the edit dirty, so a later flush retries it. *Unmount flushes* — leaving a tab mid-debounce does
not drop the edit. `flush()` cancels the pending timer, so `Ctrl+S` / Sync cannot write the same edit twice.
**Checked by mutation, and the first two attempts were false negatives worth recording:** deleting the explicit
`clearTimer()` from the timer path changed nothing, and deleting the effect's returned cleanup alone changed
nothing either — because **the returned cleanup *is* the debounce reset**, so either one on its own is
redundant. Deleting **both** failed **exactly one** case (`expected "vi.fn()" to not be called at all, but
actually been called 3 times`) with the other ten green. The explicit call was then removed rather than left in
as a second mechanism hiding the first.

**The one unit failure is a flake, and it is a different file each run — the mechanism is the same.** On
2026-09-28 it was `tests/store/gitStore.test.ts`'s `beforeEach`, then `tests/proxy/serverUtils.test.ts`'s
`beforeEach` (`Error: Hook timed out in 10000ms`). Both are temp-directory work racing the sandbox's
safe-delete shim (`node-safe-delete-shim.cjs`); both pass in isolation (`serverUtils` **15/15 in 1.35 s**
against 10.4 s under full-suite load). `CODEBUDDY_SAFE_DELETE_ENABLED=0` does *not* help, which is itself the
evidence that it is contention rather than the shim. **Run the failing file alone before believing a
full-suite failure.**

**Four integration suites fail *teardown only*, and four `gitRemote` tests fail for a sandbox reason.** Both
groups are recorded in `.workbuddy-ai/memory/MEMORY.md` under "Measuring the suite". `fileOps`,
`importExport`, `importExportBlob` and `importExportFormats` each report `Hook timed out in 20000ms` in their
`afterAll` while **all 116 of their tests pass**; `gitRemote`'s four assert a remote ref that `git push` does
not write in this shell. Neither is attributable to any change — `grep -rn "packages/gui" tests/integration/`
matches only a comment, so no integration test can even load the GUI.

**The same move rewrote every e2e spec that drove a Save / Update button — `requests-rest`, `mocks-rest`,
`proxy-rules`, `mappings`, `environments`, `app` and `happy-path-workflows`.** Those are **the user's to run**,
so any e2e total below is stale until an owner run; `npm run typecheck:e2e` covers all of them and exits 0.
Three of the rewrites are not mechanical. `mappings` lost its Edit button *and* its `tr:has-text(...)` handle,
because a row's domain and label are `<Input>` values now and the only text a row still carries is the
`.localhost` suffix — rows are matched on `input[value="…"]` instead. `environments` lost both of its `Save`
clicks, because `handleAdd` creates the environment up front and the name and variables autosave. And
`requests-rest` / `mocks-rest` now assert persistence through `window.api.getConfig()` rather than on page text:
the entity name also appears in the **tab label**, which renames locally the instant `Enter` is pressed, so the
old text assertion passed even when nothing had been written.

---

### Earlier record (same day) — the requests empty state becomes the new-tab page, 2026-09-28

**The last move was the requests panel's empty state becoming the new-tab page, 2026-09-28.** `2,638 → 2,647`
is **`+9` tests and `+1` file, and the new file is the whole delta** — unit moved `2,178 → 2,187` while
integration stayed at `460 / 30`, which is what says no existing case changed behaviour. The nine cases are
`tests/renderer/requestsEmptyState.test.tsx`, checked by **two narrow mutations**: reverting `TabBar`'s
`onClick={() => onNewTab()}` to `onClick={onNewTab}` failed **exactly one** case — the `+` one — because a
`MouseEvent` then lands in `draftProtocols[tabId]` as a truthy non-protocol and the tab renders blank; and
widening `ProtocolSelectorTab`'s mock-mode REST-only filter to `() => true` also failed **exactly one** — the
mock-mode card-count case — which is what says the two mock cases are not riding on the request half.

**The same move cleared the 5 failures the tree had been carrying, and both causes were measured rather than
guessed.** Three were stale expectations in `tests/renderer/utils.test.ts`: `a618982` ("Minor UX changes")
moved the 2xx colour to `--c-success` and gave each HTTP method its own `--c-method-*` token, while the test
still named `var(--c-signal)` / `var(--c-amber)`. The **expectations** were the defect, and `git show a618982
-- packages/gui/src/lib/utils.ts` is the evidence — not an inference from the source. The other two were
**non-hermetic integration tests** (`engineSmoke`, `hostStartup`): both assert `status.running === true`, the
engine binds the forward proxy on `settings.forwardProxyPort` (`runtime/host.ts:474`), `loadSettings()` defaults
that to **8080**, and a running `electron.exe` — the app itself — holds 8080 *and* the edge's `:80`. Both tests
pinned `port` and `webhookPort` with `getFreePort()` but not `forwardProxyPort`, so they failed whenever
anything owned 8080. **Proved rather than argued:** with `BIFURC_PROXY_BASE_PORT=18080` both passed, and after
pinning `forwardProxyPort: proxyPort` in each `saveSettings` call both pass *with the app still holding 8080*.
Pinning it also gave each test's `portIsFree(proxyPort)` poll its subject — it had been asserting the freedom
of a port nothing ever bound.

Those two tests now also **de-dupe their three ports explicitly** (`while (x === y) x = await getFreePort()`),
the shape `proxyHarness.ts:435` and `settingsMutations.integration.test.ts:325-328` already use. A comment
claiming the three `getFreePort()` calls "are distinct in practice" would have been the only place in the suite
resting on an assumption its own siblings guard against — `getFreePort()` binds `0` and reads the assignment
back, so what it returns is a **reservation, not a guarantee**, and nothing makes three back-to-back calls
distinct.

---

### Pre-2026-09-28 record — the browser-host decommission, 2026-09-26

_Last reviewed (then): 2026-09-26 · suite: **2,638 tests / 115 files** — 2,598 passed, **0 failures of any
kind**, 40 skipped. Unit **2,178 / 85 files**, integration **460 / 30 files** (`2,178 + 460 = 2,638` ✓, both
halves measured separately, not derived)._

**The last move was the decommission of the browser host and the container image, the same day.** The desktop
app is now the single build target, so three suites went with their subjects — `web/tests/*` (27 cases: 12 + 6
+ 9), `tests/docker/entrypoint.test.ts` (11) and `tests/integration/guiStaticServer.integration.test.ts` (14) —
plus the cases whose subject the engine's own removal deleted. **`120 → 115` files and `2,739 → 2,638` is
`−5` files and `−101` tests**, and the split is **unit `−4 / −86`** (`89 / 2,264` → `85 / 2,178`) and
**integration `−1 / −15`** (`31 / 475` → `30 / 460`). Integration moving is the load-bearing part: it says the
cut reached the browser half, which is where the 14 static-server cases lived. **The reconciliation is
`27 + 49 + 25 = 101`**, one term per commit, and no term is inferred from the other half.

---

### Pre-decommission record — `plan/17` work items 1–9, 2026-09-25/26

**Kept in full, because the reasoning below is what the surviving code was built on.** The port model, the
`WorkspaceRuntime` deletion, the REST-only mocks and the 144 → 132 client-surface change all still hold. Two
things in it do not: **the Docker image and the browser host it describes are gone**, and the commands it names
for them (`verify:web*`, `verify:docker-entrypoint`) are gone from `package.json`.

_Last reviewed (then): 2026-09-26 · suite: **2,739 tests / 120 files** — 2,699 passed, **0 failures of any kind**, 40
skipped. Unit **2,264 / 89 files**, integration **475 / 31 files** (`2,264 + 475 = 2,739` ✓, both halves
measured, not derived). **The last move was `plan/17` work item 5 on 2026-09-25/26** — the web GUI is bundled
into the Docker image and served by an entrypoint that derives its port, so `tests/docker/entrypoint.test.ts`
(11 cases, unit) and `tests/integration/guiStaticServer.integration.test.ts` (14 cases, integration) are its
coverage. `2,714 → 2,739` is **`+25` tests and `+2` files, and each new file is entirely within its own half**
— unit `2,253 → 2,264`, integration `461 → 475` — which is what says no existing case changed behaviour. The
same item also found that **`npm ci` was broken repo-wide**, so the image could never have built: the lockfile
still described `apps/web` (gone) and did not describe `packages/gui`, so it failed under either manifest set.
Fixed with zero version changes. **Before it, work item 6 the same day** — the four host-level ports
now live in one global Ports section, and `tests/renderer/portsSettings.test.tsx` is its first coverage.
`2,703 → 2,714` is **`+11` tests and `+1` file, and the new file is the whole delta**: unit moved
`2,242 → 2,253` while integration stayed at `461 / 30`, which is what says no existing case changed behaviour.
Two of its eleven cases were checked by a narrow mutation — a fifth input fails the count case, an unmounted
section fails the wiring case, and each run failed **exactly one** case. **Before it, work item 8 the same
day** — mocks became REST-only, so
the GraphQL / SOAP / gRPC mock kinds, their folder kinds and the three `grpc.*MockServer` commands were removed.
`2,745 → 2,703`: **`−42` tests and `−1` file, every unit named in `plan/17` §5's table** rather than inferred —
the deleted `protocolMockHandler` suite alone is `−23`. **The client surface went 144 → 132 keys**, a recorded
contract change rather than a re-baseline (`packages/client/src/surface.ts`'s header; the declared type went
140 → 128, so the four-key gap is unchanged). Before it, work item 7 landed the same day: a save now re-binds the
edge, the forward proxy and the webhook, so **two cases were inverted and two new ones took their place:
`0` tests, `0` files, and each new case was checked by a narrow mutation** (§5's table). Before that, work items
1, 2, 3 and 9 landed the same day: every port became engine-level, so the per-workspace allocation suite and
`WorkspaceRuntime` itself went with the mechanism — `2,766 → 2,745`, **`−21`, every unit named in `plan/17`
§5's table**. **The `gitOps` safe-delete-shim `EBUSY` flake did not reproduce in any of the three post-WI-8 runs**
(the full run, the unit half, the integration half), which is the point: a run that reports it is reporting the
environment, not a defect. When it does appear it passes alone (4/4, zero diff).
**The failure mass is closed: 66 → 0.** It was one
mechanism, and it was never the test's: `plan/15` **work item 3 moved `proxy/server.ts`'s reverse half — and
the home page with it — onto the edge** (`proxy/edge.ts`), so every case that asked `startServer()`'s port for
a `*.localhost` mapping or for `/` got a deliberate **400** instead of the mapping. `server.ts` says so at its
own line 384 (*"The reverse half is no longer here… served by `proxy/edge.ts` on the host-level **edge**
port"*), and this file's §3.3 mechanism list is the place that decision lives. **The reason it took a session
to attribute is worth more than the number, and it is the part to read before trusting any figure here:**
`plan/15` records WI 3's and WI 4's test files as **written** and **neither existed** — `tests/proxy/edge.test.ts`
(*"40 cases"*) and `tests/integration/edgeSplit.integration.test.ts` (*"14 cases… the plan's own verification
step"*). So the reverse half's cases were never moved out of `server.test.ts`, none of the eight *"migrated"*
suites called `edgeRequest`, and the *"eight mutations"* were unverifiable. **A plan's "done" is a claim about
the tree until the tree says otherwise.** The `PRODUCT` half of WI 3/4 was real and trustworthy throughout;
the `INSTRUMENT` half was owed, and is now paid.
> **The migration, and what each suite actually needed — because "migrate onto `edgeRequest()`" was wrong for
> most of them.** Re-pointing the request seam was the fix in only two files; the rest needed their *premise*
> re-read against the split.
> - **`tests/proxy/edge.test.ts` — written (28 cases).** The 17 reverse-half cases moved out of
>   `server.test.ts` (which is now the forward half, 78 cases). Mock-based on purpose; the real-I/O half is
>   `edgeSplit.integration.test.ts`.
> - **`tests/integration/edgeSplit.integration.test.ts` — written (18 cases).** The plan's own verification
>   step, and it **supersedes `harnessSplit.integration.test.ts`** (deleted; its 8 cases are folded in, which
>   its own header asked for). It pins what only real sockets can: two listeners answering **the same URL
>   differently** (edge `200` / forward `400`), the edge's raw-bytes-to-`req.socket` handler, and
>   `resolveConfig` **per request**.
> - **`settingsMutations` — 28 → 0, and it was not a re-point.** Its `bootOnConfiguredPort()` started only the
>   forward proxy while `fetchHello()` asked a `*.localhost` host; it now boots **both** listeners the way
>   `index.ts` does (edge on `settings.port` first, proxy on a resolved port). Its subject also had to change:
>   *"changing the proxy port moves the live server"* is **superseded** — `settings.port` is the **edge's**, so
>   `config.save` keeps the proxy on its resolved port (`getPort() || incoming.port`) and the setting is a
>   preference for the *edge's* next boot. 28 → 31 cases, because the P14 pair split into two honest halves.
> - **`workspaceCommandScope` — 1 → 0, and the assertion was inverted.** `portIsFree(configPort)` was false
>   because the **host binds the edge** on `settings.port` (`host.ts:493`). The port is *correctly* occupied;
>   what must not happen is a third *proxy* taking it, so the case now asserts the edge holds it and that
>   neither runtime's proxy is on it.
> - **`workspaceDispatchScope` — 3 → 0.** Relative targets to a `*.localhost` host are refused by the forward
>   half; the mocks are matched on the **absolute** URL, so the targets became absolute-form. Its fixture mock
>   also needed `mockedResponseHeaders` — without it the mock is a *partial* overlay that still calls an
>   upstream, and the cases failed **502**, measuring the upstream instead of the dispatch scope.
> - **`workspaceRuntime` — 1 → 0.** The home page is the edge's now, so the *liveness* probe became the
>   forward proxy's own documented refusal. The case's own comment had already said the home page was the
>   wrong instrument for content; the split simply removed it for liveness too.
> - **`entityConflictScope`, `foldersAndWorkspaces`, `protocolExecution` — 3 load errors → 0.** Stale
>   `@/ipc/handlers/*Kinds` imports repointed at `@bifurc/engine/store/kinds/` — the **WI 10** move these
>   files had been missed by. (Their 69 cases were `skipped`, which is why the skip count fell 109 → 40.)
> - **`ipc/handlers.test.ts` — 1 → 0.** Three `toHaveBeenCalledWith(<config port>)` assertions became the
>   **resolved** port: `server.start` / `server.restart` / `config.save` all read `getPort()` first, because
>   the port `app.json` names is the edge's.
> - **Mutations, each judged by exit code.** Harness collapse (`edge.start(edgePort)` → `edge.start(port)`):
>   **EXIT=1**, 11 of 18 red. Product regression (`sendHtml(socket, 400…)` → `200`): **EXIT=1**, exactly the
>   3 boundary cases. The edge caching `resolveConfig`: **EXIT=1**, the two edge-follows cases. Collapsing the
>   two listeners in `settingsMutations`: **EXIT=1**, 8 red. `portAllocator`'s base-skip
>   (`findFreePortFrom(proxyBase + 2)`): **EXIT=1**, 6 red. Every mutated source was verified byte-identical
>   to its pre-mutation backup afterwards.
> - **The split closes against the run's own total.** `2,289 + 464 = 2,753` · `88 + 31 = 119` (the figures as
>   the migration landed; work item 5 moved them to `2,310 + 465 = 2,775`, see the note below). Note the naive
>   reading is wrong by exactly the skip count (the per-project command lists non-skipped tests only), and
>   **a single-project run is not the same measurement as the full run** — integration alone read `454` before
>   the harness file was folded into `edgeSplit` and `464` after, which is `454 − 8 + 18` exactly.
> - **A flake was found and fixed rather than tolerated, and it was mine to fix.** `portAllocator` asserts on
>   an *ephemeral* base's **successors** (`base + 1`, `base + 2`), and this suite's extra listeners landed on
>   them: three cases failed in one full run (`expected 49529 to be 49528`, `expected 50235 to be 50234`, and
>   a `nextFreePortAbove` miss) while passing **35/35 isolated**. This is the **same mechanism**
>   `allocateFromUncontendedBases`' header already records for `workspaceCommandScope` (2026-09-21), arriving
>   through a third suite — so the remedy is now a shared `holdBaseWithFreeSuccessors()` used by every
>   successor-asserting case, and **the regression it guards still fails** (`EXIT=1`, 6 red). Two consecutive
>   full runs are green afterwards.
> - **`gitStore.test.ts` failed once during this session's measurements, and it is the documented shim
>   flake** — not a new finding: `STACK_TRACE_ERROR` in `beforeEach`, **24/24 alone**, and
>   `git diff HEAD --numstat` returns **no rows** for it. The caution further down this block stands.
> **The harness split is done and proven.** `proxyHarness` boots both listeners (`RunningProxy = { port,
> edgePort, instance, stop }`), `edgeRequest()` names the edge, and the edge's `resolveConfig` is the same
> `instance.dispatchConfig()` composition `runtime/host.ts` uses. **Measured additive** at the time it landed:
> **+8 tests, +1 file, 0 failures** (2,713/117 → 2,721/118, same 11 files red at the same per-file counts).
> **The 66, attributed — because a failure you cannot attribute is a question, not a finding.**
> - **44 in five files with _zero_ diff from the companion removal** (`proxy/server.test.ts` 17,
>   `integration/mocks` 12, `integration/capture` 7, `integration/mappingsAndRules` 7, `ipc/handlers` 1).
> - **22 in six files whose entire diff was the deletion of one `companionPort` token.** Their failures
>   carried the same signatures as the 44 — `expected 400 to be 200`, `Unexpected token '<'`,
>   `ECONNREFUSED` on a `127.0.0.1` home-page fetch, `port … did not become open/closed within 5000ms`, and
>   `toHaveBeenCalledWith(80)`.
> - **Each was run alone before being labelled** — so they were real failures, not pack-only flakes, and real
>   failures of **prior work's** uncommitted change. **Every one of them is now closed**, and the fix for most
>   was not the request seam but the premise: see the list above.
> The figure above replaces **2,815 / 123** as of the **companion removal** (`plan/15` §6.4, `plan/16`), then
> **2,721 / 118** after the harness split. The companion removal's own reconciliation (−102 tests / −6 files;
> two suites deleted = 27 tests, the other 75 leaving as *assertions inside cases* and `it.each` rows, which
> `grep -c "^\s*it("` cannot see) is unchanged and still not decomposable further — quoted as measured. **A
> delta is the signal, and it is re-measured rather than carried forward.**
> **Earlier the same day it reported 5 failures** — all port conflicts from a headless host left running
> (`80`, `8080`, `9101`). Each one **said so** — *"9101 must be free for this case to be decisive"*, or a
> preflight `[ 'port-in-use' ]` the case asserts is empty — so the suite reports the conflict rather than a
> misleading result. **Stop the host before measuring: the count is not the finding, the mechanism is.**
> (**`9271` was on this list and is not any more** — the companion server was retired outright, so nothing
> in the product binds it. A case still probes it, in `workspaceHost.integration.test.ts`, but *to assert the
> host has not re-grown one*, and it holds its own probe so a busy `9271` cannot make it indecisive.)
>
> **And a second caution, about a failure that is not one.** The **full** run has reported `gitStore.test.ts`
> and `gitOps.test.ts` failing with `EBUSY: resource busy or locked, rmdir '…\lp-gitops-test-…'` inside the
> sandbox's safe-delete shim — while **both pass in isolation** (exit 0, shim enabled *and* disabled), and the
> full-run count moves between consecutive runs. Under a parallel run a real `git` child still holds its temp
> directory when the shim tries to remove it, and Windows refuses an `rmdir` on an open handle. **Observed
> again 2026-09-24** (`gitStore.test.ts`, `STACK_TRACE_ERROR` in `beforeEach`, 24/24 alone, zero diff).
> **A failure that disappears in isolation is evidence about the *pack*, not about the file — run it alone
> before changing anything.**
>
> **▶ Later on 2026-09-24 — `plan/15` work item 5, the GUI on the edge, and the numbers above include it.**
> `+22 tests` (`2,753 → 2,775`), `+0 files`, `0 failures`, reconciling exactly as `+21` unit
> (`tests/proxy/edge.test.ts` `+10`, `packages/engine/tests/runtime/hostArgs.test.ts` `+11`) and `+1`
> integration (`tests/integration/edgeSplit.integration.test.ts`). **Nothing was deleted, so the file count is
> unmoved** — which is the case most likely to go unreconciled, because there is no new filename to notice.
> The feature is a *global reverse-proxy mapping* rather than a static-file server (see `plan/15` §3.3's `G`
> row), so the tests are the ordinary kind — a mocked `http.request` assertion in the unit suite, and a **real
> GUI server on a real socket** in the integration suite, which is what makes the design claim checkable:
> `index.html`, a hashed asset path and a query string all reach the GUI server unchanged, while
> `mapped.localhost` still goes to its mapping on the same listener. **Three mutations, each judged by exit
> code, each caught by exactly the right case:** the precedence inverted so a workspace mapping wins
> (`EXIT=1`, 1 case); `isLocalAddress` widened to accept every host (`EXIT=1`, 2 cases); the IPv6 arm of
> `hostnameOf()` removed (`EXIT=1`, 1 case). Every mutated source was restored byte-identically, and the
> engine's own `hostnameOf()` fix is pinned by the IPv6 routing case rather than by a test of the helper.

This document is both the **how-to for running tests** and the **coverage/regression report**
produced by the test-suite review. Read the top section for commands; read
[Findings](#findings--why-the-old-tests-didnt-catch-anything) for what was wrong and
[Backlog](#backlog--what-is-still-untested) for what remains.

> **Looking for "is the happy path for every feature covered?"** → see
> **[HAPPY_PATH_COVERAGE.md](./HAPPY_PATH_COVERAGE.md)**, a per-screen/per-feature matrix
> with the gaps listed explicitly. This document covers *how* to test and the coverage
> numbers; that one covers *what* is and is not workflow-tested.

---

## 1. Commands

Run everything from the repository root. (The app *is* the root — it was flattened on 2026-09-14,
so there is no `cd bifurc` any more.)

| Command | What it runs | Use it for |
|---|---|---|
| `npm test` | All Vitest projects (unit + integration) | The default. Run before every push. |
| `npm run test:unit` | Unit project only (`tests/**` minus `tests/integration`) | Fast inner loop while coding. |
| `npm run test:integration` | Integration project only (`tests/integration/**`) | Verifying real proxy routing. |
| `npm run test:watch` | Unit project in watch mode | TDD. |
| `npm run test:ci` | All projects **with coverage + thresholds** | What CI runs. Fails on a coverage regression. |
| `npm run typecheck` | `build:packages`, then `tsc --noEmit` over `src/` | Main-process types. Builds `packages/*` first so it cannot pass against a stale `dist/`. |
| `npm run typecheck:e2e` | `tsc -p e2e/tsconfig.json` | E2E spec types. |
| `npm run typecheck:renderer` | `tsc -p tsconfig.renderer.json` | **Known failing** — see [§7](#7-known-issues-found-during-this-review). Tracks renderer type debt. |
| `npm run test:e2e` | Builds the app, then runs Playwright against real Electron | Full UI regression. Needs a display (or `xvfb-run` on Linux). |

> **Note on running a single file — do not combine `--project` with a path filter.** On this
> environment (Vitest 4.1.5, `test.projects`) the two together break every collected file with
> `TypeError: Cannot read properties of undefined (reading 'config')` at the first `describe`, or
> `Error: Vitest failed to find the current suite`. It is a *runner* failure, not a test failure, and it
> is total: it hits root `tests/**` and `packages/**` alike, so it is easy to misread as "my change broke
> everything". Verified to be the combination rather than either half — `vitest run <path>` alone passes,
> and `vitest list --project unit` loads the project config fine and lists 1,982 cases.
>
> So the documented per-file commands in this file (e.g. `npx vitest run --project integration
> tests/integration/…`) are **wrong in this environment**; drop the `--project` flag, or run the whole
> project. The `npm run test:unit` / `test:integration` scripts are unaffected — they carry no path
> filter.
>
> **▶ 2026-09-22 — this no longer reproduces, and the warning is kept as history rather than as advice.**
> The combination was used **twelve times** in one session (`--project unit` and `--project integration`,
> against root `tests/**` and `packages/protocol/**` alike, plus single-file and whole-project runs) and
> every invocation collected and ran normally. Nothing here says the failure was never real — it was
> recorded as *verified* at the time — but a note that tells you to avoid a working command costs a
> session's worth of convenience, so the observation is recorded next to it. **Re-verify before acting on
> either version.** The general rule this is an instance of is in `TRAPS.md`: a count or a warning written
> into prose is never re-derived, so it drifts.

> **Note on coverage in restricted environments:** Vitest deletes its `coverage/` directory
> before and after each run. In sandboxes that block bulk deletes this fails with
> `SAFE_DELETE_BULK_CONFIRM_REQUIRED`. Work around it with
> `npx vitest run --coverage --coverage.clean=false --coverage.reportsDirectory=coverage-run`.

---

## 2. Findings — why the old tests didn't catch anything

The suite passed 100% before this review and it was still unable to catch the bugs it was
supposed to catch. Three root causes:

### 2.1 The proxy tests mocked the network

`tests/proxy/server.test.ts` (80 tests) mocked `net`, `http`, `https`, `@/store/config`
**and** `@/store/workspaceFs`. The result is a test suite that can only assert
_"the proxy tried to do something"_:

```ts
// The old rule test — asserts a function was called, not that routing is correct.
it("proxies via rule when URL matches proxy rule pattern", () => {
  sendHttp(socket, "GET", "http://api.example.com/data", "api.example.com");
  expect(http.request).toHaveBeenCalled();   // ← could point anywhere
});
```

This cannot detect: a mapping pointing at the wrong port, a rule resolving the wrong
target, a disabled entity being applied, a mock merging incorrectly, or a 502 where a 200
was expected.

### 2.2 The E2E tests could not fail

Every interaction in `mappings`, `proxy-rules`, `mocks-rest`, `requests-rest`, `protocols`,
`environments`, `capture-ws-webhooks` and `app.spec.ts` was wrapped in a visibility guard:

```ts
const nav = page.locator("text=Mappings, [data-testid='nav-mappings']").first();
if (await nav.isVisible()) {          // ← if the panel is gone, the test still passes
  await nav.click();
  const content = await page.textContent("body");
  expect(content).toBeTruthy();       // ← "the page has text" is always true
}
```

`expect(body).toBeTruthy()` is satisfied by any non-empty DOM. These specs were
green even against a completely broken build.

### 2.3 Type checking never covered the renderer

`npm run typecheck` runs `tsc --noEmit` against the root config, whose `include` is
`["src/**/*"]`. The renderer was **never type-checked**, and the E2E `tsconfig.json`
inherited `rootDir: ./src`, so it errored out immediately and checked nothing either.
Running the renderer through `tsc` today reports **154 type errors** (see §7).

---

## 3. What was added

### 3.1 A real end-to-end routing harness

`tests/integration/proxyHarness.ts` boots the **actual** `startServer()` against a **real
on-disk workspace** and drives it with **real TCP/HTTP sockets**. The production store
already exposes `setDataRootOverride()` and `setSettingsPathOverride()` for exactly this
purpose, so nothing here is a test-only code path.

The full pipeline under test:

```
workspace files on disk → loadConfig() → enabled.json sets → workspaceCfg()
   → dispatch() → mock / rule / mapping / passthrough → real response bytes
```

Two suites use it:

| File | Tests | Covers |
|---|---:|---|
| `tests/integration/mappingsAndRules.integration.test.ts` | 18 | `*.localhost` → target forwarding (method/path/query/body preserved), unmapped 404, disabled-mapping 404, unreachable target 502, home page, rule → external target, rule → mapping target, regex rules, missing-target 502, disabled-rule passthrough, absolute-form passthrough, 400 handling, request-script header injection, response-script body rewrite, `via=` log values |
| `tests/integration/mocks.integration.test.ts` | 15 | full mock replaces upstream (status/headers/body), regex mocks, wildcard method, base64 bodies, method mismatch → passthrough, disabled mock → passthrough, invalid-regex safety, **mock beats rule**, partial-mock merge (status-only and body-only), `{{var}}` resolution in body and headers, SSE streaming, response delay |

### 3.2 Unit tests for the routing logic that had none

| File | Tests | Covers |
|---|---:|---|
| `tests/proxy/proxyHandler.test.ts` | 11 | `matchProxyRule()` — exact vs regex, invalid regex, external vs mapping targets, missing target, first-match-wins |
| `tests/proxy/serverUtils.test.ts` | 14 | `workspaceCfg()` (the function that decides what is enabled), `activeEnv()` merge semantics, `loadEnabledSets()` against real `enabled.json`, `mkId()` |

### 3.3 Proof the new tests actually fail when routing breaks

Adding tests is worthless if they cannot fail. I temporarily forced
`matchProxyRule()` to return "no match" and re-ran the integration suite:

```
× routes a matching request to an external target
× routes a matching request to the target of a referenced mapping
× matches rules by regex
× logs a rule-routed request with via=rule
× applies a request script to the forwarded request
× applies a response script to the returned body
Tests  6 failed | 26 passed
```

The mutation was reverted. One test (`returns 502 when a rule matches but its target is not
configured`) survived the mutation because the broken request happened to 502 anyway; it was
rewritten to point at a **reachable** upstream, so it now fails if the rule is ignored.

### 3.4 Second through seventh passes — happy-path coverage for the untested features

The first pass fixed the *routing* tests. A follow-up pass asked a different question:
*does every feature have a basic-workflow test?* Three whole features had none, and are now
covered by tests that run in CI (not just in the unverifiable E2E layer). A third pass then
closed the biggest remaining hole — the **"hit Send" path** — and the Health Bar's polling.
A fourth pass closed the *data* half of the app: folder management, workspace CRUD, the
collection runner, and the git publish/sync pipeline. A fifth pass closed **capture** — the
last data workflow — and fixed a coverage-scope bug that had hidden 17 renderer files.
A sixth pass took the two highest-risk remaining 0% files — **TLS interception** and the
**companion WebSocket server** — plus the three **protocol tab reducers**, and found two
more production bugs doing it. (**The companion server was later retired outright**, so its
two suites are gone; the TLS and reducer findings stand, and the sixth pass's remaining value
is the *method* — it took the two 0% files and found bugs in both.) A seventh pass closed
**settings mutations**: the last behaviour that changed live state with nothing behind it
(and it found a third bug — see §7). An eighth pass converted the import/export tree to the
content-based interface for P3 and added the blob-layer suite — which, being the first test
ever to execute `workspace-zip` end to end, immediately found that ZIP export had been
throwing on every call (§7).

| Suite | Tests | Covers |
|---|---:|---|
| `tests/integration/applications.integration.test.ts` | 12 | The **Applications** feature end to end: save (with pre-computed command) → list → run a **real child process** → stream stdout/stderr → exit code → stop a long-running process → debug mode with its port → delete. Drives the real `applications:*` IPC handlers. |
| `tests/integration/auditLog.integration.test.ts` | 13 | The **Audit Log** screen: real git commits → `queryLog()` newest-first → filters by entity/action/id/search/file-path → pagination → changed-files resolution → point-in-time entity reads (the diff view). |
| `tests/integration/importExport.integration.test.ts` | 18 | Export→import round trip for the native Bifurc-JSON formats + collision strategies (`keep`/`override`/`new`) + preflight failure paths. Reads and writes **plain files**, through the client-side shims in `importExportHarness.ts` — this suite owns the *codecs*. |
| `tests/integration/importExportFormats.integration.test.ts` | 17 | Round trips for the foreign formats (Postman, WireMock, HAR, Insomnia, OpenAPI) and the **whole-workspace snapshot**. Also codec-level, same harness. |
| `tests/integration/importExportBlob.integration.test.ts` | 45 | **The blob layer, through the real command layer** — `export.create` → `blob.put` → `import.preflight` → `import.commit` → `blob.release` driven by a real `CommandRegistry`, so every payload also passes the frozen Zod schemas. Owns the *transport*: the inline-vs-blob threshold (and that an inline export never creates the blob root), the `blob.read` chunk loop, "upload once, read twice", the blob surviving a commit so a retry works, `blob-not-found` as a result rather than a throw, the filename reaching the importer (dotenv naming, OpenAPI YAML), `workspace-zip`'s stream-shaped branch, the `keep`/`override`/`new` strategies over the wire, and a sweep asserting **all 17 registered formats** round-trip. |
| `tests/integration/upstreamFetch.integration.test.ts` | 12 | `fetchUpstreamResponse()` against a real upstream: status/headers/body, gzip decoding, target-path resolution, request/response scripts, hop-by-hop stripping. |
| `tests/proxy/decompressUtils.test.ts` | 19 | Response decompression (gzip/deflate/br/zstd/stacked) — on the path of every proxied response. |
| `tests/proxy/responseUtils.test.ts` | 16 | Header framing: hop-by-hop stripping and `set-cookie` handling (must not be comma-joined). |
| `tests/proxy/scriptContext.test.ts` | 33 | The chai-like `expect()` that backs user-written `lp.test(...)` scripts — every matcher in passing, failing and negated form. |
| `tests/proxy/scriptExecutor.test.ts` | 33 | Proxy-rule scripts + request pre/post/test scripts, including sandbox isolation (`require`/`process` unreachable). |
| `tests/applications/commandGenerator.test.ts` | 54 | Every run-config type (node/npm/python/java/maven/gradle/dotnet/go/docker/compose/spring-boot) plus its debug variant. |
| `tests/applications/portUtils.test.ts` | 7 | Real port occupancy checks. |
| `tests/ipc/importExport/registry.test.ts` | 15 | Import/Export registry invariants — most importantly that no format claims support without an implementation, and that only `workspace-zip` uses the path-shaped slots. |
| `tests/subscription/entityCount.test.ts` | 8 | The licensing gate stubs, pinned so adding a real limit is a deliberate change. |
| `tests/integration/protocolExecution.integration.test.ts` | 44 | **The "hit Send" path** — `request:replay` (REST), `graphql:execute` + `graphql:introspect`, `soap:execute` + `soap:fetchWsdl`, and `healthbar:checkUrl`, all against real local servers. Also pins the (unimplemented) gRPC contract and the protocol-entity CRUD round trips. |
| `tests/integration/gitSync.integration.test.ts` | 29 | **Git publish, locally** — `entity:publish` / `git:sync` / `folder:publish` against a real repo: the commit *subject format* the Audit Log parses (`create mock Name`), the bundled folder form, explicit messages, the no-op case, `git:diff` for clean/modified/new/deleted, `git:discard` + `entity:restore` (incl. the `names.json` re-sync), `git:history` / `history:list` / `history:diff`, and `sync:getEntityStatus` dirty markers. |
| `tests/integration/gitRemote.integration.test.ts` | 24 | **Git sync, remotely** — `sync:setRemote` in all three of its branches (empty→empty push, empty→non-empty clone, refusal when both sides hold data), against a real **bare repo** as the remote. Plus `sync:push`, `sync:pull` (fast-forward + `updatedIds`), `sync:disconnect`, `sync:setAutoSync`, `getRemoteHead`, and the workspace-identity adoption on clone. |
| `tests/integration/foldersAndWorkspaces.integration.test.ts` | 25 | **Folder management and workspace CRUD** — `folder:add/rename/move/delete` and `workspace:add/rename/setActive/delete`, asserting the on-disk result (`index.json`, real directories, surviving entities) rather than in-memory config. |
| `tests/renderer/collectionRunner.test.ts` | 30 | **The Run button's engine** — `runCollection` against the *real* `executeIpcScript` sandbox (only `replayRequest` stubbed): per-request orchestration, `{{var}}` resolution, start/done callbacks, cancellation, error isolation, pre/post/test scripts, and `generateHtmlReport`. |
| `tests/integration/runnerStorage.integration.test.ts` | 17 | **Runner persistence** — `runner:saveReport/getHistory/saveConfig/loadConfig/listFolderIds/exportReport` against a real workspace, asserting the files that land in `requests/.runs/`. |
| `tests/integration/capture.integration.test.ts` | 20 | **The capture half of capture-and-replay** — real traffic through the real proxy, asserting the emitted `RequestLogEntry`: method/url/host/status, all five `via` values (`rfc6761`/`mock`/`rule`/`proxy`/`error`), target, duration, header capture with hop-by-hop stripped, base64 request/response bodies (incl. multi-byte UTF-8), the **512 KB response-body cap**, "no capture for the home page", and `capture:shareJson` write/cancel. |
| `tests/renderer/captureUtils.test.ts` | 26 | **Capture → mock / saved-request conversion** — `buildMockInitial` (status defaulting, binary-vs-text body encoding, `{}` fallback, raw capture preserved for byte-exact replay), `reqToHeadersBody` (decodes the body, strips `host`/`connection`/`content-length`/…), `deriveType` (content-type first, extension fallback, `other`), plus the table formatters and base64 plumbing. The `via` table is **enumerated** rather than sampled — every member of the union is asserted to have a label and a colour, which is what makes a new member's absence a failure here and a blank badge in the app. `mapping` (work item 4) shares `rfc6761`'s label and colour, because both are "served by the proxy" from the user's side. |
| `tests/integration/tlsIntercept.integration.test.ts` | 11 | **TLS interception** — generates a **real CA** with mkcert (`createCA`, not mocked), boots the real proxy with TLS on, and speaks real TLS through a real `CONNECT` tunnel. The decisive assertion is a handshake with `rejectUnauthorized: true` against the generated CA: only a leaf genuinely signed by our CA *and* valid for the requested hostname can pass. Also covers the negative control (a cert for host A must not validate for host B), the cert cache, routing over the decrypted stream (mock + proxy rule), passthrough to a real HTTPS upstream, capture of the decrypted request, the blind-tunnel fallback when `tlsEnabled` is false, and the 502 path when the CA is unusable. |
| `tests/integration/workspaceHost.integration.test.ts` | — | **`plan/14` WI 3's criterion, in its post-companion form.** The companion was retired outright (`plan/15` §6.4), so the criterion is asserted in the only surviving falsifiable shape: two **real bind attempts**. Direction 1 — the host **holds** its edge port (`runtime/host.ts` composes the edge deliberately, so a `listen()` must FAIL). Direction 2 — the host has not *re-grown* a companion, probed against the retired `9271` and compared to the pre-start reading, so a busy port cannot make the case indecisive. Both proven fail-able by mutation. |
| `tests/renderer/protocolTabReducers.test.ts` | 45 | **The GraphQL / SOAP / gRPC tab state machines** — init from entity and from draft, the send lifecycle, `SET_FIELD`/dirty, the save lifecycle, and the serialization helpers (`stateToMockPayload`'s `operationNameMatch` round trip and `responseDelay \|\| undefined`, SOAP's default-envelope fallback and always-enabled mock payload, gRPC's `LOAD` merge and `protoFileId ?? ""`). Includes the regression guard for the `REFRESH` bug (see §7). |
| `tests/integration/settingsMutations.integration.test.ts` | 28 | **The Settings screen, measured on the running app** — what a user changes in Settings actually reaching the *running* app. **Inverted 2026-09-23 by the proxy split** (`plan/15` work item 3): the port in Settings is the **edge's** now, so a save no longer moves the forward proxy — it keeps the port it resolved (D16's *remembered resolved port → fall through*), and `server:status` reports that port rather than the settings one. Also `server:restart`/`stop`/`start` reusing the resolved port, toggling TLS reloading the CA, theme and zoom persisting and clamping, and — the part that caught a bug — a **live config mutation (`mock:add`/`update`/`delete`, workspace switch) reaching the running proxy with no restart**, sent as **absolute-form** requests so they exercise the forward proxy's mock path rather than the edge's. Registers the real `config:save`/`server:*`/`theme:*`/`zoom:*` handlers; only Electron and `@/main` are mocked. **15 of its 28 are red as of the companion removal** — all the WI 3 mechanism, not this suite's own subject: 10 are the home-page/`*.localhost`-against-`startServer()` 400, and 5 wait on the *edge's* port after booting only the forward proxy. |
| `tests/proxy/edge.test.ts` | 47 | **The edge router** (`proxy/edge.ts`, `plan/15` work items 3 and 4) — the reverse half, on the host-level listener. Drives the real `http.Server` by capturing `http.createServer`'s handler: proxying a mapped `*.localhost` to its target, mocks applied **ahead** of mappings, hop-by-hop filtering, array-valued headers, mid-stream errors, the 512 KB log slice, the home page (**including that it never writes through `res`** — the measured basis for the whole design), the unmapped-domain 404, **re-resolving the active workspace per request**, the bind guard, `listen` called **exactly once** (D16: bound, not scanned), and lifecycle. **Work item 4:** any configured domain routes — a mapped `app.example.com` proxies and logs `via=mapping`, a mock applies to it, a LAN `target` reaches the upstream call verbatim, the two `via` labels are asserted **as a pair**, and the **negative** that matters: a host that is neither mapped nor `*.localhost` gets the home page and is neither proxied nor logged. |
| `tests/integration/edgeSplit.integration.test.ts` | 18 | **The split itself, end to end** — two listeners with two jobs, real I/O. A `*.localhost` request reaches its mapped target **through the edge**; the forward proxy refuses a relative request **naming the edge**; and the assertion no other suite can make — **the same `Host` and path answered on each port gives two different answers**, because that difference *is* `plan/15` work item 3. Plus §3.4's headline: a forward-proxied request can now be **mocked**, with no upstream contacted. **Work item 4:** a mapped non-`.localhost` domain forwards to its target, a mock applies to it, an unmapped one gets the home page **without the upstream being contacted**, and an unmapped `*.localhost` still gets its 404. The request is made from the same machine — the *"from a machine that is not the engine's"* half of that work item's verification step is the deployment half, and is not claimed here. |

---

## 4. Coverage report

### 4.1 Headline numbers

| Metric | Before (same scope¹) | **Now** | Change |
|---|---:|---:|---:|
| Statements | 24.16% | **49.07%** | +24.9 pts |
| Branches | 14.87% | **32.66%** | +17.8 pts |
| Functions | 17.11% | **37.26%** | +20.2 pts |
| Lines | 25.49% | **51.52%** | +26.0 pts |

¹ Both columns use the *widened* `include` scope (which adds `packages/gui/src/panels/**/*.tsx`, ~0%
covered), so the comparison is apples-to-apples. The original narrow-scope baseline was
27.71% statements; the widened scope *lowered* the headline to 24.16% at the time, and the
current 49.07% is a real gain on top of that larger denominator.

² These are the figures from the last full run. V8 instrumentation is not perfectly
deterministic — two identical runs move a given figure by ~0.05 pt (the seventh pass read
46.14% statements where the sixth read 45.96%). Treat them as "the last run", not a
constant, and re-measure rather than copy when you change the suite.

³ **Scope changed on 2026-09-15/16** when the engine extraction moved the whole engine out of
`src/` in four layers: `src/{store,lib,subscription}`, then `src/{proxy,sync}` + `src/eventBus.ts`,
then `src/{applications,companion,commands}`, then `src/{startup,shutdown}.ts`. Those files are the
*same code under the same tests*, and their per-file coverage is provably unchanged (see §4.8). Two
new groups entered the report:

| Group | Files | Statements | Note |
|---|---:|---:|---|
| `packages/engine/src` | 54 | 3,227 | The moved files + `index.ts`, which now implements `createEngine()` and is covered at 90.7% statements / 94.6% lines by the smoke test. Was 14 files / 606 statements after layer 1, 41 / 2,433 after layer 2, 49 / 2,914 after layer 3, 51 / 3,019 after `createEngine()`. The last +3 files / +208 statements are P3's blob layer (§4.9). |
| `packages/protocol/src` | 17 | 114 | **Sourcemap artefact** — `packages/protocol/dist/index.js` is executed through its `exports` map, and V8 remaps it back through `dist/index.js.map` (whose `sources` list `../src/commands/*.ts`, i.e. `packages/protocol/src/commands/*.ts`). Numbers are therefore approximate and ~100%. Present since P2 work item 7 wired the registry to `@bifurc/protocol`. The 17th file / +8 statements are `commands/blob.ts` (P3). |

The denominator went 11,129 → 11,443 (layer 1) → 11,446 (layer 3) → 11,489 (`createEngine`) →
**11,705** (P3 blob layer), while the percentage kept rising, so this is a genuine improvement rather
than a scope artefact. Every step is accounted for: the layer-1 jump was `packages/protocol/src`
entering the report; layers 2 and 4 changed it not at all; layer 3's +3 statements are
`applications/types.ts`, which had been wrongly excluded as "type-only" (§5); and P3's +216 is
exactly `blob/{store,sweep,commands}.ts` (+208, all covered above 90%) plus the four new
`blob.*` Zod schemas in `packages/protocol/src/commands/blob.ts` (+8).

⁴ **`packages/client/src/**/*.ts` joined the coverage scope on 2026-09-18** (P5), alongside the
engine's entry. The percentages in the table above **predate it** and have not been re-measured — the
scope change moves the denominator, so treat them as "the last full coverage run" rather than as
current. The direction will be **upward**: the client is the most thoroughly covered package in the
repo relative to its size (5 suites / 54 tests against 5 source files), which is what a phase whose
entire value is a shape guarantee should look like. Re-measure rather than copy.

Suite size: **101 files / 2,471 tests** (measured 2026-09-18 after 3b-2's second slice, undeselected:
`2430 passed | 1 failed | 40 skipped`; the failure is the documented `soap.execute` dead-endpoint flake
below). The last steps were P6's shell seam — 101 / 2,471 after moving `config.save` + the three
`workspace.*` commands into `packages/engine/src/store/configCommands.ts` and splitting the ratchet by
category (+2), 101 / 2,469 after the six proxy/server-lifecycle commands moved into
`packages/engine/src/proxy/serverCommands.ts` (+2), 101 / 2,467 after the
registry-coverage ratchet in `tests/ipc/handlers.test.ts` (+3), 101 / 2,464 after step 3a (the push
channel), 100 / 2,445 after finding 1, 99 / 2,436 after step 2. Before that, P5's client package —
97 / 2,418, 96 / 2,401 with
`client.test.ts`, 95 / 2,388 before it, and 75 / 1,791 at the previous review, 73 / 1,714 before that,
69 / 1,616 before that. The growth is P2 work items 5–7, P3's blob layer, P3's items 3–4 channel
conversion, P4's transport + conformance suite, P5's client, and P6's seam.
**Up to two tests fail, and *which* two varies between runs** — they are always
drawn from one known pre-existing sandbox family (§7, and `plan/baseline.md`'s "Environment
caveats"). The family is *every test that asserts a connection to a dead endpoint fails*: the
sandbox intercepts such connections and answers with a synthetic 404 instead of refusing. Its
members:

> **The 2026-09-17 items 3–4 run reported `1785 passed | 6 skipped | 0 failed`, and that is not
> evidence the family is gone.** It was run with the three members deselected by name (§8's
> `--testNamePattern`), which is the same command the coverage runs use. Undeselected, the family
> still trips — the reason to say so explicitly is that a green run is otherwise easy to read as a
> fix. The 6 skipped are the 4 deselected members plus 2 pre-existing `.skip`s.

| Test | What the sandbox does to it |
|---|---|
| `tests/integration/protocolExecution.integration.test.ts` → `returns { ok:false, error } when the WSDL cannot be fetched` | `http://127.0.0.1:1/…` returns `ok: true` |
| `tests/integration/protocolExecution.integration.test.ts` → `rejects when the upstream is unreachable` | the synthetic 404 is treated as a real response |
| `tests/integration/protocolExecution.integration.test.ts` → `rejects when the endpoint is unreachable` | same shape |
| `tests/integration/upstreamFetch.integration.test.ts` → `rejects when the upstream is unreachable` | resolves `{ status: 404, headers: { "content-length": "0" } }` instead of rejecting |

> **The family lost its most environment-sensitive member on 2026-09-21, when the P0 spike was retired.**
> Two rows left this table with it — both in `tests/spike/protocolPoc.test.ts` (`soap.execute` and
> `graphql.introspect`, "a dead/unreachable endpoint never throws…"). That is why the count above is
> **four**, not six.
>
> **The distinction that matters, and it is not merely arithmetic.** The spike's version asserted against
> **`127.0.0.1:1`**, the address this sandbox intercepts — so its result depended on which answer the host
> gave, which is why it was the suite's one standing failure. The surviving members use `deadPort()`
> (`protocolExecution.integration.test.ts:232`), which starts a real server, takes its **OS-assigned**
> port, closes it, and returns that port — a genuine dead port, and **not port 1**. On the 2026-09-21
> full-suite run, **none of the four tripped.** So the family is now a *contingent* risk (a host that
> intercepts those connections) rather than a *standing* one, and this table is a warning about a
> possible environment rather than a roster of observed failures. Deselect them by name when a run needs
> to be comparable to a clean-machine baseline; do not expect them in every run.

**Deselect these four by their precise names — do not match on the shared word `unreachable`.**
`tests/integration/gitRemote.integration.test.ts` → `reports an error for an unreachable remote` is a
git test that has nothing to do with the interceptor, and it is the test that **assigns the `rootC`
fixture** its neighbour reads two tests later (`:421` assigns, `:433` calls `rootC.activate()`).
Filter it out and the neighbour fails with `Cannot read properties of undefined (reading 'activate')`
— which looks like a product regression and is not one. A loose pattern converts a harmless flake
into a confusing false alarm.

Observed rosters: `soap.execute` + `upstreamFetch` (the blob-layer session), then
`protocolExecution`'s WSDL + `upstreamFetch` (the layer-5 move) — with `soap.execute` passing cleanly
the second time, then **`soap.execute` alone** (the item-5 session), undeselected and failing 3/3 in
isolation. **Never attribute one of these to your own change without first checking that the failure
touches the code you moved.** This is why the coverage command in §8 deselects by name rather than just
`soap.execute`. *(The rosters above predate the retirement and name the spike's case, which no longer
exists; they are kept because the lesson is about the *variability*, not about which file tripped.)*

**The direction of the trip is not always the one the table predicts**, which is worth knowing before
the message misleads you. The roster describes `soap.execute` as "the envelope reports success where
failure was expected"; the item-5 session saw the mirror image — `ProtocolClientError: [INTERNAL]
connect ECONNREFUSED 127.0.0.1:1`, i.e. **failure where a caught result was expected**, because
`soap.execute` does `req.on("error", reject)` and the test asserts `typeof result.status === "number"`.
The mirror image is the more alarming-looking of the two (it reads like a broken handler), and it is
still the same test on the same list. Confirming it is not yours takes three commands: is the failing
assertion in the listed test; is `git diff HEAD` empty for that path; does it fail the same way with
the sandbox disabled.

> **The P3 delta reconciles exactly, and it is worth showing how** — a count that moves without an
> explanation is how a suite quietly loses tests. The last run recorded before P3 was **70 files /
> 1,617 tests** (unit 1,284 / integration 333); P3 is **73 files / 1,714 tests** (unit 1,381 /
> integration 333). That is `+3` files and `+97` unit tests, splitting as:
>
> - **`+89`** — the three new `tests/blob/*` suites (58 + 20 + 11).
> - **`+8`** — `packages/protocol/src/commands/index.test.ts`, **a file nobody edited**.
>
> The second one is the interesting one. That suite generates **two tests per entry in `COMMANDS`**
> (`for (const action of actions) { it("… accepts a valid payload"); it("… rejects an invalid payload"); }`)
> plus one completeness check, so its total is `1 + 2 × commands`. P1 froze **89** commands → 179
> tests; P3 added four `blob.*` commands → **93** commands → **187** tests, which is what the file
> reports today. Adding a command therefore adds two tests to a file you did not touch, and any
> future test-count delta should be reconciled rather than waved through.
> `npx vitest list | wc -l` (or `--project unit`) is the cheap way to confirm a total without
> running anything.
>
> **Correcting the record:** the session note written when the blob primitives landed recorded this
> as "187 protocol tests green (183 + 4)". The 187 is right; the arithmetic was not. It is
> `179 + 8`, not `183 + 4` — the suite emits **two** tests per command, not one.
>
> **The 2026-09-17 conversion delta also reconciles exactly:** **73 files / 1,714 tests** →
> **74 files / 1,762 tests**. `+1` file and `+48` tests, splitting as:
>
> - **`+45`** — `tests/integration/importExportBlob.integration.test.ts`, new.
> - **`+3`** — `tests/ipc/importExport/registry.test.ts`, **12 → 15**.
>
> The `+3` is the one worth naming, because it is a *rewrite* rather than an addition: the suite had
> two tests asserting "an exporter/importer exists for every format that claims support", and
> `workspace-zip` legitimately moved out of `exporter`/`importer` into `pathExporter`/`pathImporter`.
> Rather than weaken those tests to accept anything, both now resolve `exporter ?? pathExporter` and
> three new tests pin the new invariant — that `workspace-zip` is served through the path-shaped
> slots **and nothing else is**. A count that stayed flat here would have meant the split was
> untested.
>
> The two format-level suites were **rewritten in place** (18 and 17 tests, both unchanged in count)
> and so do not appear in the delta at all. That is the intended outcome of routing them through
> `importExportHarness.ts`: the *transport* moved to the new suite, the *codecs* stayed put, and
> neither suite's contract changed. A file-count delta is therefore not a completeness signal on its
> own — here 58 of the 80 import/export integration tests were converted without moving the total.
>
> **The 2026-09-17 items 3–4 delta also reconciles exactly:** **74 files / 1,762 tests** →
> **75 files / 1,791 tests** (`1785 passed | 6 skipped | 0 failed`). `+1` file and `+29` tests, and
> the 29 is *entirely* `tests/integration/fileOps.integration.test.ts` — the new suite for the six
> converted channels (`tls.exportCert`, `runner.exportReport`, `capture.shareJson`, `audit.export`,
> `tls.importCert`, `tls.importKey`).
>
> A delta this clean is worth stating because of what it implies. The three suites that drive the
> **real shell handlers** — `runnerStorage` (17), `capture` (20), `auditLog` (13) — were **modified
> but did not change count**. That is the intended outcome of a thin-client conversion: the
> renderer-facing contract is unchanged, so every existing assertion still holds, and what changed is
> the `beforeAll`, which now registers the engine half the way `src/ipc/handlers.ts` does. A count
> that *moved* in those files would have meant the contract had changed too.
>
> **Two files and four tests failed on the first run of this pass, all with one cause**, and the
> failure mode is worth recording because it reads like the wrong bug. The symptom is
> `{ ok: false, error: 'No handler registered for command "runner.exportReport".' }` followed by
> `ENOENT: no such file or directory, open '…\export.json'` — i.e. a *filesystem* error, from a
> *registration* mistake. `CommandRegistry.invoke()` validates against the frozen schema and then
> looks up the handler, and these three suites capture `ipcMain.handle` registrations and call them
> directly, so nothing had registered the engine command the client now delegates to. The fix is
> three lines in each `beforeAll`; the lesson is that a client that delegates needs its delegate
> registered in the test process, and that the resulting error message will not say so.
>
> **One of the four was a genuine finding rather than a fixture gap.** `capture.integration.test.ts`
> builds its workspace with `createWorkspace()`, which calls `setDataRootOverride()` (the workspace
> store) and `setSettingsPathOverride()` (the settings file) but **not** `setDataRoot()` (which is
> what `dataDir()` — and therefore `blobRoot()` — resolves through). Until P3 nothing in that suite
> needed a data root, so the gap was invisible. It is now supplied, because a capture large enough to
> cross `BLOB_INLINE_THRESHOLD_BYTES` would otherwise fail with `DataRootNotInitialisedError` instead
> of staging a blob. The three overrides are genuinely three different things and are easy to
> conflate: `setDataRoot()` → `dataDir()`, `setDataRootOverride()` → the workspace tree,
> `setDataDirOverride()` → a thin alias for the second that also clears `gitStore`'s git cache.
>
> **The 2026-09-17 item-5 delta also reconciles exactly:** **75 files / 1,791 tests** →
> **77 files / 1,863 tests** (`1,862 passed | 1 failed`). `+2` files and `+72` tests, and the 72
> breaks down with no remainder:
>
> | Suite | Δ | What it is |
> |---|---:|---|
> | `tests/ipc/certTrust.test.ts` | **+34** (new file) | per-platform argv, the Linux chain, the probe |
> | `tests/ipc/certLifecycle.test.ts` | **+21** (new file) | ordering and record-keeping |
> | `tests/proxy/certManager.test.ts` | **+10** | the fingerprint, `removeCA`, the corrupt-CA state |
> | `tests/integration/fileOps.integration.test.ts` | **+7** | the three cert-lifecycle commands |
>
> The `+10` is worth naming because it is a **replacement, not an addition**: the four `installCA`
> tests that lived in `certManager.test.ts` (mocking `execSync`, redefining `process.platform`) are
> **gone**, because `installCA` left the engine. Their successors are 34 exact-argv tests in
> `certTrust.test.ts` that need no mocking at all. A file-count delta is not a completeness signal on
> its own, here as in items 3–4.
>
> **The single failure is a documented flaky-family member, not this change.** It was
> `tests/spike/protocolPoc.test.ts > soap.execute — a dead/unreachable endpoint never throws through
> the envelope` — named by exact string in the deselect pattern in §8, and listed in the roster above.
> **That case no longer exists: the spike was retired 2026-09-21** (`spike/README.md`), and the same
> contract now lives in `tests/integration/protocolExecution.integration.test.ts` on a real dead port.
> The reasoning below is kept verbatim because it is the *method*, which is what a reader needs when the
> next environment-sensitive failure appears: three checks before attributing it, per the standing rule
> ("never attribute one of these to your own change without first checking that the failure touches the
> code you moved"): the failing assertion is
> in `soap.execute`; `git diff HEAD -- src/ipc/handlers/soapHandlers.ts packages/engine/src/soap` is
> **empty**; and it fails identically with the sandbox disabled, so it is not the interceptor behaving
> differently. Undeselected, the family is expected to trip — §8 records "expect
> `2 failed | 1712 passed (1714)`" for exactly this reason.


### 4.2 Coverage by area (current config)

| Area | Statements | Branches | Functions | Lines |
|---|---:|---:|---:|---:|
| `packages/engine/src` | **85.2%** | **72.4%** | **86.7%** | **87.6%** |
| `packages/protocol/src` ³ | 97.4% | — | 25.0% | 97.4% |
| `renderer/components` | **13.5%** | 13.3% | 8.7% | **14.6%** |
| `renderer/lib` | **82.9%** | **74.8%** | **75.0%** | **84.1%** |
| `renderer/panels` | 0.0% | 0.0% | 0.0% | 0.0% |
| `src/ipc` | **79.0%** | 55.7% | 79.7% | 82.1% |
| **TOTAL** | **49.07%** | **32.66%** | **37.26%** | **51.52%** |

³ `packages/protocol/src` has no branch data because V8 records none for those remapped Zod
schemas. Its numbers come from the sourcemap-remapped `dist/` bundle (see §4.1 footnote 3), so
treat them as indicative. Its 25.0% functions is a single arrow function in
`packages/protocol/src/commands/index.ts`, not a broad gap.

**There is no `src/` (root files) row any more.** It held `startup.ts` (98.2%), `shutdown.ts`
(100%) and `eventBus.ts` (100%) — all three are in `packages/engine/src` now. `src/ipc` is the only
measurable area left in `src/`, alongside `main.ts` and `preload.ts`, which stay excluded as process
entry points.

**Rows removed on 2026-09-15/16.** `src/lib`, `src/store`, `src/subscription` (layer 1);
`src/proxy`, `src/sync` (layer 2); `src/applications`, `src/companion`, `src/commands` (layer 3);
`src/startup.ts`, `src/shutdown.ts` (layer 4) no longer exist in `src/`. They all live under
`packages/engine/src/` now, reported as the single `packages/engine/src` row above. Their pre-move
figures were `src/store` 82.8/68.8/84.1/87.6, `src/subscription` 100/100/100/100, `src/lib`
30.0/2.8/22.2/31.8, `src/proxy` 87.2/77.9/89.7/88.9, `src/sync` 77.8/56.7/80.3/80.6,
`src/applications` 87.5/77.0/81.3/91.6, `src/companion` 95.3/83.3/76.5/95.1, `src/commands`
100/100/100/100, `src/startup.ts` 98.1/85.0/100/97.9 and `src/shutdown.ts` 100/100/100/100 — so the
combined row reading 84.1/71.2/85.6/86.5 is a *weighted average of unchanged numbers*, diluted only
by the new 0%-covered `index.ts` barrel (§4.8). Nothing regressed: §4.8 verifies every moved file
individually.

`src/` now contains only `ipc/` (the Electron registration layer P2 replaces), `main.ts` and
`preload.ts`. **The engine extraction is complete** — every engine module is in
`packages/engine/src/`.

Bold areas are the ones the original review moved materially. Named as they are today (all of the
first six now live under `packages/engine/src/`): `applications` 1.1% → 87.5%, `companion` 0.8% →
**95.3%**, `ipc` 24.5% → 79.1%, `proxy` 61.7% → 87.2%, `store` → 82.8%, `sync` 64.5% → 77.8%,
`renderer/lib` 67.5% → 82.9% (the collection runner and its report generator were at 0%), and
`renderer/components` 1.8% → 13.5%.

> **Scope correction (2026-09-14, fourth pass).** `renderer/components` read **1.8%** until
> this review, and the change is not from new tests — the `include` glob listed only `.tsx`
> under `renderer/components`, so **17 `.ts` files were invisible to the report**, including
> three that already had passing tests (`captureUtils.ts`, `restTabReducer.ts`,
> `searchUtils.ts`). Adding them *raised* the headline (43.22% → 43.49% statements at the
> time) because the previously-hidden files were largely covered. The correction also
> exposed three genuinely untested tab reducers (`graphql`/`grpc`/`soap`) and two React
> hooks; the reducers are now covered (sixth pass), the hooks are still in §6.

`renderer/panels` is still ~0%: the panels are covered by the Playwright E2E suite instead
(see `HAPPY_PATH_COVERAGE.md`), which is not measured here.

### 4.3 The routing path specifically

| File | Statements before | Statements after |
|---|---:|---:|
| `packages/engine/src/proxy/proxyHandler.ts` | 43.9% | **89.8%** |
| `packages/engine/src/proxy/serverUtils.ts` | 88.5% | **100.0%** |
| `packages/engine/src/proxy/routingUtils.ts` | 56.0% | **76.0%** |
| `packages/engine/src/proxy/server.ts` | 71.6% | **90.3%** |
| `packages/engine/src/proxy/mockHandler.ts` | 74.0% | **77.9%** |
| `packages/engine/src/proxy/serverReplay.ts` | 100.0% | **100.0%** |
| `packages/engine/src/proxy/tlsIntercept.ts` | 0.0% | **88.5%** |
| `packages/engine/src/proxy/tlsCert.ts` | 71.0% | **90.3%** |
| **`src/proxy` area** | **54.9%** | **87.1%** |

### 4.4 The "hit Send" path specifically

The protocol execution handlers were the largest untouched area. They are now driven
against real local servers.

| File | Statements before | Statements after |
|---|---:|---:|
| `src/ipc/handlers/graphqlHandlers.ts` | 10.9% | **98.4%** |
| `src/ipc/handlers/soapHandlers.ts` | 12.1% | **94.8%** |
| `src/ipc/handlers/grpcHandlers.ts` | 27.0% | **83.8%** |
| `src/ipc/handlers/coreHandlers.ts` | ~40% | **79.2%** |
| `packages/engine/src/proxy/serverReplay.ts` (REST replay) | 100.0% | **100.0%** |
| **`src/ipc` area** | **24.5%** | **76.4%** |

> `grpcHandlers.ts` coverage is of its **stubs** — `grpc:execute` and `grpc:reflect`
> return "runtime not yet configured". The tests pin that contract, so implementing gRPC
> will fail the suite until the tests are updated deliberately.

### 4.5 The git sync / publish path specifically

Publishing is how a change becomes a shared, auditable fact, and `setRemote()` is the
gateway to every remote workspace. Both were mocked-unit only; they are now driven against
a real repository and a real bare remote.

| File | Statements before | Statements after |
|---|---:|---:|
| `src/ipc/handlers/syncHandlers.ts` | 24.5% | **74.5%** |
| `packages/engine/src/sync/publishService.ts` | 54.3% | **84.8%** |
| `packages/engine/src/sync/syncManager.ts` | 69.3% | **79.9%** |
| `packages/engine/src/sync/gitOps.ts` | 73.9% | **80.7%** |
| `packages/engine/src/sync/gitSyncOps.ts` | 19.0% | **50.0%** |
| **`src/sync` area** | **64.5%** | **78.1%** |

> `gitSyncOps.ts` is capped at 50% because `fetchRemoteHead()` and `performGitClone()`
> are **dead code** — exported but never called. `syncManager.setRemote()` re-implements
> cloning inline, and `getRemoteHead()` duplicates `fetchRemoteHead()`. See §7.

### 4.6 The capture path specifically

Capture is in-memory: the proxy builds a `RequestLogEntry` per request and emits it on
`logEmitter`, which the Capture screen renders and turns into a mock or a saved request.
Both halves are now covered — the emitter against real traffic, and the conversions directly.

| File | Statements before | Statements after |
|---|---:|---:|
| `packages/gui/src/components/capture/captureUtils.ts` | not measured | **100.0%** |
| `renderer/components` (area) | 1.8% | **13.5%** |
| `packages/engine/src/proxy/server.ts` (the emit sites) | 75.5% | **90.3%** |

> `captureUtils.ts` was not merely uncovered, it was **not measured** — see the scope
> correction in §4.2. It now sits at 100% statements / 96.5% branches.

### 4.7 TLS interception and the companion server specifically

The two highest-risk files still at 0% after the fifth pass. `tlsIntercept.ts` is the code
that terminates TLS for every `https://` request a user routes through Bifurc; the companion
server was the browser extension's only write path into the workspace — **and it has since
been retired outright** (`plan/15` §6.4, 2026-09-23), so its rows below are a **historical
measurement**, not a current one. They are kept because the sixth pass's finding is the
method, and because the retired code's 94% is part of the record.

| File | Statements before | Statements after |
|---|---:|---:|
| `packages/engine/src/proxy/tlsIntercept.ts` | 0.0% | **88.5%** (100% of lines) |
| `packages/engine/src/proxy/tlsCert.ts` | 71.0% | **90.3%** |
| `packages/engine/src/companion/companionServer.ts` | 0.0% | **94.0%** |
| `packages/gui/src/lib/createTabReducer.ts` | 100.0% | **100.0%** |
| `packages/gui/src/components/graphql/graphqlTabReducer.ts` | 0.0% | **92.9%** |
| `packages/gui/src/components/grpc/grpcTabReducer.ts` | 0.0% | **88.9%** |
| `packages/gui/src/components/soap/soapTabReducer.ts` | 0.0% | **83.1%** |
| **`src/companion` area** | **0.8%** | **94.1%** |

> **Path note (P4 work item 1, 2026-09-18).** The two rows above record a P2-era measurement, and the
> files have since moved: `packages/engine/src/companion/companionServer.ts` is now
> `packages/engine/src/transport/legacyCompanion.ts`, and `companion/allowedActions.ts` is
> `packages/engine/src/transport/legacyCompanionActions.ts`. **`src/companion/` no longer exists.** The
> numbers are unchanged and still describe these two files — only the paths are historical.
>
> The move deliberately did **not** rewrite the four action bodies, and the reason is recorded in
> `legacyCompanion.ts`'s header and `plan/05`'s "Item 1": aliasing them onto `config.get` /
> `entity.create` / `folder.add` would change four behaviours, one of which (`onAddConflict` disabling
> an existing enabled mock) contradicts this file's documented **additive-only** guarantee. So the
> 94.1% above still covers the hand-written implementations, not registry delegations.

> The TLS suite's key property: it does **not** mock `mkcert`. It generates a real CA and
> performs the client handshake with `rejectUnauthorized: true`, so the pass/fail decision is
> made by OpenSSL's chain and hostname validation rather than by an assertion we wrote. A
> regression in certificate generation cannot slip through it.
>
> The remaining 11.5% of `tlsIntercept.ts` is the post-handshake body/header parsing branch
> and the `tlsSocket` error handler, which the integration path exercises only indirectly.

### 4.8 The engine extraction specifically

P2 work item 8 moved the engine out of the app's `src/` into `packages/engine/src/`, in four
layers: `src/{store,lib,subscription}`, then `src/{proxy,sync}` + `src/eventBus.ts`, then
`src/{applications,companion,commands}`, then `src/{startup,shutdown}.ts`. That is a pure move: no
logic changed, so **each moved file must report exactly the coverage it reported before the move** —
the same denominator (proof the code is untouched) and the same numerator (proof the same tests
still reach it). Anything else means the move was not behaviour-preserving.

#### Layer 1 — `store/`, `lib/`, `subscription/`

| File (engine-relative) | Lines before | Lines after | |
|---|---:|---:|---|
| `lib/randomNames.ts` | 100.0% | 100.0% | ✓ |
| `lib/randomizer.ts` | 26.2% | 26.2% | ✓ |
| `store/appSettings.ts` | 85.7% | 85.7% | ✓ |
| `store/config.ts` | 95.6% | 95.6% | ✓ (statements 84.8→86.2) |
| `store/gitStore.ts` | 94.4% | 94.4% | ✓ |
| `store/paths.ts` | — | 87.5% | new to the report ⁵ |
| `store/workspaceFs.ts` | 88.0% | 88.0% | ✓ |
| `store/workspace/fsDirectorySync.ts` | 100.0% | 100.0% | ✓ |
| `store/workspace/fsEnabledSet.ts` | 100.0% | 100.0% | ✓ |
| `store/workspace/fsNamesIndex.ts` | 100.0% | 100.0% | ✓ |
| `store/workspace/fsPendingDeletions.ts` | 50.0% | 50.0% | ✓ |
| `store/workspace/fsRead.ts` | 47.1% | 47.1% | ✓ |
| `subscription/entityCount.ts` | 100.0% | 100.0% | ✓ |

11 of the 12 comparable files are identical on **all four** metrics *including the raw
covered/total counts*. `store/config.ts` is identical on lines and has the same totals
(138 statements / 134 branches / 55 functions) but a **higher** numerator — P2 items 5–7 added
tests that reach more of it. Identical denominators are the load-bearing part: they prove no
statement was added, removed or restructured.

⁵ `store/paths.ts` is absent from `coverage-baseline/` because that snapshot predates the
data-dir resolver (commit `d814e38`), not because the move lost it.

#### Layer 2 — `proxy/`, `sync/`, `eventBus.ts`

The same test, run against the layer-1 report (`coverage-run6`) as the "before": **27 files moved,
26 identical on all four metrics including raw covered/total counts, 0 regressions, 0 missing, 0
stale entries left behind in `src/`.**

| | Count |
|---|---:|
| Files compared | 27 |
| Identical on all 4 metrics (incl. covered/total counts) | **26** |
| Higher numerator, same totals | 1 |
| **Lower numerator (regression)** | **0** |
| Missing from the post-move report | **0** |
| Stale `src/{proxy,sync}` / `src/eventBus.ts` entries remaining | **0** |

The one file that moved is `sync/gitOps.ts` (lines 78.8% → 82.4%) with **identical totals**
(88 statements / 113 branches / 7 functions / 85 lines). Line-level lcov data shows exactly what
changed: line 114 (`git checkout HEAD -- <path>`) succeeded **4× before, 1× after**, and the
`ls-files` fallback at lines 118–128 ran 2× before and 5× after — including the inner checkout at
lines 125–127, which was **never executed before and ran 3× after**.

That is **real-git nondeterminism, not the move**: whether the first checkout succeeds depends on
the temp workspace's working-tree state, which the integration suite creates with real `git`
commands. The direction is benign — a previously-dead fallback path is now exercised.

**Layer 2 also needed two extra rewrite passes that layer 1 did not**, both silent-failure traps
(see `plan/03`): engine-internal imports must be *relative* (23 `@bifurc/engine/store/*`
self-imports would otherwise load a second copy of module state from `dist/`), and `vi.mock()`
calls are not matched by an import-shaped regex (12 stale mock specifiers survived the first four
passes — a mock that no longer matches its subject is worse than no mock).

#### Layer 3 — `applications/`, `companion/`, `commands/`

The cleanest layer yet: **8 files moved, 7 of 7 that were in the pre-move report identical on all
four metrics including raw covered/total counts, 0 regressions, 0 missing, 0 stale `src/` entries.**
48 statements rewritten across 26 files (45 imports + 3 `vi.mock`). `ws` joined the engine as its
third runtime dependency.

(The 8th file is `applications/types.ts`, absent from the pre-move report because it was excluded —
see the next paragraph.)

The layer-3 rewrite also produced the one **deliberate** scope change in this document:
`coverage.exclude` listed `src/applications/types.ts` under "Type-only modules", but that file
exports `DEFAULT_DEBUG_PORTS`, `RUN_CONFIG_TYPE_LABELS` and `RUN_CONFIG_TYPE_ICONS` — real runtime
values. The exclusion was made on the strength of the filename and had been hiding live code. Rather
than carry a wrong exclusion to the new path, the entry was **removed**; the file now reports 3/3
lines at 100%, which is why the denominator grew by exactly 3 statements (11,443 → 11,446) and the
numerator by exactly 3.

**Layer 3's structural note:** `companion/allowedActions.ts` was the frozen API surface for the
external `bifurc-extension` client. It moved with everything else, but its *contents* were not to
change — the extension is a released client that cannot be updated atomically with the engine.
**That constraint is now history:** the companion was retired outright on 2026-09-23 (`plan/15`
§6.4), so the file and its allowlist are gone, and the extension's migration is a **protocol change**
onto `transport/ws.ts`, not a port swap. The *reason* the note existed is still the reason the
extension was not aliased onto `config.get`/`entity.create` while it lived — recorded in
`ws.ts`'s header.

#### Layer 4 — `startup.ts`, `shutdown.ts`

**2 files moved, 2 of 2 identical on all four metrics, 0 regressions, 0 missing, 0 stale `src/`
entries.** 17 statements rewritten across 6 files. This was the last layer, and the cleanest to
reason about: neither file had a single `@/` import, and every dependency was either a Node builtin
or an already-moved `@bifurc/engine/*`.

`startup.ts` sits at 98.07% statements / 97.87% lines; the remainder is the port-in-use and
git-not-installed preflight branches, which the integration suite cannot exercise without breaking
its own environment. `shutdown.ts` is at 100%.

**The one number that moved in this layer was not in this layer.** The headline fell 48.00% →
47.98% (3 statements), and the cause is `sync/gitOps.ts` — the same file layer 2 flagged. Its totals
are unchanged at 85 lines; only the *covered* count moved, 68 → 71 → 71 → 68 across the four runs.
That oscillation, observed in both directions across layers, is the definitive proof that the file's
coverage depends on real-git temp-workspace state rather than on the move. **No file that actually
moved changed its coverage in any layer.**

#### `createEngine()` — the item-8 acceptance criterion

The extraction's end state is the package's public entry point, and it is covered by
`tests/integration/engineSmoke.integration.test.ts` — the one test that starts the engine for real:

| Assertion | Why it matters |
|---|---|
| `status().running` is `false` before `start()`, `true` after | `status()` is documented as safe pre-start |
| `status().proxyPort` / `.edgePort` match the configured ports | the engine bound what it was told to |
| a real HTTP request through the proxy returns 200 | it actually serves, not merely listens |
| `status().settings.activeWorkspaceId` is set | `bootstrapWorkspaces()` ran and produced a workspace |
| a second `start()` does not re-bind | `start()` is idempotent |
| the port is free again after `stop()` | teardown genuinely released the socket |
| a second `stop()` resolves | `stop()` is idempotent |
| `start()` after `stop()` **rejects** | the engine is single-use, and says so |

Two things this test caught that a unit test could not:

1. **`start()` resolved before the engine was listening.** `startServer()` and the companion
   server's `startCompanionServer()` — the latter since **retired** — were fire-and-forget: both
   handed off to an async `listen()` and returned immediately, so `isRunning()` was still false on
   the next line. `start()` now waits for readiness (bounded), because otherwise the single promise
   a caller awaits would mean nothing.
2. **`registry.list()` is empty.** `createEngine()` deliberately registers no commands: the ~112
   commands are registered by the *consumer* — today the shell's in-process layer, which P6 replaces
   with the RPC client. The test asserts registry *identity*, not population.

**The barrel is no longer 0% — and that was the point of not excluding it.** `packages/engine/src/index.ts`
is the package's public entry point. While it only re-exported namespaces nothing imported it, so it
sat at 0%. It was deliberately *not* excluded (unlike `packages/gui/src/components/ui/index.ts`, a barrel of
presentational re-exports), on the grounds that it becomes the real engine API and should then be
covered rather than hidden. `createEngine()` now lives there, the new
`tests/integration/engineSmoke.integration.test.ts` exercises it, and it reports **90.7% statements /
94.6% lines**. There are now **no engine files at 0%**.

> **Why this measurement was impossible until 2026-09-15.** The engine originally reported **all
> 14 files at 0%** while the suite still passed 1615/1616. `@bifurc/engine` is a real npm
> workspace package, so Vitest treated it as *external* and loaded the **built**
> `packages/engine/dist/*.mjs` through native `import()` — bypassing Vite's resolvers entirely.
> Tests were therefore running against compiled output, and a source edit that was never rebuilt
> would have been **silently untested**. The fix is `resolve.alias` mapping `@bifurc/engine/*` to
> `packages/engine/src/*` — but it only works when redeclared **inside each `test.projects`
> entry**. Root-level `resolve.alias` (like root-level `plugins`) is not inherited by project runs;
> that is why `dualAliasPlugin` had always been duplicated in. A `deps.inline` pattern was tried
> and is **not** needed. See §5 for the config details.

> **`@bifurc/protocol` has no such alias, and that is a live trap.** `vitest.config.ts` declares only
> the two `@bifurc/engine` aliases, and `packages/protocol/package.json` points `main` / `types` /
> `exports` at `./dist/*`. So a `packages/protocol/src/**` edit that is not rebuilt is **silently
> untested** by the protocol suite *and* by every engine suite — they keep passing against the previous
> `dist/`, and nothing warns. `npm test` does not build first. After any protocol source edit, run
> `npm run build:packages` (or `npm run typecheck`, which runs it first), and check a value you changed
> directly — e.g. `node -e "console.log(require('./packages/protocol/dist/index.cjs').EVENT_NAMES)"`.
> Re-confirmed 2026-09-18 while adding `Capability.SOCKET`: the engine's `socket` tests only see the new
> capability because `build:packages` had been run, and they would have passed without it.

A full HTML report is written to `coverage/index.html` by `npm run test:coverage`.

### 4.9 The blob layer specifically (P3 work item 1)

| File | Statements | Branches | Functions | Lines |
|---|---:|---:|---:|---:|
| `packages/engine/src/blob/commands.ts` | **100%** | **100%** | **100%** | **100%** |
| `packages/engine/src/blob/store.ts` | **98.3%** | **95.9%** | **100%** | **99.1%** |
| `packages/engine/src/blob/sweep.ts` | **90.1%** | **91.7%** | **100%** | **92.2%** |
| `packages/protocol/src/commands/blob.ts` | **100%** | **100%** | **100%** | **100%** |

89 tests in `tests/blob/{store,sweep,commands}.test.ts`. Every one runs against a **real temp data
root** with real files, real hashes and real `rename`s — mocking `fs` here would test the mock,
because the whole reason the store exists is that it touches the filesystem in a specific way.

**The three suites pin different things, deliberately:**

- `store.test.ts` (58) pins the **contract**: round-trip, SHA-256 against an independently computed
  digest, chunked reads that reassemble byte-for-byte, `eof` at the right slice, offset bounds,
  release semantics, the sliding lease, and the ingress limits.
- `sweep.test.ts` (20) pins the **reclamation rules**: expiry by `createdAt`, the mtime fallback for
  a metadata-less directory, abandoned staging files, stray files, and the two rules that keep it
  safe — it never creates the root, and it never throws.
- `commands.test.ts` (11) pins the **seam**: that the frozen protocol schemas validate before the
  store is reached, and that a valid payload reaches the real store.

**Four properties are asserted because they are the ones a future edit would silently break:**

1. **The blob root is under `dataDir()`** (`path.relative(dataDir(), blobRoot()) === "blobs"`). This
   is the Docker-volume requirement from `plan/04`'s risk table — a root outside the data root is a
   blob that vanishes when a container restarts, and nothing else in the suite would notice.
2. **The content is a real file.** `fs.statSync(...).isFile()` — because `unzipper.Open.file()` will
   not accept a buffer and `archiver` pipes to a `WriteStream`. A future "optimisation" to an
   in-memory store would pass every other test and break both zip paths.
3. **`meta.json` is written last, and "metadata exists ⇒ the blob is usable" is an invariant.**
   The suite deletes the metadata and asserts the blob then reads as *not found* rather than as
   truncated content — which is the reason for the write order. It also deletes the *content* and
   asserts *not found*, which is why `readMeta()` checks both: without that check `statBlob` would
   happily report a size and a SHA-256 for bytes that are not on disk.
4. **`blobId` cannot express a path.** 15 hostile ids (`"../../etc/passwd"`, `"..\\..\\windows"`,
   `"blob_000…0/../../x"`, …) are rejected on all four entry points with `blob-invalid-id`, and every
   accepted path is asserted to be a direct child of the root. P13 lists the blob traversal test as
   *the most likely thing to be missed*; it is here.

**Time is a parameter, not a mock.** `putBlob`/`statBlob`/`readBlob`/`sweepBlobs` all take an
optional `now`. That is why every TTL assertion is exact and readable (`T0 + BLOB_TTL_MS` is the
boundary, `- 1` is one millisecond short of it) instead of depending on fake timers and a
clock-skewed race. The one place fake timers *are* used is `startBlobSweeper`, where the interval is
the thing under test rather than the arithmetic.

**The ingress cap is tested without allocating 140 MB.** The pre-decode rules are extracted as
`assertIngressWithinLimit(declared, base64Length, maxBytes)` and exercised directly, and the staging
cap takes an optional `maxBytes` — the alternative would be a test that writes a real 100 MB file to
disk or builds a 140 MB base64 string, which is a test nobody keeps running.

### 4.10 The file-operation channels specifically (P3 work items 3–4)

`tests/integration/fileOps.integration.test.ts` (29 tests) covers the six converted channels:
`tls.exportCert`, `runner.exportReport`, `capture.shareJson`, `audit.export`, `tls.importCert`,
`tls.importKey`. It drives a real `CommandRegistry`, so every payload also passes the frozen Zod
schemas — the same shape as `importExportBlob.integration.test.ts`, and deliberately so.

**The assertions are about the boundary, not the content.** A format-level suite already proves the
HTML report renders and the CSV writer writes; what nothing else can prove is that *the engine no
longer touches the user's disk*, and that is what this file is for:

1. **Every successful artifact has exactly six keys** — `{ok, inline|blobId, suggestedName, size,
   mimeType, sha256}` and nothing else. `expectArtifactKeys()` asserts the key *set*, not just the
   presence of the expected ones, so a `path` or `filePath` field re-added for a renderer
   convenience fails here. That is item 6's acceptance criterion expressed as a runtime assertion
   rather than a grep — which matters, because the one leak this pass actually found was a runtime
   *string*, not a type (see §7).
2. **The inline/blob decision is the engine's, and both branches agree on `sha256`.** Small
   artifacts are asserted to leave `blobRoot()` non-existent — the common case is one round-trip and
   no store involvement at all — and large ones are pulled back through the real `blob.read` chunk
   loop, not through `blobContentPath()`. Using the transport primitive is the point: a broken `eof`
   or a mis-computed offset fails here rather than only in the app.
3. **Ingress takes a `blobId` and never auto-releases.** The engine copies the staged bytes into its
   data dir and the client releases; releasing the staging copy is asserted not to disturb the
   imported one. The retired inline `content` parameter is asserted to be **rejected**, because
   `blob.put` is what applies the size cap and the three integrity checks, and a second way in would
   skip both.
4. **Failures are results, never exceptions.** A released blob, a never-staged id and a traversal
   attempt all come back `{ok: false, error}` with a typed code — and the error is asserted to
   contain **no filesystem path at all**: not the blob root, not the data dir, not an `ENOENT`.
5. **A failed import cannot destroy a working certificate.** The previous cert is written *after* the
   blob read succeeds, so a bad id leaves it byte-identical. Ordering bugs here would silently break
   TLS on a failed re-import.

**Two fixture traps, both recorded because they cost real debugging time.** `appDataDir()` is where
the CA lives and it is **not** `dataDir()` — on Windows it is `%LOCALAPPDATA%/Bifurc` and does not
follow `setDataRoot()`, which is why `tls.exportCert` and the two imports resolve it that way. And
`gitStore` needs the workspace root while `dataDir()` needs the fixture root: passing the same
string to both would put the blobs inside the workspace tree. `proxyHarness.createWorkspace()`
already points `appDataDir()` at the fixture root via `setSettingsPathOverride()`, so no test can
touch the developer's real certificate.

**`audit.export` needs a real `git log`, and its one unexercised property is stated rather than
faked.** `limit: 0` means "the whole log" to `queryLog` (the default is 200), and that is why this is
the one egress artifact with no natural size ceiling. Proving the >200 case needs 201 real commits —
roughly ten seconds of git in a suite that already spends most of its time in real-git temp
workspaces — so the tests assert content fidelity over a three-commit log and the property is
recorded here instead of being asserted. If `limit: 0` ever regressed to the default, a
three-commit fixture would not notice.

### 4.11 The certificate lifecycle specifically (P3 work item 5)

Three suites, three different jobs, and the split is the point:

| Suite | Tests | What it can prove |
|---|---:|---|
| `tests/ipc/certTrust.test.ts` | 34 | the exact per-platform argv, the Linux chain, the verification primitive |
| `tests/ipc/certLifecycle.test.ts` | 21 | ordering and record-keeping across the engine/trust-store boundary |
| `tests/proxy/certManager.test.ts` | 19 | the fingerprint itself, and the four states of `getCertStatus()` |

**Why the platform logic is a pure function of an injected context.** Two of the three platforms cannot
be reached from this development machine, and the pre-P3 tests handled that by mocking
`child_process.execSync` and redefining `process.platform` — which asserts that *a* command ran, not
*which* one. It would not have caught `-delstore` where `-addstore` belongs, a missing `-user`, or a
keychain path built with the wrong separator. `installPlan()` / `uninstallPlan()` are pure, so
`certTrust.test.ts` asserts the argv arrays exactly and needs no mocking at all.

**Two `certutil` behaviours were measured rather than assumed, and one of them would have shipped a
silent security-relevant lie:**

- `certutil -delstore -user Root <thumbprint>` prints *"command completed successfully"* and exits **0**
  for a certificate that is **not in the store**. A result built on that exit code would tell the user
  their CA had been un-trusted when it had not.
- `certutil -store -user Root <thumbprint>` exits **0** when present and **17** (`NTE_NOT_FOUND`) when
  absent. That is the verification primitive.

So `probeTrust()` is a separate concept from `run()`, `TrustResult.verified` is a separate field from
`TrustResult.ok`, and `uninstallCA()` re-queries the store after deleting. `probeTrust()` returns
`true | false | null`, where `null` means *this platform cannot answer* — Linux has no probe that could
be verified here, and `verified: undefined` is the honest report rather than a `false` claiming a check
nobody performed. Both directions are pinned, including the awkward one: *"reports ok but not verified
when the delete command exits 0 and the store still has it."*

**The real exec path is exercised, not only faked.** Two tests run against the real `certutil` on
Windows (`it.runIf(process.platform === "win32")`): probing an absent thumbprint must return `false`,
and `has("certutil")` must resolve while a nonsense binary must not. Both are read-only — querying a
certificate that is not in the store changes nothing, which is why this is safe to run on a developer
machine and the install path is not.

**What the lifecycle suite is really asserting is ordering**, because each wrong order is a distinct
bug: the record is written *only* on a successful install (a record for a certificate that is not in the
store makes the next drift comparison lie); the record is *kept* when an un-trust fails (the PEM is the
only copy of the certificate the OS still trusts — clearing it makes the certificate unremovable); a
stale entry is un-trusted *before* the new one is added (never two Bifurc roots trusted at once); and
the staged certificate file is kept when the platform degraded to manual instructions and deleted
otherwise. The fingerprint is taken from the **bytes that were installed**, not from a separate
`tls.certStatus` call, so a regeneration between the two calls cannot be recorded wrongly.

**The PEM fixture is real, and that is load-bearing.** `generateCA()` now refuses to report success on
output it cannot parse, so the placeholder PEM the old suite used makes it *throw*. The fixture lives in
`tests/fixtures/certFixtures.ts` with its expected `sha256:` and SHA-1 values pinned literally, and one
test cross-checks the fingerprint against a hash of the DER computed a **different** way (decode the PEM
body ourselves) — a test that only re-read `X509Certificate#fingerprint256` would prove nothing.

**What is deliberately not tested, and why.** The full loop against a real OS trust store. That would
mean a test run mutating the developer's certificate store, which a suite must not do. The command
shapes are asserted instead, and the loop is recorded as unchecked **P9/P12** rows in
`plan/13-checklist.md` for Linux (`trust anchor` without elevation, and a probe for `--remove`) and
macOS (`delete-certificate -Z … -t` clearing the trust settings, not just the keychain).

**A registration bug, in the same shape as items 3–4 and in a third suite.**
`tests/spike/protocolPoc.test.ts` registered handlers by hand rather than calling `registerIpcHandlers()`,
so when `tls.generate` moved from the shell into the engine the channel still existed with nothing
behind it — failing with `No handler registered for command "tls.generate"`. Same lesson: **a client
that delegates needs its delegate registered in the test process.** *(That suite was retired with the P0
spike on 2026-09-21 — `spike/README.md`. The lesson is kept because it generalises to any hand-registering
consumer, and `tests/integration/settingsMutations.integration.test.ts` is still such a consumer.)*

That suite's `tls.generate` case also pinned the **renderer-facing** shape (`certPath`, `keyPath`) that
`TlsSettingsSection.tsx` depends on, so the retirement had to establish that the pin survived. **It did,
at a better layer.** `packages/client/tests/client.test.ts:1176` asserts the whole object —
`expect(res).toEqual({ ok: true, fingerprint: "sha256:abc", certPath: CERT, keyPath: KEY })` — which is
the shim that actually appends those two fields, and a `toEqual` on the full shape rather than the
spike's three `toContain` calls. `tests/proxy/certManager.test.ts:117` covers `generateCA()`'s own
`certPath` below it. **The spike was asserting the shape two layers away from where it is produced.**

---

### 4.12 The transport layer specifically (P4)

```
npx vitest run packages/engine/tests/conformance/ packages/engine/tests/transport/
```

**Measured 2026-09-18 (after item 1's scope-gated conflict resolution) — 482 tests, 442 passed, 40
skipped, 0 failed.** Of the 40 skips, **35 are the conformance suite's**, spread across the runners that
do not claim a capability — 16 on `run-in-process`, 14 on `run-stdio`, 3 on `run-authenticated`, 1 each
on `run-ws` and `run-socket` — and **5 are `socket.test.ts`'s POSIX-only cases**, which have no Windows
counterpart. All are emitted rather than omitted so the remaining gaps stay visible.

Note that the per-runner numbers rose by **two** each (55 → 57) while the totals rose by **10 tests, 5
passes and 5 skips**: the session work added a *pair* of cases, and the second one — *"a handler sees no
session"* — is gated by the **inverse** of `handshakeIt`, so the handshake-less runners **run** it while
the others skip it. That is why the counts move in both columns at once, and it is the reason a single
new case can shift five runners rather than one.

| Suite | Tests | What it is |
|---|---|---|
| `conformance/protocol.conformance.ts` | — | the **shared** suite. Transport-agnostic; no runner may hold assertions |
| `conformance/run-in-process.test.ts` | 57 (41 + 16 skipped) | the fastest runner, and the reference behaviour; claims **no** capability, so it *runs* the no-session case |
| `conformance/run-stdio.test.ts` | 57 (43 + 14 skipped) | a `PassThrough` pair through the real `framing.ts` codec, claiming `backpressure` only |
| `conformance/run-authenticated.test.ts` | 57 (54 + 3 skipped) | the auth decorator over `in-process`, claiming `handshake` + `auth`; also the worked example of handing **one** context to both halves |
| `conformance/run-ws.test.ts` | 57 (56 + 1 skipped) | a **real** `ws` server on an ephemeral loopback port, claiming all four |
| `conformance/run-socket.test.ts` | 57 (56 + 1 skipped) | a **real** unix socket / named pipe, claiming all four |
| `transport/framing.test.ts` | 17 | the length-prefixed codec — it has no `Transport` to run against |
| `transport/session.test.ts` | 17 | the shared **client core**, over a fake channel — the one place its contract is testable as a contract |
| `transport/stdio.test.ts` | 17 | pipe-lifecycle cases the shared suite cannot express |
| `transport/auth.test.ts` | 62 | scopes, token, decorator, **the session identity**, the scopes the gate will enforce, and **the host's own domain as a wire field** (`plan/15` work item 9 — **the flag behind it was removed 2026-09-24**, so the value a host supplies is now always `localhost`; the *transport option* stays optional, so these cases still pin the field reported when the host was told one and **absent** when it was not — asserted with `"hostDomain" in response`, because `toBeUndefined()` cannot tell an absent key from a present-but-`undefined` one, and the in-process transport passes objects by reference) |
| `transport/eventLog.test.ts` | 30 | the ring buffer: both bounds, lazy retention, the replay windows |
| `transport/eventPump.test.ts` | 24 | the coalescing pump, driven synchronously against an **injected clock** |
| `transport/ws.test.ts` | 14 | socket-specific cases: bind guards, TLS refusal, close codes, payload ceiling, and the `peer` address |
| `transport/socket.test.ts` | 18 (13 + 5 POSIX-only) | path naming, the stale/live/not-a-socket guards, `0600`, unlink-on-close, the frame ceiling |

**Why one shared suite and N thin runners.** `plan/05` lists "conformance suite written per-transport
instead of shared" as a risk, and the mitigation is that a runner holds **no assertions of its own** — it
wires a transport to the suite and nothing else. The runners are 30–115 lines each, and that is the
reviewable property. `run-in-process.test.ts` runs in 41 ms; `run-ws.test.ts` and `run-socket.test.ts`
in ~1.2 s each.

**The `socket` runner needed no changes to the shared suite, and that is the point of it.** The delivery
barrier and the async-capable `make()` that `run-ws` forced out are now properties of the suite, so the
second genuinely asynchronous transport cost only wiring. `plan/05` predicted this; the prediction
holding is the evidence that the `ws` step's findings were fixed at the right layer rather than worked
around at the call site. A corollary worth knowing before adding the next transport: **if a new runner
needs a suite change, that change is the finding, not the runner.**

**The client core is shared by `stdio`, `ws` and `socket`, and it has its own suite because two of its
rules are not assertable through a `Transport`.** `session.ts` owns correlation, the listener tokens,
the single `teardown()` and the envelope rule; a transport supplies a six-member `ClientChannel`.
`transport/session.test.ts` drives it with a **fake** channel that encodes through the real
`framing.ts` codec, which buys three things a real transport cannot: a channel can be made to *misbehave
on purpose* (a `write()` that throws), the bytes on the wire can be inspected directly, and the envelope
rule can be tested as a rule rather than as a handler's behaviour. The case that earns the file is the
**verbatim outbox replay**: `opened()` must not re-encode queued frames, because `encodeFrame` does
`JSON.stringify` and an already-encoded `Buffer` would go out as `{"type":"Buffer","data":[…]}` — which
the peer rejects as `not_an_object`, silently losing every subscription queued before the channel came
up. That defect was caught in review before it ran, and the failure it produces ("the subscription
succeeds and never fires") is the hardest class in this layer to attribute.

**The 5 POSIX-only cases are skipped on Windows, and that is a real coverage hole rather than a
formality.** A unix domain socket is a filesystem entry — it has a mode, it can be left behind by a
crashed process, and it must not be confused with a user's file. A Windows named pipe has none of those
properties, so `socket.test.ts`'s stale-path, live-path, not-a-socket, `0600` and unlink-on-close cases
are `skipIf(win32)` and do not run on a Windows development machine. They are written so CI on Linux and
macOS exercises them. The consequence to carry forward: on Windows, `socket.ts`'s permission assertions
are all `!isWindows`-guarded, so a named pipe's access control is its default DACL and nothing the engine
sets — which makes `auth` the only control a Windows named pipe actually has.

**Two suites cover item 1's scope gate, and neither is in the command above.** The gate is not a transport
concern, so it is tested where the behaviour lives:

```
npx vitest run packages/engine/tests/commands/registry.test.ts tests/integration/entityConflictScope.integration.test.ts
```

`packages/engine/tests/commands/registry.test.ts` (**10**) tests `callerHasScope()` and
`mayAffectUnnamedEntities()` as predicates: the engine's own caller gets every scope, a session answers
from its own scopes, there is **no hierarchy** (`admin` alone does not imply `read`), and a session that
reported no scopes is refused rather than assumed. `tests/integration/entityConflictScope.integration.test.ts`
(**9**, three of them `it.each` rows) is the one that matters, because it runs against a **real** temp
workspace and reads **real disk** through `loadConfig()` — which is exactly how it caught that
`onAddConflict` had never persisted anything (`writeEntity()` strips `enabled`; it lives in
`enabled.json`, and only `syncEnabledSet` writes it). It asserts the incoming mock is always stored in all
three contexts, that the sibling is disabled for the engine's caller *and* for a shell session, that it
**stayed enabled for the companion** (the additive-only guarantee — the companion has since been retired,
so the case now pins the guarantee for the **engine's own caller and a shell session**, which are the two
contexts that still exist), and that the gate is inert for a kind
with no conflict rule. **Assert against disk, not a mocked config object, or you will re-ship that bug** —
`tests/ipc/handlers.test.ts`'s equivalent case could not see it for precisely that reason.

**The suite is capability-gated, and claiming an unverifiable capability throws at definition time.** Four
capabilities exist — `handshake`, `auth`, `replay`, `backpressure` — and all four are in `VERIFIABLE`, so
every runner's claim is honoured and no case is skipped for a capability a runner actually has. The gate
is still load-bearing rather than vestigial: a runner that claimed something the suite could not verify
would get its cases skipped and ship a green run that proves nothing, so `VERIFIABLE` is what makes the
gap a build failure instead. **What `backpressure` means now** is that `log.entry` arrives as a protocol
**batch** (`{entries}`) and is coalesced under flood; the **flow-control window** is not covered by it and
is still open (item 3).

**The coalescing cases exist because the batch shape is in the protocol, not because the pump has a
counter.** Coalescing is invisible to a client *by construction* — it sees the same events whether the
engine sent one frame or a hundred — so no case written against `Transport` could tell a coalescing engine
from a forwarding one, and the alternative would have been a frame counter handed to the suite, i.e. a
testing hook in a production contract. `LogEntryBatchEventSchema` had been declared in `@bifurc/protocol`
**since P1 and never used**; it is the protocol's own statement of what a client receives for
`event.log.entry`, so delivering the batch makes the batching visible through the interface and therefore
assertable. Two consequences for anyone extending these cases: **the ungated batch-shape case runs on
every transport including `in-process`** (which has no wire and does not coalesce, but still produces
`{entries}` — a batch of one — because two shapes for one event name would be worse than either), and the
**coalescing cases assert strictly fewer deliveries than entries**, which is the assertion a forwarding
transport fails and the one a `toHaveLength(n)` assertion would not.

**`deliver()` is the delivery barrier, and it is the one piece of the suite that is not obvious.** It
emits a sentinel on `event.entity.changed` — a channel no case uses as its subject — and waits for it,
rather than sleeping. Delivery is FIFO within a session, so a sentinel arriving proves every event
emitted before it has already been delivered; in the negative cases it proves the event was *skipped*
rather than merely slow. **Never `setTimeout(0)`:** a socket's bytes take several ticks, and a timeout
that is *usually* long enough is a flaky test.

**The barrier had a defect that only a real transport could expose, and `run-ws` exposed it.** `deliver()`
awaited *delivery* but not *subscription establishment*. `subscribe()` is synchronous, so `in-process`
and `PassThrough` make a subscription live before the next line — and the suite silently depended on it.
Over a socket a `subscribe` is only *queued* as a control frame, and because retention is lazy an event
emitted before anything had subscribed was never numbered and never buffered, so it was **dropped rather
than delayed**. Nine `expected [] to have a length of 1` failures on the first `run-ws` run, none of them
a defect in `ws`. Fixed by giving `deliver()` an **emit callback** so the emit happens inside the barrier
after a command round trip; all ten affected call sites were rewritten, and the four pre-existing runners
came out byte-identical, which is the regression signal.

**The negative control was retired, not forgotten.** `run-deferred-delivery.test.ts` ran the same suite
against a double that deferred event callbacks by one macrotask, so the barrier could be proven
load-bearing before any real asynchronous transport existed. It was deleted on 2026-09-18, the day
`run-ws.test.ts` landed — a real socket is asynchronous for real reasons rather than by construction, and
a permanent fifth runner re-running the whole suite for a property now covered would be exactly the
per-transport duplication the risks table warns about. The evidence it produced (neutering `deliver()`
failed **exactly six** cases, all on that runner, with `in-process` and `stdio` green throughout) is kept
in `plan/05`.

**The every-command matrix is a sweep of 92, not a sample.** `COMMAND_FIXTURES` — the 93-entry
valid/invalid table in `packages/protocol/src/commands/fixtures.ts` — drives **two assertions per command
per transport**: the valid payload must resolve, and the near-miss invalid payload must reject
`BAD_REQUEST`. The invalid half is the load-bearing one: those payloads are otherwise well-formed, so a
`BAD_REQUEST` there is evidence that **the frozen Zod schema actually ran on the far side of the
transport**, which a `null` payload would not be. Two guards stop the sweep from quietly shrinking, since
both failure modes look like a green run: `MINIMUM_MATRIX_COMMANDS = 50` is a floor, and a
namespace-diversity check catches a matrix that narrowed to one namespace and still cleared it. The
protocol's own test asserts `COMMAND_FIXTURES` keys `COMMANDS` exactly, so a runner cannot narrow the
matrix and the matrix cannot fall behind the protocol.

**The session cases assert that a handler can learn *who* called it, and the pair is deliberately
two-sided.** `CommandContext.session` is optional because `in-process` and `stdio` have no handshake and
therefore no identity; the suite asserts both directions — *"tells a handler which session invoked it"*
(`handshakeIt`) and *"gives a handler no session, because nothing established one"* (the new
`noHandshakeIt`, i.e. `capabilities.handshake ? it.skip : it`). Both use the **handler itself as the
probe**, registered as the reserved `config.get`, so there is nothing for a transport to special-case.
The `peer` assertion is *not* in the shared suite and must not be: only `ws` has a peer address, since a
unix socket's path names the engine's own endpoint — so that one case lives in `transport/ws.test.ts`.
`transport/auth.test.ts` holds the other side of the contract: the decorator **reports** the session
through `onSession` but never writes the context itself, fires it **exactly once**, and never fires it for
a rejected handshake (wrong token, major mismatch, malformed `hello`, a pre-handshake command, or a second
`hello`) — each of those is a separate case, because "recorded a session that never opened" is the failure
that would matter in an audit log. The end-to-end wiring is only provable where the caller *constructs*
the decorator: `run-authenticated.test.ts` builds **one** context, hands it to the inner transport, and
mutates that same object from `onSession`. Two objects and the identity lands on the one nobody invokes.

---

### 4.13 The typed RPC client specifically (P5)

`packages/client/tests/` — **5 files / 54 tests**, all green. Run just this package with:

```
npx vitest run packages/client/tests/
npm run typecheck --workspace @bifurc/client
```

The second command is **not** optional. `tests/shape.test-d.ts` is a **compile-time** test, and the
`unit` project does not run it: vitest's `include` is `**/*.{test,spec}.?(c|m)[jt]s?(x)`, which
`shape.test-d.ts` does not match (it ends in `.test-d.ts`). It is checked only because
`packages/client/tsconfig.json` sets `include: ["src/**/*.ts", "tests/**/*.ts"]` — so if that
`tests/**` entry is ever dropped, every assertion in the file becomes a comment while the suite stays
green. Note also that the **root** `tsc --noEmit` leg cannot see it either: the root tsconfig is
`include: ["src/**/*"]`.

**`tests/surface.test.ts` (7) — the inventory is total.** Imports `src/preload.ts` **for real** (with
`electron` mocked) and captures the object handed to `exposeInMainWorld`, so the comparison is against
the runtime rather than the type. Asserts both-directions set equality, `toHaveLength(144)`, that the
four keys `BifurcApi` omits are classified `local`, that the 7 subscriptions all name `event.*`, and
the kind counts **73 transport / 48 entity / 7 subscribe / 3 shim / 13 local**.

**`tests/client.test.ts` (18) — the surface is byte-identical, and the policy is applied.** The
headline case is the plan's *"a script asserting `Object.keys(oldApi)` vs `Object.keys(newApi)`
reports zero differences"*, written as a test rather than a script so it runs inside the gate instead
of being remembered. It found a **real defect on its first run**: `close()` was enumerable, so
`Object.keys(client)` returned **145** against the preload's 144. `close()` is now attached with
`Object.defineProperty(api, "close", { enumerable: false })` — the plan's own instrument enumerates
*enumerable own* properties, so the plan's choice of instrument is what defines the surface. The
shell can still call `close()` because `contextBridge.exposeInMainWorld` copies only enumerable own
properties, so a non-enumerable `close` is reachable by the process that needs it and invisible to
the one that must not see it. A second case asserts the descriptor directly (`enumerable: false`,
`writable: true`) so a refactor that moves `close` back into the literal fails with the reason
spelled out rather than as an opaque 145-vs-144 mismatch.

**`tests/shape.test-d.ts` (compile-time) — the plan's "single most valuable test".** The productionised
form of `spike/protocol/shape.ts`, which proved the same property over a 10-method `Pick<>` and was
marked "throwaway, do not productionise". It asserts: the client satisfies `BifurcApi`; every declared
key is present; the four added keys and the three tightened keys are **exhaustive** (as `Record<>`
literals over the key unions, so an eighth difference fails the build — an array literal would not,
because arrays tolerate missing members); `close` is **not** in `BifurcApi`; all seven subscriptions
return `() => void`; and direction 2 of the spike (no signature drift) over the keys whose declared
type is left alone.

Four deliberate violations were compiled against it and produced **exactly** the four expected errors,
which is what makes the assertions evidence rather than decoration:

| violation | error produced |
|---|---|
| drop `getConfig` from the client | `TS2741: Property 'getConfig' is missing … but required in type 'BifurcApi'` |
| list 3 of the 7 differing keys | `TS2739: missing … setTitleBarOverlay, completeFirstLaunch, setZoomLevel, getTheme` |
| conditional on `getConfig`'s return type | `TS2322: Type 'true' is not assignable to type 'false'` |
| `undefined extends BifurcApiFull["getTheme"]` | `TS2322: Type 'true' is not assignable to type 'false'` |

The paired control — `undefined extends BifurcApi["getTheme"]`, which **passed** — is what shows
`undefined extends X` is the right probe for optionality, and that `BifurcApiFull` really does tighten
the three keys rather than merely being declared to.

**`tests/retry.test.ts` (17) — the code half AND the command half.** `isRetryable(code)` alone is
wrong in the dangerous direction: a `TIMEOUT` on `entity.create` means *we do not know whether the
mutation landed*, and the engine has already `unshift`ed the entity and won routing for it, so a retry
duplicates it. The effective policy is `isRetryable(code) && isIdempotent(command)`, and the
**default for an unlisted command is not retryable** because the failure modes are asymmetric (a
wrongly-retryable mutation duplicates data; a wrongly-non-retryable read costs one manual retry). The
two sets are asserted to **partition** `COMMANDS`, so a new command forces the decision where it is
added. The execution half (`withRetry`) is tested with an injected `sleep` that records instead of
waiting — the default sleeps for real, so the backoff sequence (`50, 100`, capped at 1000) would
otherwise cost 150ms per case and be flaky under load.

**`tests/transports.test.ts` (3) — the client over `in-process`, `stdio` and `ws`.** One case per
transport, all three running the **same** assertion body, because the failure mode worth guarding
against is "the `ws` case asserts less than the others and nobody notices". Each drives a read, a read
whose payload the client builds itself, `listWsdls` (which needs `config.get` *and* `entity.list`, so
two round trips through the client's own plumbing), and a mutation. `ws` is the only one built with
`auth`, and the handshake is a plain `request("hello", …)` on the raw transport **before**
`createClient()` — which keeps the client unaware that it is remote, the property P6 needs. Removing
the handshake fails the `ws` case with `UNAUTHORIZED` and leaves the other two green (verified), so
the gate is genuinely exercised rather than bypassed. `stdio` uses a `PassThrough` pair for the
reasons `run-stdio.test.ts` documents; the `ws` case binds a real ephemeral port.

**`tests/subscriptions.test.ts` (9) — the multiplexer.** Two listeners on one event share **one**
transport subscription; the survivor keeps it; `unsubscribe` is idempotent (StrictMode calls effects
twice); a throwing listener does not stop the others; mid-dispatch removal does not skip a listener;
`close()` is idempotent. The unsubscribe contract is preserved exactly, because several renderer
effects depend on it and silently returning `undefined` leaks listeners.

**A trap worth recording: a `packages/*/tsconfig.json` must contain no comments.** The first version of
`packages/client/tsconfig.json` carried explanatory `/** … */` comments and **every test in the
package** failed at transform time with `[TSCONFIG_ERROR] JSON parse error` — Vite's oxc parser rejects
JSONC, unlike `tsc`. Comments are fine in `tsup.config.ts` (transpiled) but fatal in `tsconfig.json`
(parsed as JSON).

---

### 4.14 The web target specifically (P7)

**`packages/web-client/tests/browserTransport.test.ts` (10) — the browser `ClientChannel` over the platform
`WebSocket`.** Eight cases came with the package; two were added 2026-09-22 after the first full-suite run
failed on a **15 s test timeout** rather than on an assertion. That distinction is worth more than it looks:
a timeout says *something never settled*, so no message was computed and no branch was taken. A probe of the
platform's own `WebSocket` (Node 22's, i.e. undici's) settled it in one run — on a failed connect it fires
`error` and then **no `close` at all**, with `readyState` still `CONNECTING`. `ws`, which the engine's
channel uses, *does* emit `close` after `error`, so `ws.ts`'s reasoning is true where it is written and false
here, and **that handler cannot be copied**. The two new cases are the other two ways a connect can end: a
bare `node:net` server that accepts and never answers (which is what the connect timeout is for), and an
`http.Server` whose `'upgrade'` handler writes a 426 **by hand** — an ordinary `'request'` handler leaves the
upgrade in limbo, so the "refusal" fixture reproduced the very hang it was meant to be distinguished from.
Both are mutation-guarded by **exit code**, not by a failure count: never arming the timer, and silencing
`onerror`, each make the run exit 1. The second mutation is also what showed the dead-address case *passing*
through its own disjunction (`/Could not connect|Timed out after/`), which is why the refusal case exists.

**`packages/web-client/tests/browserLocal.test.ts` — the client-local half.** The 16 `RequiredClientLocal`
members, written DOM-optional (`storage()` returns `undefined` rather than throwing, `detectPlatform()` falls
back to `"unknown"`, `applyZoom()` swallows a missing `document`), which is what lets the suite below run in
the **node** environment rather than jsdom.

> ## ⚠ The `web/tests/*` sections below describe suites that were DELETED on 2026-09-26
>
> `web/tests/connect.test.ts` (12), `web/tests/api.test.ts` (6) and `web/tests/settings.test.ts` (9) went with
> the browser host, along with `npm run verify:web` and `verify:web:reconnect`. **The findings are kept,
> because most of them were about the engine rather than about the browser** — the `hello` handshake the host
> requires before the first real request, the replay contract (`lastSeq` → `replayedFrom` / `resyncRequired`),
> and the retention rule that attaches on the first *subscription* rather than the first emit. Read them as
> engine facts a browser happened to discover, which is how the extension will meet them too.
>
> The `packages/web-client` section above is **not** affected: that package survives, because the extension is
> a browser context and will need a browser `Transport`.

**`web/tests/connect.test.ts` (12) — the composition, against a real engine over a real socket.** The
first suite that can exercise the whole web path, because `createClient` **requires** a `local` and there is
no Electron shell here to supply one: a real round trip, a client-local operation over the same instance, and
the dropped-engine path through `onFatal`. It runs in node deliberately — under jsdom the first `open` event
throws, because jsdom's `WebSocket` is undici's and builds its `Event` objects from a different realm than
the `EventTarget` it dispatches them to (measured, and recorded in the file's header). What it pins is the
*wiring*: a real `WebSocket` in a real page is work item 6's Playwright job.

**Five of the eight are the `hello` handshake**, and they exist because its absence was a shipped defect: the
host wraps its socket in `createAuthenticatedTransport`, which requires `hello` first and **closes the
session** when it is not, so the browser's connection died on arrival and the engine logged `the first request
must be "hello"` once per attempt. Electron never hit it — its transport is `in-process`, never wrapped —
which is why the gap survived P6. The cases are written to **fail if the handshake is removed**, and that was
measured rather than asserted: disabling the handshake condition makes **exactly two** of them fail
(*"authenticates before the first real request"* and *"classifies a protocol major mismatch as `version`"*)
while the other six pass untouched. The remaining three pin the classification — missing token, wrong token,
and a bare unwrapped endpoint where the handshake must be **skipped**, because `hello` is deliberately not a
registry command and would come back `UNKNOWN_COMMAND`.

**Four more are the replay contract**, and they exist because the branch had **never executed**: `lastSeq`
was sent and the outcome handled, but nothing had forced either arm, so `replayedFrom` and `resyncRequired`
were shapes no client could have observed. Against a real engine: no `lastSeq` on a first connection (neither
field), a `lastSeq` in range (→ `replayedFrom`), a `lastSeq` **ahead** of anything the engine ever sent
(→ `resyncRequired` — an empty replay would leave the client believing it was up to date, which is the silent
staleness `plan/05` warns about), and **the replayed event actually arriving** on a subscription opened
*after* the handshake, which is what a real client does since the UI subscribes when it mounts. Removing
`lastSeq` from the `hello` fails **exactly the last two** and leaves the other ten untouched.

> **One engine property worth knowing, found by getting it wrong:** retention attaches on the **first
> subscription to a name**, not on the first emit — *"a bus listener for an event is attached the first time
> anything subscribes to that event and is held until `close()`"* (`eventLog.ts`). So an emit before anybody
> has ever subscribed is neither numbered nor buffered. The first version of that case emitted first and
> asserted against a log that had recorded nothing. A property, not a bug: an event nobody could have wanted
> does not need retaining.

**`web/tests/api.test.ts` (6) — the browser's `window.api`, which is the D18 seam itself.** The shared UI
reads one injected global, and the two hosts differ only in what fills it: the Electron preload does
`contextBridge.exposeInMainWorld("api", { ...client })`, and `web/api.ts` does
`(globalThis as …).api = { ...connection.client }` — **the same line, without the bridge**. This suite is what
stops the browser's half from drifting. Against a real engine: the assigned `window.api` actually answers a
command (not merely exists — a structural assertion would pass on a blank screen); its enumerable keys are the
client's surface and do **not** include `close` (non-enumerable on the client so that `Object.keys(client)`
matches `window.api`, which is why the host gets a separate teardown handle); a lost engine reaches `onFatal`
as the *dropped* message rather than the connect-failure one; and `uninstallWindowApi` really removes the
global, because a stale one leaves the UI calling a client whose transport is closed.

**And one case is about *ordering*, which is the reason `web/` is a container at all**: installing with a bad
token must reject **and leave `window.api` undefined**. An installer that assigned first and authenticated
second would hand the UI a client the engine had already disconnected — the original defect's exact shape.

**`web/tests/settings.test.ts` (9) — the connection form's pure half.** `toUrl`/`fromUrl`, the
load/save round trip, and the defaults. Split from the form so it runs in **node** with no DOM and no
`localStorage`, and the cases that matter are the ones about **untrusted input**: these settings come from
`localStorage` (hand-editable in devtools) and from `?engine=` (anyone can put anything in it). A `host` that
arrives as a number or a `port` that arrives as an object has to become the default rather than reach
`new URL()` and throw from inside a connection attempt, where it would read as a network failure and send
someone debugging the wrong thing.

> ## ⚠ All three commands below were DELETED on 2026-09-26
>
> `verify:web`, `verify:web:reconnect` and `verify:docker-entrypoint` are gone from `package.json`, along with
> the browser host and the container image they exercised. **Nothing replaced them, and that is worth stating
> rather than leaving implicit:** `npm test` is now the whole automated instrument for the desktop app, and
> `npm run test:e2e` (owner-run, 11 specs) is what covers the app itself.
>
> **The findings are kept, because the lesson outlived the scripts.** These three were the only instruments for
> a class of defect no suite sees — a *wiring* gap in code every unit test passes — and they found five of
> them, four in code written minutes earlier that was passing 471 tests. Two of those four are generic: **a UI
> that unmounts during an outage while the comment beside it claims it stays mounted**, and **`void promise`
> does not catch** — it discards the promise and its rejection alike.

**Beyond the suites: three commands that were not specs, and they were the only instruments for the classes of
defect below.** None was part of `npm test` — the two browser checks needed a running host and a served app, and the
Docker one needed the real entrypoint plus four ports.

| Command (deleted) | What it asserted |
|---|---|
| `npm run verify:web` | the form renders, Connect succeeds, the shell mounts, and **0 console errors / 0 uncaught page errors / 0 warnings** |
| `npm run verify:web:reconnect` | the host dies → banner with the **UI still mounted**; restart with the same token → automatic recovery; restart with a **rotated** token → the loop stops with the actionable message |
| `npm run verify:docker-entrypoint` | `plan/17` §5 item 5's **three-way agreement**, which no unit test can see: the static server binds `companion + 1`, the host is told `--gui-target` on that same port, and the healthcheck reads that port from `/tmp/bifurc/companion-port` — plus the SPA fallback, a 404 for a missing asset, and the edge routing the base domain to the GUI. It started the **real** entrypoint, so it bound `80`, `8080` and the `4747`/`4748` pair — which is exactly why it was owner-run rather than a spec. |

They exist because of what they found. `verify:web` caught a favicon 404 that `page.on("response")` could not
see, because Chromium requests `/favicon.ico` outside the page's network context. `verify:web:reconnect`
caught **four defects in code written minutes earlier and passing 471 tests**:

1. the UI **unmounting during an outage** while the comment beside that code claimed it stayed mounted;
2. two `TypeError`s from uninstalling `window.api` under a mounted UI — a dead client rejects, whereas
   `undefined` throws;
3. an error screen replacing the shell for a **refresh** failure, while the banner above it said "reconnecting";
4. `void refreshConfig()` producing `Uncaught (in promise) EngineError: Cannot request(): this session
   ended` — **`void promise` does not catch**, it discards the promise and its rejection alike.

**None of the four was visible to `tsc` or to the suite**, which is the whole argument for having them.

> **A gap this section has to name: nothing typechecks the shared UI.** `packages/gui` carries **152
> pre-existing type errors** — measured, not estimated, and pre-existing rather than introduced by the D18
> move (211 of the 212 moved files are `R100`, byte-identical, and the post-move run reports **0** `TS2307`).
> 105 of them are one root cause: `useProtocolEditor` is generic, but both call sites pass `as any` for its
> config functions, so TypeScript cannot infer `State` and falls back to `unknown` (`SoapTab.tsx` 58,
> `GraphQLTab.tsx` 47). Because `web/` and `renderer/` both import the UI, every typecheck that covers either
> host now inherits all 152 — so CI's web step is `continue-on-error` until they are fixed, and
> `typecheck:renderer` has never run in CI at all. This is §2.3's finding ("type checking never covered the
> renderer") finally given a number.

**A note on what that third case does *not* guard.** It asserts the fatal is the *dropped* message
(`/^Session closed/`) and rejects `Could not connect` outright, but deleting the `open` guard in `channel.ts`
leaves it **passing** — `server.close()` is a clean close and never reaches the `error` path on this host. The
instrument for that line is `browserTransport.test.ts`. See `TRAPS.md`, "A case can be sound and still guard
nothing about the line you changed".

---

## 5. Workflow changes

### `vitest.config.ts`
- Split into two **projects**: `unit` (fast, no sockets) and `integration` (real servers,
  `fileParallelism: false` so port-binding output stays readable).
- **Coverage thresholds** added as a ratchet, then raised after each pass
  (`statements 47 / branches 31 / functions 35 / lines 49`; originally `23 / 14 / 16 / 24`, then
  `46 / 31 / 34 / 48` through P2). New untested code now fails CI instead of silently lowering
  coverage. Raise these as gaps close — never lower them to make a build pass. The P3 ratchet left
  `branches` alone on purpose: 2 pts below the 32.66% actual is 30, and ratcheting *down* is the
  thing the policy forbids.
- Reporters expanded to `text`, `text-summary`, `html`, `json-summary`, `lcov`.
- `include` widened to `packages/gui/src/panels/**`, then to `packages/gui/src/components/**/*.ts` (the glob
  had matched only `.tsx`, hiding 17 files — see §4.2). Type-only and pure-constant modules
  excluded.
- **`dualAliasPlugin` no longer resolves to a directory.** The `""` extension matched
  `packages/gui/src/lib/strings/` before the `index.ts` fallback could run, so Vite was handed a
  directory id and any module importing the `strings` barrel was untestable.
- **2026-09-15 — `@bifurc/engine/*` is aliased to engine *source*.** Two things about this are
  easy to get wrong:
  1. It must be a `resolve.alias`, not a `resolveId` plugin. `@bifurc/engine` is a real npm
     workspace package, so Vitest externalizes it and native `import()` loads the built
     `dist/*.mjs`. A plugin's `resolveId` is never consulted for an externalized specifier
     (verified: a probe inside `resolveId` never fired, and a deliberately **bogus** alias target
     was ignored while the suite still passed against `dist`). `resolve.alias` runs inside Vite's
     core resolver, before the externalization decision.
  2. It must be redeclared **inside every `test.projects` entry**. Root-level `resolve.alias` is
     not inherited by project runs — which is exactly why `plugins` was already duplicated in, and
     why `dualAliasPlugin` worked while the engine alias silently did nothing. The config now
     shares one `viteOptions` object (`{ plugins, resolve: { alias } }`) across the root and both
     projects.
  A `deps.inline: [/^@bifurc\/engine(\/|$)/]` pattern was tried during the investigation and is
  **not** needed — the alias alone is sufficient and is the only mechanism that works.
- `coverage.include` gained `packages/engine/src/**/*.ts`. This is load-bearing, not decorative:
  `src/**/*.ts` does **not** match `packages/*/src/**` (verified against the same glob engine
  Vitest uses), so without it the 13 moved files would silently vanish from the report.
  `coverage.exclude`'s type-only entries followed the move: `src/store/types.ts` →
  `packages/engine/src/store/types.ts` (layer 1) and `src/sync/types.ts` →
  `packages/engine/src/sync/types.ts` (layer 2). **Check this every layer** — an exclude entry that
  no longer matches does not error, it just silently starts counting a type-only module as
  uncovered code.
- **Layer 3 found the mirror-image bug: an exclude that was always wrong.** The list carried
  `src/applications/types.ts` under "Type-only modules", but that file exports three runtime
  constants. The exclusion was made on the strength of the filename and had been hiding live,
  fully-covered code from the report. The entry was **removed** rather than carried to the new
  path, and the file now reports 100%. Filename-based exclusions are a standing hazard — check
  that a `types.ts` really is type-only before adding one.

### `package.json` (root and `bifurc/`)
- New: `test:unit`, `test:integration`, `test:ci`, `typecheck:e2e`, `typecheck:renderer`.
- `test:watch` now watches the unit project only.
- **2026-09-15, for the engine package:** `build:packages` (protocol, then engine),
  `build:main` (`build:packages && tsc && tsc-alias`), `typecheck:packages`, and a `prepare`
  hook running `build:packages`. `typecheck` now runs `build:packages` first — see the
  *Build-robustness fixes* table in §7.

### `.github/workflows/test.yml`
- New **`typecheck` job** (gates `src/` and E2E specs) so type errors fail before tests run.
- `unit-and-integration` job runs `npm run test:ci` — coverage thresholds are enforced.
- E2E job now installs **Electron's Linux system libraries**. Previously only Chromium was
  installed, so Electron could not launch on `ubuntu-latest` and every E2E test errored out
  before asserting anything.
- `setup-node` npm cache enabled for both lockfiles.

### `e2e/`
- `e2e/tsconfig.json` fixed (`rootDir` override + `packages/gui/src/types/window.ts` included) —
  E2E type-checking now actually works and is part of CI.
- `e2e/helpers/index.ts` rewritten. It now **asserts** (`openPanel` fails if the panel is
  missing) instead of silently skipping. Added `uniqueName`, `pause`, `chooseProtocol`,
  `fillVisibleCodeEditor`, `expectPanelAnchor`.
- `app.spec.ts`, `mappings.spec.ts`, `proxy-rules.spec.ts`, `mocks-rest.spec.ts`,
  `requests-rest.spec.ts` rewritten with real create/edit/validate assertions and
  deterministic cleanup through `window.api`.
- `app.spec.ts` had a genuine bug: `const window = await electronApp.firstWindow()` shadowed
  the DOM `window`, so `window.evaluate(() => window.innerWidth)` threw at runtime. Fixed.
- **The suite passes in full as of 2026-09-22: 11 specs / 46 tests — 46 expected, 0 unexpected, 0 flaky,
  0 skipped** (owner-run; real Electron). **Read the result from the run's own artifacts before asking for
  a re-run** — `playwright-report/index.html` embeds `report.json`, whose `stats` carries the counts and
  `ok`. `screenshot: "only-on-failure"` + `trace: "on-first-retry"` mean a failure leaves
  `test-results/<spec>-<test>-electron/error-context.md` — the error **plus a full accessibility snapshot
  of the page** — and `test-failed-1.png`. **All seven** failures that stood between this and the
  `46 failed / 0 passed` first attempt were diagnosed from those files alone. **That first number was an
  agent-shell constant, not a measurement:** Electron cannot start its GPU process in a sandboxed shell,
  so every spec dies at `firstWindow()` in ~850 ms. Two more consequences worth knowing: the tree row is a
  `<div title=…>` (role `generic`, name from `title`), and `Ctrl+T` opens the **protocol chooser**, not the
  request form. Details: `.workbuddy-ai/memory/TRAPS.md`.

### Renderer type fixes (prerequisites for `typecheck:e2e` passing)
- `packages/gui/src/types/window.ts` and `packages/gui/src/types/ipc.ts` imported `Workspace` from
  `./entities`, where it does not exist. `Workspace` lives in `./config`. Fixed.

---

## 6. Backlog — what is still untested

Ordered by risk. Each item names the file(s) and what a test would need to do.

> **Closed in the second through seventh passes (2026-09-14):** these items are now done —
> `decompressUtils` (100%), `responseUtils` (87%), `scriptContext` (96%) /
> `scriptExecutor` (89%), the whole import/export tree (`importExport.integration`,
> `importExportFormats.integration`, `registry.test`, and — added in the eighth pass, once the
> P3 blob layer existed to test against — `importExportBlob.integration` plus the `blob/**`
> store, sweep and command suites), `packages/engine/src/applications/**` (87%),
> `packages/engine/src/subscription/entityCount.ts` (100%), `packages/engine/src/companion/allowedActions.ts` (100%), the
> Audit Log screen (`auditLog.integration`), **the "hit Send" path**
> (`protocolExecution.integration` — REST replay, GraphQL execute/introspect, SOAP
> execute/fetchWsdl, Health Bar polling), **folder management** and **workspace CRUD**
> (`foldersAndWorkspaces.integration`), **the collection runner and its reporting**
> (`collectionRunner.test`, `runnerStorage.integration` — `packages/gui/src/lib/collectionRunner.ts`
> 96%, `runnerReport.ts` 100%), **git publish and git sync**
> (`gitSync.integration`, `gitRemote.integration` — `publishService` 85%, `syncHandlers`
> 75%, `syncManager` 80%), **capture** (`capture.integration`, `captureUtils.test` —
> `captureUtils.ts` 100%, all five `via` paths, the 512 KB cap, the export handler),
> **TLS interception** (`tlsIntercept.integration` — real CA, real handshake,
> `tlsIntercept.ts` 0% → 88%), **the protocol tab reducers** (`protocolTabReducers.test` — `graphql` 93%,
> `grpc` 89%, `soap` 83%), and **settings mutations** (`settingsMutations.integration` — the port change
> moves the live server, TLS toggling reloads the CA, theme/zoom persist, and a live CRUD mutation reaches
> routing with no restart). **The companion server was also covered here (`companionServer.integration`,
> `companionServer.ts` 0% → 94%, allowlist proven enforced) — and that coverage left with the code when the
> companion was retired on 2026-09-23.**
> Twelve production bugs and two harness/reporting bugs were fixed along the way. What
> remains is listed here.

### High — security / correctness
1. ~~**`packages/engine/src/companion/companionServer.ts` has no authentication.**~~ **RESOLVED
   2026-09-23 by removal.** The finding was real and deliberate to state: the server bound to
   `127.0.0.1`, which was the whole of its access control, and `ALLOWED_ACTIONS` was proven *enforced*
   (a forbidden action was refused and wrote nothing) — but any local process could still open a socket
   and call the four allowed actions. The companion was **retired outright** (`plan/15` §6.4) rather
   than hardened, so there is nothing left to authenticate. **The successor must not re-open it:** the
   in-process/WebSocket transport (`transport/ws.ts`) carries the *same* write paths and **does**
   authenticate — it has a fail-closed token guard at `host.ts` step **−1** and a `hello` handshake.
   Any future extension client speaks that contract, which is why the extension's migration is a
   protocol change and not a port swap.

### Medium — features users depend on
2. **gRPC is not implemented** — `grpc:execute` and `grpc:reflect` return "runtime not yet
   configured" and the mock server is a stub. The handlers are now covered *as stubs*, so
   this is a product gap rather than a testing gap: there is nothing to exercise until the
   runtime is wired up.
3. **`packages/gui/src/components/rest/{useCollectionRunner,useRestActions}.ts` (0%)** — React hooks.
   The runner's *engine* is covered, but these hooks (which wire it to the panel) are not;
   they need a DOM environment or extraction of their pure parts.
4. **Search / command palette** — `searchUtils`/`searchHelpers`/`searchModules` are now
   measured (58–85%), but `searchPanelUtils.ts` sits at 14% and no test types a query, opens
   a result and lands on the right entity.
5. **`packages/engine/src/applications/portUtils.ts` (67.8% stmt / 31.3% br)** — `killProcessOnPort`'s real
   kill path is intentionally not exercised (destructive); the resolve/free-port branches are.
6. **Dead code in `src/sync`** — `fetchRemoteHead()` and `performGitClone()`
   (`gitSyncOps.ts`) are exported but never called; `restoreEntity()` (`publishService.ts`)
   is likewise unused (`entity:restore` goes through `gitOps.discardChanges`). They cap
   `gitSyncOps.ts` at 50%. Either delete them or wire `setRemote()`/`getRemoteHead()` to use
   them instead of their inline duplicates.
7. **The `refresh` contract is now enforced only for the protocol tabs.** `createTabReducer`
   gained `LOAD_ENTITY` / `LOAD_DRAFT` / `REFRESH` handling (see §7, bug 11), so all four
   editors honour it. There is still no test that a *panel* wires `tabRefs.current[id].refresh`
   correctly — that is E2E territory.
8. **UI delete paths** — `mock:delete`/`rule:delete`/`mapping:delete` are covered at the IPC
   level (§3.4), but nothing exercises the confirmation dialogs and tab-closing behaviour in
   the panels. E2E territory.

### Lower — UI surface
9. **`packages/gui/src/panels/**` (0%)** — the panels are covered only by the Playwright E2E suite,
   which cannot run in this environment. `renderer/components` is now *measured* at 13.5%
   after the scope fix, but that is still the weakest area.

### Follow-ups identified during the review
10. **Renderer type debt** — 152 errors. Fix, then add `typecheck:renderer` to CI.
11. **Remaining guarded E2E specs** — `protocols.spec.ts` (4 guards),
    `environments.spec.ts` (12), `capture-ws-webhooks.spec.ts` (7), `settings.spec.ts` (4),
    `screenshots.spec.ts` (1, zero assertions). Harden them the same way as the ones already
    converted, then delete `screenshots.spec.ts`'s `test()` wrapper or mark it clearly as a
    screenshot generator rather than a test.
12. **`mkId()` collision risk** — `packages/engine/src/proxy/serverUtils.ts` appends only 4 base-36 random
    characters (~1.6M space) to a millisecond timestamp, so a burst of ids within one
    millisecond has a small real collision probability. Widen the random suffix.
13. **E2E binds port 80** — `e2e/fixtures/sampleData.ts` writes `port: 80`, which needs root
    on Linux CI. Consider an unprivileged port for E2E runs.
14. **Split `happy-path-workflows.spec.ts` per screen.** It is one long spec covering every
    panel; a failure names the screen only by the test title. Splitting it would make CI
    failures self-describing.
15. **Order-dependent fixtures in `gitRemote.integration.test.ts`.** `rootC` is a shared `let`
    assigned inside one test (`:421`, `"reports an error for an unreachable remote"`) and read by
    the next (`:433`, `"refuses to connect a non-empty workspace to a non-empty remote"`). The pair
    only passes because Vitest runs a file's tests in order, so **any** `--testNamePattern` that
    deselects the first breaks the second with `Cannot read properties of undefined (reading
    'activate')` — a failure that looks like a product regression. Found on 2026-09-16 while
    deselecting the flaky family for a coverage run. Move the assignment into a `beforeEach`, or
    make `rootC` local to each test.

---

## 7. Known issues found during this review

| Issue | Evidence | Status |
|---|---|---|
| Renderer is not type-checked | `tsconfig.json` `include: ["src/**/*"]` | **Open** — `npm run typecheck:renderer` reports 152 errors, none of them in the files this review touched |
| `npx tsc` is **not** the TypeScript compiler here | `npx tsc` resolves the placeholder npm package, prints "This is not the tsc command you are looking for" and **still exits 0** — so `npx tsc ... \| wc -l` cheerfully reports "0 errors" | **Documented** — always use `npm run typecheck` / `npm run typecheck:renderer`, never `npx tsc` |
| A **blank GraphQL mock is never reported as an empty draft** | `isDraftEmpty(state, "mock")` tests `!state.responseBody`, but the default body is the non-empty `{ data: {} }` envelope; SOAP avoids this by comparing against its default envelope | **Open (cosmetic)** — `useDraftPersist` uses the predicate only to skip its debounced save, and it saves on mount and unmount regardless |
| E2E `tsconfig.json` never worked | inherited `rootDir: ./src` → `TS6059` for every spec | **Fixed** |
| `app.spec.ts` shadowed `window` | `const window = await electronApp.firstWindow()` | **Fixed** |
| `Workspace` imported from wrong module | `packages/gui/src/types/{window,ipc}.ts` | **Fixed** |
| `expect(body).toBeTruthy()` used as an assertion | 8 E2E spec files | **Partially fixed** — 5 files converted, 5 remain (item 9) |
| E2E could not launch on CI | no Electron system libs installed | **Fixed** |
| Coverage had no gate | no `thresholds` in `vitest.config.ts` | **Fixed** — ratchet added |
| Coverage **under-reported** the renderer | `include` listed only `packages/gui/src/components/**/*.tsx`, hiding 17 `.ts` files — three of which already had passing tests | **Fixed** — `.ts` added; type-only/barrel files excluded. See §4.2 |
| `dualAliasPlugin` resolved to a **directory** | the `""` extension matched `packages/gui/src/lib/strings/` before the `index.ts` fallback, so Vite got a directory id and any module importing the `strings` barrel failed to load | **Fixed** — skip directories, fall through to the index lookup |
| **Engine tests ran against built `dist/`, not source** — all 14 engine files reported 0% while the suite passed 1615/1616 | `@bifurc/engine` is a real npm workspace package, so Vitest externalized it and native `import()` loaded `packages/engine/dist/*.mjs`. Proof: a source-only export marker read `false`; a `resolveId` probe never fired; a deliberately **bogus** alias target was ignored and the suite still passed | **Fixed** — `resolve.alias` → engine source, redeclared inside each `test.projects` entry. The worst case this hid: a source edit that was never rebuilt was **silently untested**. See §4.8 and §5 |
| A fresh clone could not `npm run typecheck` | `dist/` is gitignored repo-wide and CI ran only `npm ci`, so `@bifurc/protocol` had no build output → 10× `TS2307: Cannot find module '@bifurc/protocol'` | **Fixed** — see the *Build-robustness fixes* table below |
| Dead code in `src/sync` | `fetchRemoteHead()` / `performGitClone()` have zero call sites; `getRemoteHead()` re-implements the former, `setRemote()` the latter | **Open** (item 9) |
| `mkId()` collision probability | `Date.now().toString(36) + rand(4)` | **Open** (item 13) |

### Production bugs the new tests uncovered and fixed

All sixteen were invisible to the old suite. The first four because it asserted on in-memory
config instead of reading state back off disk after a real export → import round trip; the
fifth because nothing ever sent two requests to the same origin; the sixth and seventh
because no test ever rendered an exported report or published a folder; the eighth and ninth
because nothing ever published into a folder with a space in its name or cloned a workspace
from a remote; the tenth because nothing ever observed the companion server's broadcast to
the renderer; the eleventh because the protocol reducers had no test at all; the twelfth
because the CRUD tests asserted on the *config object* the handler returned rather than on
what the running proxy served afterwards. The thirteenth and fourteenth are the first two
found by the blob-layer suite, and both for the same structural reason: **no test had ever
executed `workspace-zip`, or the YAML half of the OpenAPI importer, end to end.** The
fifteenth and sixteenth came out of P3's channel conversion, and they are a matched pair in a
way worth noticing: both are defects of a **type that was narrower than its only caller**, and
in both cases the wrong type was *satisfied* by the compiler rather than rejected by it — which
is why neither could be found by reading the code, only by asserting on the data.

| Bug | Where | Fix |
|---|---|---|
| Proxy-rule export wrote the **UI stubs** (`targetType`, `targetExternal`, `targetMappingId`, `useRegex` and both scripts blanked), so re-importing silently destroyed every rule | `packages/engine/src/importExport/exporters/{proxyrules-json,workspace-json}.ts` | read `readAllEntities(wsId, "rules")` + re-inject `enabled` from `enabled.json` |
| Workspace import **never wrote rule files to disk** (relied on `saveConfig()`, which deliberately skips them) → imported workspace proxied nothing | `packages/engine/src/importExport/importers/workspace-json.ts` | explicit `writeEntity(..., "rules", …)` loop + enabled set + `upsertNameEntry` |
| Disabled mappings/mocks came back **enabled** after import (`if (m.enabled) set.add()` with no `else`; missing flag ⇒ enabled) | `packages/engine/src/importExport/importers/workspace-json.ts` | both-branch enabled sets (`set.add` / `set.delete`) |
| Audit Log showed **no changed files for a workspace's first commit** | `packages/engine/src/store/gitStore.ts` `getCommitChangedFiles` | `diff-tree` → `diff-tree --root --no-commit-id -r --name-only` |
| REST **Send** could fail with a spurious **400 / "socket hang up"** on a later request to the same origin | `packages/engine/src/proxy/serverReplay.ts` `replayRequest` | it sent `connection: close` but did not pin an agent, so Node returned the socket to the global pool whenever the upstream answered `keep-alive` (servers routinely ignore the request's close) and the next request was rejected by the server with `HPE_CLOSED_CONNECTION`. Fixed with `agent: false`. |
| The exported/saved **HTML run report rendered raw `${…}` placeholders** instead of data — the whole report was inert. The same escaped-interpolation mistake sat in the update-checker's fallback URL | `src/ipc/handlers/runnerHandlers.ts` `generateRunnerHtml`, `src/ipc/handlers/systemHandlers.ts` | un-escaped all 15 interpolations and hardened `esc()` to coerce + escape quotes; fixed the URL template |
| Publishing a folder that contained **exactly one new entity recorded a create as `update mock folder "X"`** and dropped the per-entity `entity-id` link. Cause: `simple-git` reports a newly-added file in **both** `status.staged` and `status.created`, so concatenating them double-counted every new file and made the single-entity branch unreachable | `packages/engine/src/sync/publishService.ts` `publishEntities` | dedupe with `Array.from(new Set([...staged, ...created, ...deleted]))` |
| A new entity inside a folder whose name contains a **space** was classified `update` instead of `create`. Cause: the pre-staging status map was built from C-quoted porcelain output (`"mocks/My Folder/x.json"`), so the lookup never matched | `packages/engine/src/sync/publishService.ts` | unquote git paths when building `preStatusMap`; `unquoteGitPath` is now exported from `packages/engine/src/sync/statusTracker.ts` and shared |
| Cloning a workspace from a remote **never adopted the remote's identity** and landed **without a `workspace.json`**. Cause: `initWorkspaceRepo()` committed only `.gitignore`, so the identity file was never version-controlled — making the adoption branch in `setRemote()` dead code | `packages/engine/src/store/gitStore.ts` `initWorkspaceRepo` | commit `workspace.json` alongside `.gitignore` on repo init |
| The companion server's **entity-status broadcast always sent an empty map**, so an entity added from the browser extension never showed its unsaved-changes dot. Cause: `broadcastEntityStatus()` serialized `getWorkspaceSyncStatus(wsId)` — an async call — **without awaiting it**, and `JSON.stringify()` turns a Promise into `{}` | `packages/engine/src/companion/companionServer.ts` `broadcastEntityStatus` | keep the broadcast non-blocking (the WebSocket reply must not wait on a git call) but send the *resolved* map via `.then()`, and log a rejected status query instead of swallowing it |
| **Discarding changes on a GraphQL / SOAP / gRPC tab left the editor showing the discarded edits.** `createTabReducer` handled only the save/send actions, so the `REFRESH` the panels dispatch after reloading the entity (`RequestTabContent` → `tabRefs.current[tabId].refresh(entity)`) fell through to each protocol reducer's `default:` branch and returned the state unchanged. REST was unaffected because it implements `REFRESH` itself and passes no `init` | `packages/gui/src/lib/createTabReducer.ts`, `packages/gui/src/components/grpc/grpcTabReducer.ts` | implement `LOAD_ENTITY` / `LOAD_DRAFT` / `REFRESH` in the shared layer, guarded on `options.init` so REST still falls through to its own reducer; `REFRESH` re-derives the entity fields and preserves the runtime/response fields, mirroring `restTabReducer` |
| **Adding, editing or deleting a mock/mapping/rule/request/websocket/webhook did not take effect on the running proxy until something else reloaded the config.** `entityCrudFactory` called `saveConfig(cfg)` then `reloadConfig()` **before** writing the entity file and **before** `syncEnabledSet()`. `reloadConfig()` snapshots the enabled-sets out of `enabled.json`, so `workspaceCfg()` filtered the new entity straight back out of routing — a mock you just added was not served, and a mock you just **deleted kept being served**, until an unrelated action (a settings save, a workspace switch, an app restart) happened to reload. A test that only inspects the handler's return value cannot see this: the config object is correct, the *running server* is stale | `src/ipc/handlers/entityCrudFactory.ts` (`add`, `update`, `delete`) | move `reloadConfig()` to **after** every on-disk write in all three handlers, so the reload sees both the entity file and the updated enabled-set. Proven by mutation: restoring the old ordering makes "stops serving a mock deleted through mock:delete" fail with `expected { from: 'mock' } to deeply equal { upstream: true, … }` |
| **`workspace-zip` export threw on every call.** The file opened with `import archiver from "archiver"` and called `archiver("zip", …)` — the **v5–v7** API. `archiver` 8 is pure ESM with **no default export** (`require("archiver").default` is `undefined`, so the call was `undefined(...)`), and its README's quick start is `new ZipArchive({ zlib: { level: 9 } })`. Nothing caught it: `@types/archiver` was pinned at **7**, so the declarations described the default-export API and `tsc` was satisfied, and no test had ever executed the exporter. Exports of a workspace as a ZIP — the one format that streams, and the only one that can be hundreds of megabytes — simply never worked | `packages/engine/src/importExport/exporters/workspace-zip.ts`; root `package.json` | `import { ZipArchive } from "archiver"` + `new ZipArchive({ zlib: { level: 6 } })`, **and** `@types/archiver` bumped `^7.0.0` → `^8.0.0`. The two halves must move together: fixing only the import leaves the compiler endorsing a call shape the runtime rejects, which is exactly how this stayed hidden |
| **A YAML OpenAPI spec was previewed with a fabricated item count, and a malformed file was offered as importable.** `importers/requests-openapi.ts` had two parsers: `run()` used `loadSpec()`, which picked JSON vs YAML and parsed properly, while `preflight()` tried `JSON.parse` and — on failure — returned `Math.floor(lines containing ":" / 3)` as `itemCount`. So a YAML spec's count was fiction, and *any* text that failed `JSON.parse` came back `ok: true`. `loadSpec()` was `async` (it used `await import("js-yaml")`) and `preflight` is a **synchronous** method on `ImporterFn`, which is why the two had drifted apart | `packages/engine/src/importExport/importers/requests-openapi.ts` | static `import * as yaml from "js-yaml"`, `loadSpec()` made synchronous and **shared by both halves**, and the line-count branch deleted. This is what `ImporterFn`'s contract already said they were — "preflight is a dry run against the same bytes `run` will consume" |
| **Exporting a collection run as JSON silently truncated the report.** `RunReport` in `@bifurc/protocol` was typed from the HTML renderer's field *usage* rather than from the renderer's real `CollectionRunReport`, so `requestId`, `url`, `testLogs`, `preScriptError`, `postScriptError` and `TestResultEntry.durationMs` were absent. `z.object()` **strips unknown keys** — it does not reject them — so routing a real report through the schema dropped those fields without a word. The HTML export was unaffected *because* the HTML renderer touches only fields the schema happened to have, which is precisely why this would have shipped unnoticed: the one consumer that existed could not observe the loss | `packages/protocol/src/commands/runner.ts` `RunReport` / `RunResult` | widened to carry every field of the renderer's `RunnerRequestResult`, and asserted field by field in `tests/integration/fileOps.integration.test.ts` rather than by a round-trip `toEqual` — a round-trip against a schema that strips would compare two equally-truncated objects and pass. **Same class as the `collisionStrategy` defect:** a wire type that disagrees with its only caller, where the compiler is satisfied either way |
| **`tls.importCert` leaked an absolute engine filesystem path in a protocol result.** It read `blobContentPath(blobId)` directly, and that helper validates the id's *shape* (`^blob_[0-9a-f]{32}$`) but never checks that the blob exists. So a blob that had been released, or swept past `BLOB_TTL_MS`, produced a raw `ENOENT: no such file or directory, open 'I:\…\blobs\blob_…\content'` — an engine-side path, in a result the client receives, from a channel whose entire purpose is that no path crosses the boundary (`File_Ops_Protocol.md` §8). It is also not a *typed* error, so P4's transport could not have classified it | `packages/engine/src/fileOps/commands.ts` `tlsImport` | read through `statBlob()` first, which converts absence into a `BlobError("blob-not-found")` and therefore into a path-free `blob-not-found: …` message. `statBlob` deliberately does **not** slide the lease, which is correct here — the bytes are consumed immediately after. **The generalisable lesson:** the item-6 acceptance criterion is checked by grepping for `filePath` in *types*, and this leak was a runtime string, so no grep could have found it |
| **`certutil -delstore` reports success for a certificate it did not remove.** Measured, not assumed: `certutil -delstore -user Root "00112233445566778899AABBCCDDEEFF00112233"` — a thumbprint that is **not in the store** — prints `CertUtil: -delstore command completed successfully.` and exits **0**. An un-trust result built on that exit code would tell the user their CA had been removed from the OS trust store when it had not: a security-relevant lie rather than an ordinary bug, and invisible to any test that asserts on the command's status. The mirror-image query *is* discriminating — `certutil -store -user Root <thumbprint>` exits **0** when present and **17** (`NTE_NOT_FOUND`) when absent — so the two are not interchangeable, and only measurement tells you which is which | `src/ipc/certTrust.ts` (`probeTrust`, `uninstallCA`) | `probeTrust()` split out from `run()` as a separate concept — "what does the store say?" is a different question from "did the command succeed?", and a non-zero exit is the *expected answer* half the time. `TrustResult.verified` is a separate field from `TrustResult.ok`, `uninstallCA()` re-queries after deleting instead of trusting the exit status, and `probeTrust()` returns `true \| false \| null` so Linux can say *cannot answer* rather than `false`. Pinned in both directions, including *"reports ok but not verified when the delete command exits 0 and the store still has it"* |
| **`generateCA()` could return an `ENOENT` carrying the engine's absolute path** — the same class as the `tls.importCert` leak above, reachable with no blob involved. `fs.writeFileSync` does not create parent directories, and `appDataDir()` only exists once `loadSettings()` has written the settings file at least once; a `tls.generate` before that would produce `{ok: false, error: "ENOENT: no such file or directory, open 'C:\\Users\\…\\Bifurc\\ca-cert.pem'"}`. **Latent, not demonstrated** — in the app `loadSettings()` runs at startup, so the directory is there by the time a user can click the button, and no test provoked it | `packages/engine/src/proxy/certManager.ts` `generateCA` | `fs.mkdirSync(dataDir, { recursive: true })` before writing, plus a test that generates into a non-existent nested directory. Creating the directory is cheaper than sanitising the message, and it removes the failure rather than the evidence of it — which is also why it is a one-line fix rather than a new error branch |

### Build-robustness fixes

| Issue | Where | Fix |
|---|---|---|
| The companion-port setting was reachable only through a **bare alias `require()`** — `require("@/companion/companionServer")` inside `config:save`. It resolves in the packaged app only because `tsc-alias` post-processes the build output (a static import at the top of the same file is rewritten to `./companion/companionServer`; the dynamic one to `../../companion/companionServer`). That makes a user-facing path depend on the build pipeline, and it is unresolvable at runtime for any test runner | `src/ipc/handlers/coreHandlers.ts` | promoted to a **static import**. There is no import cycle to avoid — nothing under `src/companion` imports `src/ipc`. This is what makes the companion-port test in §3.4 possible at all |
| **A fresh clone could not type-check.** `dist/` is gitignored repo-wide (for every package, not just the app), but CI ran only `npm ci` before `npm run typecheck`, so no package had build output. `tsc` then failed with 10× `TS2307: Cannot find module '@bifurc/protocol'`. Latent since the protocol package landed in P1 — it went unnoticed because a developer machine always has a stale `dist/` lying around | root `package.json` | added `build:packages`, a `prepare` hook that runs it (`npm ci` executes `prepare`, so CI builds the packages automatically), and made `typecheck` run `build:packages` first. The last part matters beyond CI: without it, `typecheck` would pass locally against a **stale** `dist/` |
| **A new assertion block in an integration suite referenced a symbol that was never imported, and nothing caught it.** The P4 item 3 block added to `tests/integration/engineSmoke.integration.test.ts` called `createInProcessTransport(...)` while that file's import line still named only `createEngine`. `tsc` never sees it: only `packages/engine/tests/**` is in a tsconfig `include`, and `tests/integration/**` is in neither. So it was a runtime `ReferenceError: createInProcessTransport is not defined` at line 164 — which reads as a broken **engine** rather than a broken **test**, and it surfaced only when the whole suite was finally run | `tests/integration/engineSmoke.integration.test.ts` | added the symbol to the existing `import { … } from "@bifurc/engine"` line. **The generalisable lesson:** the P4 conformance suite deliberately imports its siblings **relatively** (`../../src/…`) and *is* typechecked — `packages/engine/tsconfig.json` was extended to include `tests/**/*.ts` precisely because that gap once let a missed `filePath` call site through. The repo's own `tests/**` tree still has no such coverage, so a clean `tsc` run is not evidence about `tests/integration/**`, and a missing import there can only be found by executing the file |

---

## 8. Verifying this review locally

> **✅ Environment state, 2026-09-17 — the outage below is RESOLVED; kept because the diagnostic path
> is worth having.** Between roughly 12:30 and 16:00 the suite was **unrunnable**: every file failed at
> *collection* with `TypeError: Cannot read properties of undefined (reading 'config')` at its first
> `describe(...)`, and the JSON reporter's per-file message named the real cause — **"Vitest failed to
> find the runner"**. It was **not** caused by the code under review: an untouched file reproduced it
> (`npx vitest run tests/proxy/decompressUtils.test.ts`). Ruled out by direct test — the sandbox
> (disabled), the node version (pinned), vitest/vite versions (4.1.5 / 8.0.13), missing worker entries,
> the safe-delete guard, the `NODE_OPTIONS` preload, `--maxWorkers=1`, `--no-file-parallelism`,
> `--no-isolate`, `--pool=threads`, `--pool=forks`, a cold `node_modules/.vite`, and `TEMP=/tmp`.
> `vitest.config.ts` and `tests/setup.ts` were unmodified, and no fix was ever identified — **it
> cleared on its own.** The full suite then ran at **1953 tests, 1934 passed, 1 failed, 18 pending**
> (the one failure being the `soap.execute` dead-endpoint flaky-family member from §4.1). So: if a run
> fails this way again, it is the environment, not your change — **retry in a fresh shell before
> debugging.**
>
> **`--pool=vmThreads` is still not a workaround.** It runs, but it does not apply `vi.mock`: under it
> `tests/ipc/clientHandlers.test.ts` fails 25/25 with `Cannot read properties of undefined (reading
> 'handle')` (the file-local `electron` mock never applies) and `tests/proxy/certManager.test.ts` runs
> the **real** `mkcert`, producing a live `O=Bifurc CA` where the fixture is expected. Suites that mock
> nothing pass under it, so a green vmThreads run is not evidence of anything — and because it *looks*
> green on the files that do not mock, it is worse than an honest failure.


```bash
# The repository root IS the app — there is no `cd bifurc` any more (it was flattened 2026-09-14).

# 1. Everything green, thresholds enforced.
#    The deselect is not optional: a *failing* test aborts the whole coverage report in this
#    sandbox (reportCoverage -> onTestFailure -> cleanAfterRun trips the bulk-delete guard), so
#    no coverage-summary.json is written at all. Every member of the flaky family must be excluded
#    — see §4.1 for the roster and why it varies.
#
#    Match on the *precise* names, not on a shared word. A pattern of `unreachable` looks tidier
#    but also filters out gitRemote's "reports an error for an unreachable remote", and that test
#    is the one that assigns the `rootC` fixture its neighbour reads (gitRemote.integration.test.ts
#    :421 and :433) — so the next test dies with `Cannot read properties of undefined (reading
#    'activate')`. That is a test-ordering dependency in the file, not a product bug, but it makes
#    a loose deselect pattern actively harmful.
npx vitest run --coverage --coverage.clean=false --coverage.reportsDirectory=coverage-local \
  --testNamePattern='^(?!.*(WSDL cannot be fetched|rejects when the (upstream|endpoint) is unreachable))'
#    → Test Files 73 passed (73) · Tests 1708 passed | 6 skipped (1714)
#    → Statements 49.07% · Branches 32.66% · Functions 37.26% · Lines 51.52%
#    Undeselected, two of the family trip: expect "2 failed | 1712 passed (1714)". Both reproduce
#    on the pre-change tree — see plan/baseline.md "Environment caveats".
#
#    NOTE (2026-09-21): the numbers above are from the 2026-09-17 run and the suite has grown since
#    (now 2552 cases / 104 files). They are kept as an illustration of the *shape* of the output
#    rather than as a baseline to compare against — see plan/baseline.md for the current figure.
#    The `never throws through the envelope` clause was removed from the pattern when the P0 spike
#    was retired; those two cases no longer exist. The clause was harmless (a lookahead matching
#    nothing), but a pattern naming a deleted test is how a deselect slowly stops matching what it
#    was written for.

# 2. Per project
npm run test:unit
#    → Test Files 88 · Tests 2241 passed | 40 skipped (2281)
#      (the 40 skipped are platform-conditional — POSIX-only socket cases and the like)
npm run test:integration
#    → Test Files 30 · Tests 464 passed (464)
#      (Both figures are the **full run's own per-project subtotals**, summed from its per-file lines
#       rather than from two separate single-project runs — re-measured 2026-09-25 after `plan/17`
#       WI 1-3 and 9, where every port became engine-level, so `WorkspaceRuntime` and the
#       per-workspace allocation suite are gone. `2281 + 464 = 2745` · `88 + 30 = 118`, which is the
#       full run's own total. A single-project run is **not** the same measurement — see the note
#       this block used to carry.)
#
#    NOTE (2026-09-24): the unit project has **no standing failure** — a failure here is always a
#    flake to isolate, never a known one. The one this block's figures used to invite is the
#    load-induced `gitStore` / `gitOps` pair from the sandbox's safe-delete shim: it fails only
#    under a parallel run (`EBUSY … rmdir`, or a `STACK_TRACE_ERROR` in `beforeEach`), passes in
#    isolation, and `git diff HEAD --numstat` shows no rows for it. **Run it alone before changing
#    anything** — `--reporter=json --outputFile` is how you name the flake rather than assume it.
#    See plan/baseline.md for the full-suite figure.

# 3. Confirm the engine really is tested from source, not from a stale build.
#    This is the check whose absence hid 14 files at 0% coverage until 2026-09-15 (§4.8).
#    Append a marker to packages/engine/src/store/paths.ts, then:
npx vitest run --coverage --coverage.clean=false --coverage.reportsDirectory=coverage-local
#    → the coverage table must list packages/engine/src/store/paths.ts. If the engine files are
#      absent or at 0%, the alias in vitest.config.ts has been lost — and a green suite is then
#      meaningless, because tests are running against dist/ instead of your edits.

# 4. Confirm the new tests have teeth (mutation checks)
#    a) Edit packages/engine/src/proxy/proxyHandler.ts so matchProxyRule never matches, then:
npm run test:integration
#    → 6 rule tests fail. Revert the edit.
#    b) In packages/engine/src/proxy/serverReplay.ts, replace decompressBody(raw, ce) with raw, then:
npx vitest run --project integration tests/integration/protocolExecution.integration.test.ts
#    → "decodes a gzip response and strips content-encoding" fails. Revert.
#    c) In src/ipc/handlers/soapHandlers.ts, use body.length instead of xmlBody.length:
#    → "uses the UTF-8 byte length for content-length" fails. Revert.
#    d) In src/ipc/handlers/runnerHandlers.ts, re-escape one ${…} in generateRunnerHtml, then:
npx vitest run --project integration tests/integration/runnerStorage.integration.test.ts
#    → the report-content assertions fail. Revert.
#    e) In packages/engine/src/sync/publishService.ts, drop the Set() dedupe, then:
npx vitest run --project integration tests/integration/gitSync.integration.test.ts
#    → "treats a folder holding exactly one entity as a single-entity commit" fails. Revert.
#    f) In packages/engine/src/store/gitStore.ts, remove the workspace.json add, then:
npx vitest run --project integration tests/integration/gitRemote.integration.test.ts
#    → "clones the remote and adopts its workspace identity" fails. Revert.
#    g) In packages/engine/src/proxy/server.ts, change the 512 * 1024 capture slice to 256 * 1024, then:
npx vitest run --project integration tests/integration/capture.integration.test.ts
#    → "caps an oversized captured response body" fails. Revert.
#    h) In packages/engine/src/companion/companionServer.ts, drop the `await` on getWorkspaceSyncStatus (or
#       serialize the promise again), then:
npx vitest run --project integration tests/integration/companionServer.integration.test.ts
#    → "tells the renderer to refresh, and reports the new entity as dirty" fails. Revert.
#    i) In packages/gui/src/lib/createTabReducer.ts, change `if (!options.init)` to `if (options.init)`
#       so the shared layer always delegates, then:
npx vitest run --project unit tests/renderer/protocolTabReducers.test.ts
#    → 7 REFRESH/LOAD tests fail. Revert.
#    j) In packages/engine/src/proxy/tlsCert.ts, make generateHostCert ignore the loaded CA (issue from a
#       throwaway key), then:
npx vitest run --project integration tests/integration/tlsIntercept.integration.test.ts
#    → the `rejectUnauthorized: true` handshake fails. Revert.
#    k) In src/ipc/handlers/entityCrudFactory.ts, move `reloadConfig()` back to just after
#       `saveConfig(cfg)` in the DELETE handler (i.e. before the file unlink and the enabled-set
#       write), then:
npx vitest run --project integration tests/integration/settingsMutations.integration.test.ts
#    → "stops serving a mock deleted through mock:delete" fails with
#      `expected { from: 'mock' } to deeply equal { upstream: true, path: '/api/hello' }`. Revert.
#    l) In src/ipc/handlers/entityCrudFactory.ts, do the same in the ADD handler, then the same
#       command fails on "serves a mock added through mock:add immediately". Revert.
#    m) P3 blob store. In packages/engine/src/blob/store.ts, disable the `bytes.length !== params.size`
#       check in putBlob, then:
npx vitest run tests/blob
#    → 3 tests fail: "rejects a decoded length that disagrees with the declared size" and
#      "rejects a payload of unrecognised characters, which the lenient decoder would silently
#      shorten" (store.test.ts), plus "propagates a store error rather than swallowing it"
#      (commands.test.ts). That is the whole point of the check — Node's base64 decoder ignores
#      characters it does not recognise, so without it a truncated payload stages as a smaller but
#      valid-looking blob. Revert.
#    n) In packages/engine/src/blob/store.ts, disable the content-existence check in readMeta, then:
npx vitest run tests/blob
#    → exactly 1 test fails: "treats a blob directory with metadata but no content as not found".
#      Without the check, statBlob reports a size and a SHA-256 for bytes that are not on disk.
#      Revert.
#    o) In packages/engine/src/blob/sweep.ts, change `meta.createdAt + ttlMs <= now` to `< now`, then:
npx vitest run tests/blob
#    → 4 tests fail, led by "reclaims a blob whose lease has run out, and reports what it freed":
#      the boundary is inclusive by contract (`createdAt + TTL` is expired, not "expires in 1 ms").
#      The other three sweep exactly at the boundary. Revert.
#    p) In packages/engine/src/blob/sweep.ts, disable the `entry.name === BLOB_STAGING_DIR_NAME`
#       branch so `.staging/` falls through to the blob-directory path, then:
npx vitest run tests/blob
#    → 2 tests fail: "reclaims an abandoned staged file once it is old" and "does not resurrect a
#      blob that was already released". The mechanism is worth stating because the obvious guess is
#      wrong: the branch is not what protects an in-flight write (that test still passes), it is
#      what makes a staged file age by its **own** mtime rather than by its parent directory's.
#      Without it, a staged file whose directory mtime is fresh is never reclaimed — a slow leak,
#      which is exactly what the sweep exists to prevent. Revert.
#    q) In packages/engine/src/index.ts, remove the `stopBlobSweeper?.()` line from stop(), then:
npx vitest run --project integration tests/integration/engineSmoke.integration.test.ts
#    → **still passes.** Verified, and deliberately recorded as an unpinned line: the sweeper's
#      timer is `unref()`d, so a missed teardown cannot hold the process open — the unref, not the
#      stop() call, is the safety property. `startBlobSweeper`'s own stop is pinned in
#      sweep.test.ts ("runs on the interval and stops when told"); what is not pinned is
#      createEngine *calling* it. Keep the line — an engine that has stopped should stop sweeping —
#      but do not mistake it for the thing that makes teardown safe. Revert.

# 5. Static checks
npm run typecheck          # builds both packages first, then type-checks all three projects
npm run typecheck:e2e
#    (npm run typecheck:renderer currently reports 152 pre-existing errors — see §7)
```

> In a sandbox that blocks Vitest's bulk delete of its report directory, add
> `--coverage.clean=false --coverage.reportsDirectory=coverage-local` (as above). The
> coverage numbers are still written; only the post-run cleanup is skipped.
>
> **Never use `npx tsc`** — it resolves the placeholder npm package, prints a warning, and
> **exits 0**, so it silently reports success. Use `npm run typecheck`. The same trap applies to
> piping `vitest` through `grep`: the exit status you get back is `grep`'s, not Vitest's.
