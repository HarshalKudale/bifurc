# 15 — Unified ports, the edge router, and platform-aware features

**Status:** **proposed 2026-09-23**, revised the same day after review, and **work items 1 and 2 built the
same day** — see the two 2026-09-23 blocks below. Everything else is design.
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

> ## ▶ 2026-09-23 — **`plan/16` written: the GUI's own backend, and the single image**
>
> **A plan only — nothing in it is built.** Written on the owner's instruction, from two coupled proposals:
> the web UI should stop talking to the engine directly and talk to **its own thin backend**, as Electron's
> renderer already talks to the main process; and one Docker image should carry **both** the engine and the
> GUI, with the engine exposing the GUI at **its own domain** so the GUI's port can stay unpublished.
>
> **What it takes over, and what it leaves here.** Item 5 (the GUI on the edge, §3.3's branch 3) and item 7
> (`hostKind` + capabilities) **stay this plan's work** — `plan/16` lists them as items 0 and 6 precisely
> because it cannot be verified without them, and says so. What `plan/16` *adds* is a fourth listener: an
> internal, unpublished, `127.0.0.1`-bound relay between the browser and the RPC endpoint.
>
> **Two things in it bear on this plan's decisions, so they are recorded here:**
>
> - **§3.4's "custom subdomain" is deliberate: the GUI belongs at the engine's *own* domain, not at a new
>   subdomain.** A `console.engine.example.com` costs a DNS entry and a certificate (or a wildcard) for a
>   page that *is* the engine's admin surface — which is what the root name is for. An override, not the
>   default. `plan/16` §3.4 argues it; this plan's §3.3 already chose it.
> - **Serving the admin GUI on a published port raises an auth question this plan has not answered**, and
>   `plan/16` §10 records it as an **open question for the owner** rather than deciding it. §3.3's branch 3
>   makes the GUI reachable by name; nothing in either plan yet says who may read it. Worth deciding before
>   item 5 is built, because it changes what "the GUI on the edge" means.
>
> **A correction `plan/16` makes to this plan's §5, and it is the load-bearing one.** §5 says the client
> protocol is uniform and the *addressing* is not. That is right, and it understates the case: **the browser
> host is a client that authenticates, and it holds host-shaped code** (`web/connect.ts`'s handshake and
> failure classification, `web/settings.ts`'s token, `web/api.ts`'s teardown and `lastSeq`). Electron's
> equivalent lives in the main process. `plan/16` proposes that the browser stop being the odd one out — and
> its refusal to let the new backend answer **commands of its own** is what keeps §5's uniformity from
> becoming two surfaces that merely look alike.

---

> ## ▶ 2026-09-23 — **work item 1 landed, and it needed a third thing**
>
> **Built and verified.** §7 item 1's flag chain is in the tree, together with the half it turned out to
> need. `plan/10` work item 4's exposure table and `DOCKER.md` §"What would make it publishable" both name
> *two* code changes as the blockers; this is **the first of the two**, done.
>
> ### The work item as written could not be satisfied as written
>
> It asks for *"a `--ws-allow-non-loopback` flag through `hostArgs.ts` → `hostCli.ts` → `HostTransport` →
> `createWsServer`'s existing `allowNonLoopback`, plus the fail-closed token requirement"*. **That chain
> delivers nothing on its own**, and the reason is not visible from this document — it is in `transport/ws.ts`,
> which has had **two** guards since P4 (`plan/05` §"Bind address and TLS"):
>
> | `host` | `allowNonLoopback` | `tls` | result |
> |---|---|---|---|
> | non-loopback | `true` | **absent** | **refused** — *"TLS on the channel, not optional"* |
>
> So a flag wired to `allowNonLoopback` alone reaches a guard that refuses it anyway: it parses, it passes
> through, and it changes no outcome. That is the *accepted and ignored* shape `TRAPS.md` treats as a defect,
> and building it would have made work item 1 look done while the endpoint stayed unreachable — which is
> exactly the state this plan exists to end. **The missing half is not the token (the plan names that); it is
> TLS material**, and nothing in the programme produces any.
>
> ### What landed
>
> | Piece | Where |
> |---|---|
> | `--ws-allow-non-loopback` (presence flag), `--ws-tls-cert`, `--ws-tls-key` (value flags) | `runtime/hostArgs.ts` |
> | Flag-coherence rules: the trio needs WebSocket mode, the TLS pair must be complete, and the opt-in needs a **non-loopback** `--ws-host` | `runtime/hostArgs.ts` |
> | `allowNonLoopback` + `tls` on the `ws` variant of `HostTransport`, passed through unexamined | `runtime/host.ts` |
> | **The fail-closed token guard**, at a new step **-1** — before the data root is touched | `runtime/host.ts` |
> | Cert and key read as **material**, so a bad path fails by name at startup | `runtime/hostCli.ts` |
> | `isLoopbackHost` exported, so the guard, the parser and the host share one predicate | `transport/ws.ts` |
>
> ### Three decisions worth not re-litigating
>
> 1. **The bind policy stays in `transport/ws.ts`.** The parser checks only *command-line coherence* — which
>    flags exist, which must be given together, and which cannot do anything where they were given. It does
>    **not** restate "non-loopback needs the opt-in, the opt-in needs TLS"; a second copy of that rule is a
>    copy that drifts, and the transport already reports it.
> 2. **TLS material is supplied by the operator, not generated.** `--ws-tls-cert`/`--ws-tls-key` take paths to
>    PEM. Two alternatives were deliberately **not** taken and are open decisions rather than oversights:
>    **auto-generating** a cert for the RPC endpoint from the proxy's existing `mkcert` CA (convenient, and it
>    forces a hostname-and-trust decision that belongs to a deployment, not to a bind guard), and **relaxing**
>    the TLS guard for an explicit opt-in (rejected — it would trade a two-guard rule that has held for six
>    phases for a flag).
> 3. **The fail-closed guard runs at step -1, before `setDataRoot()`.** It must not read `dataDir()`, which
>    `store/paths.ts` *throws* rather than defaulting; and a refusal that lands after `bootstrapWorkspaces()`
>    has written into the data root is a refusal that changed something on its way out. Asserted, not just
>    asserted-in-prose: a refused start leaves **no** `engine.token` on disk.
>
> ### Evidence
>
> `tests/integration/hostBindPolicy.integration.test.ts` — 3 cases, real I/O: a refusal on a token the host
> would have generated (and the absent token file), a refusal in the clear even **with** a token, and a
> **real `wss://` handshake off loopback** that completes a `hello` with the operator's token and is refused
> with a wrong one. `packages/engine/tests/runtime/hostArgs.test.ts` — 30 cases, 12 of them new. Four narrow
> mutations, each judged by exit code and each caught by the cases that should catch it:
> the fail-closed condition (**1** failure), the `allowNonLoopback` pass-through (**2**), the loopback
> coherence check (**2**), the TLS-pairing check (**1**), the WebSocket-mode check (**2**).
> Gates: `typecheck` EXIT=0; the transport + conformance gate **13 files / 0 failed / 444 passed**.
>
> ### Two things this leaves open, stated rather than implied
>
> - **The plan's own verification step is now runnable but not run.** §7's *"Work item 1 alone, verified
>   against the container: `docker run -p …` then a `hello` from the host machine"* needs a **published
>   container**, and a container has no cert material until an operator mounts some. That is the deployment
>   half of the same decision as (2) above, and it is `plan/10`'s to make.
> - ~~**Work item 2 (the proxy bind host) is untouched**~~ — **closed the same day**, see the block below.
>   The exposure table is now satisfiable in code; only its deployment half remains unrun.

---

> ## ▶ 2026-09-23 — **work item 2 landed: the proxy's bind host**
>
> **Built and verified.** §7 item 2's setting is in the tree: the literal `server.listen(port, "127.0.0.1")`
> is gone, the proxy takes a bind host, and a non-loopback value needs its own explicit opt-in.
>
> ### Two asymmetries with the RPC bind, both intended
>
> | | RPC endpoint (`transport/ws.ts`) | Proxy (`proxy/server.ts`) |
> |---|---|---|
> | non-loopback, no opt-in | refused | refused |
> | non-loopback **with** the opt-in, no TLS | **refused** | **allowed** |
> | opt-in shared with the other? | — | **no** — `--proxy-allow-non-loopback` is its own flag |
>
> **No TLS half, on purpose.** `--ws-allow-non-loopback` is worthless without `--ws-tls-cert`/`--ws-tls-key`
> because the RPC channel can carry TLS and should. A **forward proxy's client speaks plain HTTP to the
> listener** — it sends `CONNECT` for the https it wants tunnelled — so a TLS requirement there would break
> the browser the port exists for. The guard is therefore **one condition, not two**, and the proxy suite
> asserts the absence: a later "make the two binds symmetric" edit would otherwise pass everything else.
>
> **The opt-in is not shared, because the exposures are not equal** (§9): this port forwards into whatever the
> machine can reach *and* performs TLS interception. One flag for both would make accepting the smaller
> exposure imply accepting the larger.
>
> ### What landed
>
> | Piece | Where |
> |---|---|
> | `bindHost` + `allowNonLoopback` on `ProxyServerOptions`, and the guard — thrown **from the factory** | `proxy/server.ts` |
> | `DEFAULT_PROXY_BIND_HOST` — the old literal, now in one place | `proxy/server.ts` |
> | `proxyBindHost` / `proxyAllowNonLoopback`, threaded through unexamined | `runtime/workspaceRuntime.ts`, `runtime/host.ts` |
> | `--proxy-bind-host <host>`, `--proxy-allow-non-loopback`, and the coherence rule (the opt-in needs a **non-loopback** bind host) | `runtime/hostArgs.ts`, `runtime/hostCli.ts` |
> | `LOOPBACK_HOSTS` + `isLoopbackHost` in a **dependency-free** `lib/net.ts`, re-exported by `transport/ws.ts` | `lib/net.ts` |
>
> ### Four decisions worth not re-litigating
>
> 1. **The predicate moved out of `transport/ws.ts` into `lib/net.ts`.** The alternative — the proxy
>    importing it from `../transport/ws` — drags the `ws` package, `node:http`, `node:https` and
>    `assertBridgeIsTotal()`'s import-time assertion into the proxy's graph; `tests/proxy/server.test.ts`
>    mocks `http` and `https` outright, and the parser suite would have loaded a WebSocket server to compare
>    two strings. One definition, no edge between the layers, and `ws.ts` re-exports both names so its own
>    callers and the transport barrel are unchanged.
> 2. **`bindHost` is a host-local option, not an `AppConfig` field.** `AppConfig` is the git-tracked,
>    **syncing** per-workspace config (`store/gitStore.ts`), so a bind host stored there would carry one
>    machine's interface name to another and back. Same rule that keeps resolved ports in `runtime.json`
>    (`plan/14` §4) rather than in `workspace.json`.
> 3. **It throws from `createProxyServer()`, not from `start()`.** A configuration error has nothing to
>    retry, and throwing early means the host never bootstraps a data root it did not need to touch. The
>    test asserts the stronger half: `net.createServer` is never reached.
> 4. **The bind host is host-wide, not per-workspace.** A host running N workspaces publishes all N proxies
>    or none of them. A per-workspace choice needs an override source that does not exist yet, so the limit
>    is stated at the option and at the flag rather than discovered later.
>
> ### Evidence
>
> `tests/proxy/server.test.ts` — 6 new cases: the default binds `127.0.0.1`; a named host is what `listen`
> actually receives; the factory throws for a non-loopback bind with no opt-in; every non-loopback spelling
> needs the opt-in while all four loopback spellings do not; **no TLS is required**; and
> `allowNonLoopback: false` reads as "not given".
> `packages/engine/tests/runtime/hostArgs.test.ts` — 8 new cases, including the opt-in refused against a
> loopback *or absent* bind host, and the proxy opt-in accepted beside `--rpc-socket` (the two binds are
> independent, so the WebSocket-versus-socket rule must not reach these flags).
> `tests/integration/proxyBindPolicy.integration.test.ts` — **new, 5 cases, real I/O**: the default answers
> on loopback; a non-loopback bind with no opt-in fails the whole `host.start()`; a loopback bind host beside
> the opt-in still binds loopback; and, on this machine's LAN address, the listener **moves off loopback** (a
> connection to `127.0.0.1` is refused — which no firewall can fake) and is **reachable** on the address it
> was given. The two cases that need a LAN interface are **skipped and say so** when the machine has none.
>
> Four mutations, each judged by exit code and each caught by the cases that should catch it: the host
> dropping the pass-through (**3** integration failures — the wiring a mocked unit suite cannot see), the
> guard removed (**1** file), the opt-in read as `!== undefined` instead of `!== true` (**1** file), and the
> parser's coherence check removed (**2** failures).
> Gates: `typecheck` EXIT=0; unit **2,261 tests / 87 files** and integration **473 tests / 33 files**, both
> with 0 failures except the documented load-induced `gitStore` flake, which passes alone (24/24).
>
> ### Two things this leaves open, stated rather than implied
>
> - **Item 2's deployment half is unrun, exactly as item 1's is.** What is measured here is a *local*
>   connection to the machine's own LAN address: that proves the bind moved and that the interface answers,
>   but not that a **second machine** can reach it — a router or a firewall between them is not in this test.
> - **Both bind guards throw `EngineError` with code `ENGINE_ERROR`, and that code is classified
>   *retryable*** (`packages/protocol/src/errors.ts`). A refused bind is not retryable, so a client that
>   honours `retryable` would retry a configuration error. Not changed here — it is the transport's existing
>   convention, and matching it is what keeps the two guards consistent — but it is a taxonomy question worth
>   settling once, for both.

---

> ## ▶ 2026-09-23 — **work item 3 landed: the proxy is split**
>
> **Built and verified.** One socket served two jobs; there are now two servers. The **edge router**
> (`proxy/edge.ts`) is host-level and serves the reverse half; the **forward proxy** keeps its job, and the
> reverse branch is gone from it.
>
> ### The split, and the rule that produced it
>
> §3.1's rule — *a port belongs to the thing that has to be addressed once* — decides which half moves. A
> mapping is a hostname (`bifurc.localhost`), and a hostname carries no workspace in it, so the port that
> answers it must be one port for the host. A forward proxy is addressed by *a browser's proxy setting*, a
> per-machine choice the user makes per workspace, so it stays per workspace.
>
> | | Edge router | Forward proxy |
> |---|---|---|
> | Tier | **host** — one listener for every workspace | **workspace** — one per workspace, as before |
> | Port | `settings.port`, **inherited** | resolved from D16's base, **moved off** `settings.port` |
> | Serves | branch 2 (mappings, mocks first) and branch 5 (the home page) | branch 3 — `matchProxyRule`, `handleProxyRouting`, `proxyWithScripts` |
> | Bind opt-in | `--edge-allow-non-loopback` | `--proxy-allow-non-loopback` |
> | TLS half | none — a browser speaks plain HTTP | none — same reason |
>
> **The port swap is §6.2's migration, and it needs no mapping edited.** `settings.port` is *host-local*
> `app.json` (`AppSettings.port`, default `80`) — the port every existing `*.localhost` mapping already
> points at — so the **edge inherits it** and the **forward proxy moves off it** onto D16's base. That is the
> whole of §6.2's promise. It is also why the edge is **not** allocated: D16's scan-upward rule is for a port
> nobody addressed, and this one *is* addressed.
>
> ### What landed
>
> | Piece | Where |
> |---|---|
> | `createEdgeRouter()` — an `http.Server`, the routing table, the bind guard, `getError()` | `proxy/edge.ts` (**new**) |
> | The reverse branch **deleted** from `dispatch()`; the refusal names the edge | `proxy/server.ts` |
> | `dispatchConfig()` — the public method the edge composes the **same** config through | `proxy/server.ts` |
> | `buildUnservedPage()` (503), `buildNotMappedPage()`, `REASON_PHRASES` | `proxy/pages.ts` |
> | The edge wired in at step **3b**, before the runtime loop, with its port `reserved` | `runtime/host.ts` |
> | `--edge-port`, `--edge-bind-host`, `--edge-allow-non-loopback`, and the coherence check | `runtime/hostArgs.ts`, `runtime/hostCli.ts` |
> | The edge created and started before the forward proxy; stopped first on quit | `src/main.ts` (shell) |
> | The edge in the single-workspace facade too, with `edgePort` / `edgeError` on `EngineStatus` | `packages/engine/src/index.ts` |
> | `getPort() \|\| …` instead of `loadConfig().port` — the proxy's port is the **resolved** one | `proxy/serverCommands.ts`, `store/configCommands.ts` |
>
> ### Five decisions worth not re-litigating
>
> 1. **It is an `http.Server` whose handler never touches `res`.** The reverse branch hands the request to
>    `proxyToUpstream()` / `serveMock()`, which write raw bytes to a **socket**, and it cannot buffer instead
>    because `handleProxyResponse()` streams — buffering would break SSE and large downloads. So the handler
>    takes `req.socket` and leaves `res` alone. **Measured, not assumed** (node 22.22.2): a handler that
>    writes a complete raw response to `req.socket` and never touches `res`, twice on one keep-alive
>    connection, delivered both responses with no `error` and no `clientError`. Every response it writes is
>    `connection: close` and ends the socket, so a hijacked response is never followed by one Node still
>    thinks it owes.
> 2. **`Expect: 100-continue` gets no `checkContinue` listener.** Registering one would *take the interim
>    response away from Node*, which does the right thing on its own. **Measured**: headers first and the body
>    only after the interim reply reached the handler with the whole 64-byte body, and the client saw
>    `HTTP/1.1 100 Continue`. Writing a listener here would replace correct platform behaviour with a
>    hand-maintained copy of it.
> 3. **The active workspace is re-resolved per request, not cached.** `resolveConfig` is a parameter — a
>    function, not a value — because the answer belongs to the host, which is the only thing holding
>    `runtimes`. That is what makes `workspace.setActive` need **no invalidation step**: the next request
>    re-reads which workspace is active. `null` is a real answer, not a defensive one (the host skips a
>    workspace whose override was taken, and `--workspace` can filter the active one out), and it gets its own
>    **503** page — because the home page's whole content is the active workspace's mappings, and rendering
>    "No mappings yet" for a workspace that has plenty is a lie.
> 4. **The edge's bind policy is its own opt-in, and the count is the point.** There are now three ways this
>    engine can be made reachable — the RPC endpoint, the forward proxy, and the edge — and they are not
>    equally dangerous: the RPC endpoint executes commands, the forward proxy forwards into the machine's
>    network, and the edge routes a **client-chosen `Host`** to a user-configured target. One flag for all
>    three would make accepting the smallest exposure imply accepting the largest.
> 5. **The edge's port is bound, not scanned for** (see the port swap above). A bind failure is **reported**
>    through `getError()` rather than thrown, matching every other listener, and it is reported **first** —
>    before N workspaces have been placed behind it.
>
> ### A latent bug fixed on the way past, and a change that is not a translation
>
> The old hand-rolled parser took the body from the same `data` chunk that carried the header terminator, so
> **a body split across two reads was silently truncated**. The edge buffers the body fully before routing.
> That is a behaviour change, not a port: the cost is that a large upload is held in memory, and the
> alternative — a streaming `proxyToUpstream` — is a bigger change than this work item makes. Asserted in
> `tests/proxy/edge.test.ts`.
>
> ### Evidence — ⚠ **the test half of this work item was NOT written**, corrected 2026-09-23
>
> **The block that stood here described a test suite that does not exist in the tree.** It was written in
> the completed past tense and it was wrong; it is replaced by what is verifiably there, because a plan that
> records unwritten tests as done misleads the next reader in exactly the way it misled this one — the
> failures it caused were attributed to "the harness has not caught up" for a session.
>
> **Verified absent (each was asserted as existing):**
>
> | Claimed | Verified |
> |---|---|
> | `tests/proxy/edge.test.ts` — *"new, 40 cases"* | **does not exist** |
> | `tests/integration/edgeSplit.integration.test.ts` — *"new, 14 cases… the plan's own verification step"* | **does not exist** |
> | `tests/proxy/server.test.ts` — *"the reverse half's 16 cases moved out, and three negative cases added"* | **not moved**: 37 `.localhost` references remain, and **17 cases fail** asking this module for the reverse half it no longer owns |
> | eight suites *"migrated"* (`mappingsAndRules`, `facadeBinding`, `workspaceCommandScope`, `engineSmoke`, `hostStartup`, `workspaceRuntime`, `workspaceDispatchScope`, `settingsMutations`) | **two** reference `edgePort` at all, and only because an unrelated removal touched their fixtures; **zero** of the eight call `edgeRequest` |
> | `proxyHarness` *"gained the pair… `forwardRequest()`, `edgeRequest()`, `requestToPort()`"* | **none of the three existed**; `startProxy()` booted only the forward proxy |
> | *"Eight mutations, each judged by exit code"* | **unverifiable** — the suites the mutations were performed against do not exist |
>
> **What is real and was verified:** `proxy/edge.ts` itself (the module, its bind guard, `resolveConfig`
> per-request, `dispatchConfig()` public) — the *product* half of WI 3 is done and its behaviour is
> confirmed by the 400/404/503 answers this module produces. **It is the test half that is missing.**
>
> ### The harness split, done 2026-09-23 (the first real step of this backlog)
>
> `tests/integration/proxyHarness.ts` now genuinely boots **two listeners**, per the owner's instruction
> (*"forward proxy rests on separate port so split them and both boot separately"*):
>
> - `RunningProxy` is `{ port, edgePort, instance, stop }`.
> - `startProxy()` creates the proxy **instance** explicitly and `bindProxyInstance()`s it (so the module
>   façade still works), starts it, then creates the edge with
>   `resolveConfig: () => instance.dispatchConfig()` — **the same composition `runtime/host.ts` uses**, so
>   the two servers provably read one config rather than two independently-composed copies.
> - `proxyRequest()` (forward half) and `edgeRequest()` (reverse half) are two named seams over one shared
>   `rawRequest()` implementation, so they cannot drift in framing.
> - The new docblock states which listener owns which job, and that asking the **forward** proxy for a
>   `*.localhost` mapping is a deliberate **400**.
>
> **Proven, not asserted:** `tests/integration/harnessSplit.integration.test.ts` (8 cases, real I/O) shows both
> listeners binding, the edge serving a mapped `Host` / the home page / the 404, the forward proxy serving
> absolute-form traffic, and the forward proxy answering **400** for the reverse half. **Mutation:**
> `edge.start(edgePort)` → `edge.start(port)` (collapsing the two listeners, i.e. the old harness's bug) gives
> **`EXIT=1` with exactly the 4 edge cases red** — so the assertions are sensitive to the split.
> **The four existing `startProxy()` consumers are unchanged by it**: `mappingsAndRules` 7, `mocks` 12,
> `capture` 7, `tlsIntercept` 0 failures before *and* after. **And the whole-run total pins it: the split moved
> the suite by `+8 tests, +1 file, 0 failures` (2,713/117 → 2,721/118, the same 11 files red at the same
> per-file counts)** — so the harness edit is strictly additive and nothing it touched moved a prior result.
>
> ### The backlog above, closed 2026-09-24 — the 66 failures are now 0
>
> Every item in that list is done, and **the order turned out not to be the one the failures suggested**:
> re-pointing the request seam was the fix in only two files, and for the rest the *premise* had to be
> re-read against the split. What each needed, with the evidence:
>
> | Suite | Failures | What it actually needed |
> |---|---|---|
> | `tests/proxy/edge.test.ts` | — | **Written, 28 cases.** The 17 reverse-half cases moved out of `server.test.ts`, which is now the forward half (78 cases). Mock-based on purpose: it owns the routing *decisions* and the configs the filesystem cannot produce. |
> | `tests/integration/edgeSplit.integration.test.ts` | — | **Written, 18 cases** — the plan's own verification step. Supersedes `harnessSplit.integration.test.ts` (**deleted**, its 8 cases folded in as its header asked). Owns what only real sockets can: the same URL answering **differently** on the two ports (edge `200` / forward `400`), the raw-bytes handler, and `resolveConfig` per request. |
> | `settingsMutations` | 28 | **Not a re-point.** Its boot helper started only the forward proxy while `fetchHello()` asked a `*.localhost` host; it now boots **both** listeners the way `index.ts` does (edge on `settings.port` first, proxy on a resolved port). Its subject was also superseded: *"changing the proxy port moves the live server"* is no longer true or wanted, because `settings.port` is the **edge's**. 28 → 31 cases. |
> | `workspaceCommandScope` | 1 | **The assertion was inverted.** `portIsFree(configPort)` is false because the **host binds the edge** there (`host.ts:493`). The port is *correctly* occupied; the case now asserts the edge holds it and that no third *proxy* does. |
> | `workspaceDispatchScope` | 3 | Relative targets are refused by the forward half; mocks match on the **absolute** URL. Also `mockedResponseHeaders` — without it the fixture mock is a *partial* overlay that still calls an upstream, and the cases failed **502**, measuring the upstream rather than the dispatch scope. |
> | `workspaceRuntime` | 1 | The home page is the edge's, so the *liveness* probe became the forward proxy's own documented refusal. The case's own comment already said the home page was the wrong instrument for content. |
> | `entityConflictScope`, `foldersAndWorkspaces`, `protocolExecution` | 3 load errors | Stale `@/ipc/handlers/*Kinds` imports repointed at `@bifurc/engine/store/kinds/` — the **WI 10** move these three had been missed by. Their 69 cases were `skipped`, which is why the skip count fell 109 → 40. |
> | `ipc/handlers.test.ts` | 1 | Three `toHaveBeenCalledWith(<config port>)` assertions became the **resolved** port: `server.start` / `server.restart` / `config.save` all read `getPort()` first. |
>
> **Mutations, each judged by exit code, each source verified byte-identical to its pre-mutation backup:**
>
> | Mutation | Result |
> |---|---|
> | Harness collapse — `edge.start(edgePort)` → `edge.start(port)` | `EXIT=1`, **11 of 18** red (the old file caught 4) |
> | Product regression — `sendHtml(socket, 400, …)` → `200` | `EXIT=1`, **exactly the 3** boundary cases |
> | The edge caches `resolveConfig` (first call wins) | `EXIT=1`, the two edge-follows cases |
> | Collapse the two listeners in `settingsMutations`' boot helper | `EXIT=1`, 8 red |
> | `portAllocator` base-skip — `findFreePortFrom(proxyBase + 2)` | `EXIT=1`, 6 red |
>
> **The split closes against the run's own total:** `2,289 + 464 = 2,753` · `88 + 31 = 119`, with **two
> consecutive full runs green** and `tsc --noEmit` / `typecheck:packages` both `EXIT=0`. **(Superseded the same
> day by work item 5, which added 22 tests: the current figures are `2,310 + 465 = 2,775` · `88 + 31 = 119`,
> `2,735 passed / 0 failed / 40 skipped`.)** **And a single-project
> run is not the same measurement as the full run** — integration alone read `454` before the harness file was
> folded into `edgeSplit` and `464` after, which is `454 − 8 + 18` exactly.
>
> **One flake was found and fixed rather than tolerated, and the mechanism is the point.** `portAllocator`
> asserts on an *ephemeral* base's **successors** (`base + 1`, `base + 2`), and this work's extra listeners
> landed on them: three cases failed in one full run (`expected 49529 to be 49528`, `expected 50235 to be
> 50234`, and a `nextFreePortAbove` miss) while passing **35/35 isolated**. That is the **same mechanism**
> `allocateFromUncontendedBases`' header already records for `workspaceCommandScope` (2026-09-21), arriving
> through a third suite — so the remedy is a shared `holdBaseWithFreeSuccessors()` used by every
> successor-asserting case, **and the regression it guards still fails** (`EXIT=1`, 6 red). The generalisation
> worth carrying: *adding a suite that binds ports makes every "the scan lands on `base + N`" assertion in the
> tree less reliable, and nothing connects the two files.*
>
> **Still owed, and unchanged by this:** the mapping editor half of WI 4, §3.3's write-time collision refusal,
> items 8b/8c, and the deployment halves of WI 1/2/4/9/10.
>
> ### Two findings that outlive the missing suite
>
> The mutation tally that stood here (`try/catch` around `route()` deleted → 1; `res.writeHead` touched → 1;
> `config.save`'s port read reverted → 5; `server.start` → 1; `server.restart` → 1) **cannot be re-run**,
> because the suites it names were never written.
>
> **The two lessons it recorded are kept, but one of them named a case that does not exist either.**
> Re-checked 2026-09-23: there is **no** case called `server:restart reuses the port the proxy already held,
> not the one in Settings`. `settingsMutations` has exactly two restart cases —
> *"server:restart leaves the proxy answering on the configured port"* (:417) and *"server:restart picks up a
> port changed while the server was down"* (:426) — and **both are currently red** for the WI 3 reason, so
> neither can evidence anything until the harness migration lands. What survives is the *rule*, which needs
> no suite to be true:
>
> 1. **A case whose name claims more than its body does.** Two restart cases must not be satisfied by
>    `config.save`'s `!isRunning()` branch — a case that stops the server and *saves* exercises the save
>    path, not `server.restart`'s port read. **Reverting `server.restart`'s port read to
>    `loadConfig().port` left such a case green**, which is how it was found. Corollary still worth having:
>    the settings port must be moved **before** the restart, or the settings port and the resolved port are
>    both `portA` and the two reads are indistinguishable.
> 2. **A cache-primed re-resolution case cannot fail.** A case that swaps the config *before* its first
>    request passes even with `resolveConfig`'s result cached, because the cache is primed with the new
>    config. The fix is to make a request against the **old** config first, then swap. **This is why
>    `engineSmoke`'s and `hostStartup`'s re-resolution assertions matter** — they drive the real edge, and
>    they were added to real files this session.
>
> ### One gap, stated rather than papered over
>
> **The host's `reserved` set — `new Set([edgePortWanted])` — is not covered by any mutation that fails
> anything, and cannot be.** Replacing it with an empty set leaves `hostStartup`, `engineSmoke` and
> `workspaceCommandScope` all green (measured, exit 0). The reason is what the guard protects: the edge is
> started *before* the runtime loop, so by the time `allocatePorts` runs the edge's own bind usually makes
> the port look busy, and the reservation only decides the outcome in the **race window** where `listen` has
> not completed. In every integration fixture `settings.port` is a random high port while the proxy's scan
> starts at 80, so the two can never meet — and on the one platform where they do (Windows, where both
> default to 80) the collision is a timing question. A test that entered the window would have to be
> nondeterministic to notice the guard, which is the opposite of a test. The **allocator's** `reserved`
> contract is covered where it belongs (`tests/runtime/portAllocator.test.ts`, *"skips a reserved port the
> scan would otherwise have taken"*); what is uncovered is the host's one-line wiring of it.
>
> ### One correction to §3.4
>
> §3.4 says the forward half *"loses only the reverse branch"*. It loses **two** things: the reverse branch
> (branch 2) **and the home page** (branch 5, the old `host === "localhost"` page). The home page moved
> because its links are built from the port it is served on, and after the swap that port is the edge's — a
> forward proxy that still served it would build links pointing at itself. Its last trace in the forward half
> is the `400`, which no longer reads `Not a localhost domain` but names both halves: *this is the
> forward-proxy port, and mappings and the home page are the edge's*. **A subtraction that leaves a page
> behind leaves it pointing at a port that refuses it**, and that is the shape to look for next time.


---

> ## ▶ 2026-09-23 — **work item 4 landed: the edge routes any configured domain**
>
> **Built.** Branch 2's gate is the mapping lookup rather than `host.endsWith(".localhost")`, so a domain a
> user pointed at this engine routes. **⚠ Its two named test files do not exist** — see the correction in
> work item 3's block above: this claim cited `tests/proxy/edge.test.ts` (40 → **47**) and
> `tests/integration/edgeSplit.integration.test.ts` (14 → **18**), and neither is in the tree. The suite
> total this block quoted (**2,784 / 122**, +11) therefore cannot be reconciled against a run, and should
> not be used as a baseline. **WI 4's product change is real** — it is the branch-2 gate and the `via` split
> below, both visible in `proxy/edge.ts` — **but its instrument is owed**, together with WI 3's.
>
> **Update, 2026-09-24: the instrument is now present, though not at the size this block claimed.**
> `tests/proxy/edge.test.ts` exists (**28** cases, not 47) and
> `tests/integration/edgeSplit.integration.test.ts` exists (**18** cases, which happens to match). The three
> rows of the gate table below are pinned: the mapped domain and the unmapped `.localhost` in
> `tests/proxy/edge.test.ts`, and the third row — *unmapped domain of any other kind → the home page* — in
> both files, which is the row this block correctly identifies as the one no earlier test covered. **The `47`
> is not reconciled and should not be quoted as a count**; the two files' actual sizes are 28 and 18. See WI 3's
> block for the mutation evidence and the run totals.
>
> ### What the gate became, and why it is two conditions rather than one
>
> `if (mapping !== undefined || isLocalhostDomain)`. The halves are not the same condition, and what each
> one's *miss* does is the difference:
>
> | request | result |
> |---|---|
> | a mapped domain, of any kind | the reverse proxy |
> | an unmapped `.localhost` | the 404 — whoever typed it expected a mapping |
> | an unmapped domain of any other kind | §3.3 row 5, the home page |
>
> The third row was already true before this work item — the suffix test sent it there too — so the change
> **adds a route without moving one**. That is what makes it safe to land alone, and it is the row no test
> pinned until now: every existing home-page case used `Host: localhost`, so a change that sent every
> unmapped host to the 404 would have passed the whole file.
>
> ### The `via` value had to split, and the cost was measured before it was paid
>
> The route emitted `via: "rfc6761"` for every mapping. That label is **false** for `app.example.com` — RFC
> 6761 is why `.localhost` resolves with no DNS entry, and it has nothing to do with a domain the user
> pointed at the engine — and it is shown to the user as a badge (`ViaBadge`) and as `"Proxy"` in the capture
> list. So the mapping branch emits `"mapping"` for anything that is not `.localhost`; `"rfc6761"` stays for
> `.localhost`, where it is true. **One mechanism, two honest labels.**
>
> Two facts made this cheap, and both were checked rather than assumed:
>
> - **`via` is engine-local.** `@bifurc/protocol`'s `LogEntryEventSchema` is
>   `{id, timestamp, method, url, status, durationMs}` — no `via` — and `transport/eventPump.ts` projects an
>   entry down to that shape before it goes on the wire. So widening the union is **not** a protocol change.
> - **It is not persisted.** Nothing writes a log entry to disk, so there is no stored value to migrate.
>
> What it *did* cost is a hand-maintained mirror: `packages/gui/src/types/ipc.ts` copies the union, and
> `ViaBadge`'s two `Record<Via, string>` tables plus `captureUtils`'s two switches are exhaustive over it.
> That is the good kind of cost — **a forgotten case is a compile error at the file that forgot it** — and
> `packages/gui`'s typecheck confirmed all three touched files are clean.
>
> ### The gap that splitting the label exposed, and it is not this work item's
>
> **A remote client never sees `via` at all.** The Electron host subscribes to the engine's bus and forwards
> the raw `RequestLogEntry` over IPC, so the renderer's capture panel has `via`, `host`, `target` and both
> bodies. `ws` / `socket` / `stdio` all pump through `eventPump`'s projection, so a **web** client's capture
> panel receives the light shape — `via` is `undefined`, `ViaBadge` renders an empty badge, and `fulfilledBy`
> returns `undefined`.
>
> That is pre-existing, it is P7's, and it is the same seam §4.1 is about: **what a host gives the UI varies,
> and nothing declares it.** Recorded here because work item 4 is where it became visible. Not fixed here
> because it is a protocol-shape question rather than a routing one, and the fix has to decide whether the
> wire grows the field or the panel asks for the entry.
>
> ### The half of this work item that is not built, and it is not buildable yet
>
> The item also says *"make the mapping editor's suggestions depend on the host's own domain."* **That cannot
> be done in work item 4.** The editor's `domain` field is a **subdomain** input with a hardcoded `.localhost`
> suffix (`MappingsPanel.tsx`; `strings/entities.ts`'s `localHostSuffix`), so against a remote engine it
> offers a domain that cannot resolve — and the fix needs the host's own domain, which is `hello`'s capability
> signal. §4.1 is explicit that `hostKind` is *"decided by the host, not guessed by the client"*, so this is
> **work item 7's**. The engine half is complete and independently verifiable, which is what "how to start"
> item 3 asks for: *pure routing, no new port.*
>
> ### The write-time refusal this makes possible, and why it is still not here
>
> §3.3 requires refusing *"a mapping whose `domain` is the engine's own hostname, which would shadow the GUI
> and make the admin surface unreachable"*, and §8 carries it as a criterion. **Before this work item that
> collision was theoretical** — a hostname that is not `.localhost` could not route at all. Now it can, so the
> refusal becomes a real prerequisite of serving a GUI on this port. It belongs with **branch 3 (work item
> 5)**, which is what puts something there to shadow, and it needs the same missing input as the editor half:
> the host's own domain (**work item 7**). Recorded rather than silently skipped.
>
> ### One page's sentence, and one that was deliberately left alone
>
> The home page's instruction line read *"Requests to `*.localhost:8080` are routed to mapped services"* —
> **true but now misleading**, because it is the only place a user learns what this port serves and it teaches
> the one kind that needs no setup as though it were the only kind. It now reads *"Requests to a mapped domain
> are routed to that mapping's target. `*.localhost:8080` needs no setup; any other domain needs a DNS or
> hosts entry pointing at this engine."*
>
> `<h1>Local proxy is active</h1>` was **left alone**. It is loose — the page is the edge's now, and a remote
> engine's page saying "Local" is wrong — but it is a status line, this work item is not what makes it loose,
> and its role changes again when work item 5 puts the GUI on this port. Noted so it is a decision rather than
> an oversight.
>
> ### A work item 3 leftover, found by this work item's gate run
>
> **`tests/ipc/handlers.test.ts` had two assertions that only passed against the pre-split port read.** They
> expected `startServer` to be called with the *configured* port (`currentConfig.port`, 80) and with
> `config.save`'s **incoming** port (9876). Work item 3 changed both reads to prefer the facade's **resolved**
> port — `getPort() || …` — and this test mocks `getPort()` to 8080, so both assertions were false and the
> case failed **in isolation** (not a flake: the two real-git suites that failed in the same run each pass
> alone, which is the documented `EBUSY` shim behaviour).
>
> Work item 3's sweep updated `tests/integration/settingsMutations.integration.test.ts` for exactly this
> behaviour and missed the **unit** copy of it. Both are fixed, and each now asserts **both halves** of the
> expression — the resolved port winning, and the incoming port as the fallback for a process that has never
> started a proxy — because those are two branches and only one was covered.
>
> **The lesson is about the sweep, not the assertion:** *grep for the behaviour you changed, not for the layer
> you changed it in.* A port read is asserted wherever a port is read, and that was two suites, not one.

---

> ## ▶ 2026-09-23 — **a new work item 9: the host's own domain, and it had to come before 5 and 7**
>
> > **⚠ SUPERSEDED 2026-09-24 — the flag this block describes has been REMOVED.** The engine is no longer
> > deployable in the cloud, so every deployment answers as `localhost` and there is nothing for an operator to
> > choose. Read this block for the *reasoning* it records — why a domain was not an `AppConfig` field, why the
> > client learns the suffix from `hello` rather than guessing it — and **not** as a description of the tree.
> > See the 2026-09-24 revision block, and `runtime/hostArgs.ts`'s header, which is the live record.
>
> **Built and verified.** `--domain <host>` is in the tree, **required** — a headless host refuses to start
> without it — and reported to clients in `hello` as `hostDomain`. It was not in §7; it was added on the owner's
> instruction, because three other work items were each waiting on the same missing input:
>
> | Waiting on it | Why |
> |---|---|
> | §3.3 **branch 3** (work item 5) | it has to recognise which `Host` is *this engine's own* before it can serve a GUI there |
> | §3.3's **write-time collision refusal** (§8's criterion) | what a mapping must not shadow is the engine's own hostname |
> | §4.3's **mapping editor** (work item 7) | the suffix it suggests is derived from this, instead of the hardcoded `.localhost` |
>
> ### The flag, and the two shapes it has
>
> | `--domain` | what it says | what follows |
> |---|---|---|
> | `localhost` | the engine is on the operator's own machine | `*.localhost` mappings are meaningful — RFC 6761 resolves them to the *browser's* loopback, which is this engine's |
> | a `domain.TLD` | the engine is reached by name from somewhere else | `*.localhost` resolves to the visitor's machine, so a mapping is `<sub>.<domain>` |
>
> **Only the operator can answer that**, and it decides whether `*.localhost` mappings are *offered at all*.
> Hence required, and hence no environment fallback: `BIFURC_DATA_DIR` exists because a data root is an
> operational detail an image can state once, while a domain is precisely the fact that must not be inherited
> from an ambient default — inheriting it is how an engine on a server ends up claiming to be `localhost`.
>
> ### Why it is required here and defaulted in the desktop app
>
> The desktop shell starts the engine **in-process** (`createWorkspaceHost`, not `hostCli.ts`) and never sets the
> field, because its engine *is* on the user's machine: `localhost` there is a fact of the app rather than a
> choice, and a setting that can hold one value is not a setting. So `host.ts` defaults the **option** and
> `hostArgs.ts` — which only a headless host ever reaches — requires the **flag**. One rule, two entry points,
> and the difference between them is real rather than an inconsistency.
>
> **A wrong default would have been worse than no default**, which is the argument that settled it: defaulting
> to `localhost` would offer `*.localhost` mappings on every headless deployment, including the ones where they
> resolve to the visitor's machine and quietly do not reach this engine.
>
> ### Four decisions worth not re-litigating
>
> 1. **It reaches the client, or it does nothing.** Every consumer listed above is unbuilt, so a flag that
>    stopped at `WorkspaceHostOptions` would be *accepted and ignored* — the shape this plan's own work item 1
>    record is about. It is therefore advertised in `hello` as `hostDomain`, the channel §4.1 already
>    designates. **It is not `hostKind`:** §4.1's `local`/`remote` signal is a separate field and work item 7's,
>    and a client that derived it from this string would be making the decision the *host* is supposed to make.
> 2. **A CLI flag, not an `AppConfig` field.** `AppConfig` is git-tracked and syncs (`store/gitStore.ts`), so a
>    hostname stored there would carry one deployment's name to another machine and back. `proxyBindHost`'s
>    reason, and sharper here: a bind host describes a *machine*, a domain describes a **deployment**.
> 3. **Shape-checked, though the two bind hosts deliberately are not.** Not a change of policy — the difference
>    is what the value is *used for*. A bind host's only consumer asks "is it loopback?", which is total over any
>    string. This value is **displayed** (it becomes the editor's suggested suffix) and **compared** against a
>    mapping's `domain`. `http://engine.example.com/` and `engine.example.com:8080` would sail through a bind
>    guard and each produce a suggestion that resolves to nothing, plus a collision check that silently never
>    fires. Intent is still not checked: a single-label name (`bifurc`, a compose service) and an IP literal are
>    **accepted**, and two cases pin that rather than leaving it to chance — refusing one would turn a
>    deployment that works into a host that will not start.
> 4. **Canonicalised at the boundary.** `normalizeHostname` (lower-case, one trailing dot stripped) joins
>    `isLoopbackHost` in `lib/net.ts`, with `DEFAULT_HOST_DOMAIN` beside it. Two spellings of one name would make
>    the collision check miss the collision it exists to catch, and the miss is invisible because the check's
>    *passing* is also silence. Applied twice on the real path (parser, then host) — harmless because it is
>    idempotent, which a second *validation* would not be.
>
> ### What landed
>
> | Piece | Where |
> |---|---|
> | `--domain` (value flag, **required**), shape check, canonicalisation, usage text | `runtime/hostArgs.ts` |
> | `normalizeHostname`, `DEFAULT_HOST_DOMAIN` | `lib/net.ts` |
> | `WorkspaceHostOptions.domain` (default `localhost`), `status().domain`, `auth.hostDomain` | `runtime/host.ts` |
> | `hostDomain` on the hello response — optional, and **absent** when the host was not told | `protocol/envelope.ts`, `transport/auth/authenticated.ts`, `transport/ws.ts`, `transport/socket.ts` |
> | Passed through, and logged at startup | `runtime/hostCli.ts` |
> | `--domain localhost` in the container's `CMD`; the spawner's args; the documented command | `Dockerfile`, `scripts/verify-web-reconnect.mjs`, `scripts/verify-web-ui.mjs` |
>
> ### Evidence
>
> `hostArgs.test.ts` **38 → 56** cases; a new `packages/engine/tests/lib/net.test.ts` (**11**) which also gives
> `isLoopbackHost` its **first direct test** — until now it was only ever asserted through the three bind
> guards; `transport/auth.test.ts` 60 → **62**; and `workspaceHost.integration.test.ts` asserts the domain over
> a **real socket**, from a host started with a deliberately non-default one — the hop no unit test can reach,
> because it is four pass-throughs deep. Five narrow mutations, each judged by exit code:
>
> | Mutation | Caught by |
> |---|---|
> | the required check defaults instead of refusing | **1** failure — `fails when there is no domain` |
> | the shape check removed | **4** — the URL, empty-label, hyphen and length cases |
> | canonicalisation removed | **1** — `lower-cases and strips one trailing dot` |
> | the host stops passing its own domain down | **1** — the integration case, naming the line |
> | the `hello` field becomes always-present | **1** — `OMITS hostDomain when it was not told one` |
>
> Gates: `typecheck` EXIT=0 (four packages + the root); `typecheck:web` **33** / `typecheck:renderer` **34** —
> unchanged, and **none in `web/connect.ts`, `types/ipc.ts` or anything touching `hello`**, which is the
> assertion that matters for an additive protocol field.
>
> ### Three findings this work item's own gate run produced
>
> 1. **The parser suite's usage list had been stale since work item 3.** `--edge-port`, `--edge-bind-host` and
>    `--edge-allow-non-loopback` were in the parser and in `HOST_USAGE` but in **none** of the suite's
>    assertions — so the third opt-in's coherence rule was the one rule in that file with nothing testing it,
>    while the first two had a block each. Found by grepping the suite for the *flags* rather than for the
>    layer that changed, which is work item 4's own lesson, not applied at the time. Seven cases now cover them,
>    plus a case that takes the flags out of `HOST_USAGE` itself and checks the parser recognises each — the
>    direction that cannot drift.
> 2. **The deployment artifacts were stale, and one would have stopped starting.** `Dockerfile`,
>    `docker-compose.yml` and `DOCKER.md`'s opening section still said *"the opt-in exists but no entry point
>    exposes it"* and *"that needs a code change"* — claims work items 1–3 had falsified, while `DOCKER.md`'s own
>    tail already carried the corrections, so the two halves of that page contradicted each other. Corrected.
> 3. **`docker run <image> <args>` *replaces* the `CMD`; it does not append to it.** `DOCKER.md` described the
>    overrides as *"appending"* — harmless while every flag had a default, and **fatal** the moment `--domain`
>    had none: both documented `docker run` examples would have died on a missing required flag. The examples
>    now repeat it, and the page says why.
>
> ### What this leaves open, stated rather than implied
>
> - **Work items 5 and 7 are still unbuilt**, so nothing yet *consumes* the domain beyond `hello` and the
>   startup log. The flag is read, the value crosses the wire, and no UI acts on it — the honest state, not a
>   claim that the mapping editor is done.
> - **`host.ts`'s default is a *desktop* fact, not a general one.** A second in-process host that is reached
>   remotely would inherit `localhost` silently, because only `hostCli.ts` enforces the requirement. If such a
>   host is ever built, the requirement belongs in `createWorkspaceHost` too.
> - **`ws.ts`'s one-line passthrough is covered by typecheck only.** The `socket` leg is asserted end to end in
>   the integration suite; the `ws` leg is not, because no integration test starts a `wss://` host with a domain
>   — `hostBindPolicy.integration.test.ts` does a real `hello` over TLS, but asserts the bind policy, not this.
>   A one-line passthrough is a low-value place for a test; recorded so the gap is a decision.
> - **The deployment half is still unrun**, as work items 1–4's is: no container has been built with a real
>   hostname.

---

> ## ▶ 2026-09-23 — **a defect found while starting work item 5, and fixed: the headless host's CRUD kinds**
>
> **This was not a work item.** It was found while reading `edge.ts` for item 5, and it is why item 5 is
> *blocked* rather than merely next: a hosted Bifurc whose web UI cannot create a mapping is not hosted.
>
> ### What was broken
>
> The four `entity.*` commands are engine-side, and a headless host registers them through
> `registerAllEngineCommands()` — but the **options containers those commands dispatch through** were
> populated by five `register*Kinds()` files living in the **Electron shell's** `src/ipc/handlers/`. So the
> one registration path a headless host has registered the commands and left every container empty.
> Measured, with a control:
>
> | after | `entity.delete({kind:"mocks"})` |
> |---|---|
> | `registerAllEngineCommands(reg)` | `kind "mocks" is not routed through the CommandRegistry yet` |
> | …then `registerCrudKind({kind:"mocks",…})` | **`{ok:true}`** |
>
> **All sixteen kinds** answered the same way, and `isRegistered("entity.delete")` was **`true` throughout** —
> so `@bifurc/client` advertised the command, the browser UI would have offered the affordance, and the
> failure blamed the *kind* rather than the host. It is `registerAll.ts`'s own bug class one layer down: P14
> moved the commands engine-side, and the containers they read stayed behind.
>
> ### Why no gate saw it
>
> The guard that exists for exactly this shape — `workspaceHost.integration.test.ts`'s *"the engine-side list
> covers the shell's"* — compares **command names**, and enumerates
> `register([A-Za-z]+)Commands(commandRegistry);`. A container is not a command, and the kind calls take no
> argument, so the regex does not match them either. **It was green the whole time, and correctly so:** it
> asserts what it says it asserts. What was missing was an assertion about the other axis.
>
> ### The fix
>
> | Piece | Where |
> |---|---|
> | the five files, moved into the engine (`git mv` — file moves are permitted here) | `packages/engine/src/store/kinds/` |
> | `registerAllEngineKinds()` — the engine-side list | `packages/engine/src/registerAll.ts` |
> | called from `registerAllEngineCommands()`, first | `registerAll.ts` |
> | the shell's five explicit calls **kept**, now imported from the engine | `src/ipc/handlers.ts` |
> | the new guard — **invokes** rather than enumerates; input derived from `EntityKind.options` | `tests/integration/workspaceHost.integration.test.ts` |
>
> **Nothing in the five files was ever Electron** — between them they import `registerCrudKind`,
> `registerSimpleKind` and the config types, and nothing else. That is what made this a `git mv` plus five
> import rewrites rather than a port, and it is also the evidence that they belonged in the engine from the
> start: they are the **engine half** of P6 stage 3b's inversion, which the header of the file they were split
> out of already said.
>
> **The shell keeps its five explicit calls** rather than collapsing into `registerAllEngineKinds()`, for the
> reason `registerAll.ts` gives about the eighteen command calls: each carries a comment naming the P6
> reduction that made it the only registration. The two lists are kept honest by the new guard, which is
> behavioural rather than a name comparison — **strictly stronger**, because one question settles both drift
> directions: *does the command answer?*
>
> ### Evidence
>
> - **The guard is new, and it fails when it should.** Mutation: comment out `registerCrudKinds();` inside
>   `registerAllEngineKinds()` → **1 failed**, `EXIT=1`, the failure naming the unrouted kinds. Restored and
>   re-grepped in the same invocation.
> - **Touched suites:** the five integration files that register or import the kinds —
>   `workspaceHost` (3 → **4** cases), `entityConflictScope` (9), `foldersAndWorkspaces`, `protocolExecution`
>   (44), `settingsMutations` — **112 passed**, `EXIT=0`.
> - **The probe re-run after the fix:** all **16** kinds answer `{ok:true}` where they had all said *not
>   routed*. The command that found the defect is the one that confirms the fix.
> - **Gates:** `typecheck` `EXIT=0` (four packages + the root); `dist/store/kinds/` is emitted, so the Electron
>   shell — which resolves `@bifurc/engine/…` through `dist/` — loads the moved files.
>
> ### A second finding, and it is **not** this work item's — but it surfaced in its gate run
>
> The first full run after the fix reported **4 failed in 3 files**, and only two of them were the known
> `EBUSY` flake. A second run reported **2 failed in 1 file** with the flake absent, which is how the
> deterministic one separated itself: `tests/integration/workspaceDispatchScope.integration.test.ts`,
> `expected 502 to be 200`, **reproducible in isolation in 1.1 s**.
>
> **It is a stale fixture left by work item 3's harness rewrite, and it was green for an accidental
> reason.** `isFullyMocked()` asks whether *every* response header is declared mocked, and the fixture writes
> `responseHeaders` **without** `mockedResponseHeaders` — so the mock is **partial** and the engine fetches
> live headers from the request's own URL: `http://app.localhost/api/b`, i.e. **port 80 of the machine
> running the test**. Before item 3 the *mapping* supplied that upstream (a real test server) and the two
> mock cases passed for a reason unrelated to what they assert; after the split the mapping is the edge's, so
> the forward path falls back to the literal URL.
>
> | state | result |
> |---|---|
> | with a probe listener on `127.0.0.1:80` | **4 passed**, `EXIT=0` |
> | without one | **2 failed / 2 passed**, `EXIT=1` |
>
> So the file passes in a full run — whichever other suite happens to hold port 80 — and fails alone. **That
> asymmetry is the diagnosis**, and it is why "it passed in the full run" is not evidence about a case's
> soundness. Fixed in the fixture (`mockedResponseHeaders: ["content-type"]`), which makes the mock complete
> and removes the upstream entirely: **4 passed in isolation**, `EXIT=0`.
>
> **Attribution matters here, and it is recorded rather than assumed:** this file imports neither
> `registerAll` nor the kinds, and its failing path is `routingUtils.ts`'s mock-merge branch. It is item 3's
> leftover, not item 10's — and it had been passing *because* item 3's own sweep ran it inside a full suite.
>
> ### Left open, stated rather than implied
>
> - **`plan/07`'s P6 ledger still names the old paths**, and it is left alone: it is a record of a period
>   rather than a map of the tree, which is this repo's convention for those ledgers. `plan/README.md`'s
>   *file count* was a state claim, so it **is** corrected there — the five files are named, and `src/` is
>   now 12 rather than 17.
> - **A guard is now needed for the next axis too**, and this work item is the evidence: the shape to look for
>   is *"a command whose behaviour depends on state registered elsewhere"*. There is no registry of such
>   dependencies, so the general form of the check is what the new case does — **invoke, do not enumerate**.
> - **The deployment half**, as ever: no container has run this.

---

> ## ▶ 2026-09-24 — **the `--domain` flag is REMOVED: the engine is no longer deployable in the cloud**
>
> **Built.** `--domain` is gone from `hostArgs.ts` — flag, shape check, canonicalisation and the required-check
> — and `WorkspaceHostOptions.domain` is gone with it. `runtime/host.ts` composes `lib/net.ts`'s
> `DEFAULT_HOST_DOMAIN` (`localhost`) directly; `hostCli.ts` passes nothing; the container's `CMD` is now
> `--ws-host 127.0.0.1 --ws-port 9271`. **A `--domain` on a command line is now a fatal
> `unrecognised flag "--domain"`**, which is deliberate rather than incidental: the image's own `CMD` carried
> the flag until today, so a stale `docker run … --domain localhost` must fail loudly rather than start a host
> that quietly ignores the argument.
>
> ### The decision it records, which is bigger than the flag
>
> **The engine will not be deployed in the cloud.** It is either part of the Electron application or a locally
> deployed Docker image — a **per-user instance**, which is the shape the admin console and the login flow will
> need (both out of scope today). So the name an engine is addressed as is `localhost` in every deployment that
> exists: a fact about the product rather than a choice an operator makes. Subdomains of it remain the
> reverse-proxy mappings, the reverse-proxy port stays host-level and shared across workspaces, and the mappings
> table is still the **active** workspace's. None of that changes — §3.1 and §3.3 already say it.
>
> ### What it supersedes in this document
>
> - **§7 item 9, and the 2026-09-23 block above it.** That work item is *undone*, and its whole argument — "only
>   the operator knows whether their engine is local or remote, so there is no default to guess with" — is
>   retired with it. The engine cannot be remote, so there is nothing to know.
> - **§3.3's write-time collision refusal** (with §8's criterion for it and §9's risk row). It was already
>   *unnecessary for its stated reason* once the GUI's mapping became global — see §3.3's two consequences — and
>   with the engine's own name fixed it has no subject left: the name it would protect **is** `localhost`, and a
>   mapping on `localhost` is already dead by precedence. §3.3 is corrected.
> - **§4.3's mapping editor** (work item 7) is *simplified rather than deleted*: it still must not guess the
>   suffix, and `hello`'s `hostDomain` still carries it, but the suffix is always `.localhost` now, so there is
>   one suggestion and no per-deployment branch.
>
> ### What is deliberately NOT changed
>
> - **The three bind opt-ins stay** (`--ws-`, `--proxy-`, `--edge-allow-non-loopback`). They publish a *port* to
>   a LAN — a locally deployed image reached from another device is still a deployment this programme supports —
>   and they were never about a domain.
> - **`hello`'s `hostDomain` stays on the wire**, now always `localhost`. `plan/16`'s relay reads it to decide
>   its own same-origin story, and §4.1's rule (the host decides, the client does not guess) is unchanged.
> - **`EdgeRouterOptions.hostDomain` stays**, still defaulted to the constant. The edge is a module that takes
>   its own address as an input, exactly as it takes `bindHost`; what was removed is the *deployment* knob.
>
> ### Evidence, and the one gap it opens
>
> The deployment half is unverified from a second machine, as ever — but the flag's removal is locally
> checkable and is checked: `hostArgs.test.ts` **67 → 58** cases (the nine domain cases become one that pins the
> refusal), the engine typechecks, and `Dockerfile` / `docker-compose.yml` / `DOCKER.md` no longer pass the flag.
> **Measured, full run: 2,775 → 2,766 tests, 119 files both times, 0 failed** — `−9`, which is exactly the
> parser's nine.
>
> **The gap: "a headless engine's base domain maps to the web GUI" is still conditional on `--gui-target`.**
> The owner's model is that this is the *default* for a headless engine and the *only* difference from the
> Electron engine, whose base domain shows the mapping directory instead. It cannot be a default yet, because
> **the image does not carry the web GUI at all** — `plan/16` Proposal A is what puts engine and GUI in one
> image, and a default target before that would answer **502** on a fresh install, worse than the home page it
> replaced (§3.3's own reasoning). So the intent is recorded here and the default lands with `plan/16`.

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
| **G** | `Host` is the engine's own name — `localhost`, fixed since 2026-09-24 — or a bare IP literal, **and a GUI target is configured** | reverse proxy to the GUI's own port — the **global mapping** |
| 1 | the `upgrade` event, path matches the companion's | the companion bridge — **retired**, see §6.4 |
| 2 | `Host` matches a mapping in the **active workspace's** config | reverse proxy to that mapping's `target` |
| 5 | otherwise | the **home page** — the directory of the active workspace's mappings, today's `localhost` page |

**`G` is checked first, and it is where this table's original rows 3 and 4 went (work item 5, built
2026-09-24).** Two things changed when it was built, both on the owner's instruction, and both are worth
stating rather than leaving as a diff:

**The GUI is a *mapping*, not a branch with a payload.** The owner's model: *"the web UI starts as usual on
some different server like 5753 or any other port. Our edge port reverse proxy will forward base domain
requests to this web UI port, so that this base domain → webUI port becomes the global mapping in reverse
proxy config."* So the action is `proxyToUpstream()` — the very call row 2 makes — and the engine learns
nothing about the GUI beyond *"it is at this address"*. A GUI run by `vite preview`, nginx or anything else
is indistinguishable from one this repository built. **The alternative — serving `dist/web` from this
listener — was started and abandoned**, because it would have taught a *router* about MIME types, cache
headers, path traversal and Vite's `assetsDir` convention. As a target, none of that exists to get wrong.

**The GUI's target is `host:port` or a bare port meaning loopback** (`--gui-target`, `BIFURC_GUI_TARGET`,
`WorkspaceHostOptions.guiTarget`, `EngineOptions.guiTarget`). There is deliberately **no default port**: a
plausible-looking `5753` would forward the base domain to a dead port and answer **502** on a fresh install,
which is worse than the home page and misleading because it looks like a bug rather than an unconfigured
feature. `undefined` means no GUI mapping, the branch is inert, and the base domain falls through to row 5 —
**which is the desktop's normal state**, since its own window *is* the GUI.

**Two consequences of `G` being global, stated because they are decisions rather than accidents:**

- **A workspace mapping for the engine's own hostname is now *dead*, and it is not an error.** The global
  mapping wins, so the mapping is simply never consulted for that host. That makes the **write-time collision
  refusal** this section originally called for **unnecessary for its stated reason** — *"which would shadow
  the GUI and make the admin surface unreachable"* — because the GUI can no longer be shadowed. The refusal
  would still be a *courtesy* (telling a user their mapping is inert), so it remains a possible follow-up; it
  is not a correctness gap, and the request log already shows the base domain going to the GUI's port, which
  is the diagnostic an operator needs.
- **Row 4's original *"and the path is not `/`"* clause is dropped.** It would have kept the home page on a
  bare IP's root while sending every other path to the GUI, which is incoherent for a bundle whose entry
  point **is** `/` and whose assets are resolved relative to the document (`base: "./"`). The GUI's own server
  owns the whole of the base domain; the mapping directory page remains at row 5 for hosts that are neither
  this engine's own nor mapped.

**`G`'s second gate is not `isLoopbackHost`.** That predicate answers *"may a listener bind here?"* — a
property of an interface. This one answers *"did the client type this machine's address?"* — a property of the
request, and an edge bound to a LAN address is reached at `192.168.1.9`, which is not loopback and **is** the
machine. So every IP literal qualifies. **`localhost` is not `*.localhost`**: `"localhost".endsWith(".localhost")`
is false, which is what keeps the bare name (`G`) separable from a subdomain (row 2) at all.

**Branch 2 is today's branch 2, moved.** The lookup is
`activeConfig.mappings.find((m) => m.enabled && m.domain.toLowerCase() === host)`, with
`matchMock(activeConfig.mocks, …)` applied first — the same code against the same config, on a host-level
port instead of a workspace-level one. **There is no aggregate table and no cross-workspace routing**: the
edge is one port, and the active workspace's mappings are what it serves.

**The lookup *is* the gate — the `Host` does not have to end in `.localhost`** (work item 4, 2026-09-23).
`.localhost` remains special for what it does *not* need: RFC 6761 resolves it to the browser's own loopback
with no DNS entry, which is why it is a mapping's default domain. It is not special for *routing*. The two
halves are still separate conditions, because their misses differ — an unmapped `.localhost` is a mapping the
user expected and gets the 404, while an unmapped domain of any other kind is a host this engine does not
serve and gets row 5's home page. §2.4 measured that nothing but this suffix test ever stood in the way.

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

**There is no collision left to reject at write time — the paragraph that used to sit here was retired on
2026-09-24.** It asked for a refusal of *"a mapping whose `domain` is the engine's own hostname, which would
shadow the GUI and make the admin surface unreachable"*. The GUI's mapping is global and checked first, so such
a mapping is already dead by precedence (see the two consequences above), and with the engine's own name fixed
to `localhost` — `--domain` was removed the same day — the refusal would have no subject but `localhost` itself.
Duplicate domains *across* workspaces were never a collision under this model, because only the active
workspace's list is consulted. §8's criterion and §9's risk row for it go with it.

### 3.4 The forward-proxy port

The forward half keeps everything it has — `matchProxyRule`, `handleProxyRouting`, `proxyWithScripts` — and
loses only the reverse branch, which moves to the edge. Its port is the existing per-workspace proxy port
(D16's base and scan unchanged), so **a browser configured to proxy through `127.0.0.1:8080` keeps working**;
what changes is that it can now also be mocked, because mocks no longer share a socket with the reverse path.

That is *"any middle data can be mocked while using forward proxy"*, and it is a consequence of the split
rather than a feature to build.

> **Correction, 2026-09-23 (work item 3).** *"Loses only the reverse branch"* is one thing short. It loses the
> **home page** too — §3.3's branch 5, the old `host === "localhost"` page — and that is not a detail, because
> the page's links are built from the port it is served on (`buildHomePage(cfg, currentPort)`). After the swap
> that port is the edge's, so a forward proxy that still served the page would build links pointing at
> itself. What is left in the forward half where the reverse branch used to be is the `400`, and it now names
> **both** halves — *this is the forward-proxy port, and mappings and the home page are the edge's* — rather
> than the old `Not a localhost domain`.
>
> **The one user-visible cost of the port swap**, recorded here because it belongs beside §6.2's promise: on
> Windows the forward proxy's D16 base **is** `settings.port` (`80`), so the forward proxy moves to `81` while
> `80` becomes the edge. A browser whose proxy setting named `127.0.0.1:80` must be re-pointed at the port the
> app now reports. **Mappings need no edit** — that half of §6.2 holds exactly as written, which is what the
> swap was chosen for. See §9's risk row.

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

**`hostDomain` landed first** — work item 9, 2026-09-23, and its **flag was removed 2026-09-24** — **and it is
not a substitute for `hostKind`.** The host's own name is *data*: the suffix a mapping's domain is derived from,
already carried in `hello`, and now always `localhost`. `hostKind` stays a separate field decided by the host,
because a client that computed `local`/`remote` from that string would be making the call this section says the
**host** makes — and with the name fixed, that computation is now impossible as well as forbidden. Both are still
owed; the one that exists is what makes §4.3 possible at all.

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

**▶ 2026-09-24 — the first half of that is retired, and the work item is smaller for it.** The engine is no
longer deployable in the cloud (`--domain` was removed the same day; see the revision block at the top), so
there is no remote engine and **no other suffix**: the editor suggests `.localhost`, always. What survives from
work item 9 is the *channel* — `hello`'s `hostDomain`, so the client still does not guess — and what remains
genuinely open is the `target` half (a LAN address) plus the `hostKind`/capability signal that decides whether
to show the localhost variant at all. The call sites the change touches are known and are all in one place each:
`MappingsPanel.tsx` lines 71, 78 and 94 hardcode `.localhost` in the strip, the suggest and the submit paths,
`strings/entities.ts`'s `localHostSuffix` (and the panel copy at 137/139/150) names it in the UI, and
`ServicesPanel.tsx:38` with `ServerSettingsSection.tsx:79`/105 put it in the two port hints. `target`'s
`localhost:` default is a separate half and does not depend on either field.

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

> **⚠ SUPERSEDED 2026-09-23 by §6.4, and it is kept rather than deleted because the *reasoning* is still
> right.** This section moves the companion **to the edge port** and keeps its v1 contract; §6.4's decision
> (the owner's, same day) **retires the contract instead**. What survives from below and still governs: **the
> extension ships first, the engine second**, and **the extension's connection string must be derived rather
> than hardcoded** — which §6.4 measured to be literally true (`background.js:148` is
> `ws://127.0.0.1:${companionPort}`, loopback hardcoded). Read §6.4 for the decision; read this for why.

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

**Decided 2026-09-23 (owner): the companion port is deprecated outright, not relocated.** The owner's reason,
and it is the right one: *"our ws port for webGUI does everything that can be done over ws. We can later update
the extension to use same port and same contract as webGUI."* So the endpoint the companion serves is **not
kept as a second contract on a new port** — it is **retired**, and the extension is updated to speak the
**client protocol** the web GUI already speaks. The extension becomes *another authenticated client*, which is
what P4 work item 1 said the word "companion" was hiding (*"the directory name `companion` will make no sense
once the browser extension is just another client"*).

**This is stronger than §6.1 above, and §6.1 is now superseded where they disagree.** §6.1 moves the companion
*to the edge port* and keeps its v1 contract; this decision removes the contract entirely. **Item 6 changes
shape accordingly** — see §7.

`AppConfig.companionPort` is therefore **removed, not repointed**: it becomes a setting nothing reads, and
removing a config field means a migration for anyone whose `app.json` still carries it. Small, but it is the
kind of field that otherwise lingers for years.

**The extension half is measured, and it is not a one-line change.** `../bifurc-extension` (present locally,
plain JS): `background.js:22` and `popup.js:26` default `companionPort: 9271`; `background.js:64–71` reads it
and connects; **`background.js:145–148` builds `ws://127.0.0.1:${companionPort}` — loopback, hardcoded host**;
and `background.js:277` lets the popup **set** the port. So the extension has a user-facing port field, and
retiring the port means retiring **that field** too — the extension must derive the endpoint the way the web
GUI does instead of asking the user to type a number.

**The migration is therefore three coupled changes, and the order matters:**

| # | Change | Why this order |
|---|---|---|
| 1 | The extension speaks the **v2 contract**: `hello` first, `Envelope`, `RpcError`, the token | The engine cannot drop v1 while a released extension needs it. Extension first, engine second — §6.1's rule, unchanged |
| 2 | The extension's **endpoint derivation** replaces its port field | Same change, same release: there is no port to type once there is no companion port |
| 3 | The engine **deletes** `legacyCompanion.ts`, its commands, and the `9271` default | Only safe once 1 and 2 have shipped |

**What v1 gave the extension that v2 must not silently take away.** `legacyCompanion.ts`'s own header is
explicit that these are **not** aliases of registry commands, and the differences are load-bearing:

| | v1 (the companion) | the registry equivalent |
|---|---|---|
| mock validation | `"urlPattern is required"` | `"urlPattern is required **for mocks**"` |
| request validation | `"url is required"` | `"url is required **for requests**"` |
| duplicate mock | adds verbatim | `onAddConflict` **disables an existing enabled mock** |
| blank folder name | refused | **no check** — creates a folder named `"   "` |
| unknown folder kind | `"Unknown folder kind: bogus"` | `TypeError` from `FOLDER_CONFIG[undefined]` |

The third row is the serious one and its own header says so: `onAddConflict` **mutates existing state**, which
contradicts `V1_COMPANION_ACTIONS`' documented rule that the companion is a lower-trust caller and may only
**add**. So the extension moving to v2 **inherits de-duplication that switches a user's existing mock off** —
which is a **behaviour change the extension's release notes must carry**, not a refactor to perform quietly.
The first, second and fifth rows change **user-visible error strings**. `tests/integration/companionServer.
integration.test.ts` asserts four of them **verbatim**, so that suite is **deleted with the server**, and the
replacement assertions belong to the v2 client suites — not copied across, because the messages genuinely differ.

**The token is the one thing v2 needs that v1 never had.** The companion takes **no token** and sends **no
`hello`** — it is a bare `{id, action, payload}` client. The RPC surface requires both. So the extension must
learn to hold a per-start secret, which for a browser extension is a **new category of problem**: the web GUI
solves it by having a human paste it (`web/settings.ts`) or by a backend holding it (`plan/16` §3.3), and a
background service worker has neither a form nor a filesystem. **This is unsolved in `plan/16` too**, and it is
the real cost of this decision — it should be decided before item 6 starts, not during it.

---

## 7. Work items

Ordered so each is independently verifiable, and so nothing depends on something unbuilt. **Item 9 is out of
order by number because it was added on 2026-09-23, after 1–8 were written, and built the same day** — it is a
prerequisite for 5 and 7, so it came first despite its number.

1. ✅ **The bind opt-in — DONE 2026-09-23.** `--ws-allow-non-loopback`, `--ws-tls-cert` and `--ws-tls-key`
   through `hostArgs.ts` → `hostCli.ts` → `HostTransport` → `createWsServer`'s existing `allowNonLoopback`,
   plus the fail-closed token requirement `plan/10` work item 5 specifies. **The TLS flags were not in this
   work item and are not optional** — see the 2026-09-23 block above for why the item as written delivered
   nothing. *Without this, nothing later is reachable, so it was first.*
2. ✅ **A configurable proxy bind host — DONE 2026-09-23.** `--proxy-bind-host <host>` and
   `--proxy-allow-non-loopback` through `hostArgs.ts` → `hostCli.ts` → `WorkspaceHostOptions` →
   `WorkspaceRuntimeOptions` → `createProxyServer()`, which owns the guard and defaults to `127.0.0.1`.
   Larger exposure than (1) — a proxy that forwards into a private network — so it takes its **own** opt-in
   rather than sharing the RPC's, and it deliberately has **no TLS half**, because a forward proxy's client
   speaks plain HTTP to the listener. See the 2026-09-23 block above.
3. ✅ **Split the proxy — DONE 2026-09-23.** The edge router (`proxy/edge.ts`, §3.3) and the per-workspace
   forward proxy (§3.4) are two servers from one codebase. The forward half was a subtraction and the edge is
   the reverse branch moved, as this item says — with **one correction**: the subtraction took the home page
   as well as the reverse branch, because the page's links are built from the port it is served on. See the
   2026-09-23 block above, which also records the one guard no mutation can cover and why.
4. ✅ **Subdomain routing — the engine half DONE 2026-09-23.** Branch 2's gate is the mapping lookup rather
   than the `.localhost` suffix, so any configured domain routes; `via` split into `mapping` / `rfc6761`;
   the home page's instruction line corrected. **The mapping-editor half is work item 7's**, because it
   needs the host's own domain and §4.1 forbids the client guessing it — see the 2026-09-23 block above,
   which also records the write-time hostname-collision refusal and why it lands with item 5.
5. ✅ **The GUI on the edge — DONE 2026-09-24, as a global mapping rather than a static-file branch.**
   The edge reverse-proxies its own hostname, `localhost` and any bare IP literal to the port the web GUI is
   served on (`--gui-target` / `BIFURC_GUI_TARGET`; `host:port`, or a bare port meaning loopback). This is
   the owner's model and the smaller one: *"the web UI starts as usual on some different server like 5753 or
   any other port. Our edge port reverse proxy will forward base domain requests to this web UI port, so that
   this base domain → webUI port becomes the global mapping in reverse proxy config."* **The engine therefore
   learns nothing about the GUI beyond its address** — no MIME types, no cache headers, no path traversal, no
   knowledge of Vite's `assetsDir` convention — and a GUI served by `vite preview`, nginx or anything else is
   indistinguishable from one this repository built. §3.3's `G` row is the spec; the two consequences of its
   precedence, and why the write-time collision refusal this item used to imply is **no longer needed for its
   stated reason**, are recorded there. **Item 10's blocker is cleared** (it was the defect, fixed 2026-09-23).
   **Evidence:** 10 unit cases in `tests/proxy/edge.test.ts`, 1 real-I/O case in
   `tests/integration/edgeSplit.integration.test.ts` (a real GUI server on a real socket, `index.html` **and**
   an asset path **and** a query string), 11 parser cases in
   `packages/engine/tests/runtime/hostArgs.test.ts`, and **three narrow mutations, each judged by exit code**:
   the precedence inverted so a workspace mapping wins (`EXIT=1`, exactly the precedence case);
   `isLocalAddress` widened to accept every host (`EXIT=1`, exactly the two precision cases); and the IPv6 arm
   of `hostnameOf()` removed (`EXIT=1`, exactly the IPv6 case). Every mutated source restored byte-identically.
   **A small correction landed with it:** both listeners derived the host with `hostHeader.split(":")[0]`,
   which yields `"["` for `Host: [::1]:80`. That was harmless while the value was only a log label, and
   stopped being harmless once `G` routes on it — a user reaching the engine at `http://[::1]/` would have got
   the home page with no way to tell why. Now one `hostnameOf()` in `serverUtils.ts`, two callers.
   **Still open, and unchanged by this item:** who may read the GUI once it is on a published port
   (`plan/16` §10, open question 4) — now answerable, because the surface exists. **`plan/16`'s item 0** is
   this item plus a fourth internal listener; its deployment half is still owed.
6. **The companion retired, and the extension made an ordinary client.** **Reshaped 2026-09-23 by the owner's
   decision (§6.4)** — this item used to say *"the companion on the edge, with the extension's connection
   string derived from the engine's host"*, which kept the v1 contract and moved its port. The decision
   **retires the contract**: the extension speaks the **same protocol as the web GUI** (`hello`, `Envelope`,
   `RpcError`, a token), and `legacyCompanion.ts` + its commands + the `9271` default are **deleted**.
   **Three coupled changes, in this order:** (1) the extension speaks v2, (2) the extension's **port field is
   removed** and its endpoint derived (`background.js:277` currently lets the popup set the port),
   (3) the engine deletes the v1 bridge. **Extension first, engine second** — §6.1's rule, which survives.
   **Two things §6.4 records that this item must not lose:** the five behavioural differences between v1 and
   the registry are **user-visible** (four error strings, and `onAddConflict` *disabling an existing mock* —
   the serious one), and **v2 needs a token, which v1 never had and a service worker cannot paste.** That last
   one is unsolved in `plan/16` as well, and it is the real cost of this item — **decide it before starting,
   not during.**
7. **Capability negotiation.** `hostKind` and the capability list in `hello`; the UI reads it and hides.
   **Now also `plan/16`'s item 6** — the browser backend needs `hostKind` to decide its own same-origin story
   rather than inferring it from `hostDomain`, which §4.1 forbids.
8. **Per-workspace port overrides** for webhook and socket, surfaced in the settings UI with the
   preference/outcome distinction of §6.3, and `companionPort` retired (§6.4 — **retired outright, not
   repointed**: the decision removes the field and the v1 contract with it).
8b. **TLS termination on the edge — NEW 2026-09-23, and it is a blocker for the owner's model.** The owner's
   model is *"bare port 80/443 will serve the GUI"*, and **`443` does not exist yet**: `edge.ts`'s header says
   *"There is deliberately no TLS half… §3.1's `443` with TLS is a later item about terminating TLS here."* So
   this item is that later item, and it is the same shape as **work item 1** one layer out:
   `https.createServer` + `--edge-tls-cert` / `--edge-tls-key`, **operator-supplied** — `mkcert`'s local CA
   (`certManager.ts`) cannot serve a public hostname, so the existing machinery is the *wrong* half here. The
   TLS key material `ws.ts` already takes for the RPC bind is the precedent for the flag pair.
   **Renewal/rotation is unsolved and is deliberately not folded in** — a terminator that silently serves an
   expired cert is worse than one that will not start.
8c. **A `target` safety policy for mappings — NEW 2026-09-23, and it is a consequence of §6.4's decision.**
   The owner's *"[webhook and socket are] not accessible directly outside of the engine host runtime so user
   can expose them too using custom subdomain mapping"* is correct, and it makes `target` the only thing
   between a client-chosen `Host` and the host's own ports. Today **nothing checks `target`** — it is passed
   through verbatim (§2.4 measured the data model never refused one). So a mapping can point the edge at
   `127.0.0.1:<the RPC port>`, at another workspace's ports, or **at the edge itself** (a request loop).
   **§9's SSRF row is the general statement; this is the concrete rule the model needs**: refuse a `target`
   that resolves to the engine's own edge or RPC port, and decide explicitly whether loopback targets are
   allowed at all (they must be for the *local* case, so this is a **remote**-engine rule — which is
   `hostKind`'s, i.e. item 7's).
9. ⛔ **The host's own domain — UNDONE 2026-09-24.** It was built on 2026-09-23 as `--domain <host>` through
   `hostArgs.ts` → `hostCli.ts` → `WorkspaceHostOptions` → `hello`'s `hostDomain`, **required** for a headless
   host. **The flag and the option are removed**, because the engine is no longer deployable in the cloud: every
   deployment is on the machine the client is on, so the name is `localhost` and there is nothing for an
   operator to choose. `runtime/host.ts` composes `lib/net.ts`'s `DEFAULT_HOST_DOMAIN` directly, and a
   `--domain` on a command line is now a fatal unrecognised flag. **What survives the work item is the `hello`
   field**, which is still how the client learns the suffix instead of guessing it (§4.1) — it now always
   carries `localhost`. See the 2026-09-24 block above.
10. ✅ **The headless host's CRUD kinds — DONE 2026-09-23.** A defect, found while starting item 5. The four
    `entity.*` commands are engine-side and a headless host registers them, but the **options containers they
    read** are populated by five `register*Kinds()` files that lived in the **Electron shell's**
    `src/ipc/handlers/`. So `registerAllEngineCommands()` — a headless host's only path — registered
    `entity.create`/`update`/`delete` and left every container empty. Measured 2026-09-23, with a control:

    | after | `entity.delete({kind:"mocks"})` |
    |---|---|
    | `registerAllEngineCommands(reg)` | `kind "mocks" is not routed through the CommandRegistry yet` |
    | …then `registerCrudKind({kind:"mocks",…})` | **`{ok:true}`** |

    All six CRUD kinds and the three simple kinds answered the same way, and `isRegistered("entity.delete")`
    was **true** throughout — so a client advertises a command that cannot run, and the message blames the
    *kind* rather than the host. It is `registerAll.ts`'s own bug class one layer down: the commands moved
    engine-side, the containers they consult did not. Invisible to the existing drift guard by construction,
    which compares **command names** and a container is not a command. **Fixed** by moving the five files to
    `packages/engine/src/store/kinds/` and adding `registerAllEngineKinds()`, with a behavioural guard that
    invokes rather than enumerates. See the 2026-09-23 block above.

### How to start — the first three things

1. ~~**Work item 1 alone**~~ — **built** (2026-09-23), verified by real I/O against the host in
   `tests/integration/hostBindPolicy.integration.test.ts`. What remains is the *deployment* half: a
   container that mounts cert material, then a `hello` from a machine that is not the engine's. That is
   `plan/10`'s call, because a container has no certs until an operator gives it some.
2. ~~**Work item 3's forward half alone**~~ — **built** (2026-09-23). The reverse branch is deleted from the
   workspace proxy, and `*.localhost` works *through the edge*, verified by real I/O in
   `tests/integration/edgeSplit.integration.test.ts` — including the assertion no other suite can make: the
   same `Host` and path answered on each port gives two different answers.
3. ~~**Work item 4 alone**, driven by `curl -H "Host: app.example.com" http://<engine>/` from a machine that
   is not the engine's.~~ — **the engine half is built** (2026-09-23): pure routing, no new port, verified by
   real I/O in `tests/integration/edgeSplit.integration.test.ts`. The *"from a machine that is not the
   engine's"* half is **not** done, and cannot be from here — see the criterion below. What remains of this
   starting point is the mapping editor, and that is work item 7's.

---

## 8. Acceptance criteria

- [ ] A remote engine is reachable at one published port, serving the GUI, the reverse proxy and the
      companion.
- [ ] `curl -H "Host: app.example.com" http://<engine>/` reaches the mapping's `target`, from a machine that
      is not the engine's. — **the routing half is met 2026-09-23** (work item 4): a mapped non-`.localhost`
      domain forwards to its target over real sockets, asserted in
      `tests/integration/edgeSplit.integration.test.ts`, together with the negative that an *unmapped* such
      domain is not proxied. What is **not** met is *"from a machine that is not the engine's"*: that is the
      deployment half work item 1 left open (a published port, a `hello` from elsewhere), and no suite here
      can stand in for it.
- [ ] A mapping's `target` may be a LAN `host:port`, not only `localhost`. — **met 2026-09-23, and met by
      construction rather than by this work item.** §2.4 measured that nothing in the data model or the
      write path ever refused one (`entityCrudCommands.ts` validates neither `domain` nor `target`), and the
      edge passes the string through verbatim — pinned by a unit case with `192.168.1.50:3000`, which is not
      loopback at all, and exercised throughout the integration suite, whose upstream target is
      `127.0.0.1:<port>`. What work item 4 removed was the *other* obstacle: the domain's suffix.
- [ ] The edge serves the **active** workspace's mappings, and `workspace.setActive` changes which domains
      resolve without restarting anything. — **half met 2026-09-23** (work item 3): the edge resolves the
      active workspace's config **per request**, which is what makes the switch need no invalidation step
      (`tests/proxy/edge.test.ts`), and the host passes `runtimes.get(activeWorkspaceId)` through
      (`runtime/host.ts` step 3b). **The `.localhost`-gate half is now met too** (work item 4, 2026-09-23):
      branch 2 is a general domain rule, so what a switch changes is which *domains* resolve, not merely
      which `*.localhost` names do. What is **not** yet asserted end to end is the switch itself against a
      running host.
- [ ] Every workspace's forward proxy, webhook receiver and socket server run simultaneously, each on its own
      port, regardless of which workspace is active.
- [x] A forward-proxied request can be mocked, and a reverse-proxied request is unaffected by that mock.
      — **met 2026-09-23** (work item 3). The two jobs are now two servers, so a mock on the forward port
      cannot reach the reverse path. `tests/integration/edgeSplit.integration.test.ts` mocks a
      forward-proxied request with **no upstream contacted**, and the same file's first case has the reverse
      path answering from the mapping on the edge's port in the same run.
- [ ] Webhook and socket ports are user-settable per workspace, and a taken override fails loudly naming a
      free port (D16, unchanged).
- [x] ~~A mapping whose domain is the engine's own hostname is refused at write time.~~ — **WITHDRAWN
      2026-09-24.** The criterion had no subject left: the GUI's mapping is global and checked first, so such a
      mapping is dead by precedence rather than dangerous, and the engine's own name is now the fixed
      `localhost` (`--domain` removed). A refusal would be a courtesy, not a correctness gap — §3.3 records it.
- [ ] The UI does not offer `*.localhost` mapping, native dialogs or a tray against a remote engine.
      **Amended 2026-09-24 — narrower than it was.** The engine is no longer deployable in the **cloud**, so
      "remote" now means at most *another device on the same LAN* — the three bind opt-ins survive for that (see
      the revision block). The mapping half is therefore **less** wrong than it was, not more: a LAN engine is
      still reached by a name, and `*.localhost` still resolves to the browser's own machine. The capability
      half (native dialogs, a tray) is unchanged and still owed by item 7's `hostKind`.
- [ ] The extension connects to a local and a remote engine with no change to its own code.
      **Amended 2026-09-23 (§6.4).** This criterion is **half wrong now**, and the half that survives is worth
      keeping: *"with no change to its own code"* was about **not editing the extension per deployment** — i.e.
      the endpoint is derived, not hardcoded to `127.0.0.1` — and that still holds. What is **no longer true**
      is that the extension needs no code change *at all*: the decision requires it to speak v2, so it needs a
      **one-time** migration. Restated as what is actually assertable: **one extension build works against a
      local and a remote engine**, with the deployment supplied by configuration rather than by an edit.
- [x] The RPC endpoint is not published by default, and refuses to start unauthenticated on a public bind.
      — **met 2026-09-23** (work item 1). *Not published by default* is the socket/loopback default; the
      refusal is `host.ts` step -1, and it is **stricter than the criterion asks**: a *generated* token counts
      as unauthenticated, so a published bind requires a token the operator supplied. Verified by real I/O.
- [x] The forward-proxy port is not published by default, and a non-loopback bind needs its own explicit
      opt-in, separate from the RPC endpoint's. — **met 2026-09-23** (work item 2). The default is
      `127.0.0.1`; the opt-in is `--proxy-allow-non-loopback`, and it deliberately has **no TLS companion**,
      because a forward proxy's client speaks plain HTTP to the listener. Verified by real I/O.
- [ ] Electron's embedded engine is unchanged: IPC, no address, all features.

---

## 9. Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| A published forward-proxy port forwarding into a private network | **High** | **Its own** explicit opt-in (`--proxy-allow-non-loopback`), landed 2026-09-23 — deliberately not the RPC's, so accepting the smaller exposure does not imply accepting the larger. Documented as the larger of the two, and the port stays on `127.0.0.1` until an operator says otherwise |
| The edge router's `Host` routing used as an SSRF vector | **High** | A mapping's `target` is user-configured and workspace-scoped; refuse loopback/link-local targets on a *remote* engine, and never route by an unvalidated `Host` to an arbitrary address. **Widened 2026-09-23 (work item 4):** the `Host` no longer has to end in `.localhost` to select a mapping, so an unmapped host is the only thing the gate turns away and the *target* is now the whole of the control. The mitigation is unchanged and still the load-bearing one — the `Host` selects among **configured** mappings, it is not a destination — but a remote engine's `target` allow-list is now the thing to get right, and it is work item 8's neighbour, not this plan's |
| ~~The GUI and a mapping colliding on one hostname~~ | ~~Medium~~ | **Closed 2026-09-24 — the risk is retired, not mitigated.** The GUI's mapping is global and checked first, so a mapping on the engine's own hostname is *dead by precedence* rather than dangerous; and the engine's own name is now the fixed `localhost` (`--domain` removed), so there is no deployment-specific hostname left to collide with. §3.3 records it, and §8's criterion is withdrawn. The refusal was already described here as unnecessary for its stated reason once work item 5 landed |
| `AppConfig`'s port fields drifting from `runtime.json` | Medium | §6.3 — show both, and keep user choices on the preference side only |
| The extension shipping out of step with the companion's port | Medium | Extension first, engine second (work item 6) — a new extension against an old engine degrades, the reverse breaks. **Narrowed 2026-09-23, not removed:** the decision retires v1 outright (§6.4), so "out of step" now means a **released extension that only speaks v1 and an engine that has deleted the bridge** — i.e. the break is total rather than degradable, which is why the deletion (change 3) is gated on 1 and 2 having shipped |
| **The extension's v2 migration silently changes behaviour** | **High** | New, from §6.4's decision. `onAddConflict` **disables an existing enabled mock** where v1 added verbatim, and four error strings differ. Both are user-visible. Mitigated only by **release notes and a decision** — the extension inherits de-duplication and loses the additive-only guarantee, so that must be chosen deliberately rather than inherited by default |
| **The extension cannot hold a token** | **High** | New, from §6.4's decision, and **unsolved here and in `plan/16`**. v1 needed no credential; v2 does. A service worker has no form (`web/settings.ts`) and no filesystem (`plan/16` §3.3), so neither existing answer transfers. **Blocks item 6 and should be decided first** |
| ~~This plan landing half-done, leaving two port models~~ | ~~**High**~~ | **Closed 2026-09-23** — work item 3 landed whole, so there is one port model rather than two. The split either landed whole or the reverse path stayed where it was; it landed whole |
| **A browser's saved proxy setting pointing at the old proxy port** | Medium on Windows, low elsewhere | **New, from work item 3, and user-visible.** The forward proxy scans from D16's base, and on Windows that base **is** `settings.port` (`80`) — so a Windows user who set their browser to proxy through `127.0.0.1:80` now reaches the **edge**, which refuses a proxy request with a `400` that names the edge. Their *mapping* workflow is unaffected (§6.2, no mapping edited); their *browser proxy* setting needs the port the app now reports. Not mitigated in code — the app reports the forward proxy's port as `server.status.port`, and §6.3's preference-versus-outcome UI is work item 8 |

---

## 10. What this plan does not do

- **It does not change the client protocol.** `hello`, the command surface and `window.api` are unchanged;
  the additions are capability fields in an existing response.
- **It does not merge the RPC endpoint with the edge port.** §5 gives the reason.
- **It does not touch the client protocol.** `hello`, the command surface and `window.api` are unchanged;
  the additions are capability fields in an existing response. **The companion's retirement (§6.4) is the
  opposite of this for one client, and it is stated rather than hidden:** the extension **does** adopt the
  client protocol, which is the point — it becomes an ordinary client instead of a second contract. No
  *engine-side* protocol change is implied by any of it.
- **It does not aggregate mappings across workspaces.** The edge is one port; the active workspace's mappings
  are what it serves (§3.3).
- **It does not change the forward proxy's behaviour**, only its socket.
