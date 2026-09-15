# Happy-Path Coverage Matrix

**Question:** *is the happy path workflow for all features and screens covered?*

**Short answer: yes for the main-process workflows, no for the screens themselves.**
Every screen had *something* touching it, but three whole features (Applications, Audit Log,
and the proxy's own routing behaviour) had **no happy-path test at all**, and about half the
screens only had a "does the panel render" assertion rather than a real workflow.

Seven passes later: those three features are covered, the **"hit Send" path** (REST replay,
GraphQL execute + introspect, SOAP execute + fetchWsdl) is covered against real local
servers, the Health Bar's actual polling is covered, the **data layer** — folder
management, workspace CRUD, the collection runner, the git publish/sync pipeline, and
**capture** — is covered end to end against real files, real git repos and real proxied
traffic, the two remaining 0% files — **TLS interception** and the **companion
server** — plus the **protocol editors' state machines** are covered too, and the
**Settings screen is now measured against the running app** (changing the proxy port
actually moves the live server). The remaining gaps are almost entirely **UI-level**: the
panels themselves are still E2E-only, and the E2E suite cannot run here. See §3.

Legend:

| Symbol | Meaning |
|---|---|
| ✅ | Happy path covered by a test that actually exercises the workflow |
| 🟡 | Partial — covered, but with a real hole (listed in the Notes column) |
| ❌ | No happy-path test |
| 🔒 | Runnable in this repo/CI right now |
| ⚠️ | Written but **cannot be executed here** (Electron E2E needs a desktop session) |

---

## 1. Screens

| # | Screen | Basic workflow | Status | Test | Notes |
|---|---|---|---|---|---|
| 1 | **Services** | Open, see discovered localhost processes | 🟡 ⚠️ | `e2e/app.spec.ts`, `happy-path-workflows.spec.ts` | Asserts the heading only — the list itself is never asserted non-empty |
| 2 | **Health Bar** | Add a service, see it listed, poll it | ✅ 🔒 | `protocolExecution.integration.test.ts`, `happy-path-workflows.spec.ts` | `healthbar:saveServices`/`getServices` round-trip on disk, and `healthbar:checkUrl` is polled for real (up / down / invalid URL / 10 000-char truncation) |
| 3 | **Mappings** | Create → edit → see row | ✅ ⚠️ | `happy-path-workflows.spec.ts`, `e2e/mappings.spec.ts` | Delete goes through IPC, not the UI |
| 4 | **Proxy Rules** | Create → update → see row | ✅ ⚠️ | `happy-path-workflows.spec.ts`, `e2e/proxy-rules.spec.ts` | Delete goes through IPC, not the UI |
| 5 | **Capture** | Open, see captured requests | ✅ 🔒 | `capture.integration.test.ts`, `captureUtils.test.ts`, `happy-path-workflows.spec.ts` | **A real proxied request is now asserted to land in the capture stream** (all five `via` values, bodies, the 512 KB cap) and to convert into a mock/saved request. The panel itself is still ⚠️ unvisited |
| 6 | **Requests** | Create → update → **send** for REST / GraphQL / SOAP / gRPC | ✅ 🔒 | `protocolExecution.integration.test.ts`, `happy-path-workflows.spec.ts`, `requests-rest.spec.ts`, `protocols.spec.ts` | **Sending is now really tested** for REST, GraphQL and SOAP against live servers. gRPC send is ❌ — but because the runtime is unimplemented, not because it is untested (see §3) |
| 7 | **Mocks** | Create → update for REST / GraphQL / SOAP / gRPC | ✅ ⚠️ | `happy-path-workflows.spec.ts`, `mocks-rest.spec.ts`, `protocols.spec.ts` | ✅ The *serving* side is now really tested (see §2) |
| 8 | **WebSocket** | Create a connection | 🟡 ⚠️ | `happy-path-workflows.spec.ts`, `capture-ws-webhooks.spec.ts` | **Update bypasses the UI** via `window.api` — the edit form is untested |
| 9 | **Webhooks** | Create a webhook | 🟡 ⚠️ | `happy-path-workflows.spec.ts` | **Update bypasses the UI** via `window.api` |
| 10 | **Environments** | Create → rename → add variable | ✅ ⚠️ | `happy-path-workflows.spec.ts`, `environments.spec.ts` | Delete goes through IPC |
| 11 | **Workspace** | Open workspace settings, create/rename/delete/switch a workspace | ✅ 🔒 | `tests/integration/foldersAndWorkspaces.integration.test.ts` | **Create/rename/delete/switch is now covered against real directories** (incl. "promotes another workspace when the active one is deleted"). The panel itself is still ⚠️ unvisited |
| 12 | **Audit Log** | Commit a change → list → filter → diff | ✅ 🔒 **NEW** | `tests/integration/auditLog.integration.test.ts` | Was ❌ completely untested. Screen itself still ⚠️ unvisited |
| 13 | **Settings** | Open, see server controls | 🟡 ⚠️ | `happy-path-workflows.spec.ts`, `settings.spec.ts` | Changing the port / theme / data dir is not exercised |
| 14 | **Applications** (run configs) | Save → run → see logs → stop → debug | ✅ 🔒 **NEW** | `tests/integration/applications.integration.test.ts` | Was ❌ **entirely untested** — panel, handlers and process spawner |

## 2. Core engine workflows (no screen of their own)

| Feature | Basic workflow | Status | Test |
|---|---|---|---|
| **Domain mappings route traffic** | `app.localhost` → mapped target, method/path/query/body preserved | ✅ 🔒 **NEW** | `tests/integration/mappingsAndRules.integration.test.ts` |
| **Proxy rules route traffic** | Rule pattern → external target / mapping target | ✅ 🔒 **NEW** | same |
| **Mocks replace responses** | Full mock replaces upstream; partial mock merges | ✅ 🔒 **NEW** | `tests/integration/mocks.integration.test.ts` |
| **Mock beats rule** | Precedence order is correct | ✅ 🔒 **NEW** | same |
| **Passthrough** | Unmatched traffic reaches the real upstream | ✅ 🔒 **NEW** | same |
| **Enabled/disabled routing** | A disabled mapping/rule/mock stops applying | ✅ 🔒 **NEW** | same |
| **Import/Export** | Export → import round trip for all 8 entity kinds | ✅ 🔒 **NEW** | `importExport*.integration.test.ts` |
| **Workspace snapshot** | Export whole workspace → import creates it intact | ✅ 🔒 **NEW** | `importExportFormats.integration.test.ts` |
| **Request scripts** | Rule request/response scripts mutate traffic | ✅ 🔒 **NEW** | `mappingsAndRules`, `upstreamFetch` |
| **REST Send** | `request:replay` → real server → status/headers/body back to the pane | ✅ 🔒 **NEW** | `protocolExecution.integration.test.ts` |
| **GraphQL Send + introspect** | `graphql:execute` / `graphql:introspect` → real server | ✅ 🔒 **NEW** | same |
| **SOAP Send + WSDL fetch** | `soap:execute` / `soap:fetchWsdl` → real server | ✅ 🔒 **NEW** | same |
| **Health Bar polling** | `healthbar:checkUrl` → up / down / invalid URL / truncation | ✅ 🔒 **NEW** | same |
| **Protocol entity CRUD** | GraphQL/SOAP/gRPC requests, schemas, WSDLs, protos — add → load → delete, read back off disk | ✅ 🔒 **NEW** | same |
| **Folder management** | `folder:add/rename/move/delete` → assert the real directory, `index.json` and surviving entities | ✅ 🔒 **NEW** | `tests/integration/foldersAndWorkspaces.integration.test.ts` |
| **Workspace CRUD** | `workspace:add/rename/setActive/delete` against real dirs | ✅ 🔒 **NEW** | same |
| **Git publish** | Change a file → `entity:publish` → the commit the Audit Log parses (`create mock Name`), bundled folder form, explicit message, no-op, delete | ✅ 🔒 **NEW** | `tests/integration/gitSync.integration.test.ts` |
| **Git diff / discard / restore** | `git:diff` for clean/modified/new/deleted; `git:discard` reverts or deletes untracked, and re-syncs `names.json` | ✅ 🔒 **NEW** | same |
| **Git history (sidebar)** | `git:history` / `history:list` / `history:diff` on a real repo, path normalisation, limit | ✅ 🔒 **NEW** | same |
| **Entity sync status** | `sync:getEntityStatus` clean / modified / new / deleted markers, cleared after publish | ✅ 🔒 **NEW** | same |
| **Git sync / publish (remote)** | `sync:setRemote` in all three branches against a real bare repo, then push → clone → pull → disconnect | ✅ 🔒 **NEW** | `tests/integration/gitRemote.integration.test.ts` |
| **Collection runner** | Run a folder of requests: orchestration, `{{var}}` resolution, scripts, cancellation, HTML report | ✅ 🔒 **NEW** | `tests/renderer/collectionRunner.test.ts` |
| **Runner persistence** | `runner:saveReport/getHistory/saveConfig/loadConfig/exportReport` → files land in `requests/.runs/` | ✅ 🔒 **NEW** | `tests/integration/runnerStorage.integration.test.ts` |
| **Capture** | Real traffic through the proxy → an entry on the `log:entry` stream (method/url/status/target/`via`/bodies), then `capture:shareJson` | ✅ 🔒 **NEW** | `tests/integration/capture.integration.test.ts` |
| **Capture → mock / saved request** | A captured entry converts into a mock body and into a replayable saved request (header stripping, binary vs text) | ✅ 🔒 **NEW** | `tests/renderer/captureUtils.test.ts` |
| **TLS interception** | Enable TLS, `CONNECT host:443` → a per-host cert signed by the configured CA, then routing over the decrypted stream | ✅ 🔒 **NEW** | `tests/integration/tlsIntercept.integration.test.ts` |
| **TLS blind tunnel (off path)** | With TLS disabled, `CONNECT` pipes raw bytes and no cert is served | ✅ 🔒 **NEW** | same |
| **Companion server** | Start it, connect, dispatch `config:get` / `mock:add` / `request:add` / `folder:add`, and refuse everything else | ✅ 🔒 **NEW** | `tests/integration/companionServer.integration.test.ts` |
| **Protocol editors (GraphQL/SOAP/gRPC)** | Init from entity or draft, send, save, revert, and serialize to a payload | ✅ 🔒 **NEW** | `tests/renderer/protocolTabReducers.test.ts` |
| **Settings mutations** | Change the proxy port → the **running server moves** (old port refuses, new port serves the same mock); toggle TLS → the CA loads/unloads; change the companion port → the WebSocket server moves; theme and zoom persist and clamp | ✅ 🔒 **NEW** | `tests/integration/settingsMutations.integration.test.ts` |
| **Live config → routing** | Add / edit / delete a mock through the app and the **running proxy** serves it (or stops serving it) with no restart; switch workspace and the same URL re-routes | ✅ 🔒 **NEW** | same |

## 3. Still NOT covered (happy path)

These are real features with **no** basic-workflow test. Ordered by risk.

| Feature | What a happy-path test would do | Why it matters |
|---|---|---|
| **gRPC (a product gap, not a test gap)** | `grpc:execute`, `grpc:reflect` and `grpc:startMockServer` all return "not yet configured" | The handlers *are* covered — as stubs. There is nothing to exercise until the runtime is implemented. |
| **Search / command palette** | Type a query, open a result, land on the right entity | `searchUtils`/`searchHelpers`/`searchModules` are measured (58–85%), but `searchPanelUtils.ts` is 14% and nothing types a query end to end |
| **UI delete paths** | Click Delete → confirm dialog → row disappears | Every cleanup uses IPC, so the delete buttons/confirm dialogs are unverified. The IPC handlers themselves *are* covered (`mock:delete` is asserted to stop the proxy serving the mock) |
| **The panels themselves** | Any real click-through of a panel | `renderer/panels/**` is 0% in the unit/integration suite; the Playwright E2E suite covers them but cannot run in this environment (§4.1) |

## 4. Caveats you should know about

1. **The E2E suite cannot run in this environment.** Electron will not launch without
   a desktop session (CI installs `xvfb` + the Electron system libraries, but the
   job has never been green here). Everything marked ⚠️ is *written* but *unverified*.
   All the ✅🔒 rows above are verified — they run in `npm test`.
2. **The E2E happy path is one monolithic test.** `happy-path-workflows.spec.ts` is a
   single `test()` covering 12 screens. If any step fails, the remaining screens are
   never exercised, and the failure does not name the feature. Splitting it into one
   spec per screen would make "which feature is broken?" answerable at a glance.
3. **"Visit-only" assertions are weak.** `expect(page.locator("body")).toContainText("…")`
   proves the panel mounted, not that it works.
4. **Coverage of the renderer is still low** (46.14% statements / 48.39% lines overall) — the
   panels themselves are mostly untested; the coverage above is concentrated in the main
   process, where the logic lives. The exception is `renderer/lib`, which reached 82.9%
   once the collection runner and its report generator were covered. Note the overall figure
   is measured against a `src/**` + `renderer/**` denominator that includes every panel; a
   change to the coverage `include` glob in `vitest.config.ts` moved it by half a point
   without a single test changing (see `TESTING.md` §4.2).

## 5. Recommended next steps

1. **The panel hooks** (`renderer/components/rest/{useCollectionRunner,useRestActions}.ts`,
   0%). The runner's *engine* is covered but the hooks that wire it to the panel are not;
   they need a DOM environment, or their pure parts extracted. This is now the highest-value
   remaining gap: it is the last piece of *logic* (as opposed to markup) with no test.
2. **Search / command palette** — `searchPanelUtils.ts` is at 14%; nothing types a query and
   lands on the right entity.
3. Split `happy-path-workflows.spec.ts` into one spec per screen so failures are
   attributable (mechanical, low risk). The E2E suite is now the only coverage for
   `renderer/panels/**`.
4. Add UI delete coverage (click Delete → confirm → assert the row is gone) for one
   entity type, then reuse the pattern.
5. Fix the WebSocket/Webhook E2E steps to drive the edit form instead of `window.api`.
6. Implement the gRPC runtime. The handlers are pinned *as stubs* today, so the suite will
   fail deliberately the moment real behaviour lands — update those tests with the feature.
7. Clean up the dead code in `src/sync`: `fetchRemoteHead()` and `performGitClone()` have no
   callers, and `getRemoteHead()` / `setRemote()` re-implement them inline.
8. Decide whether the companion server should authenticate. It binds to `127.0.0.1` and that
   is the whole of its access control; `ALLOWED_ACTIONS` is now proven to be enforced, but
   any local process can still call the four allowed actions.
