# Running Bifurc in Docker

A container image that runs a **workspace host** headlessly — the same process `hostCli.js` starts on a
developer machine, with `git` in the base image and `/data` as the volume — and **serves the web GUI on the
base domain**. It starts no Electron and no extension bridge: those are app-level and stay in the desktop app.

This is phase 9 (`plan/10-phase-9-docker.md`), built on P14's host.

---

## The four ports, at a glance

Since `plan/17` §1 every port is **engine/host-level**: one of each for the whole host, whatever workspace is
selected. `plan/17` §5 work item 5 added the fourth, and it is the one that is not a setting:

| Port | What | How it is set |
|---|---|---|
| `80` | the **edge** — reverse proxy, home page, and the web GUI on the base domain | `--edge-port`, or `settings.port` in `app.json` |
| `8080` | the **forward proxy** | `BIFURC_PROXY_BASE_PORT` |
| `4747` | the **companion** — the RPC endpoint a client connects to | `BIFURC_COMPANION_PORT` |
| `4748` | the **web GUI** | **derived — companion `+ 1`**, never a setting |

The GUI's port is derived **once**, by `scripts/docker-entrypoint.mjs`, which then binds the static server on
it and tells the host `--gui-target 127.0.0.1:<companion + 1>`. It cannot be a setting: `plan/15`'s model is
that the GUI is a **target**, not a directory, so the engine never learns what is behind the flag — and the
derivation has to be owned by whoever starts both sides. See "The web GUI" below.

---

## Read this first: the container is reachable **only** through `--network=host`

**Every socket binds loopback, and since 2026-09-25 there is no opt-in that could change that.** Not "loopback
by default" — loopback as the only value the guards accept:

| Socket | Bind | Why |
|---|---|---|
| The forward proxy | `127.0.0.1` | it is a local development proxy; a wide bind would forward the internet into your machine |
| The RPC endpoint | `127.0.0.1` | `plan/05` work item 5: *"Never default to `0.0.0.0`"* |
| The edge router | `127.0.0.1` | it routes a **client-chosen `Host`** to a target you configured — `plan/15` §9's SSRF row |
| The web GUI | `127.0.0.1` | reached **through the edge** on the base domain, so it has no bind of its own to publish |

Each of the three listeners refuses a non-loopback address **from its own factory** — `createWsServer`,
`createProxyServer` and `createEdgeRouter` — and the CLI has no flag that could express one. So a `ports:`
mapping publishes a host port onto a container port nothing is listening on, and the connection is refused.

**`--network=host` is therefore the way in**, and it is what `plan/10` work item 4 recommends (*"reach it via
`docker exec`, a sidecar, or an SSH tunnel"*). Sharing the host's network namespace puts the container's
loopback on the host's loopback, so a browser on that machine reaches the GUI at `http://localhost/` and a
client reaches `ws://127.0.0.1:4747` directly:

```bash
docker run --rm --network=host \
  -v bifurc-data:/data \
  bifurc:latest
```

That is a real deployment — Bifurc is local-first, so "run it on the box I work on" is the normal case — and it
is also the **only** deployment this image supports. See "Why it is not publishable" below.

**Corrected 2026-09-25, and this section has now been wrong in both directions.** It first said the opt-ins did
not exist — *"the opt-in exists but no entry point exposes it, including `hostCli.js`"* — and was corrected on
2026-09-23 when `plan/15` work items 1-3 added all three (`--ws-allow-non-loopback` with its TLS pair,
`--proxy-allow-non-loopback`, `--edge-allow-non-loopback`), along with a promise that publishing was *"a matter
of flags rather than of code"*. **That promise is withdrawn**, and the flags are gone: the engine is not
deployable remotely, so there is no address to publish and no boundary for a token to guard. `--ws-host`, the
three `--*-allow-non-loopback` flags, the three `--*-bind-host` flags they went with, `--ws-tls-cert`,
`--ws-tls-key` and `--token-file` are all **fatal flags** now — each refused *by name, with the reason it
went*, rather than as an unrecognised typo, because a compose file written before that date is the expected
input. `packages/engine/src/runtime/hostArgs.ts`'s `REMOVED_FLAGS` is the table.

**The token is gone with them.** `BIFURC_ENGINE_TOKEN` no longer exists and the host no longer writes
`/data/engine.token`: the RPC surface binds loopback, so the only peers that can reach it are processes on this
machine, and a process on this machine can already run code here. What keeps a page the user merely *visited*
out is the **`Origin` guard on the WebSocket upgrade** — it refuses with a `403` that never upgrades unless the
`Origin` is loopback or a `*.localhost` name. A secret could not replace that guard, because a page cannot be
trusted to keep one; see `packages/engine/src/lib/net.ts#isAllowedOrigin`.

---

## Build

```bash
docker build -t bifurc:latest .
```

The build is two stages. The build stage installs from source, runs `build:packages` **and** `build:web`, then
drops the dev tree with `npm prune --omit=dev`; the runtime stage copies `node_modules` after the prune, plus
the three `dist` trees, the manifests and the three `scripts/` files the entrypoint and healthcheck need.

**`npm ci` is the first thing that can fail, and it did — found and fixed 2026-09-26.** The build stage's
manifest `COPY`s have to be **exactly the workspace set `package-lock.json` describes**, because `npm ci`
validates that set and refuses with `EUSAGE: Missing: @bifurc/<name> from lock file` when a manifest is present
for a package the lockfile does not know. The committed lockfile was stale in two ways, and **neither was
reachable before because the build had never been run**: it still described `apps/web` — the GUI package at its
pre-extraction path, `@bifurc/web`, with `node_modules/@bifurc/web` linked to a directory that no longer exists
— and it did not describe `packages/gui` (`@bifurc/gui`) at all. So the image could not build under *either*
manifest set: with `packages/gui`'s manifest copied it failed the sync check, and without it npm followed the
dead `apps/web` link and tried to fetch `@bifurc/gui@*` from the public registry (404 — a private name).

The repair is `npm install` once, which reconciles the workspace set: **one key removed, two added, and zero
version changes** (`node_modules/@bifurc/web` out; `packages/gui` and `node_modules/@bifurc/gui` in; root
`workspaces` `["packages/*","apps/*"]` → `["packages/*"]`). That also un-breaks `npm ci` for a fresh clone and
for CI, which is worth knowing independently of this image. `Dockerfile` now copies **five** manifests.

`build:web` is the browser host's bundle (D18), and it needs three things this image copies explicitly:
`web/` itself, and `postcss.config.js` + `tailwind.config.js` at the working directory — Vite finds PostCSS
config by **walking up** from `web/`, and Tailwind's `content` globs are relative to the working directory.
**Measured 2026-09-26: without those two files the build still exits 0 and still emits a CSS file — but it
carries no Tailwind utilities** (20,134 bytes and zero `w-24` occurrences, against 60,042 bytes and one), so the
app renders **unstyled** with nothing failing. That is worse than a build error. The order matters too —
`build:web` runs *after* `build:packages`, because the browser bundle reaches the engine through
`node_modules/@bifurc/engine` → `dist/` (unlike Vitest, this Vite config aliases only `@` and `@bifurc/gui`).

`plan/10`'s own Dockerfile carries a warning that it **cannot boot**: `packages/*/dist/**` keeps *bare*
requires for its runtime dependencies — `bundle: false` is load-bearing, because `store/paths.ts` holds the
data root as module-level state and bundling would inline a private copy into every entry point — and it
copied only `dist/` plus the root manifest. So the first `require` of `mkcert`, `simple-git`, `ws` or
`@bifurc/protocol` throws `Cannot find module`. The two-stage build closes that without either compromise
the plan weighed.

---

## Run

```bash
docker run -d --name bifurc \
  --network=host \
  -v bifurc-data:/data \
  bifurc:latest
```

Then open **`http://localhost/`** — the edge serves the web GUI on the base domain — or connect the CLI to
`ws://127.0.0.1:4747`. There is nothing to configure to make that work: no token to choose, no bind host to
name, and no address for the engine to answer as.

`docker-compose.yml` in this repository is the same thing in compose form.

### Environment

| Variable | Default | Notes |
|---|---|---|
| `BIFURC_DATA_DIR` | `/data` | Set in the image, so `--data-dir` is optional |
| `BIFURC_PROXY_BASE_PORT` | `8080` | **The forward proxy's port, set outright** (`plan/17` §1). It is no longer a scan *base* and no longer per-workspace: there is one host-level forward proxy, a browser's proxy setting names its port, and a busy one is a **reported bind failure rather than a silent increment**. Stated rather than left to `process.platform`. The name is kept because it is documented here and in `docker-compose.yml`; the environment wins over the stored file. |
| `BIFURC_COMPANION_PORT` | `4747` | **The companion's port** (`DEFAULT_COMPANION_PORT`). The web GUI follows it to `+ 1` automatically, and the healthcheck follows it too — so this is the only variable that moves the RPC endpoint, and the one that used to require a matching `--health-cmd` override. |
| `BIFURC_GUI_TARGET` | — | The web GUI's address. The entrypoint sets `--gui-target` for you, so this is only for the bypass case below. |

**`BIFURC_ENGINE_TOKEN` is gone** (2026-09-25). It used to be documented here as *"set this for a long-lived
host"*, because a generated token rotated on every start and invalidated every client. Nothing rotates now,
because there is nothing to rotate.

### Flags

**The image has no `CMD` at all.** It used to be `--ws-host 127.0.0.1`, which named the bind address — a flag
that no longer exists. Three reasons, and each is worth stating because they are the three ways a `CMD` can be
wrong:

- `--ws-host` named a bind address, and there is no address left to name. Its removal is why `--ws-port` is now
  **the only flag that selects WebSocket mode**; it used to share that job.
- it deliberately did **not** carry `--ws-port`, because `BIFURC_COMPANION_PORT` is what an operator sets.
- the entrypoint supplies both `--ws-port` (resolved flag → environment → default) and `--gui-target`
  (`127.0.0.1:<companion + 1>`), so anything in a `CMD` would only be a second, contradictory answer.

**Arguments after the image name *replace* the `CMD`; they do not append to it** — and with no `CMD` there is
nothing to replace, so an argument list is now simply the host's whole command line. The entrypoint reads
`--ws-port` and `--gui-target` out of it if you supply them, and pins them only when you do not:

```bash
docker run --rm --network=host bifurc:latest                                  # as documented: companion 4747, GUI 4748
docker run --rm --network=host -e BIFURC_COMPANION_PORT=5000 bifurc:latest    # companion 5000, GUI 5001, healthcheck follows
docker run --rm --network=host bifurc:latest --workspace ws-1
```

> **`--ws-port 0` is not a usable example, and that is a deliberate rule rather than a missing feature.**
> `plan/17` §1: the companion is one of the two ports a **client has to be told**, so `0` — "let the OS pick" —
> is the one value it must never take. Omitting `--ws-port` entirely is different and is supported: the host then
> resolves the companion/GUI **pair** by scanning from `settings.companionPort`, requiring both `n` and `n + 1` to
> be free, and reports both ports. **The entrypoint pins it instead**, so that the static server, the
> `--gui-target` and the healthcheck are written against a number that is *stated* rather than discovered —
> which is the whole reason the healthcheck no longer needs a matching override.

**Ten flags are gone, and passing one is a startup error that names it.** Nine went on **2026-09-25** with the
bind opt-ins and the token (`--ws-host`, `--ws-allow-non-loopback`, `--ws-tls-cert`, `--ws-tls-key`,
`--token-file`, `--proxy-bind-host`, `--proxy-allow-non-loopback`, `--edge-bind-host`,
`--edge-allow-non-loopback`), and `--domain` a day earlier. They are refused **by name and with the reason**,
not as unrecognised typos, because the operator most likely to pass one copied it from this page — and the
message has to be the thing that corrects them. `packages/engine/src/runtime/hostArgs.ts#REMOVED_FLAGS` holds
the table; `hostArgs.test.ts` asserts each one's own reason.

`--domain` deserves its own sentence, because it was **required** until 2026-09-24 — the hostname the engine
answered as. It went with the same decision: the engine runs inside the Electron app or as a locally deployed
image, so it always answers as `localhost`. That is the only name this page ever documented, and removing the
flag also removed the one way a container could have been made to claim a public name, where `*.localhost`
mappings resolve to the *visitor's* own machine rather than to this container.

---

## The web GUI

`plan/17` §5 work item 5. Three files, and the split between them is the design:

- **`scripts/docker-entrypoint.mjs`** — the `ENTRYPOINT`. It resolves the companion port, derives the GUI's as
  `companion + 1`, starts the static server, publishes the resolved port to `/tmp/bifurc/companion-port`, and
  spawns the CLI with `--gui-target 127.0.0.1:<gui>`. It is also PID 1, so it forwards `SIGTERM` and exits with
  the child's code.
- **`scripts/gui-static-server.mjs`** — the static server itself. **No dependencies on purpose**: the runtime
  stage is `npm prune --omit=dev`, so there is no `express`, no `serve-handler` and no `mime`, and adding one
  would mean shipping a static-file library as a *production* dependency of a package that does not serve files.
- **`scripts/docker-healthcheck.mjs`** — the probe, which reads the port the entrypoint actually bound.

Two behaviours worth knowing, because both are deliberate and both are pinned by tests:

- **A document request for a path the bundle does not contain gets `index.html`** — that is the client router's
  route, not a 404. The test is the request's `Accept` header naming `text/html`, **not** the path's shape.
- **A missing asset is a real 404.** A browser requests a `<script src>` with a wildcard `Accept`, so a looser
  test would answer a missing script with HTML and the failure would surface as a confusing parse error instead
  of a 404. Measured live: `/deep/route` with a document `Accept` → `200 text/html`; `/assets/nope.js` with a
  wildcard one → `404 text/plain`.

The GUI bundle is **part of what this image promises**, so the entrypoint refuses to start if `dist/web/index.html`
is missing rather than serving nothing: a base domain that answers 502 is a failure with no explanation at the
point the user sees it. An operator who wants the host alone can bypass the entrypoint
(`--entrypoint node packages/engine/dist/runtime/hostCli.js`).

---

## The volume

```
/data/
  app.json                 # app settings
  data/                    # workspaces, entities, git repos
  blobs/                   # blob staging — must be on the volume
```

`engine.token` used to be listed here. The host does not write it any more (2026-09-25), so a volume carried
over from an older image still has the file and nothing reads it. Deleting it is safe; leaving it is harmless.

**The blob directory is the subtle one.** Outside the volume, an in-flight export is lost when the container
restarts and the client gets a confusing error about a missing blob. `plan/10` work item 3 flags this as the
highest-likelihood risk, and it is asserted in the engine's own tests.

---

## One container, many workspaces — the recommendation is now inverted

`plan/14` §4 (WI 6) recommended **one container per workspace**, on the grounds that *"the ports, the volume and
the lifecycle line up only in the one-per-container case"*. **`plan/17` §1 removes that reason and reverses the
recommendation**: a workspace is a **project**, every port is engine/host-level, and switching workspace changes
the *context* — which rules, mocks and webhook endpoints are in force — while the ports stay put. So one
container serves every workspace on one set of ports, and a browser's proxy setting is a per-machine choice that
does **not** have to change when the selection does. That is the whole point of the standardization.

`--workspace` remains repeatable, and it is still the way to *limit* what a host serves (a requested id that is
not in the settings is named rather than dropped). One container per workspace is still legitimate if you want
per-project volumes and lifecycles — it is a choice now, not a constraint.

**One volume per container, holding that workspace set's `data/` and `blobs/`**, is unchanged.

---

## Signals, health and logs

- **`SIGTERM` is handled.** The **entrypoint is PID 1** and forwards the signal to the host, waits for its exit,
  closes the static server and exits with the child's code — so `docker stop` reports the status the host chose.
  The host itself (`runWorkspaceHost()`) stops its listeners and exits `128 + signo`; `hostCli.js` deliberately
  installs no handler of its own, because a second one would race it and the exit code is the orchestrator's only
  signal about *why* the process stopped.
- **The healthcheck is a TCP connect**, not `GET /healthz`. There is no health endpoint yet — `plan/10` work
  item 6 asks for one — and a healthcheck that pretended otherwise would report a healthy container as
  unhealthy. **It probes the companion**, because that is the listener this image exists to expose; the edge and
  the forward proxy are *reported* rather than thrown when they fail to bind (`plan/17` §1), so probing either
  would mark a working host unhealthy over a lost port race.
  **The old coupling is retired:** this section used to warn that *"a `--ws-port` override must be paired with a
  `--health-cmd` override"*. The probe is now `scripts/docker-healthcheck.mjs`, which reads the port the
  entrypoint **resolved and bound** from `/tmp/bifurc/companion-port` — so the probe and the listener agree by
  construction, whatever the port turns out to be, and neither `--ws-port` nor `--health-cmd` needs overriding.
- **Logs go to stdout**, one line per event, prefixed `[entrypoint]`, `[host]`, `[edge:<port>]`,
  `[server:<port>]` or `[preflight]`. The entrypoint's line comes first (`web GUI: … (derived, companion N + 1)`),
  then the edge's, then the forward proxy's, then the RPC endpoint's — that is the order `host.ts` emits them in
  (`edge: …` at step 4, `forward proxy: …` just below it, `rpc endpoint: …` after the transport is up). Since
  `plan/17` §1 there is **one** of each for the whole host, so this is three port lines rather than N — the host
  no longer prints a line per workspace, and `status().workspaces` carries the per-workspace half. There is no
  `engine token: …` line any more. Not structured JSON yet; that is also work item 6.

---

## Why it is not publishable

**It is not, and the answer is now "nothing would make it so" rather than a list of flags.** This section used
to be titled "What would make it publishable" and listed three changes; all three were implemented on
2026-09-23 and all three were **deleted on 2026-09-25**, so the honest record is the reversal rather than the
list.

The three were the bind opt-ins — one per listener, each with its own flag and its own coherence rule, plus a
TLS pair for the RPC endpoint and a fail-closed token requirement behind it. They were real: `plan/10` work
item 5's *"if neither is set and the RPC port is bound to anything other than loopback → refuse to start with a
clear message. Fail closed, never open."* was implemented, and the guards refused a half-configured bind rather
than accepting and ignoring it. What removed them was a **product decision, not a simplification**: the engine
is never deployed remotely. It runs inside the Electron app or as a container on the machine the user is sitting
at, so there is no address to publish and no boundary for a credential to guard. Leaving the flags would have
left the capability, and a capability nothing exercises is a capability that rots.

So the honest statement of the current position is:

- **A published RPC port is a remote code execution surface.** The engine generates TLS CAs, executes scripts
  and spawns child processes. Any future decision to expose it starts from that sentence, not from a flag.
- **The security argument for loopback is stronger than the token was.** A loopback bind plus an `Origin`
  allowlist means the peer is a process on this machine, and a process on this machine can already run code
  here. A secret in a browser page could not have delivered that, because a page cannot be trusted to keep one.
- **What a re-introduction would have to bring back is the whole chain**, not one flag: the bind guards'
  opt-in half, the CLI flags, the `REMOVED_FLAGS` entries, and a credential or an equivalent origin story. That
  is recorded here so the next person does not read the deletion as "this was easy and nobody bothered".

**One consequence worth keeping from the old section, because it is a port-number fact rather than a publishing
one.** `settings.port` is the **edge's** port, and the forward proxy's is `BIFURC_PROXY_BASE_PORT` — which since
`plan/17` §1 is **not a base it scans from**: the proxy binds `8080` and, if that is taken, **reports the
failure** rather than taking `8081`. So "the forward proxy moves up one" is no longer a thing that can happen.
In this image `BIFURC_PROXY_BASE_PORT` is `8080` and `settings.port` defaults to `80`, so the two listeners
never contend. **Mappings need no edit** — that half of `plan/15` §6.2 holds exactly as written — but a browser
proxy setting that named the old *per-workspace* proxy port must be re-pointed at the port the app reports,
which is now a single host-level number.

---

## Verified, and not

**Verified without Docker**, by assembling the runtime stage's exact layout — the seven runtime packages,
the two `dist` trees, the manifests — and running the CLI from it:

```
[host] rpc endpoint: ws://127.0.0.1:8099
[host] engine token: …\data\engine.token
hello      -> ok, engineVersion=0.0.0 sessionId=697b29b8-…
config.get -> ok, proxyPort=80 workspaces=1
```

That is the specific failure `plan/10` predicted for its own Dockerfile, tested directly: the host runs and
**serves** with no dev dependencies, no Electron and no repository around it. The transcript is reproduced as
it was taken; the `engine token:` line is one the host no longer prints, and the run has not been repeated
since the token was removed — the two commands are unaffected either way, since neither of them carried one.

**Verified 2026-09-25, without Docker** — the work item 5 half, end-to-end, by running the entrypoint itself
(`npm run verify:docker-entrypoint`; it binds `80`, `8080`, `4747` and `4748`, so it is owner-run rather than a
suite):

```
[entrypoint] web GUI: http://127.0.0.1:4748 (derived, companion 4747 + 1)
[host] companion: 4747 — the web GUI belongs on 4748 (derived, companion + 1)
healthcheck                -> exit 0, "companion 4747 accepted a connection"
GET /            (document) -> 200 text/html
GET /deep/route  (document) -> 200 text/html     <- the SPA fallback, live
GET /assets/nope.js  (*/*)  -> 404 text/plain    <- a missing asset is not answered with HTML
Host: localhost             -> 200 text/html     <- the edge routes the base domain to --gui-target
```

`tests/docker/entrypoint.test.ts` (11 cases) covers the port precedence and the rejections;
`tests/integration/guiStaticServer.integration.test.ts` (14 cases) covers the static server over real sockets
and real files, on an **ephemeral** port so it cannot degrade any other suite's port-scan assertions.

**Not yet verified:** the `docker build` itself and a real container run. Docker Desktop was not running when
this was written, and starting it is a desktop action. What *is* verified offline, 2026-09-26, is the step that
would have failed **first**: `npm ci --ignore-scripts` against the image's exact manifest set, in the build
stage's order, with the reconciled lockfile — `added 759 packages`, exit 0 (see the Build section for what was
wrong). So the remaining risk is in the `docker build` machinery and the two `RUN` steps, not in the install.

**Not verifiable on Windows, and not a defect:** the entrypoint's `SIGTERM` forwarding. Measured directly — a
Node child on Windows cannot catch `SIGTERM` at all (`process.kill(pid, "SIGTERM")` terminates it
unconditionally: the handler never ran, and the parent saw `code=null signal=SIGTERM`). So that code path is
exercised by `docker stop` in the Linux container, not here, and this page does not claim to have measured it.
