# Bifurc — Testing Guide & Coverage Report

_Last reviewed: 2026-09-15 · suite: **1616 tests / 69 files** (1 known failure — see §7)_

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
more production bugs doing it. A seventh pass closed **settings mutations**: the last
behaviour that changed live state with nothing behind it (and it found a third bug — see §7).

| Suite | Tests | Covers |
|---|---:|---|
| `tests/integration/applications.integration.test.ts` | 12 | The **Applications** feature end to end: save (with pre-computed command) → list → run a **real child process** → stream stdout/stderr → exit code → stop a long-running process → debug mode with its port → delete. Drives the real `applications:*` IPC handlers. |
| `tests/integration/auditLog.integration.test.ts` | 13 | The **Audit Log** screen: real git commits → `queryLog()` newest-first → filters by entity/action/id/search/file-path → pagination → changed-files resolution → point-in-time entity reads (the diff view). |
| `tests/integration/importExport.integration.test.ts` | 18 | Export→import round trip for the native Bifurc-JSON formats + collision strategies (`keep`/`override`/`new`) + preflight failure paths. |
| `tests/integration/importExportFormats.integration.test.ts` | 17 | Round trips for the foreign formats (Postman, WireMock, HAR, Insomnia, OpenAPI) and the **whole-workspace snapshot**. |
| `tests/integration/upstreamFetch.integration.test.ts` | 12 | `fetchUpstreamResponse()` against a real upstream: status/headers/body, gzip decoding, target-path resolution, request/response scripts, hop-by-hop stripping. |
| `tests/proxy/decompressUtils.test.ts` | 19 | Response decompression (gzip/deflate/br/zstd/stacked) — on the path of every proxied response. |
| `tests/proxy/responseUtils.test.ts` | 16 | Header framing: hop-by-hop stripping and `set-cookie` handling (must not be comma-joined). |
| `tests/proxy/scriptContext.test.ts` | 33 | The chai-like `expect()` that backs user-written `lp.test(...)` scripts — every matcher in passing, failing and negated form. |
| `tests/proxy/scriptExecutor.test.ts` | 33 | Proxy-rule scripts + request pre/post/test scripts, including sandbox isolation (`require`/`process` unreachable). |
| `tests/applications/commandGenerator.test.ts` | 54 | Every run-config type (node/npm/python/java/maven/gradle/dotnet/go/docker/compose/spring-boot) plus its debug variant. |
| `tests/applications/portUtils.test.ts` | 7 | Real port occupancy checks. |
| `tests/ipc/importExport/registry.test.ts` | 12 | Import/Export registry invariants — most importantly that no format claims support without an implementation. |
| `tests/companion/allowedActions.test.ts` | 7 | The companion extension's IPC allowlist (a security boundary) — pinned exactly. |
| `tests/subscription/entityCount.test.ts` | 8 | The licensing gate stubs, pinned so adding a real limit is a deliberate change. |
| `tests/integration/protocolExecution.integration.test.ts` | 44 | **The "hit Send" path** — `request:replay` (REST), `graphql:execute` + `graphql:introspect`, `soap:execute` + `soap:fetchWsdl`, and `healthbar:checkUrl`, all against real local servers. Also pins the (unimplemented) gRPC contract and the protocol-entity CRUD round trips. |
| `tests/integration/gitSync.integration.test.ts` | 29 | **Git publish, locally** — `entity:publish` / `git:sync` / `folder:publish` against a real repo: the commit *subject format* the Audit Log parses (`create mock Name`), the bundled folder form, explicit messages, the no-op case, `git:diff` for clean/modified/new/deleted, `git:discard` + `entity:restore` (incl. the `names.json` re-sync), `git:history` / `history:list` / `history:diff`, and `sync:getEntityStatus` dirty markers. |
| `tests/integration/gitRemote.integration.test.ts` | 24 | **Git sync, remotely** — `sync:setRemote` in all three of its branches (empty→empty push, empty→non-empty clone, refusal when both sides hold data), against a real **bare repo** as the remote. Plus `sync:push`, `sync:pull` (fast-forward + `updatedIds`), `sync:disconnect`, `sync:setAutoSync`, `getRemoteHead`, and the workspace-identity adoption on clone. |
| `tests/integration/foldersAndWorkspaces.integration.test.ts` | 25 | **Folder management and workspace CRUD** — `folder:add/rename/move/delete` and `workspace:add/rename/setActive/delete`, asserting the on-disk result (`index.json`, real directories, surviving entities) rather than in-memory config. |
| `tests/renderer/collectionRunner.test.ts` | 30 | **The Run button's engine** — `runCollection` against the *real* `executeIpcScript` sandbox (only `replayRequest` stubbed): per-request orchestration, `{{var}}` resolution, start/done callbacks, cancellation, error isolation, pre/post/test scripts, and `generateHtmlReport`. |
| `tests/integration/runnerStorage.integration.test.ts` | 17 | **Runner persistence** — `runner:saveReport/getHistory/saveConfig/loadConfig/listFolderIds/exportReport` against a real workspace, asserting the files that land in `requests/.runs/`. |
| `tests/integration/capture.integration.test.ts` | 20 | **The capture half of capture-and-replay** — real traffic through the real proxy, asserting the emitted `RequestLogEntry`: method/url/host/status, all five `via` values (`rfc6761`/`mock`/`rule`/`proxy`/`error`), target, duration, header capture with hop-by-hop stripped, base64 request/response bodies (incl. multi-byte UTF-8), the **512 KB response-body cap**, "no capture for the home page", and `capture:shareJson` write/cancel. |
| `tests/renderer/captureUtils.test.ts` | 26 | **Capture → mock / saved-request conversion** — `buildMockInitial` (status defaulting, binary-vs-text body encoding, `{}` fallback, raw capture preserved for byte-exact replay), `reqToHeadersBody` (decodes the body, strips `host`/`connection`/`content-length`/…), `deriveType` (content-type first, extension fallback, `other`), plus the table formatters and base64 plumbing. |
| `tests/integration/tlsIntercept.integration.test.ts` | 11 | **TLS interception** — generates a **real CA** with mkcert (`createCA`, not mocked), boots the real proxy with TLS on, and speaks real TLS through a real `CONNECT` tunnel. The decisive assertion is a handshake with `rejectUnauthorized: true` against the generated CA: only a leaf genuinely signed by our CA *and* valid for the requested hostname can pass. Also covers the negative control (a cert for host A must not validate for host B), the cert cache, routing over the decrypted stream (mock + proxy rule), passthrough to a real HTTPS upstream, capture of the decrypted request, the blind-tunnel fallback when `tlsEnabled` is false, and the 502 path when the CA is unusable. |
| `tests/integration/companionServer.integration.test.ts` | 20 | **The companion WebSocket server** — the browser extension's only write path into the workspace. Real `ws` clients against the real server: lifecycle (start/restart/stop, 1001 on close), `config:get`/`mock:add`/`request:add`/`folder:add` asserted against the **files on disk** (`enabled.json`, `names.json`, `index.json`, the sanitised folder directory), the validation errors, malformed JSON, missing id/action, and — most importantly — that **every action off the allowlist is refused and writes nothing**. |
| `tests/renderer/protocolTabReducers.test.ts` | 45 | **The GraphQL / SOAP / gRPC tab state machines** — init from entity and from draft, the send lifecycle, `SET_FIELD`/dirty, the save lifecycle, and the serialization helpers (`stateToMockPayload`'s `operationNameMatch` round trip and `responseDelay || undefined`, SOAP's default-envelope fallback and always-enabled mock payload, gRPC's `LOAD` merge and `protoFileId ?? ""`). Includes the regression guard for the `REFRESH` bug (see §7). |
| `tests/integration/settingsMutations.integration.test.ts` | 30 | **The Settings screen, measured on the running app** — changing the proxy port moves the live server (the old port stops answering, the new one serves the same mock), the new port is persisted and reported by `server:status`, an unchanged port does not take the server down, `server:restart`/`stop`/`start`, toggling TLS reloads the CA, changing the companion port moves the real WebSocket server, theme and zoom persist and clamp, and — the part that caught a bug — a **live config mutation (`mock:add`/`update`/`delete`, workspace switch) reaches the running proxy with no restart**. Registers the real `config:save`/`server:*`/`theme:*`/`zoom:*` handlers; only Electron and `@/main` are mocked. |

---

## 4. Coverage report

### 4.1 Headline numbers

| Metric | Before (same scope¹) | **Now** | Change |
|---|---:|---:|---:|
| Statements | 24.16% | **48.00%** | +23.8 pts |
| Branches | 14.87% | **31.99%** | +17.1 pts |
| Functions | 17.11% | **36.30%** | +19.2 pts |
| Lines | 25.49% | **50.39%** | +24.9 pts |

¹ Both columns use the *widened* `include` scope (which adds `renderer/panels/**/*.tsx`, ~0%
covered), so the comparison is apples-to-apples. The original narrow-scope baseline was
27.71% statements; the widened scope *lowered* the headline to 24.16% at the time, and the
current 48.00% is a real gain on top of that larger denominator.

² These are the figures from the last full run. V8 instrumentation is not perfectly
deterministic — two identical runs move a given figure by ~0.05 pt (the seventh pass read
46.14% statements where the sixth read 45.96%). Treat them as "the last run", not a
constant, and re-measure rather than copy when you change the suite.

³ **Scope changed on 2026-09-15** when the engine extraction moved the engine out of `src/` in
three layers: `src/{store,lib,subscription}`, then `src/{proxy,sync}` + `src/eventBus.ts`, then
`src/{applications,companion,commands}`. Those files are the *same code under the same tests*, and
their per-file coverage is provably unchanged (see §4.8). Two new groups entered the report:

| Group | Files | Statements | Note |
|---|---:|---:|---|
| `packages/engine/src` | 49 | 2,914 | The moved files + a new re-export barrel (`index.ts`, 0%). Was 14 files / 606 statements after layer 1, 41 / 2,433 after layer 2. |
| `packages/protocol/src` | 16 | 106 | **Sourcemap artefact** — `packages/protocol/dist/index.js` is executed through its `exports` map, and V8 remaps it back through `dist/index.js.map` (whose `sources` list `../src/commands/*.ts`, i.e. `packages/protocol/src/commands/*.ts`). Numbers are therefore approximate and ~100%. Present since P2 work item 7 wired the registry to `@bifurc/protocol`. |

The denominator went 11,129 → 11,443 (layer 1) → 11,446 (layer 3) while the percentage kept
rising, so this is a genuine improvement rather than a scope artefact. The layer-1 jump was
`packages/protocol/src` entering the report; layer 2 changed nothing; layer 3's +3 statements are
`applications/types.ts`, which had been wrongly excluded as "type-only" (§5).

Suite size: **69 files / 1616 tests** (was 61 files / 1360 tests at the last review; the growth
is P2 work items 5–7). One test fails — `tests/spike/protocolPoc.test.ts` → `soap.execute`,
a known pre-existing failure that also reproduces on the pre-change tree (see §7).

### 4.2 Coverage by area (current config)

| Area | Statements | Branches | Functions | Lines |
|---|---:|---:|---:|---:|
| `packages/engine/src` | **83.9%** | **71.1%** | **85.3%** | **86.4%** |
| `packages/protocol/src` ³ | 97.2% | — | 25.0% | 97.2% |
| `renderer/components` | **13.5%** | 13.3% | 8.7% | **14.6%** |
| `renderer/lib` | **82.9%** | **74.8%** | **75.0%** | **84.1%** |
| `renderer/panels` | 0.0% | 0.0% | 0.0% | 0.0% |
| `src/` (root files) ⁴ | **98.4%** | **85.0%** | 100.0% | **98.2%** |
| `src/ipc` | **79.1%** | 55.8% | 79.7% | 82.1% |
| **TOTAL** | **48.00%** | **31.99%** | **36.30%** | **50.39%** |

³ `packages/protocol/src` has no branch data because V8 records none for those remapped Zod
schemas. Its numbers come from the sourcemap-remapped `dist/` bundle (see §4.1 footnote 3), so
treat them as indicative. Its 25.0% functions is a single arrow function in
`packages/protocol/src/commands/index.ts`, not a broad gap.

⁴ `src/shutdown.ts` (100%), `src/startup.ts` (98.2%). `src/eventBus.ts` (100%) moved into the
package in layer 2 and is now counted under `packages/engine/src`. `src/main.ts` and
`src/preload.ts` are excluded — they are process entry points, exercised by launching the app.

**Rows removed on 2026-09-15.** `src/lib`, `src/store`, `src/subscription` (layer 1);
`src/proxy`, `src/sync` (layer 2); `src/applications`, `src/companion`, `src/commands` (layer 3) no
longer exist in `src/`. They all live under `packages/engine/src/` now, reported as the single
`packages/engine/src` row above. Their pre-move figures were `src/store` 82.8/68.8/84.1/87.6,
`src/subscription` 100/100/100/100, `src/lib` 30.0/2.8/22.2/31.8, `src/proxy` 87.2/77.9/89.7/88.9,
`src/sync` 77.8/56.7/80.3/80.6, `src/applications` 87.5/77.0/81.3/91.6, `src/companion`
95.3/83.3/76.5/95.1 and `src/commands` 100/100/100/100 — so the combined row reading
83.9/71.1/85.3/86.4 is a *weighted average of unchanged numbers*, diluted only by the new
0%-covered `index.ts` barrel (§4.8). Nothing regressed: §4.8 verifies every moved file individually.

`src/` now contains only `ipc/`, `main.ts` and `preload.ts` — the Electron registration layer that
P2 replaces, plus the two process entry points. `startup.ts` and `shutdown.ts` are the last engine
files still to move.

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
| `renderer/components/capture/captureUtils.ts` | not measured | **100.0%** |
| `renderer/components` (area) | 1.8% | **13.5%** |
| `packages/engine/src/proxy/server.ts` (the emit sites) | 75.5% | **90.3%** |

> `captureUtils.ts` was not merely uncovered, it was **not measured** — see the scope
> correction in §4.2. It now sits at 100% statements / 96.5% branches.

### 4.7 TLS interception and the companion server specifically

The two highest-risk files still at 0% after the fifth pass. `tlsIntercept.ts` is the code
that terminates TLS for every `https://` request a user routes through Bifurc; the companion
server is the browser extension's only write path into the workspace.

| File | Statements before | Statements after |
|---|---:|---:|
| `packages/engine/src/proxy/tlsIntercept.ts` | 0.0% | **88.5%** (100% of lines) |
| `packages/engine/src/proxy/tlsCert.ts` | 71.0% | **90.3%** |
| `packages/engine/src/companion/companionServer.ts` | 0.0% | **94.0%** |
| `renderer/lib/createTabReducer.ts` | 100.0% | **100.0%** |
| `renderer/components/graphql/graphqlTabReducer.ts` | 0.0% | **92.9%** |
| `renderer/components/grpc/grpcTabReducer.ts` | 0.0% | **88.9%** |
| `renderer/components/soap/soapTabReducer.ts` | 0.0% | **83.1%** |
| **`src/companion` area** | **0.8%** | **94.1%** |

> The TLS suite's key property: it does **not** mock `mkcert`. It generates a real CA and
> performs the client handshake with `rejectUnauthorized: true`, so the pass/fail decision is
> made by OpenSSL's chain and hostname validation rather than by an assertion we wrote. A
> regression in certificate generation cannot slip through it.
>
> The remaining 11.5% of `tlsIntercept.ts` is the post-handshake body/header parsing branch
> and the `tlsSocket` error handler, which the integration path exercises only indirectly.

### 4.8 The engine extraction specifically

P2 work item 8 moved the engine out of the app's `src/` into `packages/engine/src/`, in three
layers: `src/{store,lib,subscription}`, then `src/{proxy,sync}` + `src/eventBus.ts`, then
`src/{applications,companion,commands}`. That is a pure move: no logic changed, so **each moved file
must report exactly the coverage it reported before the move** — the same denominator (proof the code
is untouched) and the same numerator (proof the same tests still reach it). Anything else means the
move was not behaviour-preserving.

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

The cleanest layer yet: **7 files moved, 7 of 7 identical on all four metrics including raw
covered/total counts, 0 regressions, 0 missing, 0 stale `src/` entries.** 48 statements rewritten
across 26 files (45 imports + 3 `vi.mock`). `ws` joined the engine as its third runtime dependency.

The layer-3 rewrite also produced the one **deliberate** scope change in this document:
`coverage.exclude` listed `src/applications/types.ts` under "Type-only modules", but that file
exports `DEFAULT_DEBUG_PORTS`, `RUN_CONFIG_TYPE_LABELS` and `RUN_CONFIG_TYPE_ICONS` — real runtime
values. The exclusion was made on the strength of the filename and had been hiding live code. Rather
than carry a wrong exclusion to the new path, the entry was **removed**; the file now reports 3/3
lines at 100%, which is why the denominator grew by exactly 3 statements (11,443 → 11,446) and the
numerator by exactly 3.

**Layer 3's structural note:** `companion/allowedActions.ts` is the frozen API surface for the
external `bifurc-extension` client. It moved with everything else, but its *contents* must not
change — the extension is a released client that cannot be updated atomically with the engine.

**The one 0% file is new, and it is a barrel.** `packages/engine/src/index.ts` is the package's
public entry point, re-exporting each module as a namespace. Nothing imports it yet — tests and
production both use deep specifiers (`@bifurc/engine/store/config`) — so it contributes 0% across
~14 statements. It is deliberately *not* excluded: unlike `renderer/components/ui/index.ts`
(pure re-exports of presentational components), this barrel becomes the real engine API when
`createEngine()` lands, and it should be covered then rather than hidden now.

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

A full HTML report is written to `coverage/index.html` by `npm run test:coverage`.

---

## 5. Workflow changes

### `vitest.config.ts`
- Split into two **projects**: `unit` (fast, no sockets) and `integration` (real servers,
  `fileParallelism: false` so port-binding output stays readable).
- **Coverage thresholds** added as a ratchet, then raised after each pass
  (`statements 46 / branches 31 / functions 34 / lines 48`; originally `23 / 14 / 16 / 24`).
  New untested code now fails CI instead of silently lowering coverage. Raise these as gaps
  close — never lower them to make a build pass.
- Reporters expanded to `text`, `text-summary`, `html`, `json-summary`, `lcov`.
- `include` widened to `renderer/panels/**`, then to `renderer/components/**/*.ts` (the glob
  had matched only `.tsx`, hiding 17 files — see §4.2). Type-only and pure-constant modules
  excluded.
- **`dualAliasPlugin` no longer resolves to a directory.** The `""` extension matched
  `renderer/lib/strings/` before the `index.ts` fallback could run, so Vite was handed a
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
- `e2e/tsconfig.json` fixed (`rootDir` override + `renderer/types/window.ts` included) —
  E2E type-checking now actually works and is part of CI.
- `e2e/helpers/index.ts` rewritten. It now **asserts** (`openPanel` fails if the panel is
  missing) instead of silently skipping. Added `uniqueName`, `pause`, `chooseProtocol`,
  `fillVisibleCodeEditor`, `expectPanelAnchor`.
- `app.spec.ts`, `mappings.spec.ts`, `proxy-rules.spec.ts`, `mocks-rest.spec.ts`,
  `requests-rest.spec.ts` rewritten with real create/edit/validate assertions and
  deterministic cleanup through `window.api`.
- `app.spec.ts` had a genuine bug: `const window = await electronApp.firstWindow()` shadowed
  the DOM `window`, so `window.evaluate(() => window.innerWidth)` threw at runtime. Fixed.

### Renderer type fixes (prerequisites for `typecheck:e2e` passing)
- `renderer/types/window.ts` and `renderer/types/ipc.ts` imported `Workspace` from
  `./entities`, where it does not exist. `Workspace` lives in `./config`. Fixed.

---

## 6. Backlog — what is still untested

Ordered by risk. Each item names the file(s) and what a test would need to do.

> **Closed in the second through seventh passes (2026-09-14):** these items are now done —
> `decompressUtils` (100%), `responseUtils` (87%), `scriptContext` (96%) /
> `scriptExecutor` (89%), the whole import/export tree (`importExport.integration`,
> `importExportFormats.integration`, `registry.test`), `packages/engine/src/applications/**` (87%),
> `packages/engine/src/subscription/entityCount.ts` (100%), `packages/engine/src/companion/allowedActions.ts` (100%), the
> Audit Log screen (`auditLog.integration`), **the "hit Send" path**
> (`protocolExecution.integration` — REST replay, GraphQL execute/introspect, SOAP
> execute/fetchWsdl, Health Bar polling), **folder management** and **workspace CRUD**
> (`foldersAndWorkspaces.integration`), **the collection runner and its reporting**
> (`collectionRunner.test`, `runnerStorage.integration` — `renderer/lib/collectionRunner.ts`
> 96%, `runnerReport.ts` 100%), **git publish and git sync**
> (`gitSync.integration`, `gitRemote.integration` — `publishService` 85%, `syncHandlers`
> 75%, `syncManager` 80%), **capture** (`capture.integration`, `captureUtils.test` —
> `captureUtils.ts` 100%, all five `via` paths, the 512 KB cap, the export handler),
> **TLS interception** (`tlsIntercept.integration` — real CA, real handshake,
> `tlsIntercept.ts` 0% → 88%), **the companion server** (`companionServer.integration` —
> `companionServer.ts` 0% → 94%, and the allowlist proven to be enforced, not just listed),
> **the protocol tab reducers** (`protocolTabReducers.test` — `graphql` 93%, `grpc` 89%,
> `soap` 83%), and **settings mutations** (`settingsMutations.integration` — the port change
> moves the live server, TLS toggling reloads the CA, the companion port moves the socket,
> theme/zoom persist, and a live CRUD mutation reaches routing with no restart).
> Twelve production bugs and two harness/reporting bugs were fixed along the way. What
> remains is listed here.

### High — security / correctness
1. **`packages/engine/src/companion/companionServer.ts` has no authentication.** It binds to `127.0.0.1`,
   which is the whole of its access control. `ALLOWED_ACTIONS` is now proven to be *enforced*
   (a forbidden action is refused and writes nothing), but any local process can still open a
   socket and call the four allowed actions. Worth a deliberate decision, not a test.

### Medium — features users depend on
2. **gRPC is not implemented** — `grpc:execute` and `grpc:reflect` return "runtime not yet
   configured" and the mock server is a stub. The handlers are now covered *as stubs*, so
   this is a product gap rather than a testing gap: there is nothing to exercise until the
   runtime is wired up.
3. **`renderer/components/rest/{useCollectionRunner,useRestActions}.ts` (0%)** — React hooks.
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
9. **`renderer/panels/**` (0%)** — the panels are covered only by the Playwright E2E suite,
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

---

## 7. Known issues found during this review

| Issue | Evidence | Status |
|---|---|---|
| Renderer is not type-checked | `tsconfig.json` `include: ["src/**/*"]` | **Open** — `npm run typecheck:renderer` reports 152 errors, none of them in the files this review touched |
| `npx tsc` is **not** the TypeScript compiler here | `npx tsc` resolves the placeholder npm package, prints "This is not the tsc command you are looking for" and **still exits 0** — so `npx tsc ... \| wc -l` cheerfully reports "0 errors" | **Documented** — always use `npm run typecheck` / `npm run typecheck:renderer`, never `npx tsc` |
| A **blank GraphQL mock is never reported as an empty draft** | `isDraftEmpty(state, "mock")` tests `!state.responseBody`, but the default body is the non-empty `{ data: {} }` envelope; SOAP avoids this by comparing against its default envelope | **Open (cosmetic)** — `useDraftPersist` uses the predicate only to skip its debounced save, and it saves on mount and unmount regardless |
| E2E `tsconfig.json` never worked | inherited `rootDir: ./src` → `TS6059` for every spec | **Fixed** |
| `app.spec.ts` shadowed `window` | `const window = await electronApp.firstWindow()` | **Fixed** |
| `Workspace` imported from wrong module | `renderer/types/{window,ipc}.ts` | **Fixed** |
| `expect(body).toBeTruthy()` used as an assertion | 8 E2E spec files | **Partially fixed** — 5 files converted, 5 remain (item 9) |
| E2E could not launch on CI | no Electron system libs installed | **Fixed** |
| Coverage had no gate | no `thresholds` in `vitest.config.ts` | **Fixed** — ratchet added |
| Coverage **under-reported** the renderer | `include` listed only `renderer/components/**/*.tsx`, hiding 17 `.ts` files — three of which already had passing tests | **Fixed** — `.ts` added; type-only/barrel files excluded. See §4.2 |
| `dualAliasPlugin` resolved to a **directory** | the `""` extension matched `renderer/lib/strings/` before the `index.ts` fallback, so Vite got a directory id and any module importing the `strings` barrel failed to load | **Fixed** — skip directories, fall through to the index lookup |
| **Engine tests ran against built `dist/`, not source** — all 14 engine files reported 0% while the suite passed 1615/1616 | `@bifurc/engine` is a real npm workspace package, so Vitest externalized it and native `import()` loaded `packages/engine/dist/*.mjs`. Proof: a source-only export marker read `false`; a `resolveId` probe never fired; a deliberately **bogus** alias target was ignored and the suite still passed | **Fixed** — `resolve.alias` → engine source, redeclared inside each `test.projects` entry. The worst case this hid: a source edit that was never rebuilt was **silently untested**. See §4.8 and §5 |
| A fresh clone could not `npm run typecheck` | `dist/` is gitignored repo-wide and CI ran only `npm ci`, so `@bifurc/protocol` had no build output → 10× `TS2307: Cannot find module '@bifurc/protocol'` | **Fixed** — see the *Build-robustness fixes* table below |
| Dead code in `src/sync` | `fetchRemoteHead()` / `performGitClone()` have zero call sites; `getRemoteHead()` re-implements the former, `setRemote()` the latter | **Open** (item 9) |
| `mkId()` collision probability | `Date.now().toString(36) + rand(4)` | **Open** (item 13) |

### Production bugs the new tests uncovered and fixed

All twelve were invisible to the old suite. The first four because it asserted on in-memory
config instead of reading state back off disk after a real export → import round trip; the
fifth because nothing ever sent two requests to the same origin; the sixth and seventh
because no test ever rendered an exported report or published a folder; the eighth and ninth
because nothing ever published into a folder with a space in its name or cloned a workspace
from a remote; the tenth because nothing ever observed the companion server's broadcast to
the renderer; the eleventh because the protocol reducers had no test at all; the twelfth
because the CRUD tests asserted on the *config object* the handler returned rather than on
what the running proxy served afterwards.

| Bug | Where | Fix |
|---|---|---|
| Proxy-rule export wrote the **UI stubs** (`targetType`, `targetExternal`, `targetMappingId`, `useRegex` and both scripts blanked), so re-importing silently destroyed every rule | `src/ipc/importExport/exporters/{proxyrules-json,workspace-json}.ts` | read `readAllEntities(wsId, "rules")` + re-inject `enabled` from `enabled.json` |
| Workspace import **never wrote rule files to disk** (relied on `saveConfig()`, which deliberately skips them) → imported workspace proxied nothing | `src/ipc/importExport/importers/workspace-json.ts` | explicit `writeEntity(..., "rules", …)` loop + enabled set + `upsertNameEntry` |
| Disabled mappings/mocks came back **enabled** after import (`if (m.enabled) set.add()` with no `else`; missing flag ⇒ enabled) | `src/ipc/importExport/importers/workspace-json.ts` | both-branch enabled sets (`set.add` / `set.delete`) |
| Audit Log showed **no changed files for a workspace's first commit** | `packages/engine/src/store/gitStore.ts` `getCommitChangedFiles` | `diff-tree` → `diff-tree --root --no-commit-id -r --name-only` |
| REST **Send** could fail with a spurious **400 / "socket hang up"** on a later request to the same origin | `packages/engine/src/proxy/serverReplay.ts` `replayRequest` | it sent `connection: close` but did not pin an agent, so Node returned the socket to the global pool whenever the upstream answered `keep-alive` (servers routinely ignore the request's close) and the next request was rejected by the server with `HPE_CLOSED_CONNECTION`. Fixed with `agent: false`. |
| The exported/saved **HTML run report rendered raw `${…}` placeholders** instead of data — the whole report was inert. The same escaped-interpolation mistake sat in the update-checker's fallback URL | `src/ipc/handlers/runnerHandlers.ts` `generateRunnerHtml`, `src/ipc/handlers/systemHandlers.ts` | un-escaped all 15 interpolations and hardened `esc()` to coerce + escape quotes; fixed the URL template |
| Publishing a folder that contained **exactly one new entity recorded a create as `update mock folder "X"`** and dropped the per-entity `entity-id` link. Cause: `simple-git` reports a newly-added file in **both** `status.staged` and `status.created`, so concatenating them double-counted every new file and made the single-entity branch unreachable | `packages/engine/src/sync/publishService.ts` `publishEntities` | dedupe with `Array.from(new Set([...staged, ...created, ...deleted]))` |
| A new entity inside a folder whose name contains a **space** was classified `update` instead of `create`. Cause: the pre-staging status map was built from C-quoted porcelain output (`"mocks/My Folder/x.json"`), so the lookup never matched | `packages/engine/src/sync/publishService.ts` | unquote git paths when building `preStatusMap`; `unquoteGitPath` is now exported from `packages/engine/src/sync/statusTracker.ts` and shared |
| Cloning a workspace from a remote **never adopted the remote's identity** and landed **without a `workspace.json`**. Cause: `initWorkspaceRepo()` committed only `.gitignore`, so the identity file was never version-controlled — making the adoption branch in `setRemote()` dead code | `packages/engine/src/store/gitStore.ts` `initWorkspaceRepo` | commit `workspace.json` alongside `.gitignore` on repo init |
| The companion server's **entity-status broadcast always sent an empty map**, so an entity added from the browser extension never showed its unsaved-changes dot. Cause: `broadcastEntityStatus()` serialized `getWorkspaceSyncStatus(wsId)` — an async call — **without awaiting it**, and `JSON.stringify()` turns a Promise into `{}` | `packages/engine/src/companion/companionServer.ts` `broadcastEntityStatus` | keep the broadcast non-blocking (the WebSocket reply must not wait on a git call) but send the *resolved* map via `.then()`, and log a rejected status query instead of swallowing it |
| **Discarding changes on a GraphQL / SOAP / gRPC tab left the editor showing the discarded edits.** `createTabReducer` handled only the save/send actions, so the `REFRESH` the panels dispatch after reloading the entity (`RequestTabContent` → `tabRefs.current[tabId].refresh(entity)`) fell through to each protocol reducer's `default:` branch and returned the state unchanged. REST was unaffected because it implements `REFRESH` itself and passes no `init` | `renderer/lib/createTabReducer.ts`, `renderer/components/grpc/grpcTabReducer.ts` | implement `LOAD_ENTITY` / `LOAD_DRAFT` / `REFRESH` in the shared layer, guarded on `options.init` so REST still falls through to its own reducer; `REFRESH` re-derives the entity fields and preserves the runtime/response fields, mirroring `restTabReducer` |
| **Adding, editing or deleting a mock/mapping/rule/request/websocket/webhook did not take effect on the running proxy until something else reloaded the config.** `entityCrudFactory` called `saveConfig(cfg)` then `reloadConfig()` **before** writing the entity file and **before** `syncEnabledSet()`. `reloadConfig()` snapshots the enabled-sets out of `enabled.json`, so `workspaceCfg()` filtered the new entity straight back out of routing — a mock you just added was not served, and a mock you just **deleted kept being served**, until an unrelated action (a settings save, a workspace switch, an app restart) happened to reload. A test that only inspects the handler's return value cannot see this: the config object is correct, the *running server* is stale | `src/ipc/handlers/entityCrudFactory.ts` (`add`, `update`, `delete`) | move `reloadConfig()` to **after** every on-disk write in all three handlers, so the reload sees both the entity file and the updated enabled-set. Proven by mutation: restoring the old ordering makes "stops serving a mock deleted through mock:delete" fail with `expected { from: 'mock' } to deeply equal { upstream: true, … }` |

### Build-robustness fixes

| Issue | Where | Fix |
|---|---|---|
| The companion-port setting was reachable only through a **bare alias `require()`** — `require("@/companion/companionServer")` inside `config:save`. It resolves in the packaged app only because `tsc-alias` post-processes the build output (a static import at the top of the same file is rewritten to `./companion/companionServer`; the dynamic one to `../../companion/companionServer`). That makes a user-facing path depend on the build pipeline, and it is unresolvable at runtime for any test runner | `src/ipc/handlers/coreHandlers.ts` | promoted to a **static import**. There is no import cycle to avoid — nothing under `src/companion` imports `src/ipc`. This is what makes the companion-port test in §3.4 possible at all |
| **A fresh clone could not type-check.** `dist/` is gitignored repo-wide (for every package, not just the app), but CI ran only `npm ci` before `npm run typecheck`, so no package had build output. `tsc` then failed with 10× `TS2307: Cannot find module '@bifurc/protocol'`. Latent since the protocol package landed in P1 — it went unnoticed because a developer machine always has a stale `dist/` lying around | root `package.json` | added `build:packages`, a `prepare` hook that runs it (`npm ci` executes `prepare`, so CI builds the packages automatically), and made `typecheck` run `build:packages` first. The last part matters beyond CI: without it, `typecheck` would pass locally against a **stale** `dist/` |

---

## 8. Verifying this review locally

```bash
# The repository root IS the app — there is no `cd bifurc` any more (it was flattened 2026-09-14).

# 1. Everything green, thresholds enforced
npx vitest run --coverage --coverage.clean=false --coverage.reportsDirectory=coverage-local
#    → Test Files 1 failed | 68 passed (69) · Tests 1 failed | 1615 passed (1616)
#    → Statements 47.96% · Branches 31.98% · Functions 36.30% · Lines 50.35%
#    The single failure is tests/spike/protocolPoc.test.ts → soap.execute (ECONNREFUSED
#    127.0.0.1:1), a sandbox network-interceptor caveat that reproduces on the pre-change tree.

# 2. Per project
npm run test:unit
#    → Test Files 1 failed | 51 passed (52) · Tests 1 failed | 1283 passed (1284)
npm run test:integration
#    → Test Files 17 passed (17) · Tests 332 passed (332)

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
#    i) In renderer/lib/createTabReducer.ts, change `if (!options.init)` to `if (options.init)`
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
