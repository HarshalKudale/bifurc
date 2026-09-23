# 15 — Unified ports, the edge router, and platform-aware features

**Status:** **proposed 2026-09-23.** Design only; nothing here is built.
**Depends on:** P4 (transport + auth), P14 (workspace runtime), P7 (web UI), P9 (Docker).
**Revises:** `plan/14` §4's port table, and `plan/10` work item 4's exposure table.

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
2. **The port model is the wrong shape.** Ports that belong to a host are allocated per workspace and vice
   versa, and the two jobs the proxy does — reverse and forward — share one socket, so a mock cannot be
   applied to forward-proxied traffic without also affecting reverse-proxied traffic.
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

### 2.4 `LocalMapping` already has the right shape

`{ domain, target, label, id, workspaceId, enabled }`, where `target` is a `host:port` string — the user's own
config holds `{ domain: "bifurc.localhost", target: "localhost:8080" }`. **Nothing about the data model
prevents a subdomain or a LAN address**; the only thing standing in the way is the `.localhost` gate on
branch 2. `target: "192.168.1.50:3000"` would flow through `proxyToUpstream` unchanged.

### 2.5 The companion is the compatibility trap

`legacyCompanion.ts` is a **WebSocket bridge** on `127.0.0.1:9271` whose whole reason for existing is that a
**released browser extension** talks to it — *"it stays byte-compatible and keeps serving port 9271."* The
extension hardcodes the port, so **moving the companion onto the edge port breaks every installed
extension.** That is a product decision, not a refactor.

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
| **Host** | the **edge port** | the web GUI · the reverse proxy (subdomain mapping) · the companion | **per host**, shared by every workspace | `80`, or `443` with TLS |
| **Workspace** | the **forward-proxy port** | the forward proxy · **mocks and rules for forward-proxied traffic** | per workspace | base `8080` (unchanged) |
| **Workspace** | the **webhook port** | the webhook receiver | per workspace, **user-settable** | base `9101` |
| **Workspace** | the **socket port** | the WebSocket mock server | per workspace, **user-settable** | base `9103` |
| **Host** | the **RPC endpoint** | the client protocol (CLI, GUI, web) | per host | loopback / a socket — **never published by default** |

Two rows are new; one is a relocation; one changes owner.

- **The edge port is new as a *concept*** — today there is no host-level listener for product traffic at all.
  The reverse proxy is per-workspace and loopback.
- **The forward proxy moves to its own port**, which is what lets mocks apply to forward-proxied traffic
  without touching the reverse path.
- **The webhook port gains an override**, which the allocator already supports as its first resolution
  source — it is a config field today and a *user-settable* one only in the sense that the settings UI could
  write it.
- **The companion relocates** onto the edge port, with a compatibility story (§6.1).

### 3.2 Why one edge port rather than three

Because a browser gives you one origin per `host:port`, and the user's requirement is that the reverse proxy,
the GUI and the extension all be reachable at the same origin. Three ports would mean three origins, three
TLS certificates, and a CORS boundary between the GUI and the proxy — none of which is a real constraint for
a tool whose whole job is to be in the middle of the traffic.

It also collapses the Docker exposure problem: **one published port** instead of one per workspace per
purpose.

### 3.3 The edge router

One `http`/`https` server, one `upgrade` handler. Routing, in order:

| # | Condition | Result |
|---|---|---|
| 1 | the `upgrade` event, path matches the companion's | the companion bridge |
| 2 | `Host` matches a mapping in **any** workspace on this host | reverse proxy to that mapping's `target` |
| 3 | `Host` is the engine's own hostname | the **web GUI** (static assets) |
| 4 | `Host` is a bare IP or `localhost`, and the path is not `/` | the GUI |
| 5 | otherwise | the **home page** — the directory of this host's mappings, which is today's `localhost` page |

**The routing table is host-level and is built by aggregating every workspace's `mappings`.** That is the
substantive change: today `cfg.mappings` is the *active* workspace's list — `proxy/server.ts` says so in its
own comment, *"for a host member it holds the active workspace's mocks and mappings"*. A host-level router
cannot work that way, and the fix is not to widen the active workspace but to key the table:

```
host → mapping.domain  ⇒  { workspaceId, target, enabled }
```

The `workspaceId` is what makes the rest fall out: **the request's `Host` selects the workspace**, and that
workspace's mocks, rules and environments apply to it. Today the mock lookup on the reverse path uses the
*active* workspace's mocks, which is wrong the moment two workspaces both publish mappings.

**Two collisions to resolve explicitly**, because they are the only ways this design can be wrong:

- **Two workspaces claim the same domain.** Reject at write time — `mappings` is a create/update command, so
  the validation belongs there, and the error should name the workspace that holds it.
- **A mapping's domain equals the engine's own hostname.** Refuse it, or the GUI becomes unreachable.

### 3.4 The forward-proxy port

The forward half keeps everything it has — `matchProxyRule`, `handleProxyRouting`, `proxyWithScripts` — and
loses only the reverse branch, which moves to the edge. Its port is the existing per-workspace proxy port
(D16's base and scan unchanged), so **a browser configured to proxy through `127.0.0.1:8080` keeps working**;
what changes is that it can now also be mocked, because mocks no longer share a socket with the reverse path.

That is the user's *"so any middle data can be mocked while using forward proxy"*, and it is a consequence of
the split rather than a feature to build.

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

### 6.1 The companion's port, and the released extension

`legacyCompanion.ts`'s stated contract is to *"keep serving port 9271"* for an extension that is already
released. Moving it to the edge port is a **breaking change for every installed extension**, so:

- **Recommended:** bind the companion handler on the edge port *and* keep a deprecated listener on 9271 that
  dispatches into the same handler. Old extensions keep working; new ones use the edge; the compatibility
  listener is deleted when the extension's next release is the floor. This is a two-listener cost for one
  release cycle, and it is the only option that does not break users mid-upgrade.
- **Rejected:** moving it outright. It breaks a released artifact, and the extension cannot be updated
  atomically — which is the reason `hello.clientVersion` is mandatory in the first place.

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

---

## 7. Work items

Ordered so each is independently verifiable, and so nothing depends on something unbuilt.

1. **The bind opt-in.** A `--ws-allow-non-loopback` flag through `hostArgs.ts` → `hostCli.ts` →
   `HostTransport` → `createWsServer`'s existing `allowNonLoopback`, plus the fail-closed token requirement
   `plan/10` work item 5 specifies. *Without this, nothing later is reachable, so it is first.*
2. **A configurable proxy bind host.** `server.listen(port, "127.0.0.1")` becomes a setting, defaulting to
   loopback. Larger exposure than (1) — a proxy that forwards into a private network — so it should require
   the same explicit opt-in.
3. **Split the proxy.** Two servers from one codebase: the edge router (§3.3) and the per-workspace forward
   proxy (§3.4). The forward half is a subtraction; the edge is new.
4. **The host-level routing table.** Aggregate `mappings` across every workspace, keyed by domain, with the
   two collisions in §3.3 rejected at write time. This is what makes the reverse proxy correct for more than
   one workspace, independently of subdomains.
5. **Subdomain routing.** Relax the `.localhost` gate on branch 2 so any configured domain routes, and make
   the mapping editor's suggestions depend on the host's own domain.
6. **The GUI on the edge.** Serve `packages/gui`'s built bundle from the edge's branch 3. This is what turns
   "a headless engine" into "a hosted Bifurc".
7. **The companion on the edge**, with the deprecated 9271 listener of §6.1.
8. **Capability negotiation.** `hostKind` and the capability list in `hello`; the UI reads it and hides.
9. **Per-workspace port overrides** for webhook and socket, surfaced in the settings UI with the
   preference/outcome distinction of §6.3.

### How to start — the first three things

1. **Work item 1 alone**, verified against the container: `docker run -p 9271:9271` then a `hello` from the
   host machine. If that does not work, nothing else in this plan can be tested.
2. **Work item 4 alone**, against two workspaces with mappings on different domains, driven by `curl -H
   "Host: …"`. It is pure routing and needs no new port.
3. **Work item 3's forward half alone** — delete the reverse branch from the workspace proxy and confirm
   `*.localhost` still works *through the edge*. That ordering proves the split before the edge is complete.

---

## 8. Acceptance criteria

- [ ] A remote engine is reachable at one published port, serving the GUI, the reverse proxy and the
      companion.
- [ ] `curl -H "Host: app.example.com" http://<engine>/` reaches the mapping's `target`, from a machine that
      is not the engine's.
- [ ] A mapping's `target` may be a LAN `host:port`, not only `localhost`.
- [ ] Two workspaces' mappings coexist; a duplicate domain is rejected at write time naming the holder.
- [ ] A forward-proxied request can be mocked, and a reverse-proxied request is unaffected by that mock.
- [ ] Webhook and socket ports are user-settable per workspace, and a taken override fails loudly naming a
      free port (D16, unchanged).
- [ ] The UI does not offer `*.localhost` mapping, native dialogs or a tray against a remote engine.
- [ ] The released extension still works on 9271 during the deprecation window.
- [ ] The RPC endpoint is not published by default, and refuses to start unauthenticated on a public bind.
- [ ] Electron's embedded engine is unchanged: IPC, no address, all features.

---

## 9. Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| Breaking the released extension by moving 9271 | **High** | The deprecated listener in §6.1; do not delete it until the extension's floor moves |
| A published forward-proxy port forwarding into a private network | **High** | Same explicit opt-in as the RPC bind; document it as the larger exposure of the two |
| The edge router's `Host` routing used as an SSRF vector | **High** | A mapping's `target` is user-configured and workspace-scoped; refuse loopback/link-local targets on a *remote* engine, and never route by an unvalidated `Host` to an arbitrary address |
| Two workspaces claiming one domain | Medium | Reject at write time (§3.3); the failure is otherwise a silently wrong target |
| The GUI and a mapping colliding on one hostname | Medium | Refuse a mapping whose domain is the engine's own hostname |
| `AppConfig`'s port fields drifting from `runtime.json` | Medium | §6.3 — show both, and keep user choices on the preference side only |
| This plan landing half-done, leaving two port models | **High** | Work item 3 is the commit point: the split either lands whole or the reverse path stays where it is |

---

## 10. What this plan does not do

- **It does not change the client protocol.** `hello`, the command surface and `window.api` are unchanged;
  the additions are capability fields in an existing response.
- **It does not merge the RPC endpoint with the edge port.** §5 gives the reason.
- **It does not touch the companion's wire format.** §6.1 moves the port, not the protocol.
- **It does not decide the extension's future.** If the extension is being retired, §6.1's deprecated
  listener can be dropped and this plan gets simpler — but that is a product call, not a design one.
