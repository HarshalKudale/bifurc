# 16 — The GUI's own backend, and the single-image deployment

> ## ⚠ MOOT 2026-09-26 — both halves of what this proposed are gone
>
> **Nothing in this document was ever built** — the status line below says so, and it stayed true. Both of its
> proposals were then overtaken: **Proposal A** (one image carrying engine + GUI) died with the container
> image, and **Proposal B** (a per-GUI backend wrapper) has no subject, because the browser host it was for
> is deleted and the desktop is the single build target.
>
> Kept rather than deleted, because it is the record of a decision that was made and then overtaken — and
> because **its central constraint is still a good argument**: a wrapper that adds transport, credentials and
> static files but **no commands**, or the two hosts stop being the same shape. That constraint is why the
> surviving `packages/gui` is still imported by both hosts through the same `window.api` seam rather than
> being given a backend of its own.

> **Status: PLAN, 2026-09-23. Nothing in this document is built.** It was written on the owner's instruction,
> as a *plan* — the request was explicitly *"can you write up a plan for this"*, not "build it". Every claim
> about the existing tree below is measured and cites a file; every claim about the *proposed* tree is
> labelled as proposed. **There is no implementation, no test and no run behind any recommendation here.**

## 0. What was asked

Two proposals, given together and coupled:

> **Proposal A — deployment.** *"engine + web UI both run inside same docker container on different ports.
> engine itself adds a subdomain based mapping to the webGUI so it can be accessed from outside network. so
> that single docker image can have both GUI and engine. just like our electron app. It will be easier to host
> this way and GUI won't be accessible from outside the image since port will be limited to image. engine
> itself will expose the GUI on some custom subdomain like it exposes other services."*

> **Proposal B — architecture.** *"instead of webUI talking directly to engine over websocket we can have it
> like this. 1. WebUI talks to it's own backend using ws or API. maybe next JS or something. the webGUI's
> backend talks to engine. 2. Electron's UI talks to its own backend and backend talks to engine over pipe or
> something. so the it will be uniform like Each GUI will have it's sudo backend wrapper and that wrapper
> backend will have platform related stuff and both backends can talk to engine over pipe or sockets and such
> this way it will be exactly similar apart from communication and deployment methods."*

The insight in B is correct and worth stating plainly, because it is the reason this document exists:
**today the *host* is the layer that knows the transport, and it is also the layer that is the product.**
`web/` is not a backend — it is a browser page that happens to hold connection state. So the browser has to
own a handshake (`web/connect.ts`), a token form (`web/settings.ts`) and a reconnect story, all of which are
*host* concerns leaking into a *client*. Proposal B says: put a process in between, give it those concerns,
and both hosts collapse to the same shape.

**The plan's answer is "yes, and the interesting part is not the wrapper — it is what the wrapper must not
become."** §4 is about that.

---

## 1. Why — the two problems, kept separate

It is worth separating them, because they look like one request and they are not. **A can be done without B,
and B is worth doing without A.**

| | Problem | Who it hurts | Reversible? |
|---|---|---|---|
| **A** | A container must publish **two** ports (RPC + proxy) and the GUI is *not* served by either, so a hosted Bifurc has no UI at all | whoever hosts it | Yes — it is a build/deploy change |
| **B** | The browser is a *client that authenticates* rather than a *host that serves*, so transport state, token handling and reconnect live in the page | whoever maintains `web/` | **No** — it is a layering change, and it is the one that will be hard to undo later |

The asymmetry is the point. **B is the structural change; A is a deployment that becomes easy once B has a
home.** If B is done for A's sake it will be done badly — a thin wrapper that forwards a socket is not worth
a layer, and §4 says so.

Two facts about today's tree that change how much work each is:

1. **A pipe transport already exists and is done.** `packages/engine/src/transport/socket.ts` provides
   `createSocketTransport` / `createSocketServer` (unix domain socket, or a Windows named pipe, `0600` on
   POSIX), is a first-class `Capability.SOCKET`, and has a conformance runner
   (`transport/index.ts`'s status table: *"`socket` / named pipe — **done**"*). So Proposal B's *"backend talks
   to engine over pipe"* is **already available**; what does not exist is a **backend**.
2. **The web GUI is served by nothing.** `scripts` has `build:web` (→ `dist/web`) and `dev:web` (a Vite dev
   server), and `web/vite.config.ts` deliberately sets `base: "./"` with the comment *"the bundle is servable
   from a subpath — which is what makes the 'engine serves the UI' option in work item 5 available later."*
   That is `plan/15` work item 5, and **it is not built**: nothing in `packages/engine/src/proxy/edge.ts`
   serves static assets. A container today has an engine and no GUI.

---

## 2. What exists today — measured

### 2.1 The five hosts, and what fills `window.api`

`packages/gui` reads exactly one injected global, and D18 made that the seam. Measured:

| Host | What is between the UI and the engine | Where that is composed |
|---|---|---|
| Electron | nothing — `createClient(createIpcTransport(), { local })` exposed via `contextBridge` | `src/preload.ts` |
| Browser | **the page itself**: `createClient(createBrowserTransport({ url }), { local: createBrowserLocal() })`, *after* a `hello` | `web/api.ts`, `web/connect.ts` |

`web/api.ts`'s own header states the sameness is the architecture: *"the shared UI in `packages/gui` reads one
injected global, so the only thing that varies between the two hosts is what fills it."* **That is already the
right seam.** Both proposals are about *what sits behind it*, not about moving it.

### 2.2 The browser already has host-shaped code, and it is a symptom

Three things in `web/` are host concerns that have nowhere better to live:

- **`web/connect.ts`** — the `hello` handshake, `ConnectError`/`ConnectFailure` classification (`unreachable` /
  `unauthorized` / `version` / `refused`), and the reconnect `lastSeq`. Its own header explains why it is not
  in `createClient`: *"the renderer must not be able to tell a local engine from a remote one, and a client
  that performed the handshake itself would know."*
- **`web/settings.ts`** — the endpoint and the **token**, deliberately kept in a form rather than a query
  string, because *"a URL ends up in browser history, in a referrer header and in any proxy's access log."*
- **`web/api.ts`** — `InstalledApi`, the teardown handle, and the `lastSeq()` accessor.

**Note what these are.** A handshake, a credential, a classification of connection failure, and a reconnect
position. That is a *host's* job description. Today it is done by a React page.

### 2.3 What a container publishes, and what it does not

`plan/15` §3.1 fixed the port model: **one host-level edge port** carrying the reverse proxy, the companion and
(intended) the GUI; a **per-workspace forward-proxy port**; the **RPC endpoint** deliberately separate and never
published by default, because it *"is an administrative surface that executes scripts and spawns processes"*.

So the deployment gap is precisely: **the edge port exists and serves two of its three jobs.** `Dockerfile`'s
`EXPOSE 8080 9271` and its reachability note are accurate about what it *does* publish — and `9271` is the RPC
port, which `plan/15` §5 says should not be the thing a hosted deployment exposes. A hosted Bifurc today is
reachable only by someone willing to publish the code-execution surface.

### 2.4 The engine's own domain is now known to clients

`hello`'s `hostDomain` reaches clients with the name the engine answers as — added as `plan/15` work item 9's
`--domain` on 2026-09-23, and **that flag was removed on 2026-09-24**, so the field now always carries
`localhost`. **This is what makes A possible at all**: *"engine itself will expose the GUI on some custom
subdomain"* needs the engine to know the name it is answering as, and it does. `packages/protocol/src/envelope.ts`'s
docblock says it is *"what a mapping's domain is derived from: a mapping on `engine.example.com` is
`app.engine.example.com`."* — **read that as the shape of the derivation rather than as a live example**: with
the engine local-only, the suffix is `localhost` and the mapping is `<sub>.localhost`. A *custom* subdomain is
therefore no longer available to this plan's deployment model; the GUI belongs at the base domain, which is
what §3.4 already argues.

**That sentence is the whole of Proposal A's second half**, and it already exists as data. What is missing is
that the GUI's hostname is not a mapping — it is branch 3 of the router, and branch 3 is unbuilt.

---

## 3. The proposed model

### 3.1 One shape, three hosts, and the wrapper is not a pass-through

```
 Electron UI ──IPC──▶ electron backend ──socket/pipe──▶ ┐
                                                        ├──▶ engine  (RPC endpoint)
 Browser UI  ──HTTP/WS──▶ web backend ──ws/pipe───────▶ ┘
                                                        │
                                              ──the ONE client─────────────────
```

The rule: **`@bifurc/client` is composed exactly once per deployment, and it is composed inside the backend.**

That is already true for Electron (the main process holds it; the renderer reaches it over IPC — `plan/06`).
It is **not** true for the browser. This proposal does not invent a new layer so much as **make the browser
match Electron**, which is what the owner said: *"this way it will be exactly similar apart from communication
and deployment methods."*

| | Between UI and its backend | Between backend and engine | Backend owns |
|---|---|---|---|
| **Electron** | Electron IPC, no boundary, no auth | **unchanged** — in-process, no address | `window.api`, dialogs, tray, zoom, blob paths |
| **Browser** (proposed) | HTTP + WS, **same-origin** | `ws` (or `socket`) + `hello` + token | the token, the handshake, reconnect, static serving |
| **CLI** (P8, parked) | n/a | `stdio` — already done | its own argv/exit-code shell |

### 3.2 The web backend's API — and why it is not a second protocol

**This is the load-bearing decision, and the obvious wrong one is to expose the command surface as REST.**
That would mean every command needs a route, a method and a body schema, and the two would drift — a
hand-maintained mirror of a 144-key surface, which is exactly the failure `plan/15` §4.3 already records for
`packages/gui/src/types/ipc.ts` (where a forgotten case is at least a *compile error*; a REST route is not).

So the backend exposes **two things, and only two**:

1. **A WebSocket carrying the client protocol verbatim.** The engine's framing, the engine's `Envelope`, the
   engine's `seq`. The browser's `window.api` is filled by `createClient(createBrowserTransport({ url:
   "/rpc" }), …)` — **the same call that is there today, pointed at a relative URL.** `web/connect.ts`'s
   handshake moves to the backend unchanged, because it is the same handshake.
2. **Static assets** for the built GUI, from `dist/web`.

There is no route per command, no body schema, no second version to keep in step. **The backend is a
*relay* whose only additions are the ones a page cannot perform: holding a token, reading a file, using a
socket, and deciding what the engine's name is.**

This is also why **"maybe Next.js" should not be taken**, and the plan says so directly rather than leaving it
open:

- Next's value is server-side rendering and file-system routing. **This backend has neither** — the UI is
  already a client-rendered React app with a Vite build (`web/vite.config.ts`), and the routes are two.
- Next would add a build tool, a runtime, a second bundler and an SSR boundary to a process that would
  otherwise be `node` plus the engine's own code — in an image that already has `git`, `mkcert` and the
  engine's dependency tree.
- The one thing it would genuinely give — server functions holding the token — is **twenty lines of
  `http.createServer`**, and those twenty lines are already written twice in this repo
  (`proxy/server.ts`, `proxy/edge.ts`).

**Recommendation: a plain Node process, the same shape as the engine's own servers.** If the owner prefers
Next for other reasons that is a legitimate choice, but it should be made as a preference rather than as a
technical requirement, because the technical case is against it.

### 3.3 Why the browser→backend hop is worth it — three concrete, not aesthetic, reasons

The wrapper must *earn* its layer, so the test is: **what does it do that the page cannot?**

| It does | Because the page cannot |
|---|---|
| **Holds the token** | A browser cannot read `/data/engine.token`, and `web/settings.ts`'s header records why a user-typed token in a form is the worst option: it is a per-start secret, and typing it means it is in the clipboard, in a password manager, or on a note. **In the single-image deployment the token never leaves the container and no human ever sees it.** That is a real security improvement, not a tidy-up |
| **Reads the engine's address** | The backend is *in* the deployment: it can read `engine.token`, the RPC port and `--domain` from the same place the engine wrote them, instead of asking the user. `web/api.ts`'s header already flags this as the thing that changed once the container had a form: *"the endpoint is **not** derived here… it is *chosen* rather than assumed"*. In a single image it is *known*, and the form becomes the exception rather than the entry point |
| **Survives a page reload** | Today a reload is a new connection, a new `hello`, a re-read of the token from the form, and a fresh `lastSeq` of `0` — so §2.3's reconnect work is defeated by F5. A backend holds the session across reloads and can serve `lastSeq` from memory |

And one that is about A rather than B: **a same-origin backend removes CORS and mixed-content as categories.**
The page talks to `/rpc` on its own origin, over the same scheme. There is no cross-origin policy to configure
and no `wss`-vs-`ws` mismatch, and — because the GUI is served *by the edge* — it is reachable at the
hostname the engine already answers as.

### 3.4 Proposal A, restated as two work items rather than one

The owner's sentence contains **two independent things**, and conflating them is how the shape gets lost:

- **A1 — the image carries the GUI.** `dist/web` ships in the image, and a server serves it. Who serves it is
  B's question.
- **A2 — the engine exposes the GUI on its own subdomain.** This is `plan/15` §3.3 **branch 3** (the GUI by
  the engine's own hostname), which is `plan/15` work item 5 and is *not* built. It needs `--domain`
  (work item 9, done) and nothing else.

**A2 is the part that makes the deployment feel like the Electron app**, and it is *not new work invented
here* — it is an existing `plan/15` item that this plan gives a reason and a shape. **A1 without A2 is a port
you publish; A2 without A1 is a route that serves nothing.** They must land together, which is why they are
one deliverable.

**The "custom subdomain" is worth an explicit decision: it should default to *the engine's own hostname*, not
to a new subdomain.** `--domain engine.example.com` with branch 3 already means the GUI is at
`https://engine.example.com`. Putting it at `console.engine.example.com` means a second name to resolve, a
second certificate to issue (or a wildcard), and a DNS entry the operator must create — for a page that is the
engine's *own* admin surface, which is what the root name is for. **`plan/15` §3.3 chose the root name
deliberately**, and this plan agrees; a subdomain should be available as an override, not as the default.

### 3.5 The single image — what it publishes

> **The model, settled by the owner on 2026-09-23, in his words:** *"if its just the domain then we directly
> serve GUI. for everything else we use the mapping table… bare port 80/443 will serve the GUI and subdomains
> will serve the mappings."* And: *"the edge port is the only thing a human or a browser ever addresses;
> every other listener is reached through it by name, or is not reached at all."*
>
> **That is a better statement than this plan's §3.4 had, and three things follow from it:**
>
> 1. **The GUI is a routing row, not a port** — `Host === <engine's own domain>` → the GUI; anything else →
>    the mapping table. **This settles §10 Q3: the root domain, decided by the owner.** §3.4's argument
>    stands (a `console.` subdomain costs a second cert for the page that *is* the admin surface), and the
>    owner's `domain.tld` + `*.domain.tld` observation is the cleaner justification: **one certificate
>    covers both**, so the subdomain objection disappears either way.
> 2. **One mapping must be refused, and it is the only collision:** a mapping whose domain *is* the engine's
>    own hostname shadows the GUI outright. `plan/15` §3.3 already specifies the refusal; **it is unbuilt**
>    (it needs WI 7's `hostKind`). The owner's *"for everything else we use the mapping table"* is right for
>    everything **except** this row.
> 3. **`*.ws` / `*.webhook` reach the outside as ordinary mappings** — *"user can expose them too using custom
>    subdomain mapping."* Correct, **and it makes `target` a safety boundary for the first time**: a mapping's
>    `target` is `host:port` with no loopback/link-local check today, so a mapping can point the edge at the
>    engine's *own* ports — including the edge itself (a loop) and the RPC endpoint. **A `target` policy is
>    now required, not optional.** `plan/15` §9's SSRF row names the general case; this makes it concrete.
>
> **One thing to resolve before this model can be built at all: the edge has no TLS.** `edge.ts`'s header is
> explicit — *"There is deliberately no TLS half… §3.1's `443` with TLS is a later item about **terminating**
> TLS here."* So **"port 443 will be occupied by the engine" is aspirational**: the edge is
> `http.createServer`, plain. Serving a public hostname is therefore `https.createServer` + an **operator-
> supplied** cert (`mkcert`'s local CA cannot serve a public name), i.e. `--edge-tls-cert`/`--edge-tls-key` —
> **`plan/15` work item 1's shape, one layer out.** Cert renewal is a second, unsolved problem.

| Port | Inside the image | Published? | Why |
|---|---|---|---|
| **edge** (80, and 443 once TLS lands) | the GUI, the reverse proxy, the companion | **yes, the one port** | it is the product surface (A2/`plan/15` §3.3). **The companion is being retired, not relocated** — `plan/15` §6.4 (owner's decision, 2026-09-23): the extension adopts the client protocol and the v1 bridge on 9271 is deleted. Until that lands, this row is a *destination* |
| **RPC** | the `ws`/`socket` endpoint | **no** | `plan/15` §5: it executes scripts and spawns processes. **The owner's reason for keeping it separate is different and reinforcing:** *"that does not go through edge server it introduces no latency or bloating in the edge server itself"* |
| **web backend** | the wrapper's listener | **no** | reached only by the edge, on `127.0.0.1` inside the container — which is the owner's own requirement: *"GUI won't be accessible from outside the image since port will be limited to image"* |
| forward proxy | per workspace | **no by default** | `plan/15` work item 2; opt-in is separate and deliberate |
| webhook / socket | per workspace | **no by default** | reachable **through the edge by mapping** (the owner's point 3), which is also why the `target` policy above is needed |

**The "GUI is not reachable from outside" property the owner asked for is achieved by the backend binding
`127.0.0.1` inside the container** — not by the image alone. An unpublished port is not a security property;
a listener that only answers on the container's loopback is. Saying so matters, because "it is not in the
`EXPOSE` line" is a documentation fact and `EXPOSE` does not bind anything.

---

## 4. What the wrapper must **not** become — the risk this plan is really about

Proposal B is correct, and it has one failure mode that would make it worse than today's arrangement. **A
logic-bearing backend turns one surface into two.**

Today there is **one** client protocol, and `@bifurc/client` is the single typechecked encoding of it
(`packages/client/src/surface.ts`, 144 keys; the drift guards in `tests/integration/workspaceHost…` and the
P5 surface test are built on that). If the web backend starts answering *its own* commands — "get the
mappings", "start a workspace", "upload a file" — then:

- every new engine command needs **two** implementations, and one of them is in a process nothing typechecks
  against `surface.ts`;
- the browser and Electron stop being *"exactly similar"*, which is the stated goal — the browser would have
  a smaller, differently-named surface;
- the `window.api` sameness that D18 and `packages/gui` depend on is broken **quietly**, because the UI would
  still compile: it reads a global, and only the *contents* would differ per host.

**So the rule this plan proposes, in one line: the web backend may add *transport*, *credentials* and
*static files* — and no commands.** If a capability is needed by both hosts it is an engine command; if it is
needed only by the browser it is a browser feature. There is no third category, and the moment there is, the
uniformity the proposal was made for is gone.

The same rule for Electron, stated because B applies it to both: **the Electron backend already exists**
(`src/main.ts` + `src/preload.ts`), and its job is *platform* — dialogs, tray, zoom, blob paths — not
commands. **It must not grow a command surface either**, for the same reason.

---

## 5. Compatibility and migration

### 5.1 `web/` does not go away; it changes what it talks to

The page keeps `<App />`, keeps `window.api`, keeps the container/connect split. **One line changes** — the
URL `createBrowserTransport` is given, from an absolute `ws://host:port` to a relative `/rpc` — and the
handshake moves behind it. `web/connect.ts`'s *content* is mostly reusable; its *location* is the change, and
`ConnectFailure`'s four cases are exactly the vocabulary a backend needs to report a connection problem to a
page, so they should move rather than be rewritten.

### 5.2 The direct-connect form is not deleted, it is demoted

`web/settings.ts`'s endpoint+token form exists because a *standalone* browser host pointed at a *separate*
engine is a real deployment (`plan/14` D16, and the way `scripts/verify-web-ui.mjs` works today).
**That case must keep working** — it is how the reconnect verification runs. So the backend serves the GUI
*and* the page can still be pointed at an arbitrary engine; the difference is that the single-image case
needs no form at all.

**This is a real fork in the design and it should be decided explicitly, not drifted into:**
(a) the backend *is* the only path and the form is deleted; (b) the backend is the default and the form is an
escape hatch. **This plan recommends (b)** — because (a) breaks a working deployment to make a new one tidy,
and because `plan/15`'s whole §3 model is about *not* collapsing distinctions that are real.

### 5.3 `hello` gains nothing

Deliberately. The backend performs the *existing* handshake on the page's behalf; `HelloResponseSchema` is
unchanged, and `hostDomain` (work item 9; the flag behind it was removed 2026-09-24, so it is now always
`localhost`) is what the backend *reads* to decide its own same-origin story.
**`plan/15` work item 7's `hostKind` and capability list remain the way a client learns what a host can do** —
this plan does not add a second mechanism for the same question.

### 5.4 The image stops publishing the RPC port

`Dockerfile`'s `EXPOSE 8080 9271` and `docker-compose.yml`'s `network_mode: host` both exist because the
engine binds loopback and nothing could serve a GUI. **After A2, the recommendation is `ports: ["80:80"]` and
no host networking** — the edge is a real, bindable, published surface, which is the thing `plan/15` work item
3 was for. The RPC port stays unpublished, so `plan/10`'s *"reach it via docker exec, a sidecar, or an SSH
tunnel"* becomes the *documented* path for administration rather than the only path for everything.

**This is a compatibility break for anyone running today's compose file**, and it should be released as one:
the old file's whole premise (`network_mode: host`) is that the engine answers on the host's own loopback.

---

## 6. Work items

Ordered so each is independently verifiable. **Items 1 and 2 are prerequisites from `plan/15`, not new work
here** — listed so the ordering is honest rather than implied.

0. **`plan/15` work item 5 — the GUI on the edge (branch 3).** *Not this plan's work, but it is A2 and item 1
   below cannot be verified without it.* Serves `dist/web` from the edge when `Host` is the engine's own
   domain. Needs `--domain` (done).

1. **A1 — the image carries the GUI.** `build:web` runs in the `Dockerfile` build stage; `dist/web` is copied
   into the runtime stage. **Verify by:** the bundle is present in the image and is byte-identical to a local
   `npm run build:web`. *This is the smallest change here and it unblocks the rest.*

2. **The web backend, transport-only.** A Node process: serves `dist/web`, proxies one WebSocket path to the
   engine's RPC endpoint, holds the token, reads the engine's address and domain. **No commands.** Composed
   from `http.createServer` + the engine's `createWsServer`/`createSocketTransport` as the *client* side.
   **Verify by:** `scripts/verify-web-ui.mjs` passes against the single image with **no form filled in** —
   which is the observable difference from today.

3. **`web/` repointed.** `createBrowserTransport({ url: "/rpc" })`; `connect.ts` moves to the backend;
   `web/settings.ts` becomes the §5.2 escape hatch. **Verify by:** the existing web unit tests
   (`web/tests/`) still pass, and `verify:web` + `verify:web:reconnect` still pass in **both** modes
   (backend-served and directly-pointed).

4. **The Electron backend's boundary stated, not moved.** `src/main.ts` already holds the client. This item
   is a **guard, not a refactor**: assert that the renderer's surface is the client's surface
   (`window.api` ← `{ ...client }`, the existing P5 surface test) and that nothing Electron-specific has
   been added to it. **Verify by:** a mutation — add one method to `window.api` in `preload.ts` and confirm
   the surface test fails. *Do this last, and do it because §4's failure mode is a quiet one.*

5. **The deployment: one published port, no host networking.** `docker-compose.yml` moves to
   `ports: ["80:80"]`; the RPC port is unpublished; `DOCKER.md` documents `docker exec` for administration.
   **Verify by:** `curl -H "Host: <domain>" http://<host>/` returns the GUI, and the same request on the RPC
   port is **refused** from outside. *The deployable half of `plan/15` work items 1–4 lives or dies here.*

6. **`plan/15` work item 7 — `hostKind` and capabilities.** *Not this plan's work; listed because §5.3
   depends on it and because the mapping editor's `*.localhost` half is still waiting on it.*

### How to start — the first three things

1. **Item 0** alone, because it is already specified, it is not this plan's, and **it is the thing that makes
   the GUI reachable at all** — *"engine itself will expose the GUI on some custom subdomain"* is branch 3.
2. **Item 1** alone — build `dist/web` into the image and confirm it is there. It cannot fail in an
   interesting way, which is exactly why it should go first: it turns "the image has a GUI" from a claim into
   an artifact.
3. **Item 2's smallest form**: a process that serves `dist/web` and nothing else, then one that also proxies
   one socket. **Do not start with the token or the address derivation** — those are the parts that are easy
   to get subtly wrong, and they are only worth getting right once the relay is proven to be a relay.

---

## 7. Acceptance criteria

- [ ] The GUI is reachable at the engine's own domain (`https://engine.example.com`), served by the edge,
      over a published port. *(= `plan/15` item 5 + this plan's A2.)*
- [ ] The web backend's listener is reachable **only** from inside the container — asserted by a request from
      outside to the backend's port being refused, not by the port being absent from `EXPOSE`.
- [ ] The RPC endpoint is **not** published in the recommended compose file, and the image documents
      `docker exec` for administration.
- [ ] A browser loaded from the single image reaches a working UI **without a human entering an endpoint or a
      token**.
- [ ] `scripts/verify-web-ui.mjs` and `scripts/verify-web-reconnect.mjs` pass in both modes (§5.2).
- [ ] **The web backend exposes no command of its own** — asserted by a test that enumerates the backend's
      routes/endpoints and finds exactly two (static, and the socket path). *(This is §4 as a criterion
      rather than a paragraph.)*
- [ ] `window.api` is identical in shape between Electron and the browser — the existing P5 surface test
      covers this, and item 4 confirms it still can fail.
- [ ] Electron behaves exactly as it does today: IPC, no boundary, no auth, all features.

---

## 8. Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| **The backend grows a command surface** — §4's failure mode, and the one that would make this plan worse than the status quo | **High** | The criterion above (enumerate the backend's endpoints, expect two), plus item 4's mutation. Stated as the plan's central constraint rather than as a risk row, because it is the decision this plan is *for* |
| The backend becomes a second place that knows the protocol | Medium | It carries the protocol **verbatim** (framing + `Envelope`); it does not interpret it. A backend that parses `Envelope` to *decide* something is the first step to the row above |
| **Choosing a framework (Next or otherwise) and inheriting its shape** | Medium | §3.2's recommendation is a plain Node process, with the case stated. If a framework is chosen, the reason should be a stated preference rather than a technical need |
| A same-origin backend hides the *remote engine* case and it stops being tested | Medium | §5.2 keeps the direct-connect form, and the two `verify:web*` scripts run in both modes |
| Claiming "the GUI is not reachable" because the port is unpublished | Medium | §3.5 — the property is the **`127.0.0.1` bind inside the container**, and the criterion asserts it by request, not by reading `EXPOSE` |
| Breaking today's `network_mode: host` compose deployment | **High** (it will break) | §5.4 — released as a break, with the new file and `DOCKER.md` naming the old premise. Not papered over with a dual-mode file |
| **The deployment half is never run** — the recurring gap in `plan/15` items 1/2/4/9/10 | **High** | Items 5 and 7 are the first place it *has* to run, because the acceptance criteria are external requests. Until someone runs them, this plan's criteria are unmet and should be recorded that way rather than as ✅ |

---

## 9. What this plan does not do

- **It does not change the client protocol.** `hello`, `Envelope`, the command surface and `window.api` are
  unchanged; §5.3 says why it adds nothing.
- **It does not add a command-routing layer, a REST API or a schema mirror.** §3.2 and §4.
- **It does not move `packages/gui`.** One injected global is already the right seam.
- **It does not change Electron's transport.** It stays IPC and in-process; B's *"over pipe"* is a shape
  Electron does not need, and `transport/index.ts`'s table already records that `in-process` *"has no boundary
  at all"*. A socket there would add a boundary to remove later.
- **It does not build `plan/15` item 5 or item 7.** They are prerequisites, and they stay that plan's work.
- **It does not decide the framework.** §3.2 recommends; the owner chooses.
- **It does not touch `plan/15`'s port model.** The edge, the forward proxy and the RPC endpoint stay as
  `plan/15` §3.1 settled them; this plan adds a **fourth, internal, unpublished** listener.

---

## 10. Open questions — for the owner, not for me to assume

**Answered by the owner, 2026-09-23:** §3.5 records the model — **the GUI is a routing row on port 80/443 at
the engine's own domain**, mappings are subdomains, one certificate covers both, and the RPC port stays
separate on purpose (latency/isolation, not only exposure). **Q3 below is therefore closed.** Two new items
replaced it and they are not preferences — they are blockers:

- **TLS on 443.** The edge is `http.createServer` and its header says TLS is *"a later item"*. So 443 is not
  available today, and serving a public hostname needs an **operator-supplied** certificate — `mkcert`'s local
  CA cannot serve a public name. `--edge-tls-cert`/`--edge-tls-key`, `plan/15` WI 1's shape one layer out.
  **Renewal/rotation is a second, unsolved problem.**
- **A `target` policy for mappings.** The owner's *"user can expose [ws/webhook] using custom subdomain
  mapping"* is right, and it makes `target` a safety boundary: nothing today refuses a `target` that is
  loopback or the edge itself, so a mapping can point the edge at the engine's own ports — including a loop
  back into the edge. **`plan/15` §9's SSRF row is the general case; this is the concrete one.**

**Still open:**

1. **Framework:** plain Node backend (recommended, §3.2) or Next.js? If Next, is the reason a preference
   about tooling rather than a capability the relay needs?
2. **§5.2:** is the direct-connect form an escape hatch (recommended) or deleted?
3. ~~**§3.4:** the GUI at the engine's own domain or at a dedicated subdomain such as `console.`?~~
   **CLOSED 2026-09-23 — the engine's own domain (the owner's model, §3.5).**
4. **The GUI's auth story on a published port.** Today the engine's auth is a token over the RPC endpoint, and
   the GUI is served by the edge — a **product** surface. **Serving the admin GUI on a published port with no
   authentication is a decision this plan has not made and does not presume to.** It is worth deciding before
   item 0 rather than after, because it changes what "the GUI on the edge" means.
