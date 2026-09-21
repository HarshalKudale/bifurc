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
| 2 | **Environment base** — `80` on win32, `8080` elsewhere; overridable via `BIFURC_PROXY_BASE_PORT` | Scan upward: `base + 1`, `base + 2`, … |

`BIFURC_PROXY_BASE_PORT` moves the **base**, not one workspace's port, so the increment keeps working after
it — which is what a container wants: pin the published port without giving up the fallback.

**The webhook port follows the same shape**, with `9101` as its base on every platform (already above 1024, so
no split is needed) and the same contiguous scan. *(Not specified by the owner — resolved by analogy from the
proxy rule. Correct it if the intent differs.)*

**The resolved port is recorded in the host-local registry**, so a restart reuses it. A workspace that landed
on 81 because 80 was busy **keeps 81** after 80 frees up — a stable URL is worth more to the browser-proxy
workflow than reclaiming the base. *(Alternative: re-prefer the base on every start, at the cost of the proxy
silently moving under the user. Recommended against.)*

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
>    `renderer/panels/webhooks/useWebhooksPanelState.ts:11` is a UI hint the renderer cannot stop holding
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

### Work item 1 — the protocol shape (first half landed; the port plumbing is still owed)

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

> **ALLOCATOR DONE 2026-09-21 — `packages/engine/src/runtime/portAllocator.ts`, 18 real-socket cases; the
> `preflight()` half is still owed.** The allocator is its own module rather than part of
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
node <engine>/runtime/host.js --data-dir /data --rpc-socket /run/bifurc.sock --token-file /run/token
```

Documented deployment: **one container = one workspace**, one volume = that workspace's `data/` + `blobs/`.
N is supported but not recommended — one workspace per volume makes the volume, the ports and the lifecycle
all line up. This is why P9 moves *after* this phase.

### Work item 7 — the desktop adopts it

- `src/main.ts:309-310` stops calling `startServer(cfg.port)` / `startCompanionServer(...)` directly and
  builds runtimes instead. The companion call is unchanged.
- The legacy channels `server:start|stop|restart` (`src/ipc/handlers/systemHandlers.ts:9-25`) must keep
  working: they are on `window.api`, and non-negotiable #3 says `window.api` stays byte-identical. They
  must therefore resolve "which workspace" the same way the UI does — via the active pointer, composed
  client-side if necessary. This is P6 §5b-bis's lesson applied: *a key whose shell body composes something
  the engine cannot must keep composing it client-side.*
- **Acceptance for this work item is P6's rule: zero user-visible change.**

### Work item 8 — protocol changes and the compatibility record

- Every changed command name and shape into `plan/protocol-changes.md`. **Four of them are the `blob.*`
  params gaining a required `workspaceId` (D17)** — which also invalidates their `invalid` fixtures, so
  re-derive those rather than trusting them (`TRAPS.md`).
- **Verify the extension's four frozen commands are untouched** (`mock:add`, `request:add`, `folder:add`,
  `config:get` — `src/companion/allowedActions.ts`, and they are not in the changed namespaces). Assert it
  by name rather than by argument.
- `packages/protocol/src/commands/index.test.ts` generates **two** tests per command, so any command count
  change moves the baseline by exactly 2× — name the mechanism, do not wave at the delta.
- **Widening a schema invalidates its `invalid` fixture.** Adding a required `workspaceId` changes the
  near-miss; re-derive it, never trust the old one (`TRAPS.md`).
- Any ratchet guard over a shrinking list must be rewritten around its invariant (`TRAPS.md`).

---

## 5. Sequencing

```
P6 (in scope; carried: "11 e2e specs pass, unmodified")
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
| **P6** | gate includes the process split (4 carried criteria) | **downgraded (D14).** The desktop stays in-process; the four criteria (spawn, restart-with-backoff, no orphan, self-terminate) move to the workspace host's gate, where Docker's `SIGTERM` plus a socket-close watchdog satisfy them. **Criterion 2 — "11 e2e specs pass, unmodified" — is still CARRIED, not waived.** |
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

- [ ] **Two workspaces run their proxies simultaneously on different ports, each serving its own rules**,
      with no cross-talk — asserted on *identity* (each serves a mock the other does not have), not on a
      value both could share.
- [ ] **The D16 sequence holds per environment.** On `win32`: 80, then 81, then 82. Elsewhere: 8080, then
      8081, then 8082. Assert the **sequence**, not merely that each runtime came up — and assert it for
      **both** bases, since the base is the only platform-dependent value in the whole rule.
- [ ] **The platform branch is one constant.** Assert the allocator yields base `80` under a simulated
      `win32` and base `8080` otherwise, and that nothing else in the allocation path branches on platform.
- [ ] **An override fails loudly rather than silently moving.** Set `preferredPort` to a port already held;
      assert the runtime reports the conflict and names the next free port.
- [ ] **`EACCES` advances rather than killing the runtime** — set an override below 1024 on a platform that
      forbids binding there and confirm the runtime still comes up. *(Defensive per D16: the default path
      never attempts a privileged port, so this guards the override case only.)*
- [ ] **The resolved port survives a restart:** stop the host, free the base port, start again, and confirm
      the workspace that had incremented is **still** on its incremented port.
- [ ] **The allocator never hands out a reserved port** — with the companion on 9271 and the gRPC
      placeholder at 9102, assert no runtime is ever assigned either. Include the webhook interaction: the
      second workspace's webhook skips `9102` and lands on `9103`.
- [ ] `preflight()` does **not** fail on a busy *base* port (the second-workspace case) and **does** fail on
      a busy *per-workspace override*.
- [ ] `server.status` reports the **resolved** port, and it differs between two concurrently running
      workspaces.
- [ ] The webhook receiver routes by `(workspaceId, urlSuffix)`: a suffix registered in workspace A is
      `404` in workspace B.
- [ ] **Blob staging is per workspace (D17):** a blob `put` by workspace A is *not found* when `read` with
      workspace B's `workspaceId` — asserted on identity (the blob exists under A and not under B), not on a
      value both could share. And the blob root is still under `dataDir()`: `<dataDir>/blobs/<wsId>/`, so
      `path.relative(dataDir(), blobRoot(wsId))` never starts with `..`.
- [ ] **A `WorkspaceRuntime.stop()` releases its own staging** — the sweeper is per-runtime, so stopping A
      does not stop B's, and A's in-flight blobs are not collected by B's sweep.
- [ ] `server.status` reports the port and the error of the **requested** workspace, not of the last one
      started.
- [ ] A workspace host starts with **no Electron and no companion server** — assert the companion port is
      *not* listening.
- [ ] The readiness predicate is per-runtime: starting runtime B does not observe runtime A's readiness.
- [ ] `store/paths.ts` still has **exactly one** data root in the process (`_dataRootOverride` set once) —
      assert it, do not assume it.
- [ ] The desktop's user-visible behaviour is unchanged: one proxy port, one visible workspace,
      `window.api` byte-identical.
- [ ] `SIGTERM` on the host stops every runtime's servers and flushes.
- [ ] Protocol changes recorded in `plan/protocol-changes.md`; the extension's four frozen commands
      asserted unchanged **by name**.
- [ ] Fixtures re-derived after the schema change, not trusted.
- [ ] Baseline delta accounted for **by naming its mechanism** (2 tests per command in
      `packages/protocol/src/commands/index.test.ts`), not by waving at the number.

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
- **Does not remove `registerIpcHandlers()`** — that is P6 step 3 and is still outstanding.
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
| P0 | the spike's record is inert | it drives **10 legacy channels**, 2 of which P6 stage 3b deleted | **survives, via a shim** — §12.6 |
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
  (`renderer/panels/webhooks/useWebhooksPanelState.ts:11`) is a **duplicate of the engine's constant**
  (`webhookServer.ts:63`) that WI 2 makes per-workspace; the renderer copy cannot be edited before P7
  (non-negotiable #1), so WI 2 must state whether the renderer's cap is left as a UI hint or the value is
  pushed to it. Recorded here as an open WI 2 question, not resolved.
- It did **not** re-measure P3's blob layer — §12.1 already did, and D17 owns it.
- It is a **reading, not an invariant**: "measured 2026-09-21: N". Re-derive before relying on any number
  above (`TRAPS.md`, "A count written into prose is never re-derived").
