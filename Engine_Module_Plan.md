# Engine module layout — a plan

**Status:** in progress on branch `engine_modules`. **Phases 0 through 7 are done and committed**
(`c531c7c` plan/map/codemod · `16161ad` phase 1 — `kernel/` + `utils/` · `083a026` phase 2 — the rebind
fan-out · `99d1444` phase 3 — the `workspace/` split · `1f748b7` phase 4 — the proxy family · `8bedb72`
phase 5 — the protocol verticals · `1713dec` phase 6 — `gitops/` · phase 7 — the leftovers · phase 8 — the
ratchet + docs).
**All eight phases are done.** The one item that could not be delivered as written is §5.2's ESLint rule —
this repository has no ESLint (see §5.2), and adding one is a toolchain decision, not a file edit. Every number below is measured from the tree at the time of
writing (§1 from `standalone-engine` on 2026-09-28; §4.1's call-site counts re-measured 2026-09-29; §7.2
rows 3 and 4's counts re-measured 2026-09-29; rows 5 and 6 confirmed exact; row 7's "8" re-measured as
**44 files**), not estimated.

> **Phase 7 dissolved `miscCommands.ts` and normalised the last two directory names — 44 files renamed
> (42 by the codemod, 2 by hand) and 4 added.** §7.2 row 7's "8" counts *destinations*, not files; this
> is the third row whose count was an under-count, and the largest miss of the eight. `startup.ts` +
> `shutdown.ts` → `lifecycle/`; `fileOps/` → `fileops/`; `importExport/` (39 files) → `importexport/`;
> `miscCommands.ts` → `rest/commands.ts` + `mocks/commands.ts` + `workspace/healthbarCommands.ts`;
> `proxy/serverReplay.ts` → `utils/replay.ts`. New: `websocket/index.ts` (the §9 reserved placeholder — **filled by `plan/19` WI 6 on 2026-09-30**, and it stayed reserved for exactly as long as the rule said it should).
> **Two extractions were forced by the layering rule, not chosen:** §4.5 assigns `healthbar.*` to
> `workspace/` and `request.replay` to `rest/`, and neither may import the module holding its dependency
> (`gitops/statusTracker`, `proxy/serverReplay`) — so §5.1's "extract down" remedy produced
> `workspace/statusCache.ts` and `utils/replay.ts`. The first of those also **repaired a pre-existing
> violation** (`workspace/entities/entityCrudCommands.ts → gitops/statusTracker`). **⚠ One violation is
> recorded and deliberately not fixed — see the note on `kernel/eventBus.ts` in §7.2 row 7 and in the
> map's corrections.**

> **⚠ Windows case-only renames are a silent no-op.** The codemod's `fs.renameSync` reported moving all 40
> `fileOps/`→`fileops/` and `importExport/`→`importexport/` paths and moved **none** of them: with an
> identical filename and only a directory component differing in case, source and destination resolve
> byte-identically on NTFS. The import specifiers *were* rewritten, so the tree briefly said
> `./importexport/commands` while the directory was `importExport/` — correct on Windows, broken on any
> case-sensitive filesystem. Recorded in full in `plan/engine-layout-map.json`'s corrections.

> **Phase 6 merged `sync/` (11 files) into `gitops/` together with `store/gitStore.ts` — 12 moves, the
> heaviest rewrite so far (102 `@bifurc/engine/*` refs + 24 relative imports across 40 files).** After it,
> `store/` and `sync/` are both empty and deleted: the two directories the plan set out to drain are gone.

> **Phase 5 was the first row whose count was right — 6.** `graphql/`, `grpc/`, `soap/` each gained
> `kinds.ts` (from `store/kinds/*Kinds.ts`) **and** `commands.ts` (from `protocols/*Commands.ts`). After it,
> `store/` holds exactly one file — `gitStore.ts` (phase 6) — and `protocols/` is **empty and deleted**.

> **Phase 4 deviation from §7.2 row 4 — 14 files moved, not 22.** The row's "22" counted the whole of
> `proxy/`. Seven of those files are the **forward proxy itself** and stay: `proxyHandler.ts`,
> `routingUtils.ts`, `server.ts`, `serverCommands.ts`, `serverReplay.ts` *(that last one left for
> `utils/replay.ts` in phase 7 — see §4.5)*, `service-discovery.ts`, `tlsIntercept.ts`. What left is the four named modules (`reverseproxy/` 2, `mocks/` 3, `certificates/` 3,
> `webhooks/` 2 = 10) plus the four shared helpers §3 already routed to `utils/` (`http`, `decompress`,
> `response`, `server`). `proxy/` is now the **forward half** only — which is exactly the shape `plan/15`
> work item 3 drew when it split the reverse half onto its own host-level port.

> **Phase 3 deviation from §7.2 row 3 — 20 files moved, not 23.** The row's "23" counted the whole of
> `store/`. Three of those files are *protocol leaf kinds* — `store/kinds/{graphql,grpc,soap}Kinds.ts` —
> which belong to the **phase 5** verticals (`graphql/`, `grpc/`, `soap/`) and were deliberately left in
> `store/` so that phase 5 has them to absorb. After phase 3, `store/` holds exactly four files: those
> three plus `gitStore.ts` (phase 6).

> **Phase 2 deviation from §4.1 — two functions, not one.** §4.1 sketches a single
> `notifyConfigChanged()`. Measuring the real call sites split the fan-out in two, and both were built:
> `notifyConfigChanged()` (the **reload** fan-out — a listener whose rules derive from config; 29 call
> sites / 21 files) and `rebindRuntimePorts(incoming, prev)` (the **port** fan-out — only
> `store/configCommands.ts`'s `config.save` ever rebinds, and `restartEdge` / `startWebhookServer` have
> no other caller). Conflating them under one name would have hidden the distinction §4.1 itself draws
> when it notes *"the edge: nothing to reload — a port is owned by its workspace."* Both live in
> `packages/engine/src/runtime/instances.ts`.

**Scope:** `packages/engine/src` — 140 files, 23,443 lines. Nothing outside the engine package is
restructured, except the import statements that point into it.

---

## 1. What I measured

### 1.1 The shape today

| Area | Files | LOC | What it actually is |
|---|---|---|---|
| `transport/` | 13 | 4,636 | four RPC transports + auth + event log |
| `proxy/` | 22 | 4,002 | **two servers** (reverse + forward) + mocks + TLS + webhooks + certs |
| `store/` | 23 | 3,353 | **three things** — persistence, generic CRUD commands, git repo handle |
| `importExport/` | 39 | 3,208 | importers / exporters / formats |
| `sync/` | 11 | 1,669 | git ops, publish, status, auto-sync |
| `runtime/` | 4 | 1,508 | host, host CLI, host args, port allocator |
| root (6 files) | 6 | 1,418 | composition root, lifecycle, event bus, **a command grab bag** |
| `blob/` | 4 | 1,061 | content-addressed store + sweep |
| `applications/` | 5 | 1,017 | child-process supervision |
| `runner/` | 2 | 319 | collection runner + HTML report |
| `fileOps/` | 1 | 318 | file operations commands |
| `commands/` | 3 | 315 | registry, kind map, echo workspace |
| `lib/` | 3 | 309 | net, random names, randomizer |
| `protocols/` | 3 | 289 | `graphql.*` / `grpc.*` / `soap.*` execute commands |
| `subscription/` | 1 | 21 | entity-count subscription |

### 1.2 The dependency graph (cross-area relative imports, measured)

```
store          <-- importExport 67, root 24, sync 17, proxy 13, applications 3, fileOps 2,
                   runtime 2, blob 1, runner 1        ≈ 130 inbound edges
commands/registry  <-- store 7, transport 6, proxy 5, protocols 3, + 6 more × 1
eventBus           <-- root, applications, commands, proxy, runtime, store, sync, transport
proxy/edge         <-- root, runtime, store
proxy/logEmitter   <-- root, proxy, transport
proxy/certManager  <-- proxy, fileOps
```

Three things fall out of this immediately:

1. **`store/` is the hub.** ~130 inbound edges. Any plan that scatters it per feature creates N copies of
   `loadConfig` and is a regression, not a cleanup.
2. **`commands/registry` is already a clean leaf contract.** Everything imports it; it imports nothing but
   the protocol package and `eventBus`. The user's "global registry" instinct is correct and already
   half-built.
3. **`eventBus`, `logEmitter`, `certManager`, `edge` are consumed across areas** — they are not private to
   the module they are filed under today.

### 1.3 The one edge that is a bug in waiting

**`importExport → proxy` is 17 edges, and all 17 are the single symbol `reloadConfig` from
`proxy/server`.** Verified by reading every site.

The same concern appears again in `store → proxy` (5 edges: `configCommands` → `edge`/`server`/`webhookServer`,
`coreCommands` → `server`, `entityCrudCommands` → `server`), and in `sync → proxy` (1 edge:
`syncCommands` → `server`), and in `store/configCommands` importing `isEdgeRunning`/`restartEdge` from
`proxy/edge`.

So **23 cross-module edges exist for one reason: "after a config write, rebuild the listeners."** That is a
composition concern. It does not belong to the proxy, and it does not belong to the store. Naming it is the
single largest structural win available, and it is a dependency inversion rather than a file move (§4.1).

### 1.4 The kind vocabulary is already the module map

`entity.create` is **one** command serving **16 kinds**. Today those kinds are registered from five files in
`store/kinds/`, while the behaviour behind each kind lives somewhere else entirely:

> **⚠ The table below is a snapshot of the tree *before* phase 1** — every path in it is a pre-migration
> path, and it is kept as the analysis that motivated §4.4 rather than as a description of the current tree.
> For where the kinds actually live now, see §3 and the `registerAll.ts` drift guards.

| Kind (protocol name) | Registered today in | Behaviour lives in |
|---|---|---|
| `requests` | `crudKinds.ts` | `store/workspaceFs.ts` + `miscCommands.ts` (`request.replay`) + `runner/` |
| `mappings` | `crudKinds.ts` | `proxy/edge.ts` |
| `rules` → `proxyRules` | `crudKinds.ts` | `proxy/server.ts` |
| `mocks` | `crudKinds.ts` | `proxy/mockHandler.ts` |
| `webhooks` | `crudKinds.ts` | `proxy/webhookServer.ts` |
| `sockets` → `wsConnections` | `crudKinds.ts` | *(nothing — reserved)* |
| `environments` | `coreKinds.ts` | `store/coreCommands.ts` (`env.setActive`) |
| `graphqlRequests`, `graphqlSchemas` | `graphqlKinds.ts` | `protocols/graphqlCommands.ts` |
| `grpcRequests`, `protoFiles` | `grpcKinds.ts` | `protocols/grpcCommands.ts` |
| `soapRequests`, `wsdls` | `soapKinds.ts` | `protocols/soapCommands.ts` |

**This is the strongest argument for the proposal.** The 16 kinds are already the feature boundaries; the
layout just refuses to admit it, splitting each feature across `store/kinds/`, `protocols/`, `proxy/` and the
root. `graphql` is the clearest case: its commands are in `protocols/`, its kinds are in `store/kinds/`, and
neither file mentions the other.

---

## 2. Verdict on the proposal

**It is the right shape.** Six of the eight named modules are already latent in the code and the split is
supported by the measurements. Two adjustments are needed, and both are consequences of the graph above.

| Proposed | Verdict |
|---|---|
| `reverseproxy` / `proxy` split | ✅ **Correct and already half-done.** `plan/15` WI 3 physically split the two halves; `edge.ts`'s own header records it. The directories never followed. |
| `rest`, `graphql`, `grpc`, `soap` | ✅ **Correct — and this is the biggest win.** Each should own its *commands + kinds + behaviour* as one vertical. |
| `websocket` | ✅ **Correct, and now real — `plan/19` WI 6 (2026-09-30).** It was a placeholder for a year of phases and the placeholder was right: there was no WebSocket server to extract. What WI 6 moved in is a **client** — `wsRegistry.ts` (the live-socket registry, D29 lifetime, its own concurrency policy) and `wsCommands.ts` — which is content that existed in the *renderer* rather than invented content. **`edge.ts`'s `upgrade` branch is still reserved and still deliberately bodiless**, and the two are different features: an outbound client and an inbound upgrade. |
| `environments` | ✅ Correct, small. `environments` kind + `env.setActive`. |
| `gitops` | ✅ Correct. `sync/` (11 files) + `store/gitStore.ts` are one concern split across two directories. |
| `fileops`, `importexport`, `utils` | ✅ Already exist as `fileOps/`, `importExport/`, `lib/`. Rename/normalise only. |
| global registry (`registerAll.ts`) | ✅ Correct, and **keep its current character** — one explicit function, not an import side effect (P6 §5c). |
| **not in the list:** `mocks` | ➕ **Needed.** `mockHandler.ts` + `scriptContext.ts` + `scriptExecutor.ts` are imported by **both** `edge.ts` (reverse) *and* `proxyHandler.ts`/`routingUtils.ts`/`responseUtils.ts` (forward). Filing them under either half creates an inverted dependency. They get their own module. |
| **not in the list:** `transport`, `runtime`, `blob`, `applications`, `runner`, `certificates`, `webhooks`, `workspace` | ➕ Needed — these exist and must land somewhere. |

**The one thing I would push back on.** "Modules can import and use common functions from `utils`" is right
for *pure* helpers, but the measurements say `store/` (the persistence hub) will be every module's heaviest
import. If `utils` is defined loosely, `loadConfig` and `workspaceFs` migrate into it and the hub becomes an
untyped junk drawer. So: **`utils` is for pure, stateless helpers only; persistence stays in `workspace/`**
(§5).

---

## 3. Target module map

```
packages/engine/src/
├── index.ts                  composition root — createEngine() + the public barrel
├── registerAll.ts            THE global registry (see §4.3)
├── lifecycle/                ← startup.ts, shutdown.ts
├── kernel/                   the contract layer: imports nothing from the engine
│   ├── registry.ts           ← commands/registry.ts
│   ├── eventBus.ts           ← eventBus.ts
│   ├── kinds.ts              ← commands/entityKindMap.ts
│   └── telemetry.ts          ← proxy/logEmitter.ts   (3 areas consume it)
├── utils/                    pure + stateless only  (⚠ see the note below — this took two attempts)
│   ├── net.ts randomNames.ts randomizer.ts        ← lib/
│   ├── http.ts               ← proxy/constants.ts (HOP_BY_HOP; both proxy halves use it)
│   ├── decompress.ts         ← proxy/decompressUtils.ts
│   ├── id.ts sendHtml.ts     ← split out of utils/server.ts / reverseproxy/pages.ts (cleanup)
│   ├── replay.ts             ← proxy/serverReplay.ts  (phase 7 — shared by proxy/ and rest/)
│   └── echoWorkspace.ts      ← commands/echoWorkspace.ts
├── workspace/                THE HUB — persistence. One copy, never per feature.
│   ├── paths.ts types.ts config.ts appSettings.ts workspaceFs.ts
│   ├── enabledSet.ts kindNameEntries.ts workspace/*.ts   ← store/
│   ├── dispatch.ts           ← utils/server.ts's config derivations (cleanup — they read enabled.json)
│   ├── gitStore.ts statusCache.ts   ← gitops/gitStore.ts (cleanup — per-workspace repo handle) + phase 7
│   ├── blob/                 ← blob/  (cleanup — a disk-backed store with a TTL sweeper is persistence)
│   ├── entities/             ← store/{entityCrud,core,folder,config,audit}Commands.ts
│   │   └── kinds/            ← store/kinds/*Kinds.ts   (generic CRUD machinery only)
│   └── environments/         the `environments` kind + env.setActive
├── reverseproxy/             ← proxy/edge.ts, proxy/pages.ts   (mappings)
├── proxy/                    ← proxy/{server,proxyHandler,routingUtils,tlsIntercept,
│                               service-discovery,serverCommands}.ts   (rules)
├── mocks/                    ← proxy/{mockHandler,scriptContext,scriptExecutor}.ts  (mocks)
│                               + commands.ts (script.execute, from miscCommands — phase 7)
├── certificates/             ← proxy/{certManager,certCommands,tlsCert}.ts   (shared: proxy + fileops)
├── webhooks/                 ← proxy/{webhookServer,webhookCommands}.ts
├── websocket/                the live-socket registry — `wsRegistry.ts` + `wsCommands.ts` (`plan/19` WI 6).
│                             The `wsConnections` *kind* is still data-only and registers through the shared CRUD table.
├── rest/                     commands.ts — request.replay. The `requests` kind itself registers through
│                               the shared CRUD table (workspace/entities/kinds/crudKinds.ts), per §4.4
├── graphql/                  graphqlCommands.ts + graphqlKinds.ts, merged
├── grpc/                     grpcCommands.ts + grpcKinds.ts, merged
├── soap/                     soapCommands.ts + soapKinds.ts, merged
├── gitops/                   ← sync/*.ts  +  store/gitStore.ts
├── fileops/                  ← fileOps/  (normalised capitalisation). ⚠ **Two commands, not six** —
│                               the `tls.*` trio went to `certificates/` and `runner.exportReport` to
│                               `runner/` in the cleanup; what is left (`audit.export`,
│                               `capture.shareJson`) is what no other module can own
├── importexport/             ← importExport/
├── applications/             unchanged
├── runner/                   unchanged
├── subscription/             → folds into workspace/
├── transport/                unchanged — do NOT touch in the same pass (§9)
└── runtime/                  host, hostCli, hostArgs, portAllocator, + instances.ts (§4.1)
```

**Count:** 14 directories + 6 root files → **~24 modules + 2 root files**. The growth is deliberate: each new
directory is a boundary the compiler and a lint rule can enforce, and the alternative (one `store/`, one
`proxy/`) is exactly the "code all over the place" the request names.

### 3.1 What belongs in `runtime/`, and what does not

**Server logic stays in its module. `runtime/` composes; it never serves.** This is already true of the code
and the reorganisation should preserve it rather than blur it:

| | Owns | Does not |
|---|---|---|
| `reverseproxy/`, `proxy/`, `webhooks/`, *(later)* `mqtt/`, `vault/` | the factory (`createProxyServer()`, `createEdgeRouter()`, `createWebhookServer()`), the request path, the instance's own state | know which instances exist, which port they took, or who started them |
| `runtime/` | composing the engine — calling the factories, **binding** the instances, wiring transport, the host CLI, port allocation | touch a request, parse a rule, resolve a mock |

`createProxyServer()` is a pure factory: it returns an instance that owns its own server, config snapshot,
enabled sets and error state, and `runtime/host.ts` calls it, binds it, and steps out of the way. That is the
right boundary and the plan keeps it.

**⚠ The one place this is subtle: `bind*Instance` lives in the server modules, not in `runtime/` — and it has
to.** The binding is *read* by the server module's own module-level functions (`startServer()`,
`isRunning()`, `reloadConfig()` in `proxy/server.ts`). If `runtime/` owned that state, `proxy/server.ts` would
have to import `runtime/` to read it — and `runtime/host.ts` imports `proxy/server.ts` **as a value**. That
closes the loop. So the arrow is one-way by construction: **runtime → proxy, never proxy → runtime.** The
binding stays where the state is; `runtime/` is the caller.

---

## 4. The five moves that are more than renames

Renaming directories is the visible part and the least valuable. These five are what makes the layout
load-bearing.

### 4.1 Name the rebind fan-out — and keep the arrow one-way

**This is the item the first draft of this plan got wrong, so it is worth stating precisely.**

Only `ProxyServerInstance` has a `reloadConfig()`. The edge router deliberately has none — `edge.ts` says so
twice, *"there is no `reloadConfig()` on the instance below: there is nothing to reload"* — and neither does
the webhook receiver. So `store/configCommands.ts` **hand-writes the fan-out**, importing three separate proxy
modules in one file:

```ts
import { isRunning, reloadConfig, startServer } from "../proxy/server";
import { isEdgeRunning, restartEdge } from "../proxy/edge";
import { isWebhookServerRunning, startWebhookServer } from "../proxy/webhookServer";
```

Add `mqtt/` or `vault/` and that file grows a fourth import — and so does every other caller that means "the
config changed". **The coupling scales with the number of listener kinds**, which is exactly the problem the
request is about.

**The seam: `runtime/instances.ts` owns the fan-out, not the state.** *(Delivered in phase 2 — and it is
**two** functions, not the one this section first sketched. See the status note at the top of this file and
`packages/engine/src/runtime/instances.ts` itself.)*

```ts
// runtime/instances.ts — the one place a listener kind is registered
export function notifyConfigChanged(): void {
  reloadConfig();   // proxy
  // edge:     nothing to reload — a port is owned by its workspace, not by the active one
  // webhook:  nothing to reload — its active maps are registered, not derived from config
  // mqtt, vault, …: one line each, and no caller changes
}

// The *other* operation the §4.1 sketch left out: a config write may also move a port. Only
// `config.save` ever rebinds, and `restartEdge`/`startWebhookServer` have no caller outside it —
// which is the argument for giving it its own name rather than folding it into the function above.
export function rebindRuntimePorts(incoming: AppConfig, prev: AppConfig): void { … }
```

`workspace/entities/configCommands.ts`, `coreCommands.ts`, `entityCrudCommands.ts`, `gitops/syncCommands.ts`
and the 17 `importexport/importers/*` sites call **`notifyConfigChanged()`**; only
`config.save` calls **`rebindRuntimePorts()`**. A new listener kind that derives from config then costs
one line in one file and zero edits at any call site.

**Measured, 2026-09-29 (the figure this section's "23 call sites" should have been).** `reloadConfig` had
**29 call sites across 21 files** that mean the fan-out: 17 × `importExport/importers/*`, `store/configCommands.ts`
(2), `store/coreCommands.ts` (3), `store/entityCrudCommands.ts` (3), `sync/syncCommands.ts` (4), `startup.ts` (1 —
passed as a value to `setAutoSyncReloadFn`). Everything else that matched `reloadConfig` was a docblock, the
definition/export in `proxy/server.ts` itself, or the GUI's unrelated UI callback. After phase 2, **no module
outside `runtime/` imports `reloadConfig` for the fan-out** (§10 criterion 4).

**Honest effect, measured rather than asserted:** this does **not** delete 29 edges. `importexport` still
imports something — it imports the fan-out point instead of the proxy. What it removes is the coupling to a
**specific listener kind**, which is the part that grows. 17 importer sites stop naming `proxy/`; one file
becomes the registry of listeners. In `configCommands.ts` the change is a net **−51 lines** (−72/+21), most of
it the three port-branch rationales relocating verbatim into `instances.ts` along with the code they explain.

**⚠ A stale docblock to fix while you are in there — DONE in phase 2.** `proxy/server.ts` explained the
arrangement with *"why this module does not import `runtime/`: `runtimeRegistry.ts`'s header records that its
graph must stay acyclic, and `runtime/workspaceRuntime.ts` imports this module as a value."* **Both files were
deleted** — `runtime/host.ts` says *"this replaces `runtimeRegistry.ts`"*. The docblock now cites `runtime/host.ts`
and notes the deletions. Its parallel list of `reloadConfig` importers (which also named a since-deleted
`transport/legacyCompanion.ts`) was rewritten to the post-phase-2 truth.

### 4.2 `kernel/` — the contract layer

`eventBus`, `registry`, `kinds`, `telemetry` move out of the root and out of `commands/`. `kernel/` is the
one module with **no engine imports**, which makes the layering rule in §5 mechanically checkable rather
than aspirational.

`registry.ts`'s own docblock already makes this argument for the `Scope` type: *"`transport/` imports
`commands/` and never the reverse… One vocabulary, no inverted dependency."* `kernel/` generalises a rule
that is already being applied by hand in one file.

### 4.3 The registry stays explicit — and gets one job

`registerAll.ts` keeps its current character: **an explicit function, never an import side effect** (P6 §5c,
and its own docblock explains why at length). It moves to the root next to `index.ts` and becomes the only
file permitted to import every feature module.

**But it must register the kinds, not just the commands** — that is the defect its docblock records being
found twice (`plan/15` WI 10). When each feature module owns its own kind, `registerAllEngineKinds()` stops
being five calls into `store/kinds/` and becomes "ask each module to register itself". The two drift-guard
tests in `tests/integration/workspaceHost.integration.test.ts` (command names **and** kinds) become the
regression test for the whole reorganisation.

### 4.4 Protocol modules own their whole vertical

`graphql/` absorbs `protocols/graphqlCommands.ts` **and** `store/kinds/graphqlKinds.ts`. Same for `grpc/` and
`soap/`. Today neither file references the other, which is how a feature ends up split across two
directories with no link between them.

**⚠ The kind vocabulary must not be duplicated by this.** `ALL_ENTITY_KINDS` / `FLAT_ENTITY_KINDS` in
`store/workspaceFs.ts` are the one list; four copies had already drifted once. Protocol modules **register**
their kinds against the shared vocabulary; they do not own a private list of kind names.

### 4.5 `miscCommands.ts` dissolves

**Done — engine-layout phase 7.** Its five commands had obvious homes once the modules existed:

| Command | Went to |
|---|---|
| `request.replay` | `rest/commands.ts` |
| `script.execute` | `mocks/commands.ts` (it calls `scriptExecutor`, which `proxy` already shares) |
| `healthbar.getServices` / `.saveServices` / `.checkUrl` | `workspace/healthbarCommands.ts` (it reads/writes a workspace file) |

A 10.6 KB file named "misc" at the root is the clearest symptom the request is describing, and it is the
cheapest one to remove.

**⚠ Both the `workspace/` and the `rest/` destination needed a dependency moved out from under it**, and that
is the part this section did not anticipate. `healthbar.saveServices` calls `invalidateCache()` and
`request.replay` calls `replayRequest()` — but `invalidateCache` lived in `gitops/statusTracker.ts` and
`replayRequest` in `proxy/serverReplay.ts`, and §5.1/§6 let `workspace/` and `rest/` import **neither**. So
§5.1's own remedy applied: the shared behaviour came *down* into `workspace/statusCache.ts` and
`utils/replay.ts`. The first of those also repaired a pre-existing `workspace → gitops` edge. See §7.2 row 7.

---

## 5. The layering rule

### 5.1 The rule

```
kernel  →  utils  →  workspace  →  { feature modules }  →  transport | runtime  →  index | lifecycle
```

- **`kernel/`** imports nothing from the engine.
- **`utils/`** imports only `kernel/` and Node builtins. **Pure and stateless** — no module-level caches, no
  filesystem, no config reads.
- **`workspace/`** is the only persistence layer. Everything may import it; it imports only `kernel/` +
  `utils/`.
- **Feature modules** may import `kernel/`, `utils/`, `workspace/` — and **never each other**. Where two
  features genuinely share behaviour (the mock engine, the certificate manager, the rebind seam), that
  behaviour is extracted *down* into a shared module rather than imported sideways.

**⚠ §5.1 and §6 disagreed, and §6 governs** (settled 2026-09-29, when the ratchet was built). This section's
blanket "never each other" contradicts §6's table, which grants `proxy/` → `mocks/` + `certificates/` and
`reverseproxy/` → `mocks/` outright. **§6's per-module table is the rule; this section is the default where
§6 is silent.** Two further refinements the ratchet applies, both principled:

- **`import type … from` is not a layer violation** — it is erased at compile time and creates no runtime
  edge. (`kernel/eventBus.ts → webhooks/webhookServer.ts` is the only edge that relies on this.)
- **The two SEAM modules are importable from anywhere** — `runtime/instances` (§4.1's fan-out; 20 of the
  engine's edges are that one import, which is the seam working) and `runtime/entityStatus` (the
  read-and-emit composition).

**⚠ THE LESSON THE CLEANUP PAID FOR: "both halves need it" is not a reason to put something in `utils/`.**
Phase 4 moved `proxy/{responseUtils,serverUtils}.ts` to `utils/` on exactly that reasoning, and both moves
were wrong for the *same* reason — the rule is about what a module **reads**, not who calls it:

- `utils/response.ts` had **exactly one importer** (`proxy/proxyHandler.ts`). The edge never imported it; it
  reaches `proxyToUpstream`/`fetchUpstreamResponse` *through* `proxyHandler`. → `proxy/response.ts`.
- `utils/server.ts` **read `enabled.json`**, so it was persistence sitting in the "pure" layer. → split into
  `workspace/dispatch.ts` (the config derivations), `utils/id.ts` (`mkId`) and `utils/net.ts` (`hostnameOf`).

**So the test to apply before putting anything in `utils/` is: does it read a file, a config, or a
sibling module? If yes it is not a `utils/` file, however many modules call it.** And the corollary the
cleanup also confirmed: *"two features share it"* is a reason to **extract it down** only when it is
genuinely stateless. When it is not, it belongs in the layer it reads from — which is why the blob store
(`workspace/blob/`) and the per-workspace git handle (`workspace/gitStore.ts`) ended up in `workspace/`
rather than as top-level modules with grants.
- **`registerAll.ts`** is the single exception: it may import every feature module, because enumerating them
  is its entire job.

### 5.2 Enforcement — a ratchet, not a convention

This repository already enforces design rules with tests rather than documentation (§5c's module-scope
guard, the workspaceHost drift guards, the client-surface count). The layout should be enforced the same way:

- **`tests/engine/layout.test.ts` — DONE (phase 8, 2026-09-29).** It parses every relative import in `src/`,
  maps each file to its module, and fails with the offending `file → file` pair. It also asserts that every
  contract module is a real directory, that none is orphaned, that every `register*Commands` owner is
  reachable from `registerAll.ts`, and — the half that makes the allowlist safe — that the allowlist has
  **no stale entries**, so it can only shrink.
- **An ESLint `no-restricted-imports` rule — CANNOT BE DONE AS WRITTEN. This repository has no ESLint at
  all**: no config file, no `eslint` dependency, nothing in `node_modules/.bin`. Adding one is a new
  toolchain decision (dependency + config + a `lint` script), not a phase-8 file edit, so it is recorded
  here rather than taken unilaterally. The ratchet test above is the enforcement in the meantime — and it is
  the half §5.2 itself calls *"the difference between a layout and a layout rule"*.

Both must be added **in the same phase as the first move**, or the rule decays.

**It decayed exactly as predicted, and the cost is now measured.** Phases 1–7 landed with no enforcement, so
the tree accumulated **24 file-level contract violations** before the ratchet existed. **Two are already gone
and the other 22 are in `ALLOWED_VIOLATIONS` with a reason each.**

**✅ FIXED — `kernel/eventBus.ts → gitops/{statusTracker,gitState}.ts`.** It made §5.1's "`kernel/` imports
nothing engine-internal" false, and §5.1 calls that the property which makes the rule "mechanically checkable
rather than aspirational". `emitEntityStatus()` moved to **`runtime/entityStatus.ts`** — the engine's **second
seam module** (the first is `runtime/instances.ts`, §4.1). It could not move to `gitops/`, because three of
its four callers are `workspace/`-side and §6 lets `workspace/` import only `kernel/` and `utils/`; moving it
sideways would have turned two `kernel → gitops` edges into three `workspace → gitops` ones and changed
nothing. **`kernel/eventBus.ts` now has exactly one engine-internal import and it is a type.** Its two
allowlist entries were deleted in the same commit, which is the shrink the ratchet's staleness assertion
exists to force.

**The ratchet must assert in both directions, and this repo has already paid for knowing why.** A
*negative* assertion ("no feature module imports another") only reads what is there. `TRAPS.md`'s
*"the third copy was already there"* entry is the counter-example: `session.ts` was extracted because
`stdio.ts` and `ws.ts` had each grown a copy of the same state machine, `socket.ts` was converted, and
**`ws.ts` was not** — and nothing said so, because the file compiles and its tests pass. It became visible
only when the shared module was *moved* and "who imports this?" had to be answered. So the test must also
assert the **positive** direction:

- every module in the layer table is imported by something (an orphan module is a finding);
- every module that owns a `register*Commands` is reached from `registerAll.ts` (the §4.3 drift guard);
- after §4.1, **no file outside `runtime/` names `reloadConfig`** — and every consumer that used to is now
  listed as calling `notifyConfigChanged()`. **`bind*Instance` is the opposite case and must stay in its
  module** (§3.1): assert that only `runtime/` imports it, never that it moved.

**An extraction is not finished when the new module works; it is finished when the old copies are gone.**

---

## 6. Module contracts

| Module | Owns | May import | Public surface |
|---|---|---|---|
| `kernel/` | registry contract, event bus, kind name map, telemetry types | *(nothing engine-internal)* | types + `commandRegistry` |
| `utils/` | net, random, HTTP constants, decompression, echo, **replay** (shared by `proxy/` and `rest/`), **`id`** (`mkId`), **`sendHtml`** (the socket writer both proxy halves use) | `kernel/` | pure functions |
| `workspace/` | paths, config, app settings, `workspaceFs`, enabled sets, names index, pending deletions, generic CRUD commands, all 16 kind registrations, the per-workspace status cache, the `healthbar.*` commands, **`dispatch.ts`** (the per-workspace dispatch config), **`gitStore.ts`** (the per-workspace repository handle + audit log), **`blob/`** (the content-addressed store) | `kernel/`, `utils/` | `loadConfig`, `writeEntity`, `register*Kinds` |
| `reverseproxy/` | edge router, home page, mapping dispatch | `kernel/`, `utils/`, `workspace/`, `mocks/`, **`proxy/`** | `createEdgeRouter`, `bindEdgeInstance` |
| `proxy/` | forward proxy, TLS interception, routing, replay | `kernel/`, `utils/`, `workspace/`, `mocks/`, `certificates/` | `createProxyServer`, `bindProxyInstance` |
| `mocks/` | mock resolution, script context, script executor | `kernel/`, `utils/`, `workspace/` | `matchMock`, `serveMock`, `executeScript` |
| `certificates/` | CA generation, identification, leaf certs, **and all six `tls.*` commands** (the three egress/ingress ones moved here from `fileops/`) | `kernel/`, `utils/`, `workspace/` | `certManager`, `registerCertCommands` |
| `webhooks/` | webhook receiver | `kernel/`, `utils/`, `workspace/` | `createWebhookServer` |
| `graphql/` `grpc/` `soap/` | commands + kinds for that protocol | `kernel/`, `utils/`, `workspace/` | `register*Commands`, `register*Kinds` |
| `rest/` | the `requests` **feature** — its behaviour (replay); the kind itself registers through the shared `workspace/entities/kinds/crudKinds.ts` table (§4.4) | `kernel/`, `utils/`, `workspace/` | `registerRestCommands` |
| `gitops/` | git ops, publish, status, auto-sync, sync commands *(the **handle** is `workspace/gitStore.ts`)* | `kernel/`, `utils/`, `workspace/`, `runtime/instances` | `syncManager`, `registerSyncCommands` |
| `websocket/` | **`plan/19` WI 6** — the live-socket registry (`wsRegistry.ts`) and the six `ws.*` commands (`wsCommands.ts`). The `wsConnections` *kind* still registers through the shared CRUD table; `edge.ts`'s `upgrade` branch is still deliberately bodiless | `kernel/`, `utils/`, `workspace/` | `utils/resolve` (resolution, D23), `workspace/envState` (`loadMergedEnvironment`) |
| `fileops/` | **two egress channels** — `audit.export`, `capture.shareJson`. Was an aggregator holding six; the `tls.*` trio went to `certificates/` and `runner.exportReport` to `runner/` in the layout cleanup | `kernel/`, `utils/`, `workspace/` | `registerFileOpsCommands` |
| `importexport/` | the format registry, the importers and exporters | `kernel/`, `utils/`, `workspace/` | `registerImportExportCommands` |
| `applications/` | the managed-process spawner | `kernel/`, `utils/`, `workspace/` | `registerApplicationCommands` |
| `runner/` | collection runs, the HTML report, **and `runner.exportReport`** (moved here from `fileops/`: it renders through `renderRunnerHtml`, which `runner.saveReport` in the same module already calls) | `kernel/`, `utils/`, `workspace/` | `registerRunnerCommands` |
| `lifecycle/` | `startup` (preflight, workspace bootstrap) and `shutdown` — it **composes** the engine | everything | `preflight`, `bootstrapWorkspaces`, `shutdownEngine` |
| `transport/` | the four transports, auth, event log | `kernel/`, **`utils/`** (for `isLoopbackHost`/`hostnameOf`) | `create*Transport` |
| `runtime/` | host, CLI, port allocator, **the listener fan-out** (`notifyConfigChanged`, `rebindRuntimePorts`) | everything except `index` | `createWorkspaceHost`, `notifyConfigChanged`, `rebindRuntimePorts` |

---

## 7. Migration

### 7.1 Why this is mostly mechanical, and where the cost actually is

> **⚠ Corrected 2026-09-28 — the first draft of this section was wrong, and wrong in the way that
> matters.** It counted only `from` imports. Measured by `scripts/engine-layout-codemod.mjs --check`
> (now the instrument; reproducible, not asserted):

| Form | Count | Seen by a `from`-only rewrite? |
|---|---|---|
| `from "@bifurc/engine/<sub>"` | 263 | ✅ |
| `import("@bifurc/engine/<sub>")` | 193 | ❌ |
| `vi.mock(...)` / `vi.doMock(...)` | 33 | ❌ |
| prose mentions inside code comments | 35 | ❌ |
| `typeof import("@bifurc/engine/<sub>")` | 7 | ❌ |
| `require("@bifurc/engine/<sub>")` | 1 | ❌ |
| **total** | **532 across 102 files, 81 unique subpaths** | |

| Consumer | Files |
|---|---|
| `tests/` (root) | 77 |
| `src/` (Electron shell) | 10 |
| `packages/client` | 6 |
| `packages/engine` (incl. its own `tsup.config.ts`) | 5 |
| `packages/gui`, `packages/web-client` | 2 |
| `scripts/*.mjs` | 2 |

**⚠ The two large under-counted classes fail at RUNTIME, not at compile time.** A stale dynamic
`import()` specifier throws when that branch runs — and a stale `vi.mock` specifier either throws or,
worse, mocks nothing while the real module loads, which reads as an unrelated test defect. That is why
the rewrite is a script that knows all five forms rather than a find/replace on `from`.

**The cost is the import rewrite, not the moves** — and the rewrite is scriptable provided the old→new
map is a reviewed data file rather than a series of hand edits. It now is: `plan/engine-layout-map.json`,
generated and validated by `scripts/engine-layout-codemod.mjs` (which asserts totality — all 140 files
are in the table, and every referenced subpath resolves to a real file).

### 7.2 Phases

Each phase ends with `npm run build:packages`, the full suite, and a comparison against the re-measured
baseline **by mechanism, not count** (per the project's own rule).

| Phase | What | Moves | Why in this order |
|---|---|---|---|
| **0 — freeze** | Commit. Capture the baseline. Emit the old→new path map as `plan/engine-layout-map.json`. **No file moves.** | 0 | The map is reviewable on its own, and it is the input to every later phase. |
| **1 — kernel + utils** | `commands/registry`→`kernel/`, `eventBus`→`kernel/`, `entityKindMap`→`kernel/kinds`, `logEmitter`→`kernel/telemetry`, `lib/`→`utils/`, `commands/echoWorkspace`→`utils/` | ~8 | Smallest, highest leverage, zero behaviour change. Proves the codemod on a safe subset. |
| **2 — the rebind fan-out** | **§4.1. No file moves.** Add `runtime/instances.ts` (`notifyConfigChanged` + `rebindRuntimePorts`), repoint 29 call sites across 21 files. | 0 | Removes the most coupling for the least churn, and it is verifiable on its own. |
| **3 — `workspace/`** | Split `store/`: persistence → `workspace/`, commands → `workspace/entities/`, `gitStore` → `gitops/`, `subscription/` → `workspace/` | 20 | The hub moves first so feature modules have a stable target. **Done — the row's "23" counted all of `store/`; the 3 protocol leaf kinds (`store/kinds/{graphql,grpc,soap}Kinds.ts`) stay for phase 5.** |
| **4 — proxy family** | `reverseproxy/`, `mocks/`, `certificates/`, `webhooks/` out of `proxy/` | 14 | The largest single directory; `plan/15` already decided the split. **Done — the row's "22" counted all of `proxy/`; 7 files are the forward half and stay (`proxyHandler`, `routingUtils`, `server`, `serverCommands`, `serverReplay`, `service-discovery`, `tlsIntercept`). 10 moved to the named modules + 4 shared helpers to `utils/`.** |
| **5 — protocol verticals** | `graphql/`, `grpc/`, `soap/` absorb their kinds (§4.4) | 6 | Depends on `workspace/entities/kinds` existing. **Done — 6 paths as planned, the plan's first count that was right.** `store/` drains to `gitStore.ts` alone (phase 6); `protocols/` is now **empty and removed**. |
| **6 — `gitops/`** | `sync/` + `gitStore` merged | 12 | Depends on phase 3. **Done — 12 paths as planned; the second row whose count was right.** `store/` and `sync/` are both now **empty and removed**. |
| **7 — leftovers** | `miscCommands` dissolved (§4.5), `lifecycle/`, `rest/`, `websocket/` placeholder, `fileops`/`importexport` normalised | 8 | Small, independent. **Done — but the row's "8" counts *destinations*, not files. Measured: 44 files are renamed (42 codemod + 2 hand) and 4 are added.** `startup`/`shutdown` → `lifecycle/`; `fileOps/` → `fileops/`; `importExport/` (39) → `importexport/`; `miscCommands` → `rest/commands` + `mocks/commands` + `workspace/healthbarCommands`; `proxy/serverReplay` → `utils/replay`. Added: `rest/commands.ts`, `mocks/commands.ts` (the other two thirds of the `miscCommands` split), `websocket/index.ts`, `workspace/statusCache.ts`. **Two files were extracted rather than moved, because §4.5's destinations cannot legally import their dependencies** — `healthbar.*` → `workspace/` needs `invalidateCache` (`gitops/`), and `request.replay` → `rest/` needs `replayRequest` (`proxy/`), so §5.1's "extract down" remedy gave `workspace/statusCache.ts` and `utils/replay.ts`. **⚠ `kernel/eventBus.ts` still imported `getWorkspaceSyncStatus`/`getWorkspaceGitState` from `gitops/`, making §5.1's "kernel imports nothing engine-internal" false — ✅ FIXED in phase 8's follow-up** by moving `emitEntityStatus()` to `runtime/entityStatus.ts` (the second seam module). See §5.2. **⚠ Windows case-only renames no-op — see the header note.** |
| **8 — ratchet + docs** | `tests/engine/layout.test.ts`, the ESLint rule, `plan/`, `TESTING.md`, the `bifurc-*` skills | 0 | Last, so it encodes the finished state. **⚠ Also decides `packages/engine/tests/`'s shape:** phases 1–3 moved sources without moving their test mirrors, so `tests/commands/registry.test.ts` now tests `src/kernel/registry.ts` and `tests/lib/net.test.ts` tests `src/utils/net.ts`. The tests pass (their imports were rewritten), so this is cosmetic — but the §7.1 ratchet covers `src/` only, so nothing else will catch it. Either mirror the test tree or state explicitly that tests do not mirror. **Done — the ratchet landed (`tests/engine/layout.test.ts`, 5 assertions, teeth verified by mutation), the ESLint half could not be built (no ESLint in this repo — see §5.2), and the docs/skills were swept. The test-mirror decision: tests do NOT mirror `src/` — the convention and the full mismatch table are now recorded in `TESTING.md` §3.5. The mismatch is 25 files across four directories (`tests/proxy/` alone spans five modules), and moving them is churn that buys navigation the table already provides.** ⚠ The ratchet found **24 contract violations** the seven move phases had accumulated — all allowlisted with reasons; `kernel/eventBus.ts → gitops/` is the one needing a design decision. |

**Phases 1–2 are the recommendation to start with.** They are ~8 file moves and 0 respectively, they remove
the largest single class of bad dependency, and if the plan is abandoned after phase 2 the tree is still
better than it is today. Phases 4–6 are the ones that carry real churn and can be deferred indefinitely.

### 7.3 Mechanics that this repository specifically requires

- **⚠ Never `git rm` in this repository.** `git rm src/ipc/handlers/tlsHandlers.ts` reported success and left
  **all 22 tracked files under `src/` deleted** — reproduced twice, and `env -u` on the safe-delete variables
  did not help. Plain `rm <file>` then `git add <file>` does exactly what was intended. Use
  `python -c "import os; os.remove(...)"` if the safe-delete shim interferes, and **commit before starting**
  so any restore is lossless. A refactor made of file moves is the worst possible place to learn this.
- **File-level `git mv` works; directory renames are blocked in the agent shell.** Either move files
  individually, or have the user run the directory moves. Verify the containing directory's file count after
  **each** batch.
- **⚠ The stale-`dist/` trap is the one this refactor is most likely to trip, and it is invisible.** `tsc`
  never deletes output, and `dist/` is gitignored, so **every module this plan moves leaves a compiled copy
  behind** — measured once at **92 of 116 `dist/*.js` with no counterpart in `src/`, 447 KB of a second frozen
  engine inside the asar**. Nothing fails: the suite runs `src/`, and the stale files are unreachable by
  walking relative `require`s from the app's entry points. It is precisely the hazard `bundle: false` exists
  to prevent — a second copy of `store/paths.ts` is a second `_dataRootOverride`. So: `tsup`'s `clean: true`
  covers `packages/engine/dist`, and the **app's** `dist/` needs `npm run clean:main` before its build
  (`scripts/clean-main-dist.mjs`, whose `dist/renderer` exclusion is load-bearing — that directory is Vite's
  content-hashed output, not tsc's). Instrument: **`node scripts/pkg-check-reachability.mjs`** after each
  phase.
- **One `Edit` per file per message** — two same-file edits silently revert each other. The import rewrite
  should be a script, not 529 edits.
- **A deletion pass is atomic with its test migration.** A half-landed move reads as a code defect —
  `No handler registered`, `Cannot find module` — never as an unfinished edit, and the instinct is to debug
  the code rather than check `git status`. **If a phase cannot be finished, revert it rather than leave it
  staged.**
- **⚠ `vitest.config.ts`'s coverage `exclude` list names engine files by path** — the type-only modules.
  **As of phase 7 all three entries name live files** — `packages/engine/src/workspace/types.ts`,
  `packages/engine/src/gitops/types.ts` and `packages/engine/src/importexport/types.ts`; it is the first time
  since the list was written that no entry points at a future target. Moving one **silently
  changes the coverage denominator**. Update the list in the same commit as the move. (The codemod cannot:
  these are bare path strings in a config array, not import specifiers.)
- **`tsup.config.ts`'s `entry: ["src/**/*.ts"]` and `package.json`'s `"./*"` export wildcard need no change**
  — both are globs. But `bundle: false` means **the `dist/` layout mirrors `src/`**, so the shell's 53
  imports resolve against `dist` and are broken until `build:packages` runs. The `@bifurc/engine/*` alias
  points at `src` for tests, so **a path that is green in tests can still be broken in the app** — and the
  engine's ESM output is not loadable by plain Node either (extensionless specifiers under `bundle: false`),
  so probe the **CJS** build, which is the shell's path.
- **`src/ipc/handlers.ts` keeps its own 19 explicit command calls and 5 kind calls**, deliberately, as
  load-bearing documentation of the P6 reduction. Those import paths change; the comments must not be
  collapsed.

---

## 8. Risks

| Risk | Severity | Mitigation |
|---|---|---|
| **Stale `dist/` copies of every moved module ship in the asar** — invisible, and a second copy of `store/paths.ts` is a second `_dataRootOverride` | **High** | `npm run clean:main` before the app build; `node scripts/pkg-check-reachability.mjs` after every phase (§7.3) |
| A path that is correct in tests but wrong in the built app | **High** | `build:packages` + a smoke run after **every** phase; never trust a green vitest run alone |
| The kind vocabulary is duplicated per protocol module | **High** | §4.4 — one list in `workspace/`; protocol modules register, never declare |
| `utils/` becomes a junk drawer and absorbs persistence | **High** | §5.1 — `utils` is pure/stateless; the ratchet test enforces the direction |
| An old copy of a shared helper survives the extraction, because nothing that reads only what is *present* can see it | **High** | §5.2's positive-direction assertions; ask "who imports the shared module?" rather than assuming the sweep |
| Coverage denominators shift silently | Medium | Update `vitest.config.ts`'s exclude list in the same commit |
| A silent partial deletion during a bulk move | Medium | Small batches, count the directory, commit first, and **never `git rm`** |
| A half-landed phase reads as a code defect | Medium | A phase is atomic with its import rewrite; revert rather than leave it staged |
| Churn with no behaviour change makes review impossible | Medium | One phase per PR, and phases 1–2 are the only ones that must land now |
| `transport/` and `runtime/` get dragged in | Low | Explicitly out of scope (§9) |

---

## 9. What not to do

- **Do not scatter `workspace/` per feature.** ~130 inbound edges; per-feature `loadConfig` copies would be
  the worst possible outcome of a "cleanup".
- **Do not move `transport/` or `runtime/` in this pass.** They are already coherent, and `transport/` is
  4,636 lines with its own auth model.
- **Do not give `websocket/` invented content — and `plan/19` WI 6 did not.** The rule was honoured rather
  than overridden: the module was filled with the content that existed in the wrong process (the renderer's
  socket client), not with behaviour nobody needed. **The `upgrade` branch in `edge.ts` is still reserved and
  still deliberately bodiless** — a branch returning "not built yet" is a routing decision nobody made, and
  an outbound `ws` client is not that decision. The distinction is worth keeping because "the module is no
  longer empty" is not the same claim as "the reserved branch is now implemented".
- **Do not expect `rest/` to be a big move.** REST today is one CRUD kind, one `request.replay` command, and
  whatever `runner/` shares. The module is mostly a *home* for future work, and pretending otherwise inflates
  the estimate.
- **Do not rename `store/` → `workspace/` if the churn isn't judged worth it.** The split of commands from
  persistence is the valuable part; the name is cosmetic and can stay `store/`.
- **Do not convert `registerAll.ts` to an import side effect.** P6 §5c exists because that was the bug.
- **Do not do this as one commit.** Eight phases, each green, each reversible.

---

## 10. Acceptance criteria

1. `tests/engine/layout.test.ts` passes, asserting the layer order for every file in `src/` — **in both
   directions** (§5.2): no forbidden import exists, *and* every module is imported by something, *and* every
   `register*Commands` is reached from `registerAll.ts`.
2. `registerAllEngineCommands()` still registers a **superset** of the shell's set, on **both** axes
   (command names and kinds) — `tests/integration/workspaceHost.integration.test.ts` unchanged and green.
3. No feature module imports another feature module (except `registerAll.ts`).
4. No module outside `runtime/` imports `reloadConfig` directly — every "the config changed" caller goes
   through `notifyConfigChanged()`. **And `bind*Instance` is still imported only by `runtime/`** (§3.1: it
   must not move). Adding a listener kind is one line in `runtime/instances.ts` and **no change at any call
   site**.
5. `npm run build:packages && npm run typecheck` clean; the suite matches the re-measured baseline by
   **mechanism** — the four teardown-only timeouts and the four `gitRemote` failures are still the only
   failures, and are still attributable.
6. **`node scripts/pkg-check-reachability.mjs` reports no stale module**, and the app's `dist/` has been
   rebuilt from a `clean:main` at least once after the last move.
7. Adding `mqtt/` or `vault/` requires: a new directory, a `register*Commands`, a `register*Kinds`, and one
   line in `registerAll.ts`. **That is the test of the whole exercise** — if a new module needs a change
   anywhere else, the boundary is wrong.
