# Phase 17 — Engine-level ports: one listener per port, one config for all workspaces

**Status: IN PROGRESS — work items 1, 2, 3, 5, 6, 7, 8 and 9 landed 2026-09-25/26. Item 4's Electron half is
the only one left.** The blocker in §7 was **stale**; it is resolved and replaced with the measured record.
**Item 5 does not depend on item 4** — see the note in its record in §5.

> ## ⚠ 2026-09-26 — the companion's GUI *neighbour* is gone, and the pair rule with it
>
> **The browser host was decommissioned and the desktop is the single build target** (`plan/README.md`'s
> 2026-09-26 block). Three things this file specifies were removed with it:
>
> - **`webGuiPortFor(companionPort)` and `findPortPair(base)`** — deleted from `portAllocator.ts`, whose surface
>   is now `SCAN_LIMIT`, `tryBind`, `findFreePortFrom`, `nextFreePortAbove`.
> - **`WorkspaceHostStatus.webGuiPort`**, and the `--gui-target` / `BIFURC_GUI_TARGET` pair that named the GUI's
>   address.
> - The **Ports section's** web-GUI sentence, and the `portsDerivedNote` string behind it.
>
> **The host's companion is now found with `findFreePortFrom(settings.companionPort, reserved)`** — a *single*
> port, which is Electron's own path. That is a **behaviour change, not a rename**: the pair finder required
> `n` **and** `n + 1` to be free, so a base port whose neighbour happened to be taken was a **refused start**.
> With no GUI neighbour left to reserve, the refusal lost its reason and a host that used to fail now starts.
>
> **§1's port table is otherwise unchanged and this file is still the authority on every port** — the edge, the
> forward proxy, the webhook, and the companion's *restart-only* rule all hold exactly as written. **Four
> places below still name the removed pair as current** (§4's `portAllocator.ts` row; §5's item-2 row and its
> record; the Ports-section paragraph; §7's extension-discovery note). They are the 2026-09-25 record and are
> left in place, marked where a reader could act on them. **Read this note first.**

This supersedes the per-workspace half of `plan/15` §3.1 and D16's scan-and-remember rule for the forward
proxy and the webhook. `plan/15` remains the authority on the **edge** and on the mapping model; where the two
disagree about a *port*, this file wins.

---

## 1. The model

**Every port is engine-level. None is per workspace.** A workspace is a **project**: switching workspaces
changes the **context** — which rules, mocks and webhook endpoints are in force — while the ports and the
infrastructure stay put. That is the whole point of the standardization, and it is why the config stops
varying by workspace.

| Port | Default | Level | Who must know it | Change |
|---|---|---|---|---|
| **Reverse proxy** (the edge) | `80` | host | the browser — every `*.localhost` mapping already points at it | runtime |
| **Forward proxy** | `8080` | host | the browser's proxy setting | runtime |
| **Webhook receiver** | *random* | host | whoever sends the webhooks | runtime, and **off by default** |
| **Companion** (RPC over WebSocket) | `4747` | host | the extension and the web GUI | **restart** |
| **Web GUI** | companion `+ 1` | host | the edge's global mapping | derived — **not a setting** |

### The rules that make each one behave the way it does

- **The companion and the GUI are a pair, found by scanning.** The companion starts **first**; the GUI starts
  second **with the companion's port passed to it**, so it knows where to connect without being told. A
  *base* port is the starting point and the search walks upward in **pairs** — `n` and `n + 1` must **both**
  be free, because `n` free with `n + 1` taken is a *failed* start, not a usable one. **It never fails**: a
  free pair is eventually found. **The extension scans the same way from the same base**, which is the
  discovery contract — the base is therefore part of the published protocol, not an implementation detail.
- **The forward proxy does NOT scan.** It binds `8080` and **fails if occupied**, naming the port, so the user
  enters a working one instead of the app silently incrementing. Whatever port is in force **stays saved**, so
  the user always knows which port their browser must be pointed at.
- **The webhook does NOT scan either.** It is a **feature, not infrastructure**: it is **off by default**,
  the user starts it, and the user owns the port. The default is deliberately **random** rather than a fixed
  number, because a fixed default would collide with whatever else on the machine already wanted it. Starting
  it checks the port is free and **fails if it is not**.
- **The edge is bound, not scanned** (unchanged from `plan/15`): it is the port the mappings point at.
- **Electron has no GUI port**, so it checks only the companion's `n` — requiring `n + 1` to be free there
  would refuse a port for a neighbour it is never going to use.

### Restart semantics

Only the **companion** needs a restart, and the restart is **manual**: the UI shows the new value as
*"applicable after next restart"* rather than acting on it. Everything else — edge, forward proxy, webhook —
re-binds at runtime. **Today that runtime rebind does not exist**, and `configCommands.ts` says so in a
comment: changing `settings.port` does **not** move the edge, because the port the proxy reads belongs to the
edge. Making the other three live is new plumbing, not a settings-schema edit.

### Mock and request support (owner decision, same session)

- **Mocks: REST only.** The **gRPC mock goes** and the **SOAP mock goes**. "No other type of proxy based
  mocks" — and SOAP mocks are proxy-based, so they are included in that.
- **Requests keep all four: REST, GraphQL, SOAP, gRPC.** The **gRPC client stays** — it lives in Requests, not
  in Mocks, so removing the gRPC *mock* does not touch it. **Do not delete the gRPC request client, the proto
  files, or their folders while removing the mock.**

**Extended by the owner 2026-09-25, while item 8 was being implemented** — two additions the block above does
not name, each asked rather than assumed because the block contradicted itself:

- **The GraphQL mock goes too.** The stated principle is *"no other type of proxy based mocks"*, and a GraphQL
  call is "just a POST in the end", so a GraphQL mock is REST-mockable. The bullet list named only gRPC and
  SOAP, which is exactly why this was a question rather than an inference.
- **The three gRPC mock-server commands go** — `grpc.mockServerStatus`, `grpc.startMockServer` and
  `grpc.stopMockServer`, *deleted* rather than left as `wontfix` stubs. They were the gRPC mock's serving half,
  and they had already outlived any renderer that could reach them: no component rendered the controls, and only
  `window.ts`'s declarations plus some dead `protocols.ts` strings remained.

**Requests are untouched by all of this**, and that is the invariant item 8 had to preserve: `graphqlRequests`,
`grpcRequests`, `soapRequests`, `graphqlSchemas`, `protoFiles` and `savedWsdls` all survive, each with the
folder kind it owns.

---

## 2. What this replaces

- **`plan/15` §3.1's per-workspace half.** The forward proxy and the webhook stop being per-workspace. The
  rule itself (*a port belongs to the thing that has to be addressed once*) is **strengthened**, not
  abandoned: a browser's proxy setting is a per-machine choice and a webhook URL is registered once, so both
  are now *more* correctly host-level than they were.
- **D16's scan-and-remember for the proxy and the webhook.** No scanning, so no remembered resolved port for
  them. `runtime.json`'s `preferredProxyPort` / `resolvedProxyPort` / `preferredWebhookPort` /
  `resolvedWebhookPort` all become dead.
- **`portAllocator.ts`'s per-workspace machinery** — `allocatePorts`, `PortUnavailableError`,
  `PortRangeExhaustedError`, the `AllocatedPorts` provenance flags. What survives is an **open-port finder**
  and a **pair finder**. See §6 item 2.
- **The per-workspace port overrides in the UI.** They move into one global **Ports** section.

---

## 3. The structural consequence — read this before writing code

**`WorkspaceRuntime` loses every listener it owns.** It composes exactly two things: a forward proxy and a
webhook receiver. Both move to the host. The edge was already host-level. So **nothing per workspace listens
any more**, and the abstraction that `plan/14` D10/D11 built — *"N workspaces per host, and `WorkspaceRuntime`
is the unit"* — no longer has a subject.

What remains per workspace is **config**: mappings, proxy rules, mocks, webhook endpoints, environments.

**This is a large refactor and it is deliberately not started.** The blast radius is not just
`workspaceRuntime.ts`:

- `host.ts` — the per-workspace loop, `runtimeRegistry`, `skipped`, and the status shape all assume one
  runtime per workspace with its own ports.
- `runtimeServerOps.ts`, `serverCommands.ts`, `proxyCommands.ts`, `webhookCommands.ts` — they resolve a
  **runtime scope** (`scope.kind === "runtime"`) to route a command to a workspace's proxy or webhook. With
  one host-level instance, that routing collapses to "the active workspace's config".
- `index.ts` (the single-workspace facade) and `src/main.ts` (the Electron shell) both compose the same
  servers and must agree.
- `runtime.json`, the Services panel, and every suite that asserts a port number — including the
  *"the scan lands on `base + N`"* family `TRAPS.md` warns about, whose whole premise disappears.

### ✅ The decision — RESOLVED 2026-09-25 (owner): `WorkspaceRuntime` is **DELETED**, not hollowed.

With no listener of its own it has no subject left, and a hollowed handle would keep a name alive for something
that no longer does anything. What the deletion buys is the thing the bullet list above is about:
`runtimeRegistry.ts`'s three-way scope union (`runtime` / `facade` / `absent`) has nothing left to resolve, so
the command bodies become **direct facade calls** — the honest reading of *"the routing collapses"*. The
deleted files are `workspaceRuntime.ts`, `runtimeRegistry.ts`, `hostRegistry.ts` and `runtimeServerOps.ts`.

**What was NOT deleted, deliberately:** `bindProxyInstance()` / `bindWebhookInstance()`. They already existed for
the desktop, and the host now uses the same mechanism — so the facade is *bound* rather than looked up, and no
registry is needed to find it.

---

## 4. What had already landed (before §5), and one prediction that was wrong

All additive to the product — and, it turned out, **destructive to a mock**. See §7 for the measured −10.

| File | Change |
|---|---|
| `packages/engine/src/store/appSettings.ts` | `AppSettings` gains `forwardProxyPort` + `companionPort`; the four `DEFAULT_*` constants live here as the **one home** for the numbers |
| `packages/engine/src/runtime/portAllocator.ts` | `webGuiPortFor(companionPort)` (`+ 1`, derived) and `findPortPair(base)` — the first `n` where **both** `n` and `n + 1` are free |
| `packages/engine/src/store/config.ts` | `AppConfig` gains both; `loadConfig`/`saveConfig` map them with `??` defaults — **that `??` is the migration** for an `app.json` written before 2026-09-25 |
| `packages/gui/src/types/config.ts`, `packages/gui/src/hooks/useAppConfigSync.ts` | the GUI's `AppConfig` and `EMPTY_CONFIG` gain both |

---

## 5. Work items

| # | Item | Status |
|---|---|---|
| 1 | Resolve §3's decision (delete vs hollow) | ✅ **DELETE** (owner) — landed |
| 2 | Minimize `portAllocator.ts` to the open-port finder + `findPortPair` | ✅ landed |
| 3 | Make the forward proxy and webhook host-level in `host.ts` | ✅ landed |
| 4 | The companion-then-GUI startup order | 🟡 host half landed; Electron half open — **and item 5 does NOT depend on this one** (see the note in item 5) |
| 5 | Bundle the GUI into the image; fix `CMD`/`EXPOSE`/healthcheck | ✅ landed — entrypoint + static server + healthcheck, `dist/web` bundled, `CMD` de-hardcoded. **`npm ci` was broken and is fixed** |
| 6 | The Ports section in global settings | ✅ landed — one global section, four ports, the companion's restart badge |
| 7 | Runtime rebinding for the edge, forward proxy and webhook | ✅ landed |
| 8 | Remove the gRPC mock and the SOAP mock | ✅ landed — **scope widened by the owner to all three protocol mocks** |
| 9 | Retire the per-workspace port overrides from the UI and `runtime.json` | ✅ landed |

### Items 1, 2, 3, 6, 7, 8 and 9, as landed

**1 — `WorkspaceRuntime` DELETED.** Four files removed: `runtime/workspaceRuntime.ts`,
`runtime/runtimeRegistry.ts`, `runtime/hostRegistry.ts`, `runtime/runtimeServerOps.ts`. The runtime-scope union
collapsed entirely, so `serverCommands.ts` / `webhookCommands.ts` became direct facade calls;
`commands/echoWorkspace.ts` was added because `workspaceId` survives only as an **echo**.

**2 — `portAllocator.ts` minimized** (414 → ~250 lines). Deleted: `allocatePorts`, `PortUnavailableError`,
`PortRangeExhaustedError`, `AllocatedPorts`, `AllocateOptions`, `proxyBasePort`, `WEBHOOK_BASE_PORT`,
`IS_WINDOWS`. Kept: `SCAN_LIMIT`, `tryBind`, `findFreePortFrom` (signature changed — the `kind` parameter is
gone), `webGuiPortFor`, `findPortPair`, `nextFreePortAbove`.

**⚠ Superseded 2026-09-26 — of that kept list, `webGuiPortFor` and `findPortPair` went too** (see the note at
the head of this file). **The module's surface is now four exports: `SCAN_LIMIT`, `tryBind`,
`findFreePortFrom`, `nextFreePortAbove`** — the single-port finder is the only one the host needs, now that the
companion has no GUI neighbour to reserve.

**3 — the forward proxy and the webhook are host-level.** `host.ts` step 3b creates one forward proxy with
`workspaceId: null` (which *is* "follow the active workspace" — `createProxyServer()` falls back to
`loadConfig().activeWorkspaceId` per call) and binds it on `settings.forwardProxyPort`; step 3c creates the
webhook **unstarted** (off by default) and binds it; step 4 is now just the `served` list, with no allocation
loop and no `runtime.json`. `WorkspaceHostStatus` replaced `runtimes` / `skipped` with `forwardProxyPort`,
`forwardProxyError`, `webhookPort`, `webhookError`, `companionPort`, `webGuiPort` and
`workspaces: {workspaceId, name, active}[]`. **`workspace.setActive` restores `reloadConfig()`** — the swap
§5 item 3 requires, and the reversal of P14 WI 4.

**9 — nothing per workspace remembers a port.** `runtime.json` and `hostRegistry.ts` are deleted, so the host
writes no port registry at all. On the UI side this item was **already satisfied**: the GUI has no
per-workspace port overrides (`strings/workspace.ts`'s only `Ports` string is `labelPorts: "Port Mappings"`,
which is Docker port mappings). The remaining half is item 6's global Ports section.

**7 — a save re-binds the three runtime ports.** `plan/17` §1 gives only the **companion** a restart; the edge,
the forward proxy and the webhook re-bind at runtime, and only the forward proxy did — on a port that was no
longer its own. All three `start()` implementations already begin `if (server) stop()`, so **`start(newPort)`
*is* the rebind**, and the item is entirely about *reachability*:

- the **forward proxy** and the **webhook** already had facades, so `config.save` could reach them. It now reads
  `incoming.forwardProxyPort` rather than `getPort() || incoming.port` — D16's remembered resolved port, which
  §2 makes dead;
- the **edge had no facade at all.** `proxy/edge.ts` gained `bindEdgeInstance()` / `restartEdge()` /
  `isEdgeRunning()`, **bound-only with no lazy default**: an edge needs a `resolveConfig`, and a lazily created
  one would answer **503 for every request** — worse than doing nothing, because it reads as a bug rather than as
  an unbound handle.

`config.save` is now **three parallel branches**, one per port, each a single call. The explicit
`stopServer(); startServer()` pair is gone — `start()` stops internally, so it was a second way to say one thing,
and removing it is what makes the three read alike.

**⚠ The binding had to be added to *three* compositions, not two.** `index.ts` and `runtime/host.ts` were not
enough: **`src/main.ts` is the desktop's composition and never calls `createEngine()`** — it builds the proxy,
the webhook and the edge itself, which is why it already called `bindProxyInstance()` and `bindWebhookInstance()`.
Without the third call, `restartEdge()` would have been a silent no-op **on the desktop**, the platform the
feature exists for. Found by grepping for the **factory** (`createEdgeRouter`), which lists all three, rather
than for the entry point, which lists two. Recorded in `TRAPS.md`.

**Two cases inverted, and both proved by a narrow mutation** — judged by exit code, and each caught by exactly
the case that owns its rule and by no other:

| Mutation | Result |
|---|---|
| `restartEdge(incoming.port)` removed | **EXIT=1** — 2 failures: the edge case, and `server:restart picks up a port changed while the server was down` |
| the `forwardProxyPort` arm of the proxy condition removed | **EXIT=1** — 1 failure: the forward-proxy case |

**Item 7's delta is `0` tests / `0` files** — two replacements, not two additions: `settingsMutations` reports
**31** in both the pre- and post-change full runs. `proxyHarness.ts` gained an optional `forwardProxyPort` so
`bootOnConfiguredPort()` boots the proxy on `settings.forwardProxyPort` — the way the engine does — instead of on
a free port the setting disagreed with.

**The suite delta for items 1–3 and 9, item by item — every unit named, none inferred.** Baseline 2,766 tests /
119 files; after those items, **2,745 / 118** — **−21 tests, −1 file**:

| File | Cases | Δ |
|---|---|---|
| `tests/runtime/portAllocator.test.ts` | 35 → 15 | **−20** |
| `tests/integration/workspaceRuntime.integration.test.ts` | 2 → *deleted* | **−2** |
| `tests/integration/workspaceDispatchScope.integration.test.ts` | 4 → 3 | **−1** |
| `tests/integration/workspaceCommandScope.integration.test.ts` | 9 → 8 | **−1** |
| `tests/integration/portRestartStability.integration.test.ts` | 3 → 4 | **+1** |
| `tests/integration/webhookWorkspaceScope.integration.test.ts` | 2 → 4 | **+2** |
| `tests/integration/{workspaceHost, workspaceHostSignal, facadeBinding}` | unchanged | 0 |

`−20 −2 −1 −1 +1 +2 = −21` ✓. The −20 is the per-workspace allocation suite, whose subject is gone.

**Three suites were INVERTED rather than deleted**, because their subject survived but its *answer* flipped:
`workspaceDispatchScope` (one proxy following the selection, not N), `settingsMutations`' proxy half (the
swap, not the independence), and `handlers.test.ts`'s `workspace.setActive` case. **Two more were rewritten
around a new truth**: `webhookWorkspaceScope` (one receiver, one port, one active map — with the lost
per-workspace isolation **pinned as a known gap** rather than left unasserted) and `portRestartStability`
(D16's stickiness replaced by *"bound, not scanned"*, which is discriminating by construction — a re-added
scan fails it).

**Known gap, recorded rather than implied.** The webhook's *active set* is per **instance** and not keyed by
workspace, so with one host-level receiver **switching workspace does not retire the previous workspace's
active webhooks**. §1 lists *"which webhook endpoints are in force"* among the things a switch changes, so this
is a limit rather than the intended end state; closing it needs `createWebhookServer()`'s maps keyed by
workspace. It is stated in `runtime/host.ts` step 3c, in `proxy/webhookCommands.ts`, and **asserted** in
`webhookWorkspaceScope.integration.test.ts` so it cannot drift silently.

**8 — mocks are REST-only, and the client surface lost exactly twelve keys.** The owner's decision (§1) removed
three entity kinds, three folder kinds and three commands. One invariant had to survive, and it is the one the
item's own text calls out: **the request halves all stay** — `graphqlRequests` / `grpcRequests` / `soapRequests`,
their schemas (`graphqlSchemas`, `protoFiles`, `savedWsdls`) and the folder kind each of them owns.

| layer | removed |
|---|---|
| `packages/protocol` | `EntityKind`: `graphqlMocks` / `grpcMocks` / `soapMocks`; `FolderKind`: the three singular forms; `EntitySetEnabledParams.kind` narrowed to `["mocks","mappings","rules"]`; the three `grpc.*MockServer` specs and fixtures |
| `packages/engine` | the three `Saved*Mock` interfaces and their `registerCrudKind` calls; **`proxy/protocolMockHandler.ts` deleted** (it held only the GraphQL and SOAP matchers) with its call site in `routingUtils.ts`; the seven `AppConfig` mock keys — including `grpcMockServerPort: 9102`, which was the last thing keeping that port alive; the three `grpc.*MockServer` registrations; three `entityDir` calls in `workspaceFs.ts`; `sync/gitOps.ts`'s three kind lists; `transport/auth/scopes.ts`'s three scopes |
| `packages/client` | nine mock CRUD methods, three `grpc*MockServer` transport methods, and the three command names in `retry.ts`'s idempotency lists |
| `packages/gui` | the three `Saved*Mock` mirrors; `UnifiedMock` narrowed to `MockRule & { protocol?: "rest" }`; the mock halves of the three protocol tab reducers; the mock branches in `useMultiplexedEntities` |

**The client surface went 144 → 132, and that is a contract change rather than a re-baseline.** The twelve are
the nine mock CRUD methods plus the three `grpc*MockServer` transport methods. Measured classification:
`transport` 65 → **62**, `entity` 48 → **39**, `subscribe` 7, `shim` 8, `local` 16 (sum 132). The declared type
went 140 → **128**, so **the four-key gap between the runtime and `BifurcApi` is still exactly four** — the
property that shows the change was a *drop* the runtime and the type took together, not a reclassification
hiding a fourth discrepancy. Recorded in `surface.ts`'s header, in both client test files, and in `TESTING.md`.

**The delta is `−42` tests and `−1` file, every unit named:**

| file | Δ | mechanism |
|---|---|---|
| `tests/proxy/protocolMockHandler.test.ts` | **−23** (file deleted) | its only subject, `protocolMockHandler.ts`, was deleted |
| `tests/renderer/protocolTabReducers.test.ts` | **−11** | the mock halves of the three protocol tab reducers |
| `packages/protocol/src/commands/index.test.ts` | **−6** | 3 removed command specs × 2 cases each, derived from `COMMANDS` |
| `tests/integration/protocolExecution.integration.test.ts` | **−3** | 2 `grpc.*MockServer` cases + 1 `graphqlMocks` validation case |
| `tests/ipc/handlers.test.ts` | **+1** | the `urlPattern` validation case, **moved** rather than deleted |

`−23 −11 −6 −3 +1 = −42`, and `2,745 − 42 = 2,703`. Nothing else in the tree moved.

**One case was moved rather than deleted, and the reason is a harness fact worth keeping.** The old
`"a graphql mock requires an endpointPattern"` case lived in `protocolExecution.integration.test.ts`, whose
`beforeAll` registers only `registerCoreKinds()` plus the three *protocol* kind files — `registerCrudKinds()` is
deliberately not among them, so `mocks` has **never** been routable there. That case passed only because
`registerGraphqlKinds()` registered `graphqlMocks`; once those three files registered no mock kind at all, the
REST-mock equivalent fails with `kind "mocks" is not routed through the CommandRegistry yet` — a fact about the
harness, not about mocks. The assertion (a blank pattern is rejected; `crudKinds.ts`'s `validate` checks
`urlPattern` first) therefore moved to `tests/ipc/handlers.test.ts`'s `entity.create — mocks` block, which
registers through the shell and does reach it.

**⚠ `npm run typecheck` does not typecheck `tests/`.** The root `tsconfig.json` is `include: ["src/**/*"]`, so
every test file that referenced a removed export compiled clean and the removal reported **success** — the
failure appeared only when vitest ran. `typecheck:renderer` covers `packages/gui` + `renderer/`, `typecheck:e2e`
covers `e2e/`, and **nothing** covers `tests/`. Recorded in `TRAPS.md`.

**6 — the four host-level ports now have one global Ports section.** New
`packages/gui/src/panels/settings/PortsSettingsSection.tsx` owns the edge, the forward proxy, the webhook and the
companion; the two port rows `ServerSettingsSection` used to hold are gone, leaving it the status banner, the
port hints, tray/restart/theme. The section is its own `SectionLabel` (`sectionPorts`), rendered by
`SettingsPanel` between Server and TLS. **Item 9 had already emptied the UI of per-workspace overrides**, so
there was nothing to *move* — this item is the other half of that bullet: the global section those overrides
were meant to become.

**Two content bugs went with it, and both were the same shape — a second answer to a question the split had
already answered.**

- The Server section's hint box told the user to point their browser's proxy at `127.0.0.1:{config.port}` — the
  **edge** port. That was right while the edge *was* the forward proxy; §1 splits them, and the port a browser's
  proxy setting wants is `settings.forwardProxyPort`. Now named from `config.forwardProxyPort`.
- `config.webhookPort ?? 9101` — a **stale copy of a default the engine deliberately deleted**. `config.ts:184`
  carries `webhookPort: settings.webhookPort` with no fallback, and its comment says the fallback was
  unreachable *and* was *"a second answer to 'which port is the receiver on'"*. The row now reads the value it
  is given. **One stale copy survives elsewhere** — see the gaps.

**Two decisions, both because the alternative would be a claim the UI cannot verify.**

- **The companion's restart badge is static, not conditional.** §1 says the UI *"shows the new value as
  applicable after next restart"*, which reads like a badge that appears once the value has changed. It cannot
  be: `companionPort` appears in **no protocol command** — `serverStatus` is the edge's — so "this differs from
  the port the running companion is on" is not knowable in the renderer. A badge that is always right beats one
  that is sometimes a guess, and it becomes conditional the day a command reports the live port.
- **The web GUI gets no row and no input.** Its port is derived (`companionPort + 1`) and handed to the GUI at
  start, so a settable one would be a way to configure the pair into a state where the GUI looks for the
  companion somewhere it is not. It is stated as a sentence rather than shown as a number, for two reasons: the
  arithmetic lives in `webGuiPortFor()`, in a **Node-only** engine module the browser bundle must not pull in;
  and **Electron has no GUI port at all** — only the web host does — so a number would be wrong in the desktop
  app, where this same panel renders.

  **⚠ Superseded 2026-09-26 — the conclusion survives and both reasons do not.** There is no GUI port at all
  now, so the row is absent for a simpler reason: nothing serves a separate browser bundle. The sentence that
  used to sit under the companion row (`portsDerivedNote`) and the `webGuiPortFor()` it cited are both deleted.
  **A panel rendering this section should offer exactly four ports and no GUI row** — which is what
  `PortsSettingsSection` does, and what `tests/renderer/portsSettings.test.tsx` pins.

**The delta is `+11` tests and `+1` file, and the new file is the whole of it:**

| | tests | files |
|---|---|---|
| unit | 2,242 → **2,253** | 87 → **88** |
| integration | 461 → 461 | 30 → 30 |
| **full** | 2,703 → **2,714** | 117 → **118** |

`2,253 + 461 = 2,714` ✓, `88 + 30 = 118` ✓, both halves measured. **Integration not moving is the load-bearing
part**: it says no existing case changed behaviour, which a delta of this shape could otherwise hide. `0`
failures of any kind, and the `gitOps` `EBUSY` flake did not reproduce.

**Two of the eleven cases were checked by a narrow mutation, judged by exit code — and each run failed exactly
one case:**

| Mutation | Result |
|---|---|
| a fifth port input added (a web-GUI port) | **EXIT=1** — 1 failure: *"offers exactly four ports"* |
| `<PortsSettingsSection/>` removed from `SettingsPanel` | **EXIT=1** — 1 failure: *"mounts the Ports section"* |

The second is the one worth keeping. **With the section unmounted, all ten component-level cases still passed** —
they render the component directly, so they would have stayed green over an unreachable component. That is the
shape of gap `verify:web` found four of in code that passed 471 tests, and it is the only reason the wiring case
exists.

**`npm run typecheck:renderer` is still 34 errors, and none of the three files this item touched is among
them.** The instrument is *which files*: the 21 files carrying those errors are files this item did not touch,
and `PortsSettingsSection.tsx` is absent from the list — so the new component is type-clean and nothing was
traded.

**Found, not fixed — two, recorded rather than implied.**

- **`panels/webhooks/useWebhooksPanelState.ts:26` still carries `config.webhookPort ?? 9101`** — the same stale
  default the Ports row just lost. It is outside this section, so it was left; it should go the same way.
- **Nothing in the GUI surfaces `forwardProxyError` / `webhookError`.** `WorkspaceHostStatus` reports both, and
  §1's model is that a forward proxy whose port is taken *fails and names the port so the user can free it* — but
  the panel reads neither, so the one case the design is explicit about is visible only in the host log. Wiring
  it needs a new prop through `panelFactory`, which is a change to the panel contract rather than to this
  section, which is why it is named here instead of done.

**5 — the GUI is bundled into the image, and the `CMD` no longer hardcodes the port.** Landed 2026-09-25/26.

Three new scripts, and the split between them is the design. **`scripts/docker-entrypoint.mjs`** is now the
`ENTRYPOINT`: it resolves the companion port (flag → `BIFURC_COMPANION_PORT` → `DEFAULT_COMPANION_PORT`),
derives the GUI's as `companion + 1` **once**, binds the static server on it, publishes the resolved port to
`/tmp/bifurc/companion-port`, and spawns the CLI with `--gui-target 127.0.0.1:<gui>`. It is also PID 1, so it
forwards `SIGTERM` and exits with the child's code. **`scripts/gui-static-server.mjs`** is the server, with
**no dependencies on purpose** — the runtime stage is `npm prune --omit=dev`, so a static-file library would
have to become a *production* dependency of a package that does not serve files. **`scripts/docker-healthcheck.mjs`**
replaces the inline `node -e` probe and reads the port the entrypoint **actually bound** rather than a number
that happened to match the `CMD`.

**The `CMD` lost `--ws-port 4747`, and that is the substantive change.** With the flag there, the port was
pinned by the image and **`BIFURC_COMPANION_PORT` would have been silently shadowed** — a flag wins over the
environment — so an operator's `-e BIFURC_COMPANION_PORT=5000` would have done nothing while the healthcheck
probed `4747`. `CMD` is now `--ws-host 127.0.0.1`, and the entrypoint supplies `--ws-port` itself. That is also
what **retires the Dockerfile's `--ws-port`/`--health-cmd` coupling caveat**: the probe and the listener now
agree by construction. `--ws-host` stays stated rather than assumed, because the guard's default and the
guard's refusal should not be the same value by accident.

The Dockerfile gained `COPY web ./web` and `COPY postcss.config.js tailwind.config.js ./` (Vite finds PostCSS
config by **walking up** from `web/`, and Tailwind's `content` globs are relative to the working directory —
and **measured 2026-09-26**: without both, `build:web` still exits 0 **and still emits a CSS file**, but it
carries **no Tailwind utilities** — 20,134 bytes and zero `w-24` occurrences against 60,042 and one, so the app
renders **unstyled** with nothing failing), `npm run build:web` **after**
`build:packages` (the browser bundle reaches the engine through `node_modules` → `dist/`, unlike Vitest, which
aliases the engine to `src`), `COPY --from=build /app/dist/web ./dist/web`, and the three scripts. `EXPOSE`
already listed `4748` from the earlier correction.

### ⚠ The image could not have built at all — `npm ci` was broken, and nothing had ever run it

This is the finding worth carrying forward, and it was **not** caused by this item. The build stage's manifest
`COPY`s have to be **exactly the workspace set `package-lock.json` describes**, because `npm ci` validates that
set. The committed lockfile was stale in two ways:

- it still described **`apps/web`** — the GUI package at its pre-extraction path, `@bifurc/web`, with
  `node_modules/@bifurc/web` linked to a directory that **does not exist** — and its root `workspaces` field
  still read `["packages/*","apps/*"]`;
- it did **not** describe `packages/gui` (`@bifurc/gui`) at all, even though `workspaces: ["packages/*"]`
  matches it.

So `npm ci` failed under **either** manifest set, measured both ways: with `packages/gui`'s manifest copied it
stopped at `EUSAGE: Missing: @bifurc/gui@0.1.0 from lock file`; without it, npm followed the dead `apps/web`
link and tried to fetch `@bifurc/gui@*` from the public registry — `404`, a private name. **The `DOCKER.md`
caveat "treat the Dockerfile as reviewed rather than exercised" was concealing a deterministic failure, not an
untested path.** The same staleness breaks `npm ci` for a fresh clone and for CI, independently of this image.

Repair, measured rather than assumed: `npm install` once reconciles the workspace set — **one key removed, two
added, zero version changes** (`node_modules/@bifurc/web` out; `packages/gui` and `node_modules/@bifurc/gui` in;
root `workspaces` → `["packages/*"]`). After it, `npm ci --ignore-scripts` against the image's exact five-manifest
set passes (`added 759 packages`, exit 0). `Dockerfile` now copies **five** manifests, with a comment stating the
rule, because the list is a coupling and the next workspace will re-break it silently.

### Verification

`tests/docker/entrypoint.test.ts` — **11 cases**, pure: the three-way precedence, the malformed-value throw, the
`65534` upper bound (not `65535` — the GUI is `+1`), and `defaultCompanionPort() === 4747` through the engine's
**CJS** build. `tests/integration/guiStaticServer.integration.test.ts` — **14 cases** over real sockets and real
files on an **ephemeral** port, so it cannot degrade any other suite's "the scan lands on `base + N`"
assertion: document and asset caching, the SPA fallback, a 404 for a missing asset, `HEAD`, `405`, encoded
traversal → `400`, `..not-a-traversal.txt` served, and `safeResolve` exercised directly.

`scripts/verify-docker-entrypoint.mjs` (`npm run verify:docker-entrypoint`) is the end-to-end one, and it is
**owner-run** because it binds `80`, `8080` and the `4747`/`4748` pair — the very thing that makes it
unusable as a suite. It starts the real entrypoint and asserts the three-way agreement:

```
[entrypoint] web GUI: http://127.0.0.1:4748 (derived, companion 4747 + 1)
[host] companion: 4747 — the web GUI belongs on 4748 (derived, companion + 1)
healthcheck                 -> exit 0, "companion 4747 accepted a connection"
GET /            (document) -> 200 text/html
GET /deep/route  (document) -> 200 text/html     <- the SPA fallback, live
GET /assets/nope.js  (*/*)  -> 404 text/plain    <- a missing asset is not answered with HTML
Host: localhost             -> 200 text/html     <- the edge routes the base domain to --gui-target
```

**Two anomalies from the first live run were attributed, and neither was a defect — but one of my own earlier
notes was wrong.** `/deep/route -> 404` was **my probe**: curl's default `Accept` is a wildcard, and the
fallback keys on `text/html`, so the same request with a document `Accept` returns `200`. `healthcheck exit=23`
was a **mislabelled `$?`** — the script's own status is `1`/`0` (measured against a known-free port), `23` is
not a Node exit code, and bash propagates node exits faithfully. The correction: I had recorded the entrypoint's
`SIGTERM` exit as `143` *"propagated"*. **That number came from the shell, not from the entrypoint.** Measured
directly, a Node child on Windows **cannot catch `SIGTERM` at all** — `process.kill(pid,"SIGTERM")` terminates it
unconditionally, the handler never ran, and the parent saw `code=null signal=SIGTERM`. So the forwarding path is
**not measurable on this machine** and is exercised by `docker stop` in the container; the record now says that
instead of claiming a measurement.

### Item 5 does not depend on item 4

§5's table listed item 4 as *"the companion-then-GUI startup order"* with an "entrypoint half open", which reads
as a prerequisite. It is not one: **the entrypoint pins the companion port, so no scan ever happens**, and the
static server neither knows nor cares where the companion is — nothing is *launched* for the GUI to be told a
port, because the handoff to the browser goes through `--gui-target` and the connection form. So item 5's image
half is complete on its own, and item 4's remaining half is the Electron one (`plan/16`).

**Found, not fixed — two, carried over from item 6's record and still open**: `useWebhooksPanelState.ts:26`'s
stale `?? 9101`, and nothing in the GUI surfacing `forwardProxyError` / `webhookError`.

**What remains, and it is the owner's:** the `docker build` and a container run. Docker Desktop was not running
and starting it is a desktop action. The step that would have failed first is now verified offline.

---

## 6. Open items — all three ANSWERED 2026-09-25

- **`BIFURC_PROXY_BASE_PORT` sets `forwardProxyPort` outright** (owner decision). Not removed, because it is
  documented in `Dockerfile`, `docker-compose.yml` and `DOCKER.md`; not a scan base, because there is no scan.
  `store/appSettings.ts#forwardProxyPortFromEnv()` reads it, returns `null` when unset, and **throws** on a
  malformed value rather than falling back — a bad value must fail on every path, not only on the one that
  finds a file. `applyEnvOverrides()` lays it over the stored settings, environment-wins, so a container's `-e`
  beats a value an earlier run left in the volume.
- **The image's entrypoint starts the GUI's static server and passes the resolved port in via `--gui-target`**
  (owner decision). The engine does **not** serve it. That keeps `plan/15`'s *"a target, not a directory"* model
  intact and is the smaller change. **Built 2026-09-26** (`scripts/docker-entrypoint.mjs`), as item 5's half —
  and it does **not** wait on item 4: pinning the companion port means no scan ever runs, so the GUI's port is a
  fact the entrypoint hands to both sides rather than something the host discovers.
- **The extension's discovery contract.** Unchanged: the base and the pair rule are **published** protocol
  facts and belong in the extension's own documentation when its half is written. `findPortPair()`'s header is
  the current statement of the rule.

---

## 7. ✅ Verification status — RESOLVED 2026-09-25

**The blocker this section used to describe was STALE**, and it was re-measured before anything was touched
rather than taken on trust: `npx vitest run packages/engine/tests/lib/net.test.ts` → **11/11 passed**. No
`npm ci` was needed and `npm ls` was fine. The diagnosis below was from a previous session's environment.

**Baseline, measured before any edit:** `2,766 tests / 119 files` — 2,726 passed, 0 failed, 40 skipped.
Unit **2,301 / 88**, integration **465 / 31** (`2,301 + 465 = 2,766` ✓).

### ⚠ §4's "the delta should be zero" was FALSIFIED — by a real, attributable regression

The first run after §4 alone produced **10 failures**, all in `tests/store/config.test.ts`:
`No "DEFAULT_WEBHOOK_PORT" export is defined on the "@bifurc/engine/store/appSettings" mock`.

The mechanism: §4's `appSettings.ts` work **removed** `DEFAULT_WEBHOOK_PORT` (replaced by
`WEBHOOK_PORT_BAND_MIN/MAX` + `randomWebhookPort()`) and **added** `DEFAULT_FORWARD_PROXY_PORT` /
`DEFAULT_COMPANION_PORT`, while the test's `vi.mock` replaced the **whole module** — so every `DEFAULT_*` that
`config.ts` reads came back `undefined`. Fixed by spreading `importOriginal` into the mock. **§4 was additive to
the product and destructive to a mock**, which is exactly the shape a zero-delta prediction cannot see.

### Final state

| | tests | files |
|---|---|---|
| unit | 2,281 | 88 |
| integration | 464 | 30 |
| **full** | **2,745** | **118** |

`2,281 + 464 = 2,745` ✓, **both halves measured**, 2,704 passed, 40 skipped.

**Re-measured after item 7 landed, and the totals did not move:** `2,745 / 118`, now **2,705 passed, 40 skipped,
0 failed** — the `gitOps` `EBUSY` flake did not reproduce. Item 7's delta is `0 tests / 0 files` because it
**replaced** two cases rather than adding any, which is why the figures above still stand.

**Delta −21 tests / −1 file, reconciled in §5's table.** **There is no product failure.** The one failure a full
run reports is the **documented environment flake**: `tests/sync/gitOps.test.ts` → `EBUSY: resource busy or
locked` from the safe-delete shim, which **passes alone (4/4, zero diff)** and is already an entry in
`TRAPS.md`. It has no relationship to a port.

### A regression the deletion introduced, and how it was caught

Deleting `WorkspaceRuntime` removed the **readiness wait** it performed — `await Promise.all([waitUntilReady(proxy…),
waitUntilReady(webhook…)])` in its `doStart()`. So `host.start()` began resolving **before its listeners bound**,
and since `status()` reads a port through `isRunning()` (`server.listening`), `forwardProxyPort` and `edgePort`
came back `null` for listeners that came up milliseconds later. Caught by
`workspaceHost.integration.test.ts` the moment the class was gone; fixed by restoring `waitUntilReady()` in
`host.ts` with the contract written down. **A deletion takes more than the thing you meant to delete.**

A second, smaller one the same refactor introduced: `host.start()` binds the **process-global** facades
(`bindProxyInstance` / `bindWebhookInstance`) and `stop()` initially left them bound, contradicting the rule
`facadeBinding.integration.test.ts`'s own teardown records. `stop()` now unbinds.

---

## 8. What the old §7 said (kept for the record, since it cost a session)

<details>
<summary>The stale diagnosis — do not act on it</summary>

> **No suite can run, and the refactor in §5 must not proceed unverified.** Every file in the unit project
> fails at **collection** with `TypeError: Cannot read properties of undefined (reading 'config')`, pinned to
> the *last line* of the test file, with `import 0ms` in the phase timings — so no module is ever imported and
> the failure is in the **setup** phase. … **What is left:** `npm ls` — for *any* package — fails with an
> npm-internal `Cannot read properties of null (reading 'edgesOut')`, which points at `node_modules`/lockfile
> **integrity**. The repair is `npm ci`…

**Re-measured 2026-09-25: none of it reproduces.** The runner works, `npm ls` works, and `npm ci` was not run.
The lesson is `TRAPS.md`'s own: *a failure you cannot attribute is a question, not a finding* — and a recorded
failure needs a re-measurement date before it is treated as a standing fact.

</details>
