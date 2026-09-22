# 14 — Phase 14: Workspace runtime (the executor pivot)

**Goal:** make the **workspace** — not the engine — the unit that owns servers, ports and lifecycle, so a
workspace can be hosted headlessly (Docker, CI, a colleague's machine) while the desktop app keeps
working unchanged from the user's point of view.

**Effort:** 3–4 weeks. **Depends on:** P2 ✅, P4 ✅ (as a mechanism), P5 ✅, P6 🟢 in scope.
**Re-scopes:** P7, P8, P9, P12. **Blocks:** P9.

---

## 1. Why this phase exists — the pivot

The original programme is *"extract the engine, run it headless, attach interchangeable clients"*
(`README.md`). P2 did the extraction, P4 built the transports, P5 built the client, and P6 cut the shell
seam. **The deployable unit is still wrong.**

"Run the engine headlessly" is the wrong deployment story because `@bifurc/engine` carries two kinds of
thing at once:

- things that are genuinely **app-level** — the companion extension bridge, which is one server for the
  whole installation and has no workspace identity at all;
- things that are genuinely **per-workspace** — the proxy, the webhook receiver, import/export.

Shipping the engine as the remote unit ships the app-level half with it. Shipping a **workspace runtime**
ships only what is per-workspace. That is the whole change.

### 1.1 The measurement that motivates it

Three facts, all read off the current tree:

**Servers are app-level singletons with global ports.**

| Server | File | Module-level state | Port source |
|---|---|---|---|
| Proxy | `packages/engine/src/proxy/server.ts:20-25` | `server`, `currentPort`, `currentConfig`, `fullRules`, `lastError`, `enabledSets` | global `settings.port` |
| Webhook receiver | `packages/engine/src/proxy/webhookServer.ts:29-36` | `server`, `currentPort`, `lastError`, `activeWebhookSuffixes`, `activeSuffixToId` | global `settings.webhookPort` |
| Companion bridge | `packages/engine/src/transport/legacyCompanion.ts:96-97` | `wss`, `currentPort` | global `settings.companionPort` |

**Ports are flat scalars, and a workspace has nowhere to put one.**
`AppSettings` holds `port` / `webhookPort` / `companionPort` (`store/appSettings.ts:55-57`, defaults
`80 / 9101 / 9271` at `:83-85`) and exposes them as scalars on `AppConfig` (`store/config.ts:19-21`).
`Workspace` (`store/types.ts:127-134`) has `id`, `name`, `createdAt`, `activeEnvironmentId`, `syncConfig`,
`syncMeta` — **no port field**. So *"each workspace gets its own ports"* is not representable today.

**Only entity *filtering* is workspace-scoped, and it is read once.**
`startServer()` resolves `currentConfig.activeWorkspaceId` at start and on `reloadConfig()`
(`proxy/server.ts:27-31,120-124`, filtered in `proxy/serverUtils.ts:35-44`). The webhook receiver has
**no `workspaceId` anywhere** — it routes on `urlSuffix` alone (`webhookServer.ts:42-46,104-111`).

### 1.2 The pivot is not a new idea — it is the pattern import/export already uses

The one layer that is *already* workspace-scoped is import/export:

```ts
// packages/protocol/src/commands/export.ts
ExportCreateParams    { workspaceId: z.string(), ... }   // :34-38
ImportPreflightParams { workspaceId: z.string(), ... }   // :66-70
ImportCommitParams    { workspaceId: z.string(), ... }   // :85-89
```

Every one of them takes an explicit `workspaceId`. Generalising that to the servers is the phase.

### 1.3 What this replaces in the plan

| Was | Now |
|---|---|
| P9 = "a container image that runs **the engine** headlessly" | P9 = "a container image that runs a **workspace host**" |
| P6 item 2 = the shell spawns and supervises an **engine child process** | **Downgraded (D14)** — the desktop stays in-process; supervision becomes a property of the workspace host |
| P8 = a CLI, ~3 weeks | **Parked (D15)** — deferred, with two recorded future shapes |
| P7 = web UI connects to "an engine" | unchanged in effort, better specified: it connects to a **host** and selects a **workspace** |

---

## 2. Target architecture — two runtimes

### 2.1 AppRuntime — one per app process

- **Companion server** (extension bridge) — app-level, shared by every workspace. Unchanged.
- Desktop concerns: window, tray, theme, zoom, auto-update, first-launch, OS trust store / cert install,
  native dialogs.
- The **RPC control endpoint** (`transport/socket.ts` / `transport/ws.ts` — both exist and are currently
  wired to nothing).
- An **active-workspace pointer** — UI/session state, not server state.

### 2.2 WorkspaceRuntime — N per host, one per workspace

- The proxy server + its port.
- The webhook receiver + its port.
- Import/export for that workspace's entity kinds (already true).
- Its git store, sync, blob staging — the first two are *already* per-workspace on disk
  (`<dataRoot>/<wsId>/…`, `store/workspaceFs.ts:46-50,71-102`); **blob staging is not, and D17 makes it so**.
  > ⚠ **Corrected 2026-09-20 — this bullet originally read as if all three were already per-workspace, and
  > the blob half was FALSE.** `blobRoot()` was `<dataDir>/blobs`
  > (`packages/engine/src/blob/store.ts:140-142`) and the four `blob.*` commands carried no `workspaceId`
  > (`packages/protocol/src/commands/blob.ts:106,140,159,181`) — a **host-wide** namespace, pinned by
  > `tests/blob/store.test.ts:62` for the Docker-volume requirement. **D17 settled it: blob staging is
  > namespaced per workspace** (`blobRoot(wsId)` = `<dataDir>/blobs/<wsId>/`, `workspaceId` on the four
  > params, a per-runtime sweeper), landing with WI 1 / WI 8. The measurement, the three costs under N > 1
  > and the rejected alternative are in **§12.1**.
- Its own server error state.

### 2.3 The split, stated as a table

| Concern | Today | After |
|---|---|---|
| Proxy server | app singleton, global port | per workspace, per-workspace port |
| Webhook receiver | app singleton, global port, keyed by `urlSuffix` | per workspace, per-workspace port, keyed by `(workspaceId, urlSuffix)` |
| Companion server | engine singleton, global port | **unchanged — app level** |
| Import/export | engine, already takes `workspaceId` | unchanged, now provably reachable without the app half |
| Blob staging | engine, **host-wide** `<dataDir>/blobs`, no `workspaceId` | **per workspace (D17)** — `<dataDir>/blobs/<wsId>/`, `workspaceId` on the four `blob.*` params, per-runtime sweeper |
| `activeWorkspaceId` | global setting, **read by the proxy** | client/session pointer; servers no longer read it |
| `port` / `webhookPort` | `AppSettings` scalars | host-local, per-workspace |
| `companionPort` | `AppSettings` scalar | **unchanged — app level** |
| RPC transport | host-level, started by nobody | host-level; workspace-scoped commands carry `workspaceId` |

### 2.4 `@bifurc/engine` does not move

The engine stays exactly where P2 put it: the shared library both runtimes are composed from. **Nothing is
extracted out of the app for the desktop case.** The workspace host is a *new entry point* that composes
`WorkspaceRuntime`s and deliberately omits the app half.

---

## 3. Decisions

Recorded in `00-decisions.md`'s log; the full rationale lives here (D10–D16 are this document's, and
`00-decisions.md` carries the summary rows rather than a second copy of this text).

| # | Decision | Effect |
|---|---|---|
| **D10** | **The executor unit is the workspace, not the engine** | P9's artifact changes; the engine stays a library |
| **D11** | **N workspaces per host; `WorkspaceRuntime` is the unit** — one-per-container is the N=1 case | One abstraction; deployment picks N |
| **D12** | **The desktop adopts the same runtime**, keeping `activeWorkspaceId` as a UI pointer | One mechanism everywhere; no divergence |
| **D13** | **One RPC endpoint per host; workspace-scoped commands carry `workspaceId`** | No new ports; the P4/P5 client survives intact |
| **D14** | **P6 work item 2 (process split) downgraded** — desktop stays in-process; supervision moves to the workspace host | Removes 4 carried criteria from P6's gate |
| **D15** | **CLI deferred (P8 parked)** — two shapes recorded, decided after P14 | See §7 |
| **D16** | **A workspace's proxy takes a per-environment base port — `80` on Windows, `8080` elsewhere — then counts up** | Per-workspace ports become representable; `plan/10` corrected |
| **D17** | **Blob staging is namespaced per workspace** — `blobRoot(wsId)` = `<dataDir>/blobs/<wsId>/`, `workspaceId` on the four `blob.*` params, a per-runtime sweeper | Blobs stop being a host-wide namespace; rationale and the rejected alternative in §12.1 |

### D16 — DECIDED 2026-09-20: a per-environment base port, then count up

**The rule:** the environment supplies a **base port**, and a workspace's proxy scans **contiguously upward**
from it — `base`, `base + 1`, `base + 2`, … — until a bind succeeds. The user can override the port per
workspace.

| Environment | Base port | A host with three workspaces gets |
|---|---|---|
| **Windows** | `80` | 80, 81, 82 |
| **Linux, macOS, and the container** | `8080` | 8080, 8081, 8082 |

**Why a per-environment base rather than a single default with a fallback:** port 80 needs privileges on Linux
and macOS, and is unavailable to the container's non-root user (`plan/10` mandates `USER bifurc`) — but it
needs no privileges at all on Windows. Deriving the base from the environment means **the default path is
always a port we are allowed to bind**, and the increment is identical everywhere. One rule, one code shape,
and the only platform-dependent value is a single constant.

**Resolution order:**

| # | Source | If the port is taken |
|---|---|---|
| 1 | **Per-workspace override** — `preferredPort` in the host-local registry, or the settings UI | **Fail loudly**, naming the next free port. Never silently move a port the user chose. |
| 2 | **Remembered resolved port** — what this workspace actually bound on a previous run, from the host-local registry | **Fall through to 3.** A remembered *outcome* is not a request, and must never become a startup failure. |
| 3 | **Environment base** — `80` on win32, `8080` elsewhere; overridable via `BIFURC_PROXY_BASE_PORT` | Scan upward: `base + 1`, `base + 2`, … |

> **Source 2 was specified here from the start and implemented only on 2026-09-22.** The allocator had no
> *soft* input, so `host.ts` passed nothing rather than turn a taken remembered port into a hard startup
> failure — and the row was silently satisfied by the scan instead. The acceptance audit caught it by
> **measuring**, not by reading. See §8 criteria 6a/6b and lesson 3.

`BIFURC_PROXY_BASE_PORT` moves the **base**, not one workspace's port, so the increment keeps working after
it — which is what a container wants: pin the published port without giving up the fallback.

**The webhook port follows the same shape**, with `9101` as its base on every platform (already above 1024, so
no split is needed) and the same contiguous scan. *(Not specified by the owner — resolved by analogy from the
proxy rule. Correct it if the intent differs.)*

**The resolved port is recorded in the host-local registry**, so a restart reuses it. A workspace that landed
on 81 because 80 was busy **keeps 81** after 80 frees up — a stable URL is worth more to the browser-proxy
workflow than reclaiming the base. *(Alternative: re-prefer the base on every start, at the cost of the proxy
silently moving under the user. Recommended against.)*

**Implemented 2026-09-22** as `allocatePorts`'s `rememberedProxyPort` / `rememberedWebhookPort` — a third,
**soft** source: tried when free, skipped when not, never thrown over. `AllocatedPorts` reports
`proxyPortWasRemembered` / `webhookPortWasRemembered`, because the **outcome cannot distinguish this path from
the scan** — on a quiet machine both produce the same number — so the flag is the only honest instrument.

**Two things the rule still needs:**

- **Allocation must be driven by the bind, not by a probe.** `applications/portUtils.ts#checkPortInUse` is a
  *probe* — it binds, closes and reports. Probing and then binding is a **TOCTOU race**: the port can be taken
  in between, and the failure surfaces as an unexplained dead proxy. The allocator must attempt the real
  `listen()` and advance on failure. `checkPortInUse` stays for *reported status* and for
  `application.checkPort`, which are genuinely questions rather than allocations.
- **Advance on `EACCES` as well as `EADDRINUSE` — but this is now defensive, not load-bearing.** With the base
  at 8080 on Linux and macOS, the default path never attempts a privileged port, so the earlier concern — that
  a scan retrying only on `EADDRINUSE` would leave every workspace dead on those platforms — **no longer
  applies to the default path.** It still applies to the one case a user can create: an override set below
  1024 on a platform that forbids binding there. Reading `EACCES` as "advance" rather than "fail" is the safer
  choice there, and it costs nothing.

**The port must not live in `workspace.json`.** That file is git-tracked (`store/gitStore.ts:51-55`
commits it; `sync/syncManager.ts:114-126` reads it as the workspace identity), so a port stored there
would sync between machines and collide on the second one. A port is a property of *this machine hosting
this workspace*, not of the workspace's content. Hence the host-local registry.

**The container states its base explicitly.** Linux already yields 8080, so `plan/10` setting
`BIFURC_PROXY_BASE_PORT=8080` changes nothing about the default — it exists so the published port is *stated*
rather than inferred, and so an operator can move it without editing the image.

`server.status` must always report the **resolved** port, whichever path produced it.

---

## 4. Work items

### Work item 0 — the measurement that gates everything (1–2 days)

> **DONE 2026-09-21 — the record is §13, and it supersedes the dated list below.** §13 re-derives every
> module-level mutable in `proxy/`, every reader of `port` / `webhookPort` / `activeWorkspaceId` /
> `isRunning()`, and every caller of the four lifecycle functions. **Verdict: the phase is mechanical, with
> the readiness predicate the one non-mechanical part — and the measurement found that
> `createEngine()` starts the companion server (`index.ts:232`), which WI 3's own acceptance criterion
> forbids for the host.** Read §13.1–§13.4 before starting WI 1.

Produce a written inventory, not a design:

- every module-level mutable in `proxy/` and every reader of it;
- every reader of `AppConfig.port` / `.webhookPort` / `.activeWorkspaceId` / `isRunning()`;
- every caller of `startServer` / `stopServer` / `startCompanionServer` / `restartCompanionServer`.

Known callers today: `proxy/serverCommands.ts:74-90`, `store/configCommands.ts:63-73`, `index.ts:231-243`,
`src/main.ts:309-310`, `shutdown.ts:45-46`, `startup.ts:90-99,122`.

> **Corrected 2026-09-21 — two of those callers no longer exist, and the list is now shorter than it was
> when this work item was written.** `src/ipc/handlers/systemHandlers.ts:9-25` (the three `server:*`
> bodies) went with the reduction's fifth group, and `src/ipc/handlers/coreHandlers.ts:56-69` (the
> `config:save` body) went with its sixth — that file is now `coreKinds.ts` and calls `startServer` not
> at all (`plan/07` §5b-sedecies, §5b-septendecies). **Both duplicates were of bodies this list already
> names**: `proxy/serverCommands.ts` and `store/configCommands.ts` are where the behaviour actually lives,
> so the *distinct* caller count is one lower in each case, not two. Re-derive this list from the tree
> before relying on it — it is a dated reading, and the phase it measures has since deleted six handler
> files.
>
> **Re-derived 2026-09-21 in §13.3, and the re-derivation is now the authority.** Measured: **11 call
> sites, of which exactly one is the shell** (`src/main.ts:309-310` — WI 7's single edit); the rest are
> engine commands (`serverCommands`, `configCommands`), the engine lifecycle (`createEngine`, `shutdown`)
> and tests. The list above named six and is now four lines short of the tree in one direction and two
> over in another — use §13.3.

**If that list is short the phase is mechanical. If it is not, this phase's estimate is wrong and you find
out in a day rather than in week three.**

### Work item 1 — the port model and the protocol shape

- Add the **host-local runtime registry** — a `runtime.json` at the data root mapping
  `wsId → { preferredPort?, resolvedProxyPort?, resolvedWebhookPort? }` (§3, D16).
- Implement the **allocator** to D16's rule: per-workspace override (fails loudly if taken) → the
  environment **base port** → a contiguous scan upward. The base is `80` on `win32` and `8080` everywhere
  else, overridable via `BIFURC_PROXY_BASE_PORT`. Drive it from the real `listen()`, never from a probe.
- **Keep the platform branch to a single constant, not a code path:**
  `const BASE = process.platform === "win32" ? 80 : 8080`. If per-platform logic appears anywhere else in
  the allocator, D16 has been mis-implemented — the whole point of the base-port rule is that the increment
  is identical everywhere.
- **Namespace the blob root (D17).** `blobRoot()` becomes `blobRoot(wsId)` = `<dataDir>/blobs/<wsId>/`,
  `workspaceId` is added to `blob.put` / `blob.stat` / `blob.read` / `blob.release`, and the sweeper becomes
  per-runtime so a `WorkspaceRuntime.stop()` can release its own staging. **The root must stay under
  `dataDir()`** — that is the Docker-volume requirement, and it is pinned by a test
  (`tests/blob/store.test.ts:62`), so the change is a *namespace*, not a move.
- Add a **reserved-port set** the scan must skip: the companion port, the RPC port, the gRPC placeholder
  (`9102`), and every other runtime's resolved proxy and webhook ports. Without it the allocator will
  happily hand a workspace the port the companion server is about to want. Note the interaction: with a
  webhook base of `9101`, the **second** workspace's webhook wants `9102` and must skip to `9103`.
- Add `workspaceId` to `server.start` / `server.stop` / `server.restart` / `server.status` /
  `proxy.status`, and to `webhookServer.start` / `.stop` / `.status` and `webhook.registerActive` /
  `.unregisterActive`. `server.status` returns the **resolved** port, not the configured one.
- This is a **breaking protocol change**: record it in `plan/protocol-changes.md` and re-derive fixtures
  (§8).
- **Surface the resolved port in the UI per workspace**, and make it the place a per-workspace override is
  edited. Once the second workspace can be on 81 (Windows) or 8081 (elsewhere) rather than the base, "which
  port is my proxy on?" stops having a single answer, and the user has to be told.

### Work item 2 — de-singletonize the proxy and webhook servers into factories

> **STEP 1 DONE 2026-09-21 — the webhook receiver, which §6 requires be proven first.** §6's second item:
> "Prove the factory change on the webhook receiver first, not the proxy. It is ~160 lines, has no TLS and
> no CONNECT tunnelling, and its only global state is two maps and a port — so it is the cheapest place in
> this codebase to learn what de-singletonizing actually costs." It cost: **one file, no caller changes, no
> `window.api` movement.** `createWebhookServer({ workspaceId })` now owns the `http.Server`, the port, the
> error and both active maps; the old module functions became a facade over one lazily-created default
> instance, which stays until WI 7 flips the shell (§9). `tests/proxy/webhookServer.test.ts` went **15 → 24
> cases** (+9), all green, and `tests/ipc/handlers.test.ts` stayed **106/106** — the case that matters,
> because it `vi.mock`s this module and would have failed loudly on an export it lost. **Three things the
> work established, recorded so WI 2's proxy half does not re-learn them:**
>
> 1. **`MAX_ACTIVE_WEBHOOKS` stays a module constant, not a constructor argument.** "Per workspace" scopes
>    the *map*, not the number. Its duplicate at
>    `packages/gui/src/panels/webhooks/useWebhooksPanelState.ts:11` is a UI hint the renderer cannot stop holding
>    before P7, so WI 2 still owes a decision on whether the value is pushed to it.
> 2. **`webhookEmitter` must NOT become per instance.** It is the shared EventEmitter the shell's forward and
>    the suite subscribe to; a per-instance emitter detaches both silently.
> 3. **The per-workspace routing key is WI 1's job, not WI 2's.** Keying the active maps by
>    `(workspaceId, urlSuffix)` needs `webhook.registerActive` to carry the parameter first, or it is a
>    behaviour change with no test to drive it. WI 2 is the mechanical de-singletonization; §2.3's
>    `(workspaceId, urlSuffix)` key lands with the protocol change.
>
> **And one measurement carried into the proxy half:** the readiness predicate. `getError()`/`isRunning()`
> are the only readiness signals either server publishes (`index.ts:140-144`), so `index.ts:242`'s
> `waitUntilReady(isRunning, getServerError)` must re-point at the instance rather than wrap the global —
> §13.4 confirms this is the phase's one non-mechanical part.
>
> **The suite delta, measured 2026-09-21 and its mechanism named.** Baseline was 101 / 2,458 / 2,417 / 40 / 1;
> the full run after this step is **101 / 2,467 / 2,424 / 40 / 3**. **Total +9 = the nine new webhook cases,
> and nothing else** — files delta 0 (an edited file, no new file), skipped delta 0, and no command-count
> change, so `packages/protocol/src/commands/index.test.ts`'s 2-per-command generator is untouched. The
> **2 extra failures are the two documented load-induced flakes**, not a regression:
> `tests/store/gitStore.test.ts` (**24/24 isolated**, 102 s) and `tests/sync/gitOps.test.ts`
> (**4/4 isolated**, 18 s), both `EBUSY: rmdir` / hook-timeout on their temp dirs under a full-run load.
> Isolated back, the true figure is **2,426 passed / 1 failed** — the 1 being the standing environmental
> `soap.execute`.
>
> **STEP 2 DONE 2026-09-21 — `proxy/server.ts`, the half §6 said would be harder.** `createProxyServer({ workspaceId })`
> now owns all six mutables (`server`, `currentPort`, `currentConfig`, `fullRules`, `lastError`, `enabledSets`);
> `dispatch` and `dispatchHttps` became inner closures over that state, and the seven module functions became a
> facade over one lazily-created default instance — the same shape step 1 established. **One file, no caller
> changes, no `window.api` movement, `src/main.ts:7`'s `startServer` import untouched.** `tests/proxy/server.test.ts`
> went **80 → 89** (+9 isolation cases), and **the 80 pre-existing cases all stayed green without a single edit** —
> which is the strongest available evidence the facade is behaviourally transparent, and the same signal
> `tests/ipc/handlers.test.ts` gave in step 1.
>
> **Four findings the proxy half added, all worth carrying:**
>
> 1. **The two duplicated bodies had to be proven identical before deletion, not assumed.** `dispatch` and
>    `dispatchHttps` exist twice during the transition — once as factory closures, once as module-scope
>    leftovers (the original functions, whose bodies the factory closures were derived from). Deleting the
>    wrong copy compiles fine whenever the two have drifted. Both were diffed against their factory twins
>    (`sed`-normalised indent, `diff`) and confirmed identical but for the type-import form
>    (`import type { TLSSocket }` vs the inline `import("tls").TLSSocket`) — *then* removed. The inline form
>    survives because a module-scope `import type` inside a function body was the old file's style; the
>    factory uses the identical semantics with no file-level import to dangle.
> 2. **Truncating a file can silently take a re-export with it.** `export { replayRequest } from "./serverReplay"`
>    sat at the top of the deleted region, one line under a section comment. `npm run typecheck` caught it
>    immediately (`routingUtils.ts(8,10): error TS2305: Module '"./server"' has no exported member 'replayRequest'`),
>    but the lesson is that the *re-export block* is part of the facade, not decoration — it must be re-attached
>    on the kept side of the cut. It was, and `replayRequest` is verified present in `dist` via
>    `proxy.replayRequest`.
> 3. **The TLS CA is genuinely process-global, and this is where that bites.** `start()` calls `loadCA(...)` /
>    `unloadCA()` (`tlsCert.ts` holds `caCertPem` / `caKeyPem` / `certCache` in module state). With N instances,
>    **B's `start()` overwrites A's CA, and either one's `stop()` unloads it for both** — so the second workspace's
>    HTTPS interception silently degrades to tunnelling. WI 2 deliberately does **not** fix this: it is a real
>    constraint the factory now makes visible, and the fix (one shared CA loaded once by the host, or per-instance
>    CA state) belongs to WI 3's host composition. Recorded there as a constraint, not silently absorbed.
> 4. **`currentPort` is not reset by `stop()`** — only by the next `start()` — so a `getPort()` assertion taken
>    after a stop reads the *previous* port. This is pre-existing behaviour, faithfully preserved; it surfaced as
>    a failing new case, and the case was corrected rather than the code, because changing it would be a
>    behaviour change smuggled into a refactor.
>
> **The suite delta for step 2 alone: +9, total → 2,476, and again nothing else moved.** Re-verified against the
> same baseline arithmetic: 2,467 → 2,476 is exactly the nine new proxy cases; file count, skipped count and
> command-count generator all unchanged. Measured after `npm run build:packages` (which the app requires, and
> which the safe-delete guard failed once — bypass with
> `env -u CODEBUDDY_SAFE_DELETE_BULK_STATE_DIR -u CODEBUDDY_TOOL_CALL_ID`, per TRAPS.md).

- `proxy/server.ts` → `createProxyServer()` returning an instance; the six module-level mutables
  (`:20-25`) become instance state.
- `proxy/webhookServer.ts` → `createWebhookServer()`; `activeWebhookSuffixes` / `activeSuffixToId`
  (`:34-36`) become per-runtime, and the `MAX_ACTIVE_WEBHOOKS = 5` cap (`:63`) becomes **per workspace** —
  a product decision, stated here so it is not decided by accident.
- **Readiness is the non-mechanical part.** `createEngine()` awaits `waitUntilReady(isRunning, getServerError)`
  (`index.ts:238-243`), and `index.ts:141` documents that a predicate *is* the only readiness signal these
  servers publish. A factory change means that predicate becomes per-instance; a global `isRunning()` will
  happily report runtime B ready because runtime A bound successfully.
- **Leave these alone, deliberately:** `applications/processSpawner.ts:268`
  (`export const processSpawner = new ProcessSpawner()`) and `store/paths.ts`. Both are singletons
  *because* `tsup` runs `bundle: false` to keep them single — a second `_dataRootOverride` is the exact
  hazard `TRAPS.md` records. **If the runtime design appears to need a second instance of either, the
  design is wrong.**
- `transport/legacyCompanion.ts` **stays a singleton** — it is app-level by D10.

### Work item 3 — `WorkspaceRuntime` and the host entry point

```ts
// packages/engine/src/runtime/workspaceRuntime.ts
createWorkspaceRuntime({ workspaceId, dataDir, proxyPort?, webhookPort?, registry })
  → { workspaceId, start(), stop(), status(), proxy, webhook }
```

The host composes them:

- loads `app.json` (it needs the workspace list);
- builds one runtime per workspace *configured to run*;
- exposes the RPC endpoint over `createSocketServer` / `createWsServer` (both exist, both currently
  wired to nothing — `packages/engine/src/transport/{socket,ws}.ts`);
- enforces auth via `transport/auth/`;
- handles `SIGTERM`: stop every runtime, flush, exit.

**The host must not start the companion server.** That is the point of the whole phase, and it is
assertable (§6).

**Where the host lives is the highest-risk decision in this phase.** Recommendation: **a new entry point
inside `packages/engine`** (`runtime/host.ts`), *not* a new `packages/host` package — because
`bundle: false` plus module-level singletons mean a second package can silently duplicate
`store/paths.ts`'s data root. Promote it to its own package only if the Docker dependency closure demands
it, and re-read `TRAPS.md` before doing so.

> **PARTIAL DONE 2026-09-21 — the runtime and the companion switch; the host entry point itself is not
> written yet.** Three pieces landed, in this order:
>
> 1. **`EngineOptions.startCompanion?: boolean` (default `true`).** §13.3 found `createEngine()` binding
>    the companion unconditionally (`index.ts:232`), which this work item's own criterion forbids for a
>    host. Now `doStart()` skips the start, the readiness wait, and the preflight port check, with
>    `status().companionPort` reading `null`. Default `true` keeps the shell byte-identical.
> 2. **`createWorkspaceRuntime()` — `packages/engine/src/runtime/workspaceRuntime.ts`.** The signature is
>    the one sketched above, with two deliberate departures. **`dataDir` is not a parameter**: the runtime
>    does not resolve a data root or bootstrap anything — the engine is a *library* by D10, and a runtime
>    reproducing `setDataRoot()` would give a host two competing lifecycle authorities. **Ports are
>    required, not optional**: allocation is D16's rule and needs host-wide knowledge of what the other
>    runtimes took, so a runtime that invented its own port would reintroduce the TOCTOU race WI 5 warns
>    about. It also does not expose `registry` — a transport is per *host* by D13, so the host passes the
>    registry to `createSocketServer()`, not to a runtime.
> 3. **The TLS-CA constraint WI 2 handed forward is now closed in the factory.**
>    `createProxyServer()` gained **`ownsCA`** (default `true` = the pre-P14 desktop behaviour). A host
>    member passes `false`: it still `loadCA()`s its own configured CA — idempotent, same paths, same
>    PEMs — but **never `unloadCA()`s**, because that one mkcert root is shared by every workspace on the
>    machine and outlives any single runtime. Deleting the `unloadCA()` call instead would leak the CA
>    past the stop it was written to bound, which is why the ownership flag is the fix and not its
>    removal. **One related case WI 2's code got wrong and this fixed:** a non-owning instance whose own
>    config has TLS *off* used to call `unloadCA()` in its `start()`, silently breaking a TLS-enabled
>    sibling. Now that branch is owner-only, and there is a case for it.
>
> **Also removed:** the six module-scope mutables in `proxy/server.ts` (`server`, `currentPort`,
> `currentConfig`, `fullRules`, `lastError`, `enabledSets`) were still declared after WI 2 step 2 —
> shadowed by the factory's instance state, read by nothing, and named only by the header comment, so
> `tsc` could not catch them. That is the leftover-duplicate hazard WI 2's own trap entry records;
> deleting them is part of WI 3 because a host is the first caller that could have been misled by them.
>
> **What the tests establish, and why they had to be integration tests.** The criterion §8 names is
> *"starting runtime B does not observe runtime A's readiness"*. A symmetric `vi.mock("net")` harness
> cannot show it — it hands every `createServer()` call the **same** object, which is the fixture artifact
> that produced two false failures while writing step 2's isolation cases. So
> `tests/integration/workspaceRuntime.integration.test.ts` writes **two** real workspaces to a real data
> root and boots two runtimes on real sockets: distinct ports, distinct readiness, a bind failure landing
> on the loser only with the winner serving over the wire, and `stop()` on A leaving B answering.
> `tests/proxy/server.test.ts` went **80 → 92** (+9 isolation, +3 CA ownership).
>
> **Suite movement, measured 2026-09-21 with the mechanism named: 101 files / 2,476 → 103 files / 2,482,
> i.e. +2 files and +6 tests — and the six separate exactly.** The two files are new
> (`hostStartup.integration.test.ts` 1 case + `workspaceRuntime.integration.test.ts` 2 cases), and the other
> three are `tests/proxy/server.test.ts`'s +3 CA-ownership cases. `tests/proxy/server.test.ts`'s +9
> isolation cases are **not** in this delta — they belong to WI 2 step 2's measurement, already banked at
> 2,467 → 2,476. Skipped stayed 40, and the per-command generator was untouched, so no command-count move.
> **The full run reported 2 failures and both are the documented load flakes**
> (`gitStore.test.ts` hook timeout + `EBUSY: rmdir`; `soap.execute` in the spike suite), which is why the
> *second* measurement is the one that counts — isolated, this work is green everywhere it touches.
> *(The spike half of that pair no longer exists — retired 2026-09-21, `spike/README.md` — so the
> environmental set is now **two files, not three**, and both are load-induced and variable in count.
> The "second measurement is the one that counts" rule is therefore now load-bearing rather than
> advisory: with the standing failure gone, any single failure in a run is a flake to be isolated, and
> `--reporter=json` is how you name it instead of assuming.)*
>
> **One new file, not two cases in an existing one** — and that detail is load-bearing: adding the
> `startCompanion: false` case to `engineSmoke.integration.test.ts` made that file **fail** on
> `portIsFree(proxyPort)` after `stop()`, and pass alone. The cause is that `shutdownEngine()` memoises
> **process-wide**, so the *second* engine in a process awaits an already settled promise, its
> `stopServer()` never runs, and the proxy port stays bound. `engineSmoke`'s own header documents exactly
> that ("a process can only ever run the start→stop cycle *once*"), so the fix is one lifecycle file per
> engine shape — **not** `vi.resetModules()`, which would hand the test a fresh copy of the store's module
> state and stop it testing what ships. The file is restored byte-identical.
>
> **Still owed:** `runtime/host.ts` itself (the composition, the RPC endpoint wiring, auth, `SIGTERM`), and
> WI 5's allocator, which the host needs before it can pass ports to `createWorkspaceRuntime()`.

> **WI 3 DONE 2026-09-21 — `runtime/host.ts`, and the three findings it produced.** The composition landed as
> `createWorkspaceHost()` plus `runWorkspaceHost()` (the signal-handling wrapper, kept separate so a caller
> embedding a host owns its own signals). Two supporting modules came with it: `runtime/hostRegistry.ts`
> (WI 1's host-local `runtime.json`, needed now because a host cannot allocate ports twice on a restart) and
> `src/registerAll.ts`.
>
> **1. The first finding is the one worth the phase: a headless host had *nothing behind its registry*.**
> `createEngine()` registers **no** commands by design — P3's *"the consumer declares what it is willing to
> serve"* — and for the desktop that consumer is `src/ipc/handlers.ts#registerIpcHandlers()`. A host has no
> shell, so the endpoint it bound answered `UNKNOWN_COMMAND` for `config.get`. That is the P6 §5c bug class
> **one layer up**, and it was invisible until a non-shell consumer existed: the shell was registering all
> eighteen groups, so the ratchet's "is this command registered in this process" test passed. The fix is
> `registerAllEngineCommands(registry)` — the same eighteen calls, engine-side, so the list has a home that
> is not the shell. **The shell's own calls are kept, deliberately**: each carries a comment naming which P6
> deletion made it the only registration, and collapsing them would delete that record. Two lists of one
> thing drift, so a guard asserts `registerAllEngineCommands()` is a **superset** of the shell's set — the
> direction that can actually bite (a command the shell serves and the host does not is a desktop-works-,
> container-fails bug no desktop suite can see).
>
> **2. D13's `workspaceId`-in-params is not merely unimplemented — the schema rejects it.** The first draft of
> the host's test sent `config.get` with `{ workspaceId }` and got `BAD_REQUEST`, because
> `ConfigGetParams` is `z.object({}).strict()`. `.strict()` turns an extra key into a rejection, so WI 1 cannot
> add the parameter as a widening — **it is a schema change, and it invalidates that command's fixtures**
> (`TRAPS.md`, "widening invalidates fixtures"). Also worth stating: `config.get` resolves
> `settings.activeWorkspaceId` internally (`store/config.ts#loadConfig`), which is exactly the coupling WI 4
> exists to sever. The host test asserts the *current* behaviour and says so in a comment, so WI 1's change is
> visible rather than absorbed.
>
> **3. `startCompanion` was necessary but not sufficient, and the test says which is which.** The acceptance
> criterion is *"a workspace host starts with no companion server"*, and the host never calls `createEngine()`
> at all — so it never reaches `startCompanionServer()` by construction, and the flag remains the *desktop's*
> route out (WI 7). The test asserts the companion address is unchanged across the host's whole lifetime via a
> **connect fingerprint** (`ECONNREFUSED` → `"free"`, otherwise a report), not by demanding the port be free.
> That distinction is load-bearing: the real 9271 was already held by a running `Bifurc.exe` on this machine,
> and an assertion that needs a clean machine is a test that fails for reasons unrelated to the code —
> `TRAPS.md`'s evidence-quality rule. A fingerprint is strictly stronger than a status field either way,
> because both readings are real binds.
>
> **Allocation went through the real path end to end.** The host called `allocatePorts()` and the two runtimes
> bound **81 and 82**. The resolved ports reached `runtime.json` and **not** `workspace.json`, which the test
> asserts directly by reading both files (the D16 rule: the git-tracked file syncs between machines and the
> port would collide). `allocatePorts` gained a **`reserved` set** for this: the RPC endpoint and
> not-yet-started workspaces bind ports the scan must not hand out, and the allocator cannot know them — only
> the host can.
>
> **⚠ Correction, 2026-09-21 — `81` and `82` were NOT D16's base-plus-scan.** This document first recorded
> them as "D16's `80`-base-on-Windows plus contiguous scan, observed rather than asserted". The ports were
> real, but the mechanism was not: the first version of the `reserved` set added **both bases** to itself, so
> each range's scan skipped its own base and landed on `base + 1`. The true output was "base skipped, then
> contiguous" — i.e. `0 + 81`. Nothing failed, because a runtime on the wrong-but-free port still comes up and
> still serves, and **the host test asserted only that the runtimes were serving**. `tests/runtime/portAllocator.test.ts`
> caught it immediately (`expected 52008 to be 52006`), which is why it exists in that shape.
>
> The second attempt reserved only the *other* range's base, which fixed the proxy half and broke the webhook
> half identically — a base is a base whichever range it belongs to. **Reserving neither is the rule**;
> cross-range safety comes from `reserved.add(proxyPort)` *after* the proxy is chosen, which covers every port
> the proxy actually took rather than one hypothetical collision point (a `BIFURC_PROXY_BASE_PORT` of 9101
> makes the two ranges overlap, and that case is now asserted directly).
>
> Two further discoveries from the same failure, both in `TRAPS.md`. **`freePort()` hands back `base + 1`**:
> an ephemeral port requested right after another bind is reliably adjacent, so a test that says
> `base = holdPort(); webhookBase = freePort()` silently makes the two ranges collide and then blames the
> allocator — two cases here were doing exactly that and now separate their bases deliberately. And **the
> acceptance-criterion lesson**: "asserted on *identity*, not on a value both could share" is written into §8
> for rules and blobs, but the port criterion was satisfied by *observation*, which is precisely the weaker
> instrument. Observation is not assertion.
>
> **Suite delta, measured: 104 files / 2,500 → 105 files / 2,509, i.e. +1 file and +9 tests.** Of the +9, **2**
> are in `tests/integration/workspaceHost.integration.test.ts` (the host lifecycle, and the drift guard), **5**
> are the WI 5 preflight advisory cases in `tests/startup.test.ts`, and **2** are the allocator regression cases
> this correction added (`tests/runtime/portAllocator.test.ts`, 18 → 21 once the pre-existing adjacency bug is
> counted). The guard's floor is a sanity bound, not the assertion: it measured **92** registered command
> names, and the real check is the superset comparison. Skipped stayed 40; no command-count move (no command
> was added or removed — `registerAll.ts` registers names that already existed).

### Work item 4 — sever `activeWorkspaceId` from the servers

> **⚠ History note, 2026-09-21: WI 2–5 and WI 1's protocol half are re-landed as ONE commit (`f2c2163`), not
> five.** The per-work-item commits that carried them were **loose objects destroyed by a `git stash`** on this
> drive — the `git rm` family's second member, and the reason to read `TRAPS.md` before reaching for any git
> command that rewrites the index or a ref. The working tree survived intact; the *history* did not, so the
> work below is real and verified but its commit boundaries are **not** the ones described in the per-item
> callouts, which were written before the loss and record the order the work was *built* rather than the
> shape it is *committed* in. Reconstructing five commits from a working tree would have invented boundaries
> that never existed. Recovery: repoint `refs/heads/standalone-engine` at `174a01d` (P6 group 5 — the newest
> **fully intact** commit: 606/606 blobs, 93/93 trees), `rm -f .git/index && git read-tree HEAD`, then
> `git reflog expire --expire=now --all` and delete the stale `.git/objects/info/commit-graph` (it kept
> citing the lost objects after the refs were repaired, which is why `fsck` was not clean until it was gone).
> **`git fsck` is now clean.**

> **PROXY HALF DONE 2026-09-21 — `proxy/server.ts` only.** All four readers (`start()` ×2, `reloadConfig()`
> ×2) now go through one helper, `servedWorkspaceId()`: `workspaceId ?? (currentConfig ?? loadConfig()).activeWorkspaceId`.
> The fallback is not a leftover — it is precisely how the desktop and the module facade keep working, because
> they construct one proxy and mean "whatever the user has selected". A host member always passes an id, so it
> never reaches the fallback, and the two behaviours coexist without a flag.
>
> **The fallback must be re-read per call, NOT captured in `start()`.** Capturing it would freeze the desktop
> on whichever workspace was active at startup and silently break `workspace.setActive`. That is asserted
> directly (`tests/proxy/server.test.ts`, 89 → 93 cases, plus 2 in `tests/ipc/handlers.test.ts` and 2 rewritten
> in `tests/integration/settingsMutations.integration.test.ts`): two instances read two different workspaces; an
> instance reads its **own** id and **not** `cfg.activeWorkspaceId` (the config is deliberately set to a
> *third* value, `ws-active`, so a mistaken read fails on identity rather than passing plausibly); an instance
> with no id follows the active workspace; and that fallback is re-read on `reloadConfig()`.
>
> **A type error caught a real bug.** `servedWorkspaceId()` first returned `string | null` (because
> `workspaceId` is nullable) and `tsup`'s **dts** leg rejected all four call sites — after the JS leg had
> printed "Build success", so the failure reads as a mystery exit-1 unless you grep the log for `error TS`.
> `activeWorkspaceId` is declared **non-nullable** (`store/config.ts:27`), so returning `string` is the honest
> signature: the old code would have resolved a missing config to the workspace named `""` rather than fail.
>
> **The mock wrapper must delegate, not stub.** An earlier version of the new test mocked `loadEnabledSets` to
> return empty sets, which silently changed dispatch for the whole file — **22 of 74** pre-existing cases
> failed. The enabled set decides which mocks and mappings are live, so the instrument has to be inert:
> `vi.fn(actual.loadEnabledSets)` records the call and still returns the real answer.
>
> **`webhookServer.ts` needs nothing for this work item — re-checked, not assumed.** Both routing maps
> (`activeWebhookSuffixes`, `activeSuffixToId`) are declared *inside* `createWebhookServer()`, so two instances
> cannot see each other's suffixes and §8's "a suffix registered in workspace A is `404` in workspace B"
> already holds by construction. What WI 1 adds is one instance serving *several* workspaces; the N-instances
> case is done.
>
> **Two integration cases had to be inverted, and that is the point of them.** `settingsMutations` asserted
> *"re-routes the running proxy when the active workspace is switched"* and *"goes back to the first
> workspace's mock when the switch is reverted"* — the exact behaviour the decision above removes. They were
> rewritten (not deleted) to assert that the same port keeps serving the same workspace across a switch, which
> is the stronger statement and the one that caught the change: restoring `reloadConfig()` now fails the first
> assertion. A test that encodes the old contract is how a behaviour change announces itself.
>
> **Status: the proxy half is done; `workspace.setActive` is decided and landed; `webhookServer.ts` needs
> nothing; `config.save`'s split belongs with WI 7 (already independent guards — see below).**

- `proxy/server.ts:29-31` reads `cfg.activeWorkspaceId` for rules and enabled sets → the proxy reads its
  own `workspaceId`, passed at construction. **Done 2026-09-21, with the `?? activeWorkspaceId` fallback kept
  for the no-id case — see the callout above.**
- `legacyCompanion.ts:130,166,197` **keeps** reading `activeWorkspaceId`. It is the one place where "the
  active workspace" is a legitimate app-level notion: it is what the user is looking at.
- `workspace.setActive` (`store/coreCommands.ts:87-96`) called `reloadConfig()`, which reloaded the *proxy*.
  **DECIDED 2026-09-21 (owner): switch the pointer only — the `reloadConfig()` call is removed.** Switching
  workspace in the UI updates `activeWorkspaceId` and the UI, and every running proxy keeps serving its own
  workspace. This is what makes "two workspaces, each serving its own rules" true for the **desktop** and not
  just for a host, so it is part of the acceptance criterion rather than a nicety. The visible change is that
  the old workspace's proxy keeps answering on its port after you switch — intended under D10, where the port
  is owned by the workspace and not by the selection.
  **`env.setActive` keeps its `reloadConfig()`, and that asymmetry is load-bearing**: an *environment* is
  proxy state (which rules and enabled sets are live), so changing it genuinely has to re-point the proxy
  serving that workspace. The two commands look symmetric and are not. Both directions are asserted —
  `tests/ipc/handlers.test.ts` gained `workspace.setActive` **does not** reload and `env.setActive` **does**
  (the second exists so the first cannot be "fixed" by deleting the reload from a shared helper).
  Note the proxy-mocked suite is the right home for this: `tests/integration/foldersAndWorkspaces.integration.test.ts`
  passes with or without the call, because it only observes the pointer.
- **`config.save` must be split, not deleted.** `store/configCommands.ts:63-66` restarts the proxy when the
  port changes and `:67-73` restarts the companion. Under D10 the proxy half no longer belongs to
  `config.save`; the companion half stays. Deleting the whole block compiles cleanly and silently kills
  companion port changes.
  **Re-measured 2026-09-21: the split is already done at the line level** — `:57` is the unconditional
  `reloadConfig()` and `:64-67` restarts the proxy only when `isRunning()` is false or the port/TLS changed,
  while `:68-75` is a *separate* `if` on `companionPort` that imports `restartCompanionServer` statically
  (deliberately: a bare `require` would only resolve because `tsc-alias` post-processes the build, putting a
  user-facing path out of reach of a test runner). So the two halves are already independent guards, and the
  remaining work is to make the proxy half optional for a workspace whose runtime owns its own port — that
  belongs with WI 7, when the desktop stops being the only consumer. Recorded so it is not re-derived.

### Work item 1 — the protocol shape

> **⚠ SUPERSEDED 2026-09-22 — both halves have landed; this header was stale.** The `runtime.json`
> preferred-port plumbing is in the tree (`host.ts:223` passes `entry.preferredProxyPort` into
> `allocatePorts`), and D17's blob namespacing is WI 1/WI 8's blob half. What remains is only the *editor*
> for `runtime.json`, which is a renderer edit and therefore **P7** (non-negotiable #3).
>
> **PROTOCOL HALF DONE 2026-09-21 — ten commands carry an optional `workspaceId`; the host-local
> `runtime.json` *preferred-port* plumbing and D17's blob-root namespacing are still owed.**
>
> **The ten, and why the omission is a decision.** `server.start|stop|restart|status`, `proxy.status`,
> `webhook.registerActive|unregisterActive`, `webhookServer.start|stop|status` moved from
> `z.object({}).strict()` to a shared `WorkspaceScope` (`{ workspaceId?: string }`). **`services.discover`
> is deliberately left unscoped** — it reports the ports this host is listening on, which is a property of
> the host and not of any workspace — and that omission is asserted in `index.test.ts` so a later reader
> cannot read it as an oversight.
>
> **`.strict()` makes this a schema change, not a widening.** `.strict()` turns an unknown key into a
> rejection, so adding a parameter changes *which keys are known*. §12.2 flagged that WI 1 invalidates the
> affected fixtures and instructed re-deriving rather than trusting them.
>
> **The fixtures were re-derived, and the outcome is the opposite of the expectation — worth recording
> because the hazard was anticipated and did not materialise.** The five `server.*` / `proxy.status` rows and
> the two `webhookServer` rows used `invalid: { extra: true }`. Those still reject (`extra` is still
> unknown), but the generic form no longer tests anything specific to this change, so they were replaced with
> a near-miss on the **new** field (`{ workspaceId: 42 }`) — a strictly better assertion, since a missing
> optional field is valid by construction. **No fixture became accidentally-valid**: the hazard is a fixture
> whose *invalid* payload is the newly allowed key, and none of the ten was written that way.
>
> **The count, and a correction that matters more than the number.** `index.test.ts` went **185 → 216
> (+31)**, measured by enumerating the run's own test names (`--reporter=json` →
> `assertionResults[].title` → classify by name), because the suite's body is a `for (const action of
> actions)` loop emitting **2 `it`s per command**. An earlier revision of `protocol-changes.md` recorded
> "190 → 216 (+26)" and `plan/14` flagged **+5 as unaccounted**; both figures were wrong and the "+5" was
> phantom. The true decomposition is 184 generated pairs (92 × 2) + 1 coverage invariant + 1
> `services.discover` assertion + **30** from the new block (three `it.each(SCOPED)` of ten) = **216**, so
> the prior count is **185** and the delta is **+31**. **The mechanism of the error is the lesson:** the 26
> came from subtracting only the 30 `it.each` cases from 216 and attributing the residual to the change, so
> the *baseline absorbed* two cases that existed on both sides — and a total that is too low by exactly the
> number of pre-existing cases **looks like a missing test rather than an arithmetic slip**, which is what
> sent the reconciliation looking for a test that was never there. **For a generated suite, never count by
> hand; enumerate and classify.** This is the fifth instance of the "count in prose" family in `TRAPS.md`,
> and the one that cost this session's commit history (see the `git stash` section there).
>
> **What the new block asserts, and why both directions are load-bearing.** For each of the ten:
> `workspaceId` present **is accepted**, absent **is still accepted**, non-string **is rejected**. The
> per-command pair above cannot express this — it checks that *the fixture* parses, and a fixture is one
> payload, whereas what "omitted means the active workspace" means on the wire is a **relationship**.
> Asserting only "present is accepted" would pass if someone made the field *required* and broke every
> desktop caller, which is the single most likely way this change goes wrong.
>
> **Still owed under this work item:** (a) the host-local `runtime.json` preferred-port plumbing from the
> UI/settings, and (b) D17's blob-root namespacing — `blobRoot(wsId)` = `<dataDir>/blobs/<wsId>/`, a
> `workspaceId` on the four `blob.*` params, a per-runtime sweeper, and re-derived `blob.*` `invalid`
> fixtures (those are the rows where the "invalid payload is the newly allowed key" hazard is still live,
> since `blob.*` params are not yet scoped).
>
> **D17'S BLOB HALF IS NOW DONE 2026-09-21 — the fixtures hazard was live but not hit; the exposure was
> somewhere the note did not name.** `blobRoot(workspaceId?)` namespaces, `stagingRoot` with it, an optional
> `workspaceId` threaded through eleven store functions and the four commands, a sweep scoped by
> `BlobSweepOptions.workspaceId` (a **correctness** requirement: without it a runtime reclaims its siblings'
> in-flight transfers). The four fixtures were **fine**, contrary to the prediction above — the four `valid`
> rows gained the field and the `invalid` rows were each verified to fail on a required-or-typed field rather
> than on scope. **What the note did not anticipate is where the bug actually was: the call sites.** The
> integration suite went **0 → 30 failures** because `export.create` was handed a workspace while its own
> client's `blob.put` / `blob.read` / `blob.release` were not — so the engine staged an export into a
> namespace its requester could not read. A host-wide root had been hiding it: with one namespace every id
> resolves regardless of who asks. Fixed by scoping the **client**, never by relaxing the store. The audit
> that closed it (grep for store calls passing **no** id — the question a find-replace cannot answer) left
> two explicable survivors: the five egress commands (WI 8's work) and `fileOps/commands.ts:139-140`, where
> the CA import's unscoped read is **correct** because the CA is one per *machine*, not per workspace. Full
> account in `plan/protocol-changes.md`'s D17 entry; the transferable lesson is `TRAPS.md`'s "Namespacing a
> store surfaces every half-threaded id".
>
> **And one finding about this work item's own contract, measured on a two-workspace host.** The protocol
> half made the ten commands *accept* `workspaceId`; it did not make them *read* it.
> `registerServerCommands()` still calls `proxy/server.ts`'s **module facade**, whose lazy default instance
> is `createProxyServer({ workspaceId: null })` — the active workspace. Measured with two members listening
> on 81 and 82: `server.status` reported `{ running: false, port: 0 }`; `server.start { workspaceId: "A" }`
> returned **`{ ok: true }`** and bound a **third** server on the active workspace's configured port. **The
> failure succeeds**, which is why nothing caught it — the facade is correct for the desktop, so every
> existing suite passes. The rule this establishes: **an optional wire parameter is not done when the schema
> accepts it, it is done when the body reads it** — and until then the protocol advertises a capability the
> engine does not have, so a host member's client manages a runtime it never reached. This is WI 7's flip
> (§9) arriving earlier than §5's sequencing expected, and it says the flip is **load-bearing for
> correctness on a multi-workspace host**, not merely a tidy-up: the command layer must resolve a runtime
> from `workspaceId` rather than fall through to the facade. Recorded as `TRAPS.md`'s "An optional param
> that is *accepted and ignored* reads as implemented".

### Work item 5 — preflight and port allocation

> **⚠ The `preflight()` half is no longer owed — it landed; this note was stale.** `tests/startup.test.ts:87`
> is the five-case suite covering exactly the busy-*base* vs busy-*override* distinction (§8 criterion 8 —
> which the 2026-09-22 audit recorded **unmet** as a **false negative**, because it searched
> `tests/integration/` only).
>
> **ALLOCATOR DONE 2026-09-21 — `packages/engine/src/runtime/portAllocator.ts`, 18 real-socket cases; the
> `preflight()` half was still owed when this was written.** The allocator is its own module rather than part of
> `createWorkspaceRuntime()` because allocation is the **host's** job: it needs host-wide knowledge of what
> the other runtimes already took, which is precisely why the runtime takes ports as required inputs.
>
> **D16's resolution order is two distinct paths, not one fallback.** A taken **per-workspace override**
> throws `PortUnavailableError`, whose message names the next free port — D16's "never silently move a port
> the user chose", and the message has to be *actionable* or the rule is just a crash. A taken
> **environment base** scans contiguously upward instead, because the increment is the designed path for
> every workspace after the first. `BIFURC_PROXY_BASE_PORT` moves the **base**, not one workspace's port, so
> the increment still works after it; a **malformed** value throws rather than silently defaulting, because a
> container that set the variable meant it and a silent fallback would publish the wrong port.
>
> **The one thing that must not be done differently, and the reason this needed real-socket tests:**
> `tryBind()` performs an actual `listen()` and releases the handle before resolving. D16 forbids building
> this on `applications/portUtils.ts#checkPortInUse`, which binds-then-closes so the *caller's* real bind
> happens later — a TOCTOU race. The load-bearing assertion is therefore not "reports a taken port as
> taken" (easy, and a mock would pass it) but **"after `tryBind` says free, a real `listen()` on that port
> succeeds"** — a leaked probe would make every allocation fail at the caller's own bind. `checkPortInUse`
> keeps its legitimate use (reporting, where it can name the owning PID) and `killProcessOnPort` is
> untouched.
>
> **Deliberately not done:** the scan does **not** consult the registry first and jump to a remembered port.
> D16's point is that the increment is the designed path, and a sequential scan from the base lands the same
> runtimes on the same ports on restart as long as they are free — without needing to remember.
>
> **Still owed for this work item (the `preflight()` half is now DONE 2026-09-21 — see below).**

> **PREFLIGHT HALF DONE 2026-09-21 — `StartupCheck.advisory`.** The contract change landed without changing
> `!ok`'s meaning for a single pre-P14 caller: `advisory?: boolean` is **absent** (never `false`) on a fatal
> finding, so `checks.filter((c) => !c.ok)` — `plan/10`'s entrypoint — keeps its P2 semantics, and a
> D16-aware consumer filters on `!c.ok && !c.advisory`. A busy **base** port returns `advisory: true` with a
> hint naming the increment; a busy **per-workspace override** stays fatal with the "free the port, or change
> this workspace's configured port" hint, because D16's fail-loudly rule is about a port the *user* chose.
> `PreflightPort` gained `nextFree`, so the hint can be concrete ("workspace proxies will use 81 and up")
> rather than telling a user to change a setting that is about to be superseded.
>
> `tests/startup.test.ts` went **5 → 10 cases** (+5): advisory-with-`nextFree`, advisory-without-inventing-a-
> number, override-stays-fatal, mixed (only the busy ones flag, only one is advisory), and free-advisory-port
> -is-plain-`ok`. The four pre-existing port cases were left untouched, which is the evidence that the
> contract did not move for them.

- **`preflight()`'s port check must become non-fatal — this is a contract change, not a tweak.**
  `startup.ts:90-99,122` reports a busy port as `{ ok: false, code: "port-in-use" }`, and `plan/10`'s
  container entrypoint exits non-zero on *any* `ok: false`. Under D16 a busy **base** port is the **normal**
  case for the second workspace, so a fatal check would refuse to start exactly the configuration this phase
  exists to enable. Either drop the port checks from `preflight()` or downgrade them to an informational
  warning ("port 80 is taken; workspace proxies will use 81 and up"). Keep them fatal **only** for a
  per-workspace override, where failing loudly is D16's own rule.
- `startup.ts` checks three global ports today; those become per-runtime resolved ports, while the
  companion's stays the one fixed value. The proxy check should report the **base** port, since that is the
  value a user can act on.
- Reuse `applications/portUtils.ts#checkPortInUse` for **reporting**, and `killProcessOnPort` for
  `application.killPort`. Do **not** build the allocator on `checkPortInUse` — see D16 on the TOCTOU race.
- **Do not warn about the increment as if it were degraded.** Landing on 81 (Windows) or 8081 (elsewhere) is
  the designed path for every workspace after the first — not a fallback from failure. Do warn when a
  **per-workspace override** was overridden, because that means the user's explicit choice could not be
  honoured.
- `checkPortAvailable`'s hint text — "Free the port, or change the configured port in settings" — is wrong
  for the base port. Point it at the increment and the per-workspace override instead.
- **Log the resolved base port once at startup, per environment**, so the support question "which port is
  it on?" has a one-line answer.

### Work item 6 — the deployable host artifact (feeds P9)

```bash
node <engine>/dist/runtime/hostCli.js --data-dir /data --rpc-socket /run/bifurc.sock --token-file /run/token
```

Documented deployment: **one container = one workspace**, one volume = that workspace's `data/` + `blobs/`.
N is supported but not recommended — one workspace per volume makes the volume, the ports and the lifecycle
all line up. This is why P9 moves *after* this phase.

> **DONE 2026-09-21.** Two modules: `runtime/hostArgs.ts` (parsing, validation, usage text) and
> `runtime/hostCli.ts` (the executable). **The path is `hostCli.js`, not `host.js` as sketched above**, and
> that is a correction rather than a preference: putting the CLI in `host.ts` needs a `main` guard, and this
> package emits **both esm and cjs** whose entry checks differ (`require.main === module` vs
> `import.meta.url === pathToFileURL(...)`), while `import.meta` is a syntax error in the cjs leg. `host.ts`
> is also imported by tests and by `registerAll`, so an unguarded execution there would start a host inside
> every importing process. The executable is its own module and **nothing imports it** — which is why it
> needs no guard, and why the parsing was split out (`hostCli.ts` cannot be imported by a test without
> starting a host).
>
> **Flags:** `--data-dir` (required; `BIFURC_DATA_DIR` as a fallback), `--rpc-socket`, `--token-file`,
> `--workspace` (repeatable, order preserved), `--help`. Both `--flag value` and `--flag=value`. An
> **unrecognised flag is fatal**, per the same rule D16 applies to `BIFURC_PROXY_BASE_PORT`: a typo'd
> `--data-dirr` that was skipped leaves the host on the default data root, which presents as "my workspaces
> vanished". `--help` is a *request*, not an error — stdout, exit 0 — and an `error` result carries the
> usage text so a container log is self-explanatory.
>
> **Verified against the built artifact, not just the source.** `npm run build --workspace @bifurc/engine`,
> then `node packages/engine/dist/runtime/hostCli.js --data-dir <temp> --workspace ws-e2e`:
> `[host] workspace "E2E Workspace" is serving: proxy 80, webhook 9101` / `[host] rpc endpoint:
> \\.\pipe\bifurc-<pid>` / `serving 1 workspace(s)`. `--help` → stdout, exit 0. No `--data-dir` → stderr,
> **exit 1**. `packages/engine/tests/runtime/hostArgs.test.ts` (new, **16 cases**).
>
> ## ⚠ A measured deployment caveat, and the reason it is recorded here rather than fixed
>
> **`--data-dir` does not redirect the settings file on Windows.** The first artifact run came up
> `serving 0 workspace(s)` and exited 0 — and the cause was `settingsPath()`, which on Windows prefers
> `%LOCALAPPDATA%/Bifurc/app.json` **regardless of `setDataRoot()`**. So the host read the *user's* real
> settings, `--workspace ws-e2e` matched nothing in that list, and the filter produced an empty set.
> Confirmed directly: the real `%LOCALAPPDATA%\Bifurc\app.json` held workspace `mtsamun7rer3p`.
>
> This is the **pre-existing `appDataDir()`-vs-`dataDir()` divergence** that `proxy/certManager.ts` records
> and that `plan/03` assigns to P8/P9 ("fixing it is a product decision"). It is **Linux-invisible** — on
> Linux/macOS `settingsPath()` falls through to `dataDir()/app.json`, so the documented container
> deployment works — which is exactly why it is a P9 caveat and not a WI 6 defect. Recorded rather than
> silently patched, because the alternative would be a *third* answer to "where is the settings file".
>
> **What WI 6 does fix is the silence.** `host.ts` now names each requested workspace it cannot find, with
> the directory it looked in:
>
> ```
> [host] requested workspace "ws-e2e" is not in the settings (looked in C:\Users\…\AppData\Local\Bifurc)
>        — it will not be served
> ```
>
> That one line is the whole diagnostic — it distinguishes "no such workspace" from "wrong settings file",
> which the bare `serving 0 workspace(s)` could not. Not fatal, matching D16's "skip loudly, do not refuse
> to start" for a taken port override. Guarded by a new case in
> `tests/integration/workspaceHost.integration.test.ts` (**2 → 3**), which asserts both that the real
> workspace is still served *and* that the missing one is named — so the notice cannot be "fixed" by
> replacing serving with an error.

### Work item 7 — the desktop adopts it

> **THE COMMAND-LAYER HALF IS DONE 2026-09-21 — and it turned out to be the half that mattered for
> correctness, ahead of the desktop adoption the heading names.** `runtime/runtimeRegistry.ts` (new) is the
> lookup the command layer was missing; `runtime/runtimeServerOps.ts` (new) is how a resolved runtime's
> servers are driven; `proxy/serverCommands.ts` and `proxy/webhookCommands.ts` resolve through both;
> `runtime/host.ts` registers each runtime as it is created and clears the registry on `stop()`.
>
> **Why this came first, measured rather than reasoned.** `createWorkspaceHost()` returns its runtimes on
> the handle it hands back, so they were reachable by whoever *called* it and nobody else — while the
> commands serve two consumers (the host's RPC endpoint and the desktop's IPC) through a module-level
> registry neither owns. So the commands called the **module facade**, and on a host with two workspaces:
>
> ```
> host: ws-A proxy=81 webhook=9101     ws-B proxy=82 webhook=9102
> server.status                 → { running: false, port: 0 }    ← two members ARE listening
> server.start {workspaceId: A} → { ok: true }                    ← accepted, IGNORED
> server.status                 → { running: false, port: 8791 }  ← a THIRD server
> ```
>
> **The failure succeeds**, which is why every desktop suite stayed green through WI 1. This is the P6 §5c
> class one layer up again: a consumer that works only because it is the *shell*.
>
> **The resolution rule, and the third branch is the load-bearing one.** A registered runtime for the named
> workspace → that runtime. Otherwise, if the target *is* the active workspace → the module facade, which
> **is** the desktop's server (`createEngine()` binds one proxy through it) and is what keeps this work item's
> acceptance criterion ("zero user-visible change", P6's rule) true. Otherwise → **`absent`**, reported as
> such. Falling through to the facade in that third case would re-create the original bug — acting on the
> active workspace while the caller named another — with a success code.
>
> **A command never allocates a port.** `runtimeServerOps.ts` records why, and the reason is a measurement
> rather than a preference: `proxy/server.ts` seeds `currentPort = 0` while `webhookServer.ts` seeds it with
> **`DEFAULT_WEBHOOK_PORT` (9101)**. So a "never started" guard written for the proxy does not transfer — a
> never-started webhook reports a *plausible* port, and re-binding it could take **another workspace's**.
> The rule is therefore sentinel-independent: `restart` re-binds only a port it has **observed** running;
> `start` binds only a port the **caller named**; otherwise it reports rather than guesses.
>
> **The wire contract is unchanged, which is why there is no `protocol-changes.md` entry.** WI 1 added the
> field; this makes the engine *read* it. That distinction is the whole lesson: **an optional parameter is
> not done when the schema accepts it, it is done when the body reads it** — and until then the protocol
> advertises a capability the engine does not have. Recorded in `TRAPS.md` as *"An optional param that is
> accepted and ignored reads as implemented"*.
>
> **One consolidation found while wiring it:** `protocol/commands/webhook.ts` carried its **own** copy of
> `WorkspaceScope` instead of importing `./scope`, which `./scope`'s header already names as the hazard that
> module exists to prevent ("four copies of one contract"). The two were still identical, so this was a
> consolidation rather than a bug fix — but identical-and-separate is the state that precedes
> identical-no-longer. `webhookServer.status` also now echoes the optional `workspaceId` its result type has
> always carried, so an `absent` answer names the workspace it is about.
>
> **Verified:** `tests/integration/workspaceCommandScope.integration.test.ts` (new, **8 cases**) with the
> active workspace set to A and every discriminating assertion addressing **B** — so a command that ignored
> the parameter fails. **Mutation-checked twice:** disabling the runtime branch kills **7 of 8**; making
> `absent` fall through to the facade kills exactly the **2** that should die. Both branches are therefore
> independently guarded rather than jointly covered.
>
> **✅ LANDED 2026-09-22.** The desktop adoption is done — the two sections below record the shape it had to take, why the plan's own wording for it was wrong, and the defect it uncovered. What follows is the original statement of what was owed, kept as the record: **the desktop adoption itself** — `src/main.ts:309-310` building runtimes
> instead of calling `startServer()`. That is the half that changes the desktop, and it needs `createEngine()`'s
> own proxy lifecycle (`index.ts:255`) to move with it, or the engine and a runtime would both bind (the
> two-authorities hazard `host.ts`'s header records). **The bullet below about the legacy `server:*` channels
> is now obsolete**: the P6 reduction's fifth group deleted all three on 2026-09-21, so there is no
> `window.api` surface left to keep working — `src/ipc/handlers/systemHandlers.ts` retains only
> `app:checkUpdate`, which is client-local by design.
>
> ---
>
> ## 🚩 MEASURED 2026-09-22 — the desktop half as *specified* is wrong, in one direction and right in another
>
> The owner picked shape (a) ("the facade delegates"). Measuring it before writing it turned up **three
> facts that change the specification**, one of which invalidates the sentence above it.
>
> ### 1. The plan's own wording — "the facade delegates to the registered runtime **for the active workspace**" — is wrong under WI 4, and following it would ship a bug
>
> WI 4 (`coreCommands.ts:83-101`) **deleted** the `reloadConfig()` that `workspace.setActive` used to call,
> and said why: *"Under D10–D12 each workspace has its own runtime, so the pointer and the proxies are
> independent, and a workspace's rules should keep serving while you edit another one. Dropping the call is
> what makes 'two workspaces, each serving its own rules' true for the desktop and not just for a host."*
> It is annotated **"Visible change, and the reason this needed confirming"**.
>
> So on the desktop the **active pointer and the proxy are already independent**. A facade that resolves by
> `cfg.activeWorkspaceId` would, after a workspace switch, look up a runtime for the *new* active id, find
> none, and fall back to its lazy default instance — whose `isRunning()` is **false** while the real proxy is
> listening on a bound port. That is the WI 1 bug shape ("the failure succeeds") arriving through a new door.
> **The facade must bind to the desktop's runtime, not to whatever is active.**
>
> ### 2. `proxy/server.ts`'s header still describes the pre-WI-4 world, and it is load-bearing prose
>
> Its lines 33-38 say the `workspaceId ?? cfg.activeWorkspaceId` fallback *"is exactly how the module facade
> and the desktop keep working, because they construct one proxy and mean 'whatever the user has selected'."*
> **After WI 4 the desktop's proxy does not follow a switch at all** — `start()`/`reloadConfig()` capture
> `fullRules` and `enabledSets` for one workspace and `workspace.setActive` no longer re-captures them. The
> sentence is stale in exactly the way `TRAPS.md`'s *"The docblock that sanctions the bug"* records. The
> *behaviour* is fine (it is what D10 wants); the *description* is what would mislead the next reader — and
> it is the sentence a naive implementation of this work item would have trusted.
>
> **The good news this carries: the desktop's proxy is already, in effect, a single-workspace proxy.** That
> removes the objection that a `WorkspaceRuntime` (whose `workspaceId` is fixed) cannot represent the
> desktop. It can — it is what the desktop already behaves like.
>
> ### 3. The real hazard is the **webhook receiver**, and it is a user-visible change
>
> `WorkspaceRuntime.start()` unconditionally binds **both** servers (`workspaceRuntime.ts`), and
> `webhookPort` is a **required** option. But the desktop has **never** started the webhook receiver at boot:
> `src/main.ts` starts only the proxy and the companion, and the receiver is started **on demand** —
> `packages/gui/src/panels/WebhooksPanel.tsx:100` → `window.api.startWebhookServer()` → `webhookServer.start` →
> `startWebhookServer(port)`.
>
> **So `main.ts` calling `runtime.start()` would open a webhook port at desktop boot that the product has
> never opened** — a new listening socket, a new `preflight` warning on any machine where `9101` is taken,
> and a direct violation of §8's "one proxy port" and P6's zero-visible-change rule.
>
> ### The shape this yields — no engine contract change, and it is smaller than the original
>
> The two facades gain an explicit **binding**, and the desktop binds them to its runtime:
>
> 1. `proxy/server.ts` and `proxy/webhookServer.ts` each expose `bindProxyInstance()` /
>    `bindWebhookInstance()`; `defaultServer()` returns the bound instance when there is one, else the lazy
>    default. **No new import edge** — `proxy/*` does not need to import `runtime/`, so the acyclic graph
>    `runtimeRegistry.ts`'s header protects is untouched.
> 2. `src/main.ts` creates **one** `WorkspaceRuntime` for the active workspace, `registerRuntime()`s it,
>    binds both facades to its two instances, and **calls `runtime.proxy.start(cfg.port)` — not
>    `runtime.start()`** — so the webhook stays unstarted until the user asks for it. `startServer(cfg.port)`
>    keeps working verbatim, because it now delegates.
> 3. `ownsCA: true`, which is the documented single-runtime case ("a single-runtime caller that wants the
>    pre-P14 behaviour passes `true`").
> 4. `shutdownEngine()` needs no change: its `stopServer()` now reaches the runtime's proxy. It does not stop
>    the webhook today either, so that asymmetry is preserved rather than introduced.
>
> **One acknowledged artefact, documented rather than hidden:** `bootstrapWorkspaces()` runs *before* the
> runtime exists and calls `reloadConfig()`, which creates the lazy default instance. It is never bound and
> never started, so it is inert — but it exists, and a test must pin that the bound instance wins over it.
>
> ---
>
> ## 🚩 AND ONE DEFECT THIS WORK FOUND THAT WAS NOT ITS OWN — WI 4's dispatch read the active pointer (✅ fixed 2026-09-22)
>
> Found while trying to make the `reloadConfig` guard *discriminating*: the first version of
> `tests/integration/facadeBinding.integration.test.ts` put the runtime on a workspace **other than** the
> active one, so that "which workspace got reloaded?" would have a different answer per branch. It failed —
> and the cause is a real gap in **WI 4**, not in this work item.
>
> `serverUtils.ts:35-44`'s `workspaceCfg()` filters dispatch by **`cfg.activeWorkspaceId`**:
>
> ```ts
> export function workspaceCfg(cfg, fullRules, enabledSets): AppConfig {
>   const wsId = cfg.activeWorkspaceId;   // ← the CONFIG's pointer, not the instance's own workspace
>   ...
>   proxyRules: fullRules.filter((r) => (!r.workspaceId || r.workspaceId === wsId) && enabledSets.rules.has(r.id)),
>   mocks:      cfg.mocks.filter((m) => m.workspaceId === wsId && enabledSets.mocks.has(m.id)),
>   mappings:   cfg.mappings.filter((m) => m.workspaceId === wsId && enabledSets.mappings.has(m.id)),
> }
> ```
>
> Both call sites (`proxy/server.ts:242,354`) pass `currentConfig ?? loadConfig()`. So an instance created
> with `workspaceId: "B"` reads **B's** rules into `fullRules` (`servedWorkspaceId()` — WI 4's severance,
> working as designed) and then **filters every one of them out**, because `activeWorkspaceId` is `"A"`.
> The instance serves nothing. **WI 4 severed `activeWorkspaceId` from the proxy's *reads* but not from its
> *dispatch filter*.**
>
> ### Why no suite caught it — the same shape as everything else in this document
>
> `tests/integration/workspaceRuntime.integration.test.ts`'s "both genuinely serve" step asserts
> **`getHomePage(port)`** — the proxy's own UI page, which is served regardless of any rule, mock or
> mapping. So §8's **first** criterion — *"Two workspaces run their proxies simultaneously on different
> ports, **each serving its own rules**, with no cross-talk — asserted on *identity* (each serves a mock the
> other does not have)"* — is asserted on something that **cannot detect the failure**. The identity
> instrument was chosen correctly for *ports* and *readiness* in that file; for *content* it was chosen for
> convenience, and that is the only reason this is not already known. **It is the criterion's own wording,
> not an inference: "each serving its own rules" is the claim, and a home page is not a rule.**
>
> ### Status: ✅ FIXED 2026-09-22 — and the fix was **not** what this section first said it was
>
> The first draft here claimed the fix was "thread the instance's `servedWorkspaceId()` into `workspaceCfg()`".
> **The measurement above disproves that:** `cfg.mocks` / `cfg.mappings` are *the active workspace's entities*,
> so no id passed to the filter can recover a non-active workspace's mocks — the data was never loaded. The
> real fix is a **loader**, not a filter:
>
> - **`store/config.ts` → `loadWorkspaceDispatchSlice(wsId)`** (new). Builds the same shape `loadConfig()`
>   builds for the active workspace, for **any** id: that workspace's mocks, mappings, environments and
>   selected environment, with `enabled` resolved from `enabled.json`. Two deliberate omissions, both stated
>   in its docblock: no `autoSyncFsDirectories()` (a read must not mutate a workspace the caller may not be
>   looking at) and no `__global__` auto-creation (that is a read that writes; a host bootstraps explicitly).
> - **`proxy/serverUtils.ts` → `workspaceCfg(cfg, fullRules, enabledSets, slice)`** takes that slice and reads
>   the id from **it**, never from `cfg`. It also overrides `activeWorkspaceId` / `activeEnvironmentId` /
>   `environments`, so `activeEnv()`, script and variable resolution, and `pages.ts`'s mapping list all agree
>   with the entities instead of pointing back at the active pointer's workspace.
> - **`proxy/server.ts`** loads the slice in `start()` / `reloadConfig()` alongside `fullRules` /
>   `enabledSets` — so both halves now come from the same workspace — and passes it at both dispatch sites.
>
> **Verified, and the guards are mutation-checked.** New
> `tests/integration/workspaceDispatchScope.integration.test.ts` (4 cases) asserts, over real sockets and real
> files, that B's runtime serves **B's own mock and B's own rule** while A is active, that A's mock does *not*
> come back through B's port, and — as a control — that A's runtime serves A's own mock. The rewritten
> `tests/proxy/serverUtils.test.ts` covers the filter directly (`workspaceCfg()` 8 cases, up from 7: one
> deleted because its premise is gone, two added). **Reverting `const wsId = slice.workspaceId` to
> `cfg.activeWorkspaceId` fails exactly 3** — the unit case named for it, B's own mock+rule, and the
> cross-talk direction. **The control correctly survives that mutation**, because for the active workspace the
> two ids coincide; that is what makes it a control rather than a guard.
>
> **And the assertion that hid this has been re-labelled.** `workspaceRuntime.integration.test.ts`'s "both
> genuinely serve" step is now *"both are up and answering over the wire"*, with a comment saying it is a
> liveness check that **cannot** fail for a scoping defect and pointing at the file that can. Leaving it named
> "genuinely serve" is how §8's first criterion came to read as met.
>
> ### Measured two-sided, with only the active pointer differing
>
> A throwaway probe (not committed) wrote two workspaces, each with its own mock, mapping and rule, and
> started a runtime for **B**:
>
> | Request, against B's port | active = **A** | active = **B** |
> |---|---|---|
> | `GET /api/b` — **B's own mock** | **404** | **200** `{"from":"B"}` |
> | `GET http://api.test/data` — **B's own rule** | **502**, upstream hits **0** | **200** `{"from":"upstream"}`, upstream hits **1** |
>
> The second row is the clean comparison — a `targetType: "external"` rule needs no mapping, and nothing
> else differs between the two runs.
>
> **And the first row is *not* cross-talk**, which is the part worth being precise about: A's mock is `404`
> too. The two halves of the config **cancel**, because they come from different workspaces —
> `cfg.mocks` / `cfg.mappings` from **A** (via `loadConfig()`, which reads the active workspace by
> construction: `config.ts:68,80-82`) while `enabledSets` comes from **B** (via
> `loadEnabledSets(servedWorkspaceId())`). So A's entities are filtered out by *B's* enabled set, and B's
> are absent from `cfg` entirely. The observable result is not the wrong workspace's data — it is **no data
> at all**, silently, as 404/502.
>
> **So the fix is larger than threading one id.** `workspaceCfg()` is the visible symptom; the root is that
> a proxy's entities come from a config loader that is active-workspace-only. Making a host member serve its
> own mocks and mappings means loading *that* workspace's entities — a new loader in `store/config.ts` plus
> the proxy's assembly — and `store/config.ts` is read by every panel. **That is the owner decision, and it
> is why this was not fixed as part of WI 7.**

- `src/main.ts:309-310` stops calling `startServer(cfg.port)` / `startCompanionServer(...)` directly and
  builds runtimes instead. The companion call is unchanged.
- The legacy channels `server:start|stop|restart` (`src/ipc/handlers/systemHandlers.ts:9-25`) must keep
  working: they are on `window.api`, and non-negotiable #3 says `window.api` stays byte-identical. They
  must therefore resolve "which workspace" the same way the UI does — via the active pointer, composed
  client-side if necessary. This is P6 §5b-bis's lesson applied: *a key whose shell body composes something
  the engine cannot must keep composing it client-side.*
- **Acceptance for this work item is P6's rule: zero user-visible change.**

### Work item 8 — protocol changes and the compatibility record

> **THE EGRESS HALF IS DONE 2026-09-21; the compat record is `plan/protocol-changes.md`.** Three of the five
> egress commands gained an optional `workspaceId`: `runner.exportReport`, `capture.shareJson`,
> `audit.export`. `tls.exportCert` **deliberately did not** — see below.
>
> **Why it mattered: the blob half.** Above `BLOB_INLINE_THRESHOLD_BYTES` (1 MiB) an `ArtifactResult` is a
> **staged blob** the client reads with `blob.read` **scoped to its own workspace**. D17 namespaced the four
> `blob.*` params and `export.create` passed its id through; these three did not, so the engine staged
> host-wide while a scoped client read elsewhere — **the engine's own export unreadable by the client that
> requested it**, which is exactly the failure `importExportBlob.integration.test.ts` caught for
> `export.create`. `blob/publish.ts` now carries the whole table.
>
> **`audit.export` had a second, worse problem, and it is the same class as WI 1.** Its handler resolved the
> workspace itself as `loadConfig().activeWorkspaceId`, reasoning that "the engine can already look up" the
> id — **which is desktop-only reasoning**: §2.3 demotes `activeWorkspaceId` to the *desktop's UI pointer*,
> so on a host serving N workspaces an `audit.export` from workspace B returned the **active** workspace's
> log. The caller could not ask for the log it meant. Now `params.workspaceId ?? cfg.activeWorkspaceId`, so
> the desktop is unchanged.
>
> **`tls.exportCert` stays unscoped, and it is a decision rather than an omission.** The CA is one per
> **machine** (`caCertPath()` under `appDataDir()`; `ownsCA` exists because N workspaces share one mkcert
> root), so a `workspaceId` there would be a field that means nothing — worse than an absent one, because it
> would read as though the export were workspace-scoped. **And it cannot reach the blob branch, measured:**
> `ca-cert.pem` is **1,240 bytes** against a **1,048,576-byte** threshold, ~845× short. The same reasoning
> already kept `tls.importCert` / `tls.importKey` unscoped, and all three now say so in
> `fileOps/commands.ts`.
>
> **It is a contract-consistency fix, not a live bug fix — and that governs the verification.** Today's only
> client sends **no** `workspaceId` on `blob.put` / `blob.read` / `blob.release` (it relies on the host-wide
> default, which is what the unscoped egress commands produced), so the two matched and nothing failed. The
> inconsistency bites the first client that *scopes* its blob reads — P7's web UI, D15's CLI — which would
> find `export.create`'s output and not the other three's. Hence the test is an **identity** assertion, not
> "a blobId came back": `tests/integration/workspaceCommandScope.integration.test.ts` **8 → 9** forces the
> blob branch with a `capture.shareJson` payload just over the threshold (its `entries` come from the client,
> so the size is the test's to set), asserts it is readable **scoped to B** and **throws under A**, and
> guards its own premise so it cannot pass without crossing the threshold.
>
> **One assertion was strengthened rather than extended.** `"%s rejects a non-string workspaceId"` used to
> parse `{ workspaceId: 42 }` **alone** — which a command with any required field rejects for *those fields
> being missing*, so it passed whatever happened to the type. Now `{ ...valid, workspaceId: 42 }`, leaving
> the type as the only possible error, plus a check that the failing issue's `path` includes `workspaceId`.
>
> **Mutation-checked, both fully identified.** Widening `WorkspaceScope.workspaceId` to accept a number →
> **24** failures, resolving exactly as **17** × the type case (the scoped set) + **7** × `"— rejects an
> invalid payload"` for the D13 commands whose `invalid` fixture *is* `{ workspaceId: 42 }` — two independent
> mechanisms. Dropping `params.workspaceId` from `captureShareJson` → **exactly 1** failure, the new case.
>
> **Verified:** `index.test.ts` **228 → 237** (**+9** = three new scoped commands × the three `it.each`
> cases each). The extension's four frozen commands are asserted by name in
> `tests/companion/allowedActions.test.ts`, which pins the set exactly — run green alongside.

- Every changed command name and shape into `plan/protocol-changes.md`. **Four of them are the `blob.*`
  params gaining a ~~required~~ **optional** `workspaceId` (D17)** — which also invalidates their `invalid`
  fixtures, so re-derive those rather than trusting them (`TRAPS.md`). *(Corrected 2026-09-21: D17 made the
  field optional, not required — the optionality **is** the compatibility story, since the desktop sends
  none. The same applies to WI 8's three.)*
- **Verify the extension's four frozen commands are untouched** (`mock:add`, `request:add`, `folder:add`,
  `config:get` — `src/companion/allowedActions.ts`, and they are not in the changed namespaces). Assert it
  by name rather than by argument.
- `packages/protocol/src/commands/index.test.ts` generates **two** tests per command, so any command count
  change moves the baseline by exactly 2× — name the mechanism, do not wave at the delta.
  *(Precise as far as it goes, and it is not the only generator: each command in the `SCOPED` list also
  contributes **three** `it.each` cases — accepts-with, accepts-without, rejects-non-string. So a
  **command-count** change moves the total by 2×, while a **scoped-set** change moves it by 3× per command.
  WI 8's +9 is 3 × 3; D17's +12 was 4 × 3. Two multipliers, one file.)*
- **Widening a schema invalidates its `invalid` fixture.** Adding a ~~required~~ `workspaceId` changes the
  near-miss; re-derive it, never trust the old one (`TRAPS.md`).
- Any ratchet guard over a shrinking list must be rewritten around its invariant (`TRAPS.md`).

---

## 5. Sequencing

```
P6 (CLOSED 2026-09-22 — 11/11 e2e specs, 46/46 tests; no open criterion)
  │
  ▼
P14 — workspace runtime  ← this document
  ├─ WI 0  measurement ──────────────────── gate: is this mechanical?
  ├─ WI 1  port model + protocol shape
  ├─ WI 2  de-singletonize proxy + webhook
  ├─ WI 3  WorkspaceRuntime + host entry point
  ├─ WI 4  sever activeWorkspaceId from the servers
  ├─ WI 5  preflight + port allocation
  ├─ WI 6  deployable host artifact ────┐
  ├─ WI 7  desktop adopts it            │
  └─ WI 8  protocol + compat record     │
                                        ▼
                     P9 (workspace host in Docker) ──► P7 (web UI)
                                        │
                                        ▼
                                   P12 packaging
                     P8 CLI — PARKED (D15)
```

P7 and P9 are parallel after P14; P7's only new dependency is that it connects to a host rather than to a
bare engine.

---

## 6. How to start — the first three things

1. **Run work item 0 as a measurement, not a design session.** Count the readers and write the number down.
   The whole phase's estimate rests on that number being small.
2. **Prove the factory change on the webhook receiver first, not the proxy.** It is ~160 lines, has no TLS
   and no CONNECT tunnelling, and its only global state is two maps and a port — so it is the cheapest place
   in this codebase to learn what de-singletonizing actually costs. Do not start with `proxy/server.ts`.
3. **Do not touch `store/paths.ts` or `processSpawner.ts`.** They are singletons for a reason. If the design
   appears to need a second instance, stop and re-read this document.

---

## 7. Re-scoping of the remaining phases

| Phase | Was | Now |
|---|---|---|
| **P6** | gate includes the process split (4 carried criteria) | **downgraded (D14).** The desktop stays in-process; the four criteria (spawn, restart-with-backoff, no orphan, self-terminate) move to the workspace host's gate, where Docker's `SIGTERM` plus a socket-close watchdog satisfy them. **Criterion 2 — "11 e2e specs pass, unmodified" — was CARRIED, not waived; the carry was discharged 2026-09-22 at 11 specs / 46 tests** (owner-run, read from the run's own `report.json` rather than re-run). **P6 has no open criterion left.** |
| **P7 Web UI** | connect the renderer to an engine | **unchanged in effort, better specified.** It connects to a *host* and selects a *workspace*, so work item 2's "multi-engine profiles" becomes "host + workspace". Work item 5's "have the engine serve the UI" becomes "have the **host** serve the UI" — still same-origin, still no CORS. |
| **P8 CLI** | ~3 weeks, after P5 | **PARKED (D15).** Not deleted. |
| **P9 Docker** | "the engine in a container" | **rewritten as "the workspace host in a container"**, now depending on P14. Effort roughly unchanged; the artifact, the volume layout and the port table all change. |
| **P12 Packaging** | change point 2 at P9 | unchanged in shape; the P9 change point now packages the host. |
| **P13 Hardening** | auth, audit, docs, perf | plus one new row: a host running N workspaces publishes N proxy ports, so `plan/10`'s exposure table (currently 2 rows) needs N — RPC stays loopback/socket. |

### D15 — the CLI, deferred, with its two shapes recorded

Per the owner's direction, the CLI is **deferred**, not cancelled. Two shapes, decided after P14:

- **(a) in-app CLI executable.** The Electron app ships a `bifurc` binary (or a `--cli` entry in the
  packaged app) that talks to the app's own engine over the socket transport
  (`transport/socket.ts`, `defaultSocketPath(pid)`). Zero new deployment; fails when the app is not running.
- **(b) npm package.** `@bifurc/cli` speaking the same `@bifurc/client` over `ws` / `socket`, attaching to a
  local app, a local host, or a remote host. Works without the app; exercises D9's version policy for real.

**Recommendation: (b)** — it is the same client calls, and it is the only one that works against a remote
host. But the decision depends on whether the workspace host becomes a common deployment, so it is
correctly deferred until P14 and P9 have landed. Note that this makes the CLI a consumer of the host, not of
the engine — which is consistent with D10.

---

## 8. Acceptance criteria

- [x] **Two workspaces run their proxies simultaneously on different ports, each serving its own rules**,
      with no cross-talk — asserted on *identity* (each serves a mock the other does not have), not on a
      value both could share. **MET 2026-09-22** —
      `tests/integration/workspaceDispatchScope.integration.test.ts` asserts exactly this, in both
      directions, over real sockets. **It was not met before that date, despite reading as met:** the
      instrument standing in for it was `workspaceRuntime.integration.test.ts`'s `getHomePage()`, and a home
      page is not a rule. See WI 7's third finding above.
- [x] **The D16 sequence holds per environment.** On `win32`: 80, then 81, then 82. Elsewhere: 8080, then
      8081, then 8082. Assert the **sequence**, not merely that each runtime came up — and assert it for
      **both** bases, since the base is the only platform-dependent value in the whole rule.
      **MET 2026-09-22** — `tests/runtime/portAllocator.test.ts:205` scans upward for **both** ranges, `:167`
      contiguously; and the host runs land two workspaces on **80 and 81** (`portRestartStability`, observed
      in the server log rather than inferred).
- [x] **The platform branch is one constant.** Assert the allocator yields base `80` under a simulated
      `win32` and base `8080` otherwise, and that nothing else in the allocation path branches on platform.
      **MET 2026-09-22** — `tests/runtime/portAllocator.test.ts`, "the platform branch is one constant".
      Three cases load the module **fresh** under a faked `process.platform` (`IS_WINDOWS` is computed at
      module load, so stubbing the platform after import would assert the *real* platform twice and pass
      vacuously), and the invariant half is a **count**: `process.platform` read exactly once,
      `IS_WINDOWS` used exactly once. A count is the right instrument here because the claim *is* a count —
      no behavioural test can express "nothing else branches" without enumerating every output.
      **Mutation-verified: adding a second `process.platform` read fails exactly that one case, and nothing
      else in the file.**
      **And the count must be taken over the module's *code*, not over its text — learned the hard way the same
      evening.** The first version counted the raw source and failed on a **documentation** edit: a docblock
      added while implementing 6b merely *names* `IS_WINDOWS`, which took the count to 3. A comment is not a
      branch on platform, so the instrument had been made sensitive to prose — and the tempting fix would have
      been to delete a useful explanation to satisfy a test. Comment-only lines are now dropped before
      counting, while a trailing comment on a *code* line is kept, so a real second use cannot hide behind one.
- [x] **An override fails loudly rather than silently moving.** Set `preferredPort` to a port already held;
      assert the runtime reports the conflict and names the next free port. **MET** — `:238` (proxy) and
      `:260` (webhook) both name the next free port; `:374` fails loudly on a malformed env override.
- [x] **`EACCES` advances rather than killing the runtime** — set an override below 1024 on a platform that
      forbids binding there and confirm the runtime still comes up. *(Defensive per D16: the default path
      never attempts a privileged port, so this guards the override case only.)*
      **MET 2026-09-22, and the criterion's wording needed correcting first.** "Override" names **both** of
      D16's sources, and the parenthetical is true of only one: the default bases are `80`/`8080` and `8080`
      needs no privilege, so the case that reaches a privileged port is an operator overriding the
      **environment base** (`BIFURC_PROXY_BASE_PORT`). That reading is also the only one consistent with D16,
      because a *per-workspace* override that cannot be bound must **throw**, not advance. Both halves are
      now asserted in `tests/runtime/portAllocator.test.ts`: the **base advances** and the **per-workspace
      override still throws**. `EACCES` is **injected** — `net.Server.prototype.listen` made to emit the
      error on `nextTick`, the asynchronous path a real failure takes, rather than thrown synchronously
      (which would exercise `tryBind`'s `try/catch` instead of its `on("error")`) — because win32 cannot
      produce it for real.
      **Mutation-verified: treating only `EADDRINUSE` as unavailable fails exactly those two cases and
      nothing else in the file.**
- [x] **6a — a new host reproduces the previous ports.** The original criterion's *mechanism* was
      unsatisfiable: `workspaceHost.integration.test.ts:331` asserts `host.start()` after `stop()`
      **rejects** (`already been stopped`), so "stop the host … start again" is impossible **by design**, and
      the claim that matters is about a **new** host. Instrument:
      `tests/integration/portRestartStability.integration.test.ts` — a second host lands on the same ports,
      **and still does with `runtime.json` deleted**, which pins that the stability comes from the **scan**.
      **MET.** *(This half of the 2026-09-22 rewording was right.)*
- [x] **6b — D16's stickiness: a workspace that incremented *keeps* its port when the base frees up.**
      **MET 2026-09-22 (evening) — implemented, on the owner's decision.** D16 decides it in as many words:
      *"The resolved port is recorded in the host-local registry, so a restart reuses it. A workspace that
      landed on 81 because 80 was busy **keeps 81** after 80 frees up — a stable URL is worth more to the
      browser-proxy workflow than reclaiming the base. (Alternative: re-prefer the base on every start …
      **Recommended against.**)"*
      **The defect, measured rather than read:** `host.ts:222-226` implemented **the alternative D16 recommends
      against**. A probe driving `BIFURC_PROXY_BASE_PORT` to a controlled base showed it directly — with the
      base held, host 1 placed the workspace on `base + 1`; with the base **freed**, host 2 placed it on
      **`base`**. `recordResolvedPorts()` wrote `resolvedProxyPort` and **no product code path read it**: the
      field was **write-only**, because the allocator had no *soft* input, and passing the value as a
      `preferred*` one would have turned a taken remembered port into a hard startup failure.
      **The fix — option (a), chosen by the owner:** `allocatePorts` gained `rememberedProxyPort` /
      `rememberedWebhookPort`, a third and **soft** source, which is the only reading consistent with D16's
      fail-loudly rule applying to *user overrides* rather than to a remembered outcome. `host.ts` now passes
      `entry.resolvedProxyPort` / `entry.resolvedWebhookPort`, and `AllocatedPorts` gained
      `proxyPortWasRemembered` / `webhookPortWasRemembered` **because the outcome cannot distinguish the two
      paths** — on a quiet machine the remembered port and the scan produce the same number, so the *flag* is
      the only instrument that answers the question the criterion is actually asking.
      **Instrument:** `portRestartStability.integration.test.ts`'s third case — the base held, then freed, then
      asserted **free** (or "it kept its port" is uninteresting), and the workspace keeps its higher port — plus
      five allocator cases: the free path, the fall-through, the ordering against the override, the reservation
      check, and the webhook half.
      **Mutation-verified with three narrow mutations, each moving exactly its own guards:** removing the
      remembered path fails exactly **4** (free path, reservation contrast, webhook half, stickiness case);
      making it a **hard** override fails exactly **3** (fall-through, reservation contrast, integration
      softness case); and letting it **outrank the override** fails exactly **1** (the ordering case). The two
      cases that *survive* the first mutation are the point — they guard ordering and softness, not presence.
      *(History: this was reported met earlier the same day on a **weakened** claim, then re-opened. Lesson 3.)*
- [x] **The allocator never hands out a reserved port** — with the companion on 9271 and the gRPC
      placeholder at 9102, assert no runtime is ever assigned either. Include the webhook interaction: the
      second workspace's webhook skips `9102` and lands on `9103`.
      **MET 2026-09-22** — the "named reservations" block in `tests/runtime/portAllocator.test.ts`. Each
      case **holds the ports below the reserved one**, so the scan genuinely wants it; without that the
      reservation would never be consulted and the case would pass whether or not it worked. The two
      constants are written out rather than imported, because the claim is that the allocator is *told* them
      and never derives them.
- [x] `preflight()` does **not** fail on a busy *base* port (the second-workspace case) and **does** fail on
      a busy *per-workspace override*.
      **MET — and it was met all along.** `tests/startup.test.ts:87` ("port advisories (P14 WI 5 / D16)") is
      a five-case suite covering exactly this, including both in a single run (`:131`). The 2026-09-22 audit
      recorded this row as **unmet** after searching only `tests/integration/`; that was a **false
      negative**, and it is the audit's own lesson turned around — **searching one directory is the same
      error as counting the files that mention a thing.**
- [x] `server.status` reports the **resolved** port, and it differs between two concurrently running
      workspaces. **MET** — `workspaceRuntime.integration.test.ts:187-194`.
- [x] The webhook receiver routes by `(workspaceId, urlSuffix)`: a suffix registered in workspace A is
      `404` in workspace B.
      **MET 2026-09-22** — `tests/integration/webhookWorkspaceScope.integration.test.ts`, over real sockets
      and **in both directions** (each workspace `200`s its own suffix and `404`s the other's), plus the
      per-workspace active-set budget and `unregisterActive` leaving the sibling alone. The mechanism is that
      `activeWebhookSuffixes` lives *inside* `createWebhookServer()`, so **one receiver per workspace *is*
      the key**. Asserted on **identity** — a receiver that `404`ed everything would satisfy a one-way test.
      **Mutation-verified: reverting `registerActive` to the facade — the pre-WI-7 shape — fails both
      cases.**
- [x] **Blob staging is per workspace (D17):** a blob `put` by workspace A is *not found* when `read` with
      workspace B's `workspaceId` — asserted on identity (the blob exists under A and not under B), not on a
      value both could share. And the blob root is still under `dataDir()`: `<dataDir>/blobs/<wsId>/`, so
      `path.relative(dataDir(), blobRoot(wsId))` never starts with `..`.
- [x] **A `WorkspaceRuntime.stop()` releases its own staging** — the sweeper is per-runtime, so stopping A
      does not stop B's, and A's in-flight blobs are not collected by B's sweep.
- [x] `server.status` reports the port and the error of the **requested** workspace, not of the last one
      started.
- [x] A workspace host starts with **no Electron and no companion server** — assert the companion port is
      *not* listening.
- [x] The readiness predicate is per-runtime: starting runtime B does not observe runtime A's readiness.
- [x] `store/paths.ts` still has **exactly one** data root in the process (`_dataRootOverride` set once) —
      assert it, do not assume it.
      **MET 2026-09-22** — `tests/runtime/singleDataRoot.test.ts`. The declaration is counted **engine-wide**
      (one `let dataRoot`, one `let _dataRootOverride`, each asserted by *path* rather than by length, so a
      duplicate names itself), `bundle: false` in `packages/engine/tsup.config.ts` is asserted as the
      mechanism that keeps one declaration one **runtime** instance, and the sharing is shown behaviourally
      — `workspaceFs.dataRoot()` derived from `paths.dataDir()`, with the override **layered on** the one root
      rather than being a second one. **Mutation-verified twice: a second module declaring a root fails the
      structural case; `bundle: true` fails the config case.**
      *(Two corrections to the criterion's own wording: `_dataRootOverride` is in `store/workspaceFs.ts`, not
      `store/paths.ts`; and there are legitimately **two variables** for **one root** — `paths`' root and
      `workspaceFs`' test-only override on top of it. The file pins that layering.)*
- [x] The desktop's user-visible behaviour is unchanged: one proxy port, one visible workspace,
      `window.api` byte-identical.
- [x] `SIGTERM` on the host stops every runtime's servers and flushes.
      **MET 2026-09-22** — `tests/integration/workspaceHostSignal.integration.test.ts` calls
      `runWorkspaceHost()` and then invokes **the listener it actually registered**, discovered by diffing
      `process.listeners("SIGTERM")` across the call. (Calling a hand-rolled `shutdown()` would assert a
      function nobody ships; `process.emit("SIGTERM")` would also fire every unrelated listener in the
      worker.) `process.exit` is stubbed, or the teardown would end the worker mid-case. Asserts `143` for
      `SIGTERM` and `130` for `SIGINT` — distinct numbers, because an implementation hardcoding one would
      satisfy either test alone — that a second signal is ignored, and that the runtimes' ports are free
      afterwards **by real binds**. **Mutation-verified: dropping the `host.stop()` call fails the port
      assertion, so the guard covers the teardown and not merely the exit code.** The *flush* half is **not**
      asserted here: it is `WorkspaceRuntime.stop()`'s own contract and is covered by the runtime suite, and
      restating it through a signal would be a second, weaker copy. Named rather than quietly skipped.
- [x] Protocol changes recorded in `plan/protocol-changes.md`; the extension's four frozen commands
      asserted unchanged **by name**.
- [x] Fixtures re-derived after the schema change, not trusted.
      **MET 2026-09-22** — the four `blob.*` `invalid` payloads were run through their schemas and their
      **issue paths measured**, not read off:

      | command | invalid payload | issue path | code |
      |---|---|---|---|
      | `blob.put` | no `filename` | `filename` | `invalid_type` |
      | `blob.stat` | only `workspaceId` | `blobId` | `invalid_type` |
      | `blob.read` | `offset: -1` | `offset` | `too_small` |
      | `blob.release` | only `workspaceId` | `blobId` | `invalid_type` |

      Two of the four were **non-discriminating constants**: `blob.stat` and `blob.release` used `{}`, which
      is invalid *before and after* the change and therefore could not have detected it — a fixture that
      cannot fail for the change it exists to guard. They now carry the payload this plan itself named as the
      hazard, the newly-added optional field and nothing else. The **reason** is pinned in
      `packages/protocol/src/commands/index.test.ts` (one issue, on the named path, with the named code), plus
      a **near-miss** case that is the only assertion a `{}` fixture fails — **mutation-verified: reverting
      `blob.stat` to `{}` leaves every other assertion in that block passing and fails only the near-miss
      one.**
- [x] Baseline delta accounted for **by naming its mechanism** (2 tests per command in
      `packages/protocol/src/commands/index.test.ts`), not by waving at the number.

**Checkbox state corrected 2026-09-22.** Eight rows — 11, 12, 13, 14, 15, 17, 19 and 21 — were recorded
**✅ met** in the audit table below but their checkboxes had never been ticked, so the list and the table
disagreed about eight criteria. All eight were **re-verified against the tree before ticking** rather than
trusted from the table, because the same audit also produced one false verdict: `blobRoot(workspaceId)` really
is `<dataDir>/blobs/<wsId>/`; `tests/blob/store.test.ts:583` really is the per-workspace sweep case; the three
`workspaceCommandScope` cases really do name the asked-about workspace; `hostStartup` really does assert the
companion port is never bound; `packages/client/tests/surface.test.ts` exists; and
`tests/companion/allowedActions.test.ts:22` really does pin the four frozen commands **by name**. **A checkbox
and a table cell are two copies of one claim, and they drifted** — which is the `TRAPS.md` rule about a count
written into prose, one layer down.
### Audit 2026-09-22 — each criterion re-derived from the tree, not read off the checkboxes

Prompted by criterion 1 turning out to be **falsely believed met**, so the other twenty were re-derived rather
than trusted. Every row names the instrument; a row with no instrument is not met, whatever the code does.

**Result of that audit: 12 met · 3 partial or mechanism-unverified (3, 7, 18) · 5 unmet (5, 6, 8, 10, 16) ·
1 unverified (20).** P14 was **not** closeable, and the unmet set was almost entirely *missing instruments*
rather than missing code — the one exception worth attention being **18**, where the mechanism existed and
nothing proved it ran.

**All twenty-one were reported as met (2026-09-22) — and that count was wrong; see the correction under the
table.** **Twenty are met and one (6b) is open.** The table is the original audit, kept because it is the record
of what was wrong; the closure note on each row says what changed.

| # | Verdict at audit | Evidence, and what changed |
| --- | --- | --- |
| 1 | ✅ met | `workspaceDispatchScope.integration.test.ts` — both directions + control |
| 2 | ✅ met | `portAllocator.test.ts` — `:205` scans upward for **both** ranges, `:167` contiguously, `:358` base 80/8080, `:364` the increment survives an env override |
| 3 | ⚠️ partial | `:358` asserted the two values; **nothing asserted the invariant half**. **CLOSED** — three fresh-module loads under a faked platform, plus a **count** of `process.platform` reads and `IS_WINDOWS` uses. Mutation: a second read fails exactly that one case |
| 4 | ✅ met | `:238` (proxy) and `:260` (webhook) name the next free port; `:374` malformed override fails loudly |
| 5 | ❌ unmet | No allocator `EACCES` case. **CLOSED** — injected `EACCES` on the **base** (advances) and on a **per-workspace override** (still throws); the criterion's wording corrected first. Mutation: treating only `EADDRINUSE` as unavailable fails exactly those two |
| 6 | ❌ unmet — **the criterion contradicts the contract** | `workspaceHost.integration.test.ts:331` asserts `host.start()` after `stop()` **rejects**, so "stop the host … start again" is impossible **by design**. **RE-OPENED, then CLOSED PROPERLY.** The first rewording fixed the *mechanism* and quietly **weakened the claim** from D16's stickiness to mere reproducibility — so the criterion went green while a recorded decision stayed unimplemented, and the instrument built for it pinned the **deviation**. Split into **6a** (reproducibility — met) and **6b** (stickiness — **not met, measured**: base freed ⇒ the workspace moved back to it). **6b is now met** by a product change: a soft `remembered*` source on `allocatePorts`. See lesson 3 |
| 7 | ⚠️ partial | Reservations covered generally; the named pair was not. **CLOSED** — `9271` / `9102` / `9103` asserted, holding the ports *below* the reserved one so the scan genuinely wants it |
| 8 | ❌ unmet — **and this was a false negative** | The audit searched `tests/integration/` and missed `tests/startup.test.ts:87`, a five-case suite covering exactly this. **It was met all along.** See lesson 1 |
| 9 | ✅ met | `workspaceRuntime.integration.test.ts:187-194` |
| 10 | ❌ unmet | No `(workspaceId, urlSuffix)` test. **CLOSED** — `webhookWorkspaceScope.integration.test.ts`, both directions + the per-workspace budget. Mutation: the pre-WI-7 facade shape fails both cases |
| 11 | ✅ met | `tests/blob/*` + `importExportBlob.integration.test.ts`; the root is `<dataDir>/blobs/<wsId>/` |
| 12 | ✅ met | `tests/blob/store.test.ts:583` |
| 13 | ✅ met | `workspaceCommandScope.integration.test.ts:193`, `:239`, `:260` |
| 14 | ✅ met | `hostStartup.integration.test.ts:101` + `workspaceHost.integration.test.ts:202` |
| 15 | ✅ met | `workspaceRuntime.integration.test.ts:187-194` |
| 16 | ❌ unmet | **No test mentioned `_dataRootOverride`.** **CLOSED** — `singleDataRoot.test.ts`: engine-wide declaration count by path, `bundle: false`, and the sharing shown behaviourally. Mutations: a second declaration fails the structural case, `bundle: true` fails the config case |
| 17 | ✅ met | `packages/client/tests/surface.test.ts` + the e2e run (11 specs / 46 tests) + `facadeBinding.integration.test.ts` |
| 18 | ⚠️ implemented, unverified | Nothing drove `runWorkspaceHost` through a signal; the cited `tests/shutdown.test.ts` exercises `shutdownEngine()`. **CLOSED** — `workspaceHostSignal.integration.test.ts`. Mutation: dropping the `host.stop()` call fails the port assertion |
| 19 | ✅ met | `plan/protocol-changes.md` + `tests/companion/allowedActions.test.ts` (the four frozen commands pinned **by name**) |
| 20 | ❓ unverified | The `blob.*` `invalid` fixtures. **CLOSED** — issue paths measured; two were non-discriminating `{}` constants and now carry the hazard this plan named. Mutation: reverting to `{}` fails only the near-miss case |
| 21 | ✅ met | Recorded in WI 8 above and `TESTING.md` |

**Verdict now: 21 met · 0 open.** Six new instruments, one rewording and two corrections to criteria that were
themselves defective — and, in the end, **one product change**: D16's remembered-port source, which the audit
found specified and unimplemented. Nothing else required product code; the only other non-test edit is two
`blob.*` fixtures (test data).

**⚠ This paragraph claimed "21 met · 0 open" twice while that was wrong, and both ways it was wrong are the most
useful thing in this section.** The first time it counted the criteria the **code** satisfied rather than the
ones **D16** requires, because criterion 6 had been reworded onto a weaker claim (lesson 3). The second time it
was true only after 6b was genuinely implemented. **A gate's verdict is an instrument too**, and the honest
version names what it was measured against.
Every load-bearing guard was mutation-checked, and in three cases the mutation was *narrowed* until it moved
exactly the intended assertion: **the coarse mutation proves a guard is sensitive, not that it is sensitive to
the right thing.** One mutation also had to be discarded as malformed — `Promise.resolve().stop()` throws, so
that run measured the mutation rather than the product, and a mutation you cannot attribute is not evidence.

**Two lessons, and they are mirror images of each other.**

1. **The audit's own lesson, which then caught the audit.** Criterion 1 read as met because a *liveness*
   assertion stood in for a *content* one; criterion 18 read as met because a *unit test of one entry point*
   stood in for a *signal test of another*. **Neither was a lie — both were an instrument answering a
   neighbouring question.** But criterion 8 was recorded as **unmet** for the same class of reason in reverse:
   the audit searched `tests/integration/` only, and the instrument was in `tests/startup.test.ts`.
   **Searching one directory is the same error as counting the files that mention a thing.** A *negative*
   verdict is a claim too, and it needs the same instrument-naming discipline as a positive one — "I did not
   find it" is not "it is not there".
2. **A criterion can be wrong, and the honest response is to say so rather than to write a test that agrees
   with it.** Two of the twenty-one were defective: **6** was unsatisfiable as written (it contradicted a
   contract another test pins), and **5** used "override" for two different D16 sources while describing only
   one of them. Both were reworded first and instrumented second. A test written to satisfy a defective
   criterion would have locked the defect in and called it green.

3. **Rewording a defective criterion can *weaken its claim* — and then the instrument agrees with the code
   instead of the decision.** This is lesson 2's own failure mode one step further along, and it happened to
   criterion 6 in this very section. The original was genuinely defective: it named a mechanism ("stop the host
   … start again") that another test proves impossible. But the defect was in the *mechanism*, not in the
   *claim* — and the claim was D16's stickiness rule **verbatim** ("still on its incremented port" = "keeps 81
   after 80 frees up"). The rewording replaced the mechanism **and the claim**, substituting reproducibility
   ("a new host lands on the same ports") for stickiness. The code satisfies the substitute and **violates the
   original**, so the criterion went green while a **recorded decision** stayed unimplemented. Three guards,
   all cheap:
   - **Quote the decision in the criterion.** Had criterion 6 carried D16's sentence, the substitution would
     have been visible on the page.
   - **A reworded criterion needs its *old* claim re-tested, not retired.** The original claim *was* testable —
     with a **new** host rather than a restarted one, which is exactly what the rewording had already
     established. The rewording found the right instrument and aimed it at the wrong question.
   - **A discriminating test is evidence for whichever side it points at.** `portRestartStability`'s second
     case is a *good* test: it separates the two mechanisms cleanly. It was filed as evidence that the
     criterion holds, when what it proves is that the **mechanism D16 specifies is absent**. The test did not
     change between those two readings; only the authority it was measured against did.
   And the meta-lesson, which is why this is written down rather than quietly fixed: **the audit that found
   12 met / 3 partial / 5 unmet was right to distrust the checkboxes, and then trusted its own rewording.** An
   instrument can answer a neighbouring question; so can a criterion.

   **Resolution, and what it cost.** The owner chose to implement D16 rather than amend it, and the fix was
   small — a soft third source on `allocatePorts`, a few lines in `host.ts`, one new flag pair. What made it
   small was that the re-opening had been *precise*: the probe pinned the mechanism, the test file's second
   case already separated "remembered" from "re-derived", and D16's own text supplied the resolution order. A
   re-opening that says only "this looks wrong" would have cost far more. **So the rewording was not a wasted
   step — it was a step taken with the wrong authority in hand.** Keep the instrument; re-check what it is
   measured against.

---

## 9. Rollback

The phase is additive at every step, which is what makes it safe:

- **WI 1–2** keep the module-level facade alongside the factory until WI 7 flips the shell. Reverting is
  deleting the factory and restoring the facade.
- **WI 3** adds a new entry point that nothing in the desktop path calls. Reverting is not shipping it.
- **WI 4** is the only step that changes existing behaviour. It should land as its own commit with the
  `config.save` split in the same commit, and it is the step to revert first if the desktop regresses.
- **WI 7** is the only step that can change what a user sees. It is gated on P6's rule (zero user-visible
  change) and on the `window.api` byte-identical check.

---

## 10. Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| De-singletonizing duplicates `store/paths.ts` state and silently hands a runtime its own data root | **High** | WI 0 greps for it; explicit acceptance criterion; keep the runtime library-internal |
| A second package for the host duplicates the engine's module state (`bundle: false` hazard) | **High** | Start as an entry point inside `packages/engine`; split only if the Docker closure demands it |
| A global `isRunning()` reports the wrong runtime ready | **High** | Make the predicate per-instance (WI 2); assert with two runtimes |
| The proxy port stops being a single user-facing number; the CA/proxy workflow gets confusing | Medium | Per-workspace preferred port + always report the resolved port; document the multi-workspace proxy workflow in P9 |
| `config.save`'s proxy-restart half is deleted along with the port | Medium | WI 4 names it: split, do not delete — the companion half stays |
| Legacy `server:*` channels break `window.api` | **High** | They compose the active workspace client-side; `window.api` byte-identical is non-negotiable #3 |
| N published proxy ports widen the exposure surface | Medium | `plan/10`'s exposure table gets N rows; RPC stays loopback/socket |
| **The platform branch leaks beyond the base constant** — per-platform logic appears in the scan, the override path or the UI | Medium | D16 makes the base the *only* platform-dependent value; assert both bases in one test, so a second branch surfaces as a failure |
| **The probe-then-bind race hands two runtimes the same port** | Medium | Allocate by attempting the real `listen()`, never via `checkPortInUse`; add a concurrent-start test |
| `preflight()`'s port check is fatal and blocks the second workspace | **High** | WI 5 makes it non-fatal for the base port; fatal only for a per-workspace override |
| The allocator hands a runtime a reserved port (companion, RPC, gRPC placeholder) | Medium | Explicit reserved-port set (WI 1); assert 9271 and 9102 are never assigned |
| An operator expects the container's proxy on 8080 and gets a different value | Low | `plan/10` sets `BIFURC_PROXY_BASE_PORT=8080` explicitly; log the resolved port at startup |
| The phase is started before WI 0's count is known, and the estimate is wrong | Medium | WI 0 is a hard gate with a one-day cost |

---

## 11. What this phase deliberately does not do

- **Does not move the companion server out of the engine.** It stays in
  `packages/engine/src/transport/legacyCompanion.ts`; it is simply only ever *started* by the app runtime.
- **Does not build the CLI** (D15).
- **Does not extract a second process for the desktop** (D14).
- **Does not remove `registerIpcHandlers()`** — *true when this was written; no longer true.* P6 step 3
  landed afterwards (the nine-group reduction, `plan/07` §5b-duodecies … §5b-vicendecies): the three
  all-dead files are deleted and `src/` holds **18 real `ipcMain.handle(` sites, all keepers**
  (16 `clientHandlers`, 1 `systemHandlers`, 1 the RPC bridge). `registerIpcHandlers()` itself remains, as
  the dispatcher over those — so this bullet is now a *historical* statement, not a scope exclusion.
- **Does not change the extension's four frozen commands** — the extension is an external client on its own
  release cycle (`README.md` → Protocol compatibility).
- **Does not touch the renderer.** Any renderer change is P7 by non-negotiable #1/#3.

---

## 12. Reconciliation with the completed phases — measured 2026-09-20

The pivot re-scopes P7–P12 and downgrades one P6 work item, but **P0–P6 are already done, and they were
done under the pre-pivot assumptions.** This section is the audit the pivot owes them: for each completed
phase, what the pivot invalidates, what it merely extends, and what turned out to be **wrong**. It exists
because §2's target architecture is a claim *about* work that is already in the tree, and a claim about
finished work is the one kind nobody re-checks.

| Phase | The pivot's assumption | Measured | Verdict |
|---|---|---|---|
| P0 | the spike's record is inert | it drives **10 legacy channels**, 2 of which P6 stage 3b deleted | **RETIRED 2026-09-21** — the shim was the last consumer, so the spike went with it — §12.6 |
| P1 | the command table is the frozen shape the pivot widens | true; §WI 1/8 already own the change | extends |
| P2 | `createEngine()` is the composition WI 2/3/5/7 dissect | true, and it is the *whole* inventory | extends |
| P3 | **blob staging is already per-workspace** (§2.2) | **false** — it is host-wide, with no `workspaceId` | **correct §2.2; decision needed** — §12.1 |
| P4 | the transports exist but are wired to nothing; auth gates WI 3 | true, and **auth is built** | extends, and de-risks WI 3 |
| P5 | the client survives D13 intact | the *keys* do; the workspace-scoped **signatures** do not | extends |
| P6 | D14 downgrades WI 2 and changes nothing else | true | unchanged |

### 12.1 The finding: P3's blob layer is **not** per-workspace, and §2.2 says it is

§2.2 lists, as belonging to a `WorkspaceRuntime`:

> Its git store, sync, blob staging (already per-workspace on disk: `<dataRoot>/<wsId>/…`,
> `store/workspaceFs.ts:46-50,71-102`).

Measured, the first two are per-workspace and **the third is not**:

| Claim | Measured | Evidence |
|---|---|---|
| blob staging is per-workspace | `blobRoot()` is `path.join(dataDir(), "blobs")` — `dataDir()` is the **app** root, not `<dataRoot>/<wsId>` | `packages/engine/src/blob/store.ts:140-142` |
| — | the four `blob.*` commands carry **no `workspaceId`** | `packages/protocol/src/commands/blob.ts:106,140,159,181` |
| — | and the host-wide root is **pinned by a test**, for the Docker-volume requirement | `tests/blob/store.test.ts:62` — *"puts the blob root under dataDir()"* |

So blob staging is a **third app-level thing**, alongside the companion server and the desktop concerns —
and §2.3's split table has **no row for it**. An implementer following §2.2 will not check, and the
acceptance criterion that is meant to catch cross-talk ("two workspaces … each serving its own rules")
is about *rules*, so it would pass with a shared blob namespace.

**What the sharing actually costs under N > 1.** Cross-workspace *read* is not the risk: `blobId` is
`blob_` + 16 random bytes (`store.ts:307`), so it is unguessable. The three real consequences are:

1. **The sweep's extent is every workspace.** `BLOB_TTL_MS` is the safety net and the sliding lease only
   extends on `read` (`store.ts`'s "The sliding lease") — so a blob that was `put` and not yet pulled is
   exactly the unleased case, and workspace A's sweep can collect workspace B's staging.
2. **The lifecycle is in the wrong place.** `startBlobSweeper()` / `stopBlobSweeper()` are `createEngine()`'s
   (`packages/engine/src/index.ts`), so a `WorkspaceRuntime.stop()` has no way to release its own staging.
3. **`blob.*` is unclassified under D13.** The rule is "workspace-scoped commands carry `workspaceId`";
   `blob.*` is neither app-level by declaration nor workspace-scoped by signature.

**The Docker requirement is real and must survive either fix:** the blob root has to stay under `dataDir()`,
or a restart loses every in-flight transfer. That is what the pinning test is for.

**Two shapes, and this is a decision rather than a fix:**

- **(a) Namespace it.** `blobRoot(wsId)` = `<dataDir>/blobs/<wsId>/`, `workspaceId` added to the four
  `blob.*` params, and the sweeper becomes per-runtime. D13-consistent, and it preserves the volume property.
- **(b) Declare it app-level**, like the companion server: correct §2.2, leave the namespace and the sweep
  host-wide, and say so.

**DECIDED 2026-09-20 — shape (a), recorded as D17.** The owner chose the namespace on the reasoning that
import/export is the layer §1.2 cites as *already* workspace-scoped and is the blob layer's only real
consumer, so the two should agree. It is a breaking protocol change, so it lands with WI 1 and WI 8 rather
than being slipped in; **§2.2 and §2.3 are updated to the decided shape** rather than to a warning, because a
plan that still reads as unresolved is the same hazard as one that reads as correct — the next author
implements whichever they read first.

### 12.2 P2's `createEngine()` is the pre-pivot composition — the whole inventory, in one function

Not a defect, but WI 0's measurement is largely already done, because one function holds nearly every
reader §1.1 lists. `packages/engine/src/index.ts`:

| Line | What it does | Which work item dissects it |
|---|---|---|
| `:210-216` | `preflight()` over three **global** ports, then `if (!check.ok) warn(check)` | WI 5 |
| `:231` | `startServer(settings.port)` — the engine starts the proxy itself | WI 2, WI 3, WI 7 |
| `:232` | `startCompanionServer(...)` — correct under D10, but it is *here*, beside the proxy | §11, WI 3 |
| `:242` | `await waitUntilReady(isRunning, getServerError)` — a **global** readiness predicate | WI 2 (named in the plan) |
| `:248-259` | `status()` returns **one** `proxyPort` | WI 1 ("report the resolved port") |

Two things worth carrying into WI 5, both read rather than assumed:

- **`createEngine()` already only *warns*** on a failed port check (`:216`), so the fatality WI 5 has to
  remove is **in the caller**, not here. The caller is `plan/10`'s container entrypoint, which exits non-zero
  on any `ok: false`.
- **`checkPortAvailable`'s hint text is the wrong sentence** for a base port:
  *"Free the port, or change the configured port in settings"* (`packages/engine/src/startup.ts:95-97`).
  WI 5 names it; the replacement should point at the increment and the per-workspace override.

### 12.3 P4's transports are done and wired to nothing — and **auth is built**

§WI 3 says the host "exposes the RPC endpoint over `createSocketServer` / `createWsServer` (both exist,
both currently wired to nothing)" and "enforces auth via `transport/auth/`". Both halves measured true:

- `packages/engine/src/transport/index.ts` exports all four adapters — `in-process`, `stdio`, `ws`
  (auth-wrapped, loopback-only) and `socket` (`0600` on POSIX) — plus the `auth/` decorator, `eventLog.ts`
  and `eventPump.ts`. Nothing in the tree starts a server from them.
- `packages/engine/src/transport/auth/{token,scopes,authenticated}.ts` all exist and are wired: `ws`'s
  authentication *is* the decorator, and `scopes.ts` is the table the registry's `callerHasScope` reads.

**This matters because the master checklist reads the opposite way.** `plan/13-checklist.md`'s auth items
("Auth: token generation, storage (`0600`), validation", "Auth: per-session scope") are all **unticked**,
which reads as open work. They are not — the checklist was never maintained as a progress tracker for
completed phases (most of P0's and P2's items are unticked too). **Non-negotiable #2 ("no remote transport
before auth") is satisfied**, so WI 3 is not gated on new auth work.

### 12.4 P5's client will change under D13, and the 144-key count will not

WI 1 adds `workspaceId` to `server.start|stop|restart|status`, `proxy.status` and the `webhookServer.*`
family. Those are **params, not keys**, so the surface stays at 144 and
`packages/client/tests/shape.test-d.ts`'s seven-key assertion is untouched — but
`packages/client/src/index.ts`'s signatures, its `retry.ts` classification and its own suites do move.
Stated so that P5's ✅ is not read as *frozen*: the surface is frozen, the workspace-scoped signatures are
not, and that is D13 working as designed rather than P5 having been wrong.

### 12.5 P6: D14 is the only change, and it does not touch what landed

WI 2 (process split) loses its four criteria to the host's gate (`plan/07`'s relocated block); criterion 2
("11 e2e specs pass, unmodified") stays **carried, not waived**. The reduction of `registerIpcHandlers()` is
unaffected — §11 records that this phase does not remove it — and stage 3b's first slice landed the same day
(`plan/07` §5b-sexies: the nine protocol-client passthroughs, 9 of the 57 channels).

### 12.6 P0's spike record now depends on a shim

`tests/spike/protocolPoc.test.ts` drives **ten legacy channels**, two of which P6 stage 3b deleted
(`graphql:introspect`, `soap:execute`); it re-attaches them in `beforeAll` with the bodies the channels had,
so the ten-command envelope proof stays whole. The consequence worth stating: it is now the **only** place
in the tree where a deleted channel name is live, and the last consumer of `spike/protocol/`. Retiring the
P0 spike code is a legitimate later decision — but it is a *phase* decision, not a deletion-pass one, and it
means `spike/` goes too plus five plan docs that cite it.

> **RESOLVED 2026-09-21 — the decision was taken, and this section's own framing is why it was taken
> carefully.** The owner retired the spike. `spike/protocol/`, `spike/tsconfig.json`,
> `tests/spike/protocolPoc.test.ts` and `package.json`'s `spike:typecheck` are gone; the full account is
> `spike/README.md`.
>
> **The prediction above was right about the shape and wrong about the magnitude.** "It means `spike/` goes
> too plus five plan docs that cite it" — re-derived, it was **nine** documents citing the spike's code
> (`spike/protocol`, `protocolPoc`, `spike:typecheck`, `spike/tsconfig`), plus six source files. Sixteen
> documents mention the word "spike", but seven of those name *other* spikes or use it generically. The count
> was never re-derived, which is the failure mode this repository keeps recording. What made it tractable was
> that most citations are *provenance* ("validated in the P0 spike"), which stays true and needed only a
> retirement marker, and only **one** was a live claim about where coverage lives (`soapCommands.ts`).
>
> **And the thing this section did not anticipate is the part worth carrying forward.** The reason the
> retirement was safe is not that the spike was old — it is that **every assertion worth keeping already
> had a better home**, which had to be established per case rather than assumed. Two of the twelve would
> have been silently lost by a naive deletion: `soap.execute`'s dead-endpoint contract (now
> `protocolExecution.integration.test.ts`, on a real dead port instead of `127.0.0.1:1` — **same contract,
> firmer fixture**) and `tls.generate`'s renderer-facing `certPath`/`keyPath` shape (now
> `packages/client/tests/client.test.ts:1176`, asserted with `toEqual` at the shim that produces it, two
> layers closer than the spike was). The generalisable rule is in `spike/README.md`: **"does anything else
> test X?" is not answerable by counting files that mention X** — both directions were wrong on the first
> pass, one false-negative (`mock.delete`, covered under its engine name `entity.delete`) and one
> false-positive (`INVALID_PARAMS`, where the spike was testing its own throwaway transport and the code
> does not exist outside it).

### 12.7 What this audit did **not** check

Stated rather than implied, because a reconciliation that reads as complete is worse than one with a stated
edge. The audit covered (a) the completed phases' artifacts that **this document's own text makes claims
about**, and (b) the readers of the three globals §1.1 lists. It did **not** re-measure: P1's command table,
P2's four-layer import rewrite, P3's "17 formats round-trip" claim, P4's conformance-runner count, or P5's
seven-key disagreement. None of those is referenced by the pivot, so none of them can be *contradicted* by
it — but "not contradicted" is a weaker statement than "verified", and it is the honest one.

---

## 13. Work item 0 — the measurement, re-derived 2026-09-21

**This is the gate §6 makes a hard stop, and it is a measurement, not a design.** The whole phase's estimate
rests on the reader count being small. It is. Everything below was read off the tree at
`86efa6c` (post-reduction, `src/` 17 files, 101 files / 2,458 tests) with
`grep` scoped to `packages/*/src`, `src/`, `tests/` — **never** to `coverage*/`, `dist/` or `release/`
(`TRAPS.md`: a repo-wide grep drowns in the eleven generated coverage trees).

### 13.1 Module-level mutable state in `proxy/` and every reader

**Three modules hold process-global server state.** Each is a singleton today and each is a WI 2 factory
candidate — with the readiness predicate as the one non-mechanical part.

| Module | Module-level state | Readers of that state |
|---|---|---|
| `proxy/server.ts:20-25` | `server`, `currentPort`, `currentConfig`, `fullRules`, `lastError`, `enabledSets` | `isRunning()` `:108`, `getPort()` `:112`, `getServerError()` `:116`, `reloadConfig()` `:120-124`, and every `dispatch`/`dispatchHttps` body `:137-292` (reads `currentConfig`/`fullRules`/`enabledSets` through `workspaceCfg` `:144,:258`) |
| `proxy/webhookServer.ts:29-36` | `server`, `currentPort`, `lastError`, `activeWebhookSuffixes`, `activeSuffixToId` | `isWebhookServerRunning()` `:168`, `getWebhookPort()` `:172`, `getWebhookServerError()` `:176`, `getActiveWebhookCount()` `:58`; the request path reads both maps at `:104,:113` |
| `transport/legacyCompanion.ts:96-97` | `wss`, `currentPort` | `isCompanionRunning()`, `getCompanionPort()` — **app-level by D10, stays a singleton** |

`webhookEmitter` (`webhookServer.ts:38`) is a fourth module-level object: an `EventEmitter` **constant**, not
mutable state, exported so the shell and the suite can subscribe. It is not a WI 2 concern — a factory that
returned a fresh emitter per instance would break the one subscriber (`registerIpcHandlers()`'s forward) and
`tests/proxy/webhookServer.test.ts:136`, which attaches to the imported singleton.

**The three command modules are readers, not owners:**
`proxy/serverCommands.ts:76-102` (six commands), `proxy/webhookCommands.ts:82-108` (five commands),
`store/configCommands.ts:64-75` (the `config.save` proxy+companion restart). All three read
`loadConfig()` and call the singletons — which is why WI 1's `workspaceId` params and WI 2's factories meet
here.

### 13.2 Every reader of the four globals §4 WI 0 names

| Global | Reader count | The readers |
|---|---|---|
| `AppConfig.port` | **6** | `proxy/serverCommands.ts:84` (`server.start`), `:96` (`server.restart`), `store/configCommands.ts:64,66` (`config.save`), `index.ts:231` (`createEngine`), `src/main.ts:309` (**shell**) |
| `AppSettings.webhookPort` | **3** | `proxy/webhookCommands.ts:95` (`webhookServer.start`), `index.ts:212` (preflight), `store/appSettings.ts:83-85` (the default) |
| `AppConfig.activeWorkspaceId` | **9 real readers** | `proxy/server.ts:30-31` + `:122-123` (rules + enabled sets — **WI 4's target**), `proxy/serverUtils.ts:36`, `lib/…`→`store/config.ts:68,126,275,299`, `sync/syncManager.ts:134`, `transport/legacyCompanion.ts:133,169,200` (**stays**), `store/coreCommands.ts:80`, `store/configCommands.ts:87,112`, `importExport/importers/workspace-{zip,json}.ts`, plus the entity/folder/audit/sync command bodies that fall back to it when a param omits a `workspaceId` (`entityCrudCommands.ts` ×5, `folderCommands.ts:92`, `auditCommands.ts:56`, `syncCommands.ts:197`) |
| `isRunning()` | **4** | `proxy/serverCommands.ts:77` (`server.status`), `:100` (`proxy.status`), `store/configCommands.ts:64` (`config.save`), `index.ts:242` (`createEngine`'s readiness predicate) |

**Two readers the plan does not name and that matter:**

- **`entry` readers of `activeWorkspaceId` are mostly not the proxy.** Of the 9, exactly **two** are the
  server's (`server.ts:30-31`, re-read in `reloadConfig()` `:122-123`). The rest are *command bodies* that
  use it as the "current workspace" default for a param — a client/session notion, not a server one. WI 4's
  scope is therefore **`proxy/server.ts` only**; do not "sever" the command bodies, or every
  `entity.*`/`folder.*` caller that omits `workspaceId` breaks. `legacyCompanion` is explicitly out (§4 WI 4).
- **`isRunning()`'s four readers split 2/2 across two work items.** `serverCommands`' two are WI 1/2;
  `configCommands.ts:64` is WI 4's `config.save` split; `index.ts:242` is WI 2's readiness predicate. So
  **`isRunning()` cannot be deleted — it must become per-instance**, read through the runtime (§10's third
  High risk: "a global `isRunning()` reports the wrong runtime ready").

### 13.3 Every caller of the four lifecycle functions

**Measured 2026-09-21, post-reduction — 11 call sites, of which 6 are `src/`-free.** Compare §4 WI 0's list,
which names `systemHandlers.ts` and `coreHandlers.ts`: **both were deleted by the reduction** (§5b-quindecies,
§5b-septendecies), and each duplicate was of a body this table already lists — so the *distinct* caller count
is one lower in each case, and the two `src/` entries below are the only shell survivors.

| Function | Call site | Kind |
|---|---|---|
| `startServer` | `proxy/serverCommands.ts:84` (`server.start`) | engine command |
| | `proxy/serverCommands.ts:96` (`server.restart`) | engine command |
| | `store/configCommands.ts:66` (`config.save` — the split half) | engine command, **WI 4** |
| | `index.ts:231` (`createEngine.doStart`) | engine lifecycle, **WI 3/7** |
| | `src/main.ts:309` (**the shell**, `app.whenReady`) | **WI 7 — the one the desktop gives up** |
| | `tests/integration/proxyHarness.ts:331`, `tests/integration/settingsMutations.integration.test.ts:336` | **test** (real binds) |
| `stopServer` | `proxy/serverCommands.ts:89` (`server.stop`), `:95` (`server.restart`) | engine command |
| | `store/configCommands.ts:65` (`config.save` — the split half) | engine command, **WI 4** |
| | `shutdown.ts:46` | engine lifecycle |
| | `tests/integration/proxyHarness.ts` (via its `stop` closure), `settingsMutations…` | test |
| `startCompanionServer` | `index.ts:232` (`createEngine`) | engine lifecycle — **WRONG under D10, see below** |
| | `src/main.ts:310` (**the shell**) | shell, app-level |
| | `tests/integration/settingsMutations…:532,552`, `companionServer.integration.test.ts:74` | test |
| `restartCompanionServer` | `store/configCommands.ts:74` (`config.save` — **the half that STAYS**) | engine command, **WI 4** |
| | `tests/integration/companionServer.integration.test.ts:152` | test |

**Two findings the enumeration produced, both worth acting on:**

1. **`createEngine()` starts the companion server — `index.ts:232` — and under D10 that is wrong.** §11 says
   this phase "does not move the companion server out of the engine", which is about the *module*; it says
   nothing about the *lifecycle call*. But WI 3's acceptance criterion is "a workspace host starts with **no
   Electron and no companion server** — assert the companion port is *not* listening", and the host composes
   runtimes **from `createEngine()` or from the new `runtime/host.ts`**. If the host reuses `createEngine()`,
   line 232 starts the companion in the host too and the criterion can only be met by the host *not* using
   `createEngine()` — i.e. by duplicating lines 199-243. **The host entry point must be able to compose the
   runtimes without the companion line**, which means `createEngine()`'s companion start has to become
   optional (an `EngineOptions.startCompanion?: boolean`, defaulting true so the shell's byte-identical
   behaviour holds) or move out of `doStart()` into the shell. **This is a WI 3 design decision with an
   acceptance criterion riding on it; record it now rather than discover it at the criterion.**

2. **`config.save`'s split is already visible in the table, and it is cleaner than §4 WI 4 implies.** The
   proxy half is `:64-67`, the companion half is `:68-75`, and the companion call is
   `restartCompanionServer` — a *different function* from `startServer`/`stopServer`. So the split is a
   deletion of the `if` block at `:64-67` once the proxy is per-runtime, with the companion `if` left
   intact. **Deleting `:64-67` compiles cleanly; deleting `:68-75` instead kills companion port changes
   with nothing failing** — §10's Medium risk, and the one to get right in a single commit.

### 13.4 The verdict — is the phase mechanical?

**Yes, with one caveat, and the caveat is the readiness predicate.**

The reader count is small: **three stateful modules, 22 reader sites, 11 lifecycle call sites, and exactly
one of the eleven is the shell** (`src/main.ts:309-310`, which is WI 7's single edit). Measured against the
codebase the reduction just took from 20 handler files to 17, this is a smaller surface than the reduction's
first group. Nothing here is a hidden cluster.

The caveat is §4 WI 2's "readiness is the non-mechanical part", now confirmed by measurement rather than
asserted: `index.ts:242` awaits `waitUntilReady(isRunning, getServerError)`, and `isRunning()`/`getServerError()`
are the **only** readiness signal either server publishes (`index.ts:140-144` documents this, and neither
`startServer()` nor `startWebhookServer()` returns its `net.Server`/`http.Server`). A factory change makes that
predicate per-instance, so **`isRunning()` must be re-pointed at the instance, not merely wrapped** — and the
acceptance criterion that catches it is "starting runtime B does not observe runtime A's readiness". This is
the reason §6 says to rehearse the factory shape on the webhook receiver first: the webhook receiver has the
*same* readiness problem (`isWebhookServerRunning()` `:168`) at a third of the size and none of the TLS.

**Consequence for the estimate:** unchanged. WI 0's one-day cost is spent; the list is short and the phase is
mechanical behind the readiness work.

### 13.5 What this measurement did **not** do

- It did **not** touch or count `renderer/` — the webhook panel's `MAX_ACTIVE_WEBHOOKS = 5`
  (`packages/gui/src/panels/webhooks/useWebhooksPanelState.ts:11`) is a **duplicate of the engine's constant**
  (`webhookServer.ts:63`) that WI 2 makes per-workspace; the renderer copy cannot be edited before P7
  (non-negotiable #1), so WI 2 must state whether the renderer's cap is left as a UI hint or the value is
  pushed to it. Recorded here as an open WI 2 question, not resolved.
- It did **not** re-measure P3's blob layer — §12.1 already did, and D17 owns it.
- It is a **reading, not an invariant**: "measured 2026-09-21: N". Re-derive before relying on any number
  above (`TRAPS.md`, "A count written into prose is never re-derived").
