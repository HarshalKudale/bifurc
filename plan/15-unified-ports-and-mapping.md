# 15 — Unified ports, the edge router, and platform-aware features

**Status:** **proposed 2026-09-23**, revised the same day after review. Design only; nothing here is built.
**Depends on:** P4 (transport + auth), P14 (workspace runtime), P7 (web UI), P9 (Docker).
**Revises:** `plan/14` §4's port table, and `plan/10` work item 4's exposure table.

> ## Revision note — 2026-09-23, after review
>
> Three decisions in the first two drafts were wrong, and each made the plan bigger than it needs to be.
>
> 1. **The companion moves to the edge outright.** The first draft treated the released extension's hardcoded
>    port 9271 as a compatibility constraint and proposed keeping a deprecated listener for a release cycle.
>    **The extension can be updated**, so there is no constraint to work around: the companion binds the edge
>    port, the extension changes with it, and §6.1's two-listener dance is gone.
> 2. **The edge resolves mappings from the *active workspace*, not from a host-level aggregate.** The first
>    draft built a host-wide table keyed `domain ⇒ { workspaceId, target }` and made the request's `Host`
>    select the workspace. That is not the model. **The reverse proxy is host-level by *port* and
>    active-workspace-scoped by *content*** — the edge is one port shared by every workspace, and it resolves
>    against the active workspace's `mappings`, exactly as the workspace proxy does today. Everything else
>    already has its own per-workspace port, so every workspace runs at the same time regardless.
>
> What that removes: the host-level routing table (work item 4), the duplicate-domain collision, and the
> extension-compatibility risk. §3.3 and §7 are rewritten accordingly, and §8 and §9 lose the criteria and
> risks that belonged to the deleted design.
>
> 3. **The active-workspace scope is the design, not a trade-off to mitigate.** The second revision still
>    framed it as the one thing the design gives up, and put *"make it visible"* into the risk table. It is
>    neither. The switch already exists — `workspace.setActive` (`store/coreCommands.ts`) writes
>    `activeWorkspaceId` and returns the new config, and deliberately does not call `reloadConfig()`, because
>    *"the port is owned by the workspace, not by which one is selected"* — and **a user changing the active
>    workspace is taken to know that the new workspace's mappings are the ones that now apply.** Informing
>    them is a later refinement, not a prerequisite: the risk row is deleted, and *"the home page should name
>    the active workspace"* is explicitly **not** an acceptance criterion.
>
> The same handler records the other half as the intended D10 outcome: *"after switching workspace in the UI,
> the old workspace's proxy keeps answering on its port."* So "every workspace runs at the same time, and only
> the reverse proxy follows the active pointer" is not a new behaviour this plan introduces — it is the
> behaviour the engine already documents.

---

## 1. Why

There are now five ways to run Bifurc, and they do not present the same product:

| # | What | Engine lives | Client talks over |
|---|---|---|---|
| 1 | Electron desktop app | in the main process | Electron IPC |
| 2 | Browser + headless host on the same machine | a Node process | `ws://` loopback |
| 3 | Browser + headless host on another machine | a Node process | `ws://` — **unreachable today** |
| 4 | CLI | either | `ws://` or a socket |
| 5 | Container | a Node process | `ws://` — **unreachable today** |

Three problems fall out of that, and they are the subject of this plan:

1. **A remote engine cannot serve the feature that defines the product.** Localhost mapping
   (`bifurc.localhost` → `localhost:8080`) works because the browser and the engine share a loopback. Point a
   browser at an engine on another machine and the name resolves to *the browser's* loopback, where nothing
   is listening. The remote engine has to map **subdomains** to LAN addresses instead.
2. **The ports are scoped wrong.** The two jobs the proxy does — reverse and forward — share one socket, so a
   mock cannot be applied to forward-proxied traffic without also affecting reverse-proxied traffic. And the
   surfaces that belong to a *host* (the GUI, the reverse proxy, the companion) have no host-level port at
   all, while the surfaces that belong to a *workspace* are the ones that do.
3. **The UI shows every feature everywhere.** A browser pointed at a remote engine is offered localhost
   mapping, native file pickers and a tray. None can work.

---

## 2. What exists today — measured

### 2.1 Every listening socket in the engine

| Socket | Source | Binds | Scope |
|---|---|---|---|
| The proxy | `proxy/server.ts:208` — `server.listen(port, "127.0.0.1")` | **loopback, literal** | **per workspace** |
| The webhook receiver | `proxy/webhookServer.ts:205` | loopback | **per workspace** |
| The companion bridge | `transport/legacyCompanion.ts` | `127.0.0.1:9271` | **per host** |
| The RPC endpoint (`ws`) | `transport/ws.ts:796` — `server.listen(opts.port ?? 0, host)` | loopback unless `allowNonLoopback` | per host |
| The RPC endpoint (`socket`) | `transport/socket.ts:508` — `server.listen(path)` | a named pipe or `0600` unix socket | per host |
| gRPC mocks | — | **nothing binds.** `grpcMockServerPort: 9102` is a reserved placeholder | per workspace |

### 2.2 The port *preferences* and the resolved ports

`AppConfig` (`store/config.ts`) declares four: `port` (the proxy), `webhookPort`, `companionPort`,
`grpcMockServerPort`. **But this is the git-tracked workspace config**, and `plan/14` §4 is explicit that
*"the resolved port is recorded in the host-local `runtime.json`, **never** in `workspace.json`, which is
git-tracked and therefore syncs and collides."* So `AppConfig` holds **preferences** and `runtime.json` holds
**outcomes** — and the two are not currently distinguished in the UI.

The allocator (`runtime/portAllocator.ts`) knows exactly two kinds — `"proxy" | "webhook"` — with bases:

| Kind | Base | Platform split |
|---|---|---|
| proxy | `80` on win32, `8080` elsewhere; `BIFURC_PROXY_BASE_PORT` moves it | yes |
| webhook | `9101` | no (already above 1024) |

Resolution order is D16's: **per-workspace override → fail loudly**; **remembered resolved port → fall
through**; **base → scan upward**.

### 2.3 The proxy already does both jobs, on one port

`proxy/server.ts`'s dispatch, in order:

1. `host === "localhost"` → an HTML home page listing the mappings
2. **`host.endsWith(".localhost")`** → reverse proxy: `cfg.mappings.find((m) => m.enabled && m.domain.toLowerCase() === host)`, with `matchMock(cfg.mocks, …)` applied first
3. `rawTarget` is an absolute `http://`/`https://` URI → **forward proxy**, via `handleProxyRouting` (mocks + rules)
4. anything else → TLS interception / passthrough

So the split the user is asking for is a **split of one server into two**, and the routing table for the
reverse half is already a `domain → target` lookup.

**Branch 2 is the one that moves, and it moves unchanged.** Its `cfg` is the workspace's config, and the edge
resolves the same way against the *active* workspace's config (§3.3) — so the lookup itself is not rewritten,
it is relocated.

### 2.4 `LocalMapping` already has the right shape

`{ domain, target, label, id, workspaceId, enabled }`, where `target` is a `host:port` string — the user's own
config holds `{ domain: "bifurc.localhost", target: "localhost:8080" }`. **Nothing about the data model
prevents a subdomain or a LAN address**; the only thing standing in the way is the `.localhost` gate on
branch 2. `target: "192.168.1.50:3000"` would flow through `proxyToUpstream` unchanged.

### 2.5 The companion's port is free to move

`legacyCompanion.ts` is a **WebSocket bridge** on `127.0.0.1:9271` whose header says it *"stays
byte-compatible and keeps serving port 9271"* — but that is a statement about the *released* extension, not a
constraint. **The extension can be updated**, so the companion moves to the edge port and the extension ships
the matching change. There is no deprecation window to manage and no second listener to keep.

The wire format does not change; only the port it is reached on.

### 2.6 Nothing is reachable from outside

Measured while building the image: the proxy hardcodes `127.0.0.1`, and the RPC guard refuses a non-loopback
bind while `transport/ws.ts` records that *"nothing in the programme sets `allowNonLoopback`"*. `hostCli.js
--ws-host 0.0.0.0` fails. **This plan is unreachable at step zero without the bind opt-in**, which is
therefore work item 1.

---

## 3. The target model

### 3.1 Two tiers, not five ports

The rule that produces everything else: **a port belongs to the thing that has to be addressed once.**

| Tier | Port | Carries | Scope | Default |
|---|---|---|---|---|
| **Host** | the **edge port** | the web GUI · the reverse proxy (subdomain mapping) · the companion | **per host**, one port for every workspace | `80`, or `443` with TLS |
| **Workspace** | the **forward-proxy port** | the forward proxy · **mocks and rules for forward-proxied traffic** | per workspace | base `8080` (unchanged) |
| **Workspace** | the **webhook port** | the webhook receiver | per workspace, **user-settable** | base `9101` |
| **Workspace** | the **socket port** | the WebSocket mock server | per workspace, **user-settable** | base `9103` |
| **Host** | the **RPC endpoint** | the client protocol (CLI, GUI, web) | per host | loopback / a socket — **never published by default** |

Three rows are new or relocated; one changes owner.

- **The edge port is new as a *concept*** — today there is no host-level listener for product traffic at all.
- **The forward proxy moves to its own port**, which is what lets mocks apply to forward-proxied traffic
  without touching the reverse path.
- **The webhook and socket ports gain overrides**, which the allocator already supports as its first
  resolution source.
- **The companion relocates** onto the edge port, with the extension.

**Every workspace keeps running at the same time.** Only the *edge* is shared, and only the reverse proxy
reads it; the forward, webhook and socket servers are per-workspace and each has its own port, so nothing
serialises behind the active workspace except which mappings resolve.

### 3.2 Why one edge port rather than three

Because a browser gives you one origin per `host:port`, and the requirement is that the reverse proxy, the GUI
and the extension all be reachable at the same origin. Three ports would mean three origins, three TLS
certificates, and a CORS boundary between the GUI and the proxy — none of which is a real constraint for a
tool whose whole job is to be in the middle of the traffic.

It also collapses the Docker exposure problem: **one published port** instead of one per workspace per
purpose.

### 3.3 The edge router — host-level by port, active-workspace by content

One `http`/`https` server, one `upgrade` handler. Routing, in order:

| # | Condition | Result |
|---|---|---|
| 1 | the `upgrade` event, path matches the companion's | the companion bridge |
| 2 | `Host` matches a mapping in the **active workspace's** config | reverse proxy to that mapping's `target` |
| 3 | `Host` is the engine's own hostname | the **web GUI** (static assets) |
| 4 | `Host` is a bare IP or `localhost`, and the path is not `/` | the GUI |
| 5 | otherwise | the **home page** — the directory of the active workspace's mappings, today's `localhost` page |

**Branch 2 is today's branch 2, moved.** The lookup is
`activeConfig.mappings.find((m) => m.enabled && m.domain.toLowerCase() === host)`, with
`matchMock(activeConfig.mocks, …)` applied first — the same code against the same config, on a host-level
port instead of a workspace-level one. **There is no aggregate table and no cross-workspace routing**: the
edge is one port, and the active workspace's mappings are what it serves.

**How the edge learns which workspace is active.** The host already owns both halves: `app.json` holds
`activeWorkspaceId`, and the host holds `runtimes: Map<workspaceId, WorkspaceRuntime>` (P14 §4). So the edge
resolves `activeWorkspaceId` → that runtime's config → its `mappings`, re-resolved per request (a map lookup)
rather than cached, which also means a `workspace.setActive` needs no invalidation step.

**Switching the active workspace switches which domains the edge serves — and that is the design, not a cost
to be mitigated.** The switch already exists: `workspace.setActive` (`store/coreCommands.ts`) writes
`activeWorkspaceId` and returns the new config, and it deliberately does **not** call `reloadConfig()`,
because a port is owned by its workspace rather than by which one is selected. So this needs no new command,
and the edge needs no invalidation step — it resolves `activeWorkspaceId` per request, so the next request
after a switch serves the new workspace's mappings.

**The user changing the workspace is taken to know that the new active workspace's mappings are the ones that
now apply.** An affordance that *says* so — a tooltip, a line on the home page — is a later refinement, not a
prerequisite, and is deliberately not an acceptance criterion below.

The other per-workspace surfaces are unaffected, because they have their own ports: every workspace's forward
proxy, webhook receiver and socket server keep running regardless of which is active. That too is existing,
documented behaviour — the same handler records it as the intended D10 outcome: *"after switching workspace
in the UI, the old workspace's proxy keeps answering on its port. That is the intended D10 behaviour, not a
leak — the port is owned by the workspace, not by which one is selected."*

**One collision still has to be rejected at write time:** a mapping whose `domain` is the engine's own
hostname, which would shadow the GUI and make the admin surface unreachable. Duplicate domains *across*
workspaces are not a collision under this model, because only the active workspace's list is consulted.

### 3.4 The forward-proxy port

The forward half keeps everything it has — `matchProxyRule`, `handleProxyRouting`, `proxyWithScripts` — and
loses only the reverse branch, which moves to the edge. Its port is the existing per-workspace proxy port
(D16's base and scan unchanged), so **a browser configured to proxy through `127.0.0.1:8080` keeps working**;
what changes is that it can now also be mocked, because mocks no longer share a socket with the reverse path.

That is *"any middle data can be mocked while using forward proxy"*, and it is a consequence of the split
rather than a feature to build.

---

## 4. Platform-aware features

### 4.1 The signal

`hello` already has a `capabilities` field — `packages/protocol/src/envelope.ts`, and `web/connect.ts`
currently **parses it for validity and discards it**. That is the natural channel and it needs no new
mechanism. Two additions:

- **`hostKind: "local" | "remote"`** — decided by the host, not guessed by the client. `local` means the
  engine's loopback is the client's loopback, which is the only thing that makes `*.localhost` meaningful.
- **Named capabilities** for the features that vary, rather than a platform enum: `mapping.localhost`,
  `mapping.subdomain`, `mapping.edgePort`, `port.workspaceOverrides`, `tray`, `nativeDialogs`, `zoom`.

A capability list rather than `platform` because the Electron shell and a local headless host have *different*
platforms but the *same* mapping behaviour, and because a capability can be added without a protocol version
bump.

### 4.2 What each feature needs

| Feature | Local engine | Remote engine |
|---|---|---|
| `*.localhost` mapping | ✅ works | ❌ meaningless — `.localhost` is RFC 6761 and resolves to the *browser's* loopback |
| Subdomain mapping | ✅ works (once §3.3 lands) | ✅ the point |
| Native file dialogs | ✅ Electron only | ❌ → blob flows (P7 work item 3) |
| Tray / zoom / titlebar | ✅ Electron only | ❌ |
| Forward proxy | ✅ | ✅ if the forward port is published |
| Webhook / socket mocks | ✅ | ✅ if those ports are published |
| `localhost:80` convenience | ✅ | ❌ |

### 4.3 Where the UI changes

`packages/gui` gains **one** seam for this: a capability lookup, read from the session the host already
injects (`AppProps.session`), and used to hide rather than disable. The distinction matters — a greyed-out
control invites the user to work out why; an absent one does not.

The mapping editor is the substantial case: on a remote engine the `domain` field must stop suggesting
`*.localhost` and start showing the host's own domain suffix (`app.engine.example.com`), and the `target` must
stop defaulting to `localhost:` and start accepting a LAN address.

---

## 5. The uniform channel

The requirement is that one client works against all five engines. **The client half already does** — P4–P6
built exactly that, and D18 made it one artifact with `window.api` as the seam. What is not uniform is the
*addressing*:

| Engine | Today | After |
|---|---|---|
| Electron's embedded engine | IPC, no address | IPC, no address — unchanged |
| Local headless | `ws://127.0.0.1:<rpc>` | unchanged |
| Remote headless | **unreachable** | `ws://<host>:<rpc>` or `wss://` |
| Container | **unreachable** | one published port |

So this plan does **not** change the client protocol. It changes three things around it: the bind policy
(work item 1), the number of ports a deployment publishes (§3.1), and the capability handshake (§4.1).

**The RPC endpoint stays separate from the edge port, deliberately.** It is an administrative surface that
executes scripts and spawns processes; the edge is a product surface that proxies traffic. Merging them would
mean the port you publish to serve a web GUI is also the port that runs code.

---

## 6. Compatibility and migration

### 6.1 The companion's port

The companion moves from 9271 to the edge port, and **the extension ships the matching change**. No
deprecation listener, no compatibility window — the two are released together, which is the whole reason
`hello.clientVersion` is mandatory in the protocol: it lets the engine see which build is talking to it.

The one thing to get right is that the extension's connection string is derived from the engine's host rather
than hardcoded, so the same extension works against a local engine and a remote one. That is a change in the
extension, not here, and it should land before the companion's port moves.

### 6.2 Existing `*.localhost` mappings

Unchanged. They keep working on a local engine, and the domain is still matched the same way. The only change
is that they no longer *require* the engine to be local for the *code path* to run — a remote engine will
happily match `foo.localhost` too, and it will simply never be asked for it, because no browser resolves that
name to the engine.

### 6.3 Config versus `runtime.json`

`AppConfig`'s four port fields are git-tracked preferences; `runtime.json` holds resolved outcomes. The new
per-workspace overrides must land on the **preference** side (they are user choices, and D16 says a
user-chosen port that is taken must **fail loudly** rather than move), while the resolved port stays in
`runtime.json`. The UI currently shows one number and it is the resolved one; after this it has to show both,
or a user who sets 9101 and gets 9102 will not know which they are looking at.

### 6.4 The companion's config field

`AppConfig.companionPort` becomes meaningless once the companion shares the edge — it should be removed
rather than left as a setting nothing reads, and removing a config field means a migration for anyone whose
`app.json` still carries it. Small, but it is the kind of field that otherwise lingers for years.

---

## 7. Work items

Ordered so each is independently verifiable, and so nothing depends on something unbuilt.

1. **The bind opt-in.** A `--ws-allow-non-loopback` flag through `hostArgs.ts` → `hostCli.ts` →
   `HostTransport` → `createWsServer`'s existing `allowNonLoopback`, plus the fail-closed token requirement
   `plan/10` work item 5 specifies. *Without this, nothing later is reachable, so it is first.*
2. **A configurable proxy bind host.** `server.listen(port, "127.0.0.1")` becomes a setting, defaulting to
   loopback. Larger exposure than (1) — a proxy that forwards into a private network — so it should require
   the same explicit opt-in.
3. **Split the proxy.** The edge router (§3.3) and the per-workspace forward proxy (§3.4) from one codebase.
   The forward half is a subtraction; the edge is the reverse branch moved.
4. **Subdomain routing.** Relax the `.localhost` gate on branch 2 so any configured domain routes, and make
   the mapping editor's suggestions depend on the host's own domain.
5. **The GUI on the edge.** Serve `packages/gui`'s built bundle from the edge's branch 3. This is what turns
   "a headless engine" into "a hosted Bifurc".
6. **The companion on the edge**, with the extension's connection string derived from the engine's host rather
   than hardcoded. Extension first, engine second.
7. **Capability negotiation.** `hostKind` and the capability list in `hello`; the UI reads it and hides.
8. **Per-workspace port overrides** for webhook and socket, surfaced in the settings UI with the
   preference/outcome distinction of §6.3, and `companionPort` retired (§6.4).

### How to start — the first three things

1. **Work item 1 alone**, verified against the container: `docker run -p 9271:9271` then a `hello` from the
   host machine. If that does not work, nothing else in this plan can be tested.
2. **Work item 3's forward half alone** — delete the reverse branch from the workspace proxy and confirm
   `*.localhost` still works *through the edge*. That ordering proves the split before the edge is complete.
3. **Work item 4 alone**, driven by `curl -H "Host: app.example.com" http://<engine>/` from a machine that is
   not the engine's. It is pure routing and needs no new port.

---

## 8. Acceptance criteria

- [ ] A remote engine is reachable at one published port, serving the GUI, the reverse proxy and the
      companion.
- [ ] `curl -H "Host: app.example.com" http://<engine>/` reaches the mapping's `target`, from a machine that
      is not the engine's.
- [ ] A mapping's `target` may be a LAN `host:port`, not only `localhost`.
- [ ] The edge serves the **active** workspace's mappings, and `workspace.setActive` changes which domains
      resolve without restarting anything.
- [ ] Every workspace's forward proxy, webhook receiver and socket server run simultaneously, each on its own
      port, regardless of which workspace is active.
- [ ] A forward-proxied request can be mocked, and a reverse-proxied request is unaffected by that mock.
- [ ] Webhook and socket ports are user-settable per workspace, and a taken override fails loudly naming a
      free port (D16, unchanged).
- [ ] A mapping whose domain is the engine's own hostname is refused at write time.
- [ ] The UI does not offer `*.localhost` mapping, native dialogs or a tray against a remote engine.
- [ ] The extension connects to a local and a remote engine with no change to its own code.
- [ ] The RPC endpoint is not published by default, and refuses to start unauthenticated on a public bind.
- [ ] Electron's embedded engine is unchanged: IPC, no address, all features.

---

## 9. Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| A published forward-proxy port forwarding into a private network | **High** | Same explicit opt-in as the RPC bind; document it as the larger exposure of the two |
| The edge router's `Host` routing used as an SSRF vector | **High** | A mapping's `target` is user-configured and workspace-scoped; refuse loopback/link-local targets on a *remote* engine, and never route by an unvalidated `Host` to an arbitrary address |
| The GUI and a mapping colliding on one hostname | Medium | Refuse a mapping whose domain is the engine's own hostname (§3.3) |
| `AppConfig`'s port fields drifting from `runtime.json` | Medium | §6.3 — show both, and keep user choices on the preference side only |
| The extension shipping out of step with the companion's port | Medium | Extension first, engine second (work item 6) — a new extension against an old engine degrades, the reverse breaks |
| This plan landing half-done, leaving two port models | **High** | Work item 3 is the commit point: the split either lands whole or the reverse path stays where it is |

---

## 10. What this plan does not do

- **It does not change the client protocol.** `hello`, the command surface and `window.api` are unchanged;
  the additions are capability fields in an existing response.
- **It does not merge the RPC endpoint with the edge port.** §5 gives the reason.
- **It does not touch the companion's wire format.** Only the port it is reached on.
- **It does not aggregate mappings across workspaces.** The edge is one port; the active workspace's mappings
  are what it serves (§3.3).
- **It does not change the forward proxy's behaviour**, only its socket.
