# Running Bifurc in Docker

A container image that runs a **workspace host** headlessly — the same process `hostCli.js` starts on a
developer machine, with `git` in the base image and `/data` as the volume. It starts no companion server and
no Electron: those are app-level and stay in the desktop app.

This is phase 9 (`plan/10-phase-9-docker.md`), built on P14's host.

---

## Read this first: the container is not reachable through `-p`

**Both listening sockets bind loopback, and that is deliberate.**

| Socket | Bind | Why |
|---|---|---|
| The proxy | `127.0.0.1`, hardcoded in `proxy/server.ts` | it is a local development proxy; binding wider would forward the internet into your machine |
| The RPC endpoint | loopback unless `allowNonLoopback` is passed | `plan/05` work item 5: *"Never default to `0.0.0.0`"* |

The RPC guard refuses a non-loopback bind, and `transport/ws.ts` records that *"nothing in the programme sets
`allowNonLoopback`"* — the opt-in exists but **no entry point exposes it**, including `hostCli.js`. So
`docker run -p 8080:8080` maps a host port to a container port nothing is listening on, and the connection
is refused.

**The supported way to reach it is `--network=host`**, which is what `plan/10` work item 4 recommends
(*"reach it via `docker exec`, a sidecar, or an SSH tunnel"*). With it, the container's loopback *is* the
host's loopback, so a browser or the CLI on that machine reaches `ws://127.0.0.1:9271` directly:

```bash
docker run --rm --network=host \
  -v bifurc-data:/data \
  -e BIFURC_ENGINE_TOKEN="$(openssl rand -base64 32)" \
  bifurc:latest
```

That is a real deployment — Bifurc is local-first, so "run it on the box I work on" is the normal case — but
it is not "host it on a server and connect from my laptop". **That needs a code change**, not a Dockerfile
change; see "What would make it publishable" below.

---

## Build

```bash
docker build -t bifurc:latest .
```

The build is two stages. The build stage installs from source and runs `build:packages`; the runtime stage
copies `node_modules` after `npm prune --omit=dev`, plus the two `dist` trees and the manifests.

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
  -e BIFURC_ENGINE_TOKEN="$(openssl rand -base64 32)" \
  bifurc:latest
```

Then read the token the host printed, and connect the web UI or the CLI to `ws://127.0.0.1:9271`.

`docker-compose.yml` in this repository is the same thing in compose form.

### Environment

| Variable | Default | Notes |
|---|---|---|
| `BIFURC_DATA_DIR` | `/data` | Set in the image, so `--data-dir` is optional |
| `BIFURC_ENGINE_TOKEN` | — | **Set this for a long-lived host.** Without it a token is generated and written to `/data/engine.token` on every start — so every restart invalidates every client. `--token-file` is the alternative, and the right one for an orchestrator. |
| `BIFURC_PROXY_BASE_PORT` | `8080` | The *base*; each further workspace counts up from it (D16). Stated rather than left to `process.platform` |

### Flags

The image's `CMD` is `--ws-host 127.0.0.1 --ws-port 9271`. Override it by appending, since
`BIFURC_DATA_DIR` already supplies the data root:

```bash
docker run --rm --network=host bifurc:latest --ws-port 0        # ephemeral RPC port
docker run --rm --network=host bifurc:latest --workspace ws-1  # serve only one workspace
```

---

## The volume

```
/data/
  app.json                 # app settings
  data/                    # workspaces, entities, git repos
  blobs/                   # blob staging — must be on the volume
  engine.token             # the generated token, when BIFURC_ENGINE_TOKEN is unset
```

**The blob directory is the subtle one.** Outside the volume, an in-flight export is lost when the container
restarts and the client gets a confusing error about a missing blob. `plan/10` work item 3 flags this as the
highest-likelihood risk, and it is asserted in the engine's own tests.

---

## One container, one workspace

`plan/14` §4 (WI 6) recommends one container per workspace, with one volume holding that workspace's
`data/` and `blobs/`. N workspaces per host is supported — `--workspace` is repeatable — but the ports, the
volume and the lifecycle line up only in the one-per-container case.

---

## Signals, health and logs

- **`SIGTERM` is handled**, by `runWorkspaceHost()`, which exits `128 + signo`. `hostCli.js` deliberately
  installs no handler of its own, because a second one would race it and the exit code is the orchestrator's
  only signal about *why* the process stopped.
- **The healthcheck is a TCP connect**, not `GET /healthz`. There is no health endpoint yet — `plan/10`
  work item 6 asks for one — and a healthcheck that pretended otherwise would report a healthy container as
  unhealthy.
- **Logs go to stdout**, one line per event, prefixed `[host]`, `[server:<port>]` or `[preflight]`. Not
  structured JSON yet; that is also work item 6.

---

## What would make it publishable

Two changes, and both are product decisions rather than wiring, because a published RPC port is a remote
code execution surface — the engine generates TLS CAs, executes scripts and spawns child processes:

1. **A `--ws-allow-non-loopback` flag.** `WsServerOptions.allowNonLoopback` already exists and the guard
   already enforces the pairing with TLS; nothing exposes it. `hostArgs.ts` + `hostCli.ts` + a field on
   `HostTransport`, and the guard's own docblock calls this out.
2. **A bind host for the proxy.** `server.listen(port, "127.0.0.1")` is a literal, with no setting in
   `app.json`. Publishing the proxy needs it configurable, and the wider bind is a bigger exposure than the
   RPC one: a proxy that forwards to your internal network, reachable from anywhere that can reach the port.

`plan/10` work item 5 is explicit about the second half: *"If neither is set and the RPC port is bound to
anything other than loopback → refuse to start with a clear message. Fail closed, never open."* The guard
already does that for the RPC channel; the token requirement for a published port is the piece that would
need building.

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
**serves** with no dev dependencies, no Electron and no repository around it.

**Not yet verified:** the `docker build` itself and a real container run. Docker Desktop was not running
when this was written, and starting it is a desktop action. Until that is done, treat the Dockerfile as
reviewed rather than exercised.
