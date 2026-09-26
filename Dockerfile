# syntax=docker/dockerfile:1
#
# A workspace host, headless. Phase 9 (`plan/10-phase-9-docker.md`), built on P14's host.
#
# ## Why this is two stages and the plan's Dockerfile was one
#
# `plan/10`'s Dockerfile carries a warning that it **cannot boot as written**: `packages/*/dist/**`
# keeps *bare* requires for its runtime dependencies — `bundle: false` is load-bearing, because
# `store/paths.ts` holds the data root as module-level state and bundling would inline a private copy
# into every entry point — and that image copied only `dist/` plus the root manifest. So the first
# `require` of `mkcert`, `simple-git`, `ws` or `@bifurc/protocol` throws `Cannot find module`.
#
# The build stage installs and builds from source; the runtime stage copies only what runs. That closes
# the gap without the two compromises the plan weighed (a partial-workspace lockfile, or a second tsup
# pass that would give up the singleton property `bundle: false` exists to protect).
#
# ## What the plan got right, and what it predates
#
# Right, and kept: `git` is a hard requirement (config versioning and sync depend on it), non-root, the
# `/data` volume holding both the config store and the blob staging directory, and **no extension bridge** —
# that was the original "companion server" on `9271`, and it is retired (`plan/15` §6.4). **This correction
# matters because the word was reused:** since `plan/17` §1 "the companion" means the **RPC-over-WebSocket
# endpoint**, which this image *does* run (`--ws-port 4747`) — so "no companion" and the `CMD` below are not
# in conflict, they are about two different things. The extension becomes an ordinary authenticated client on
# that same protocol.
#
# Predates the implemented CLI, and corrected here:
#
#   - the plan's entry point is `engine/runtime/host.js`, which is a **library** (`createWorkspaceHost`),
#     not an executable. Running it does nothing. The executable is `runtime/hostCli.js`.
#   - the plan's `BIFURC_RPC_HOST` / `BIFURC_RPC_PORT` / `BIFURC_RPC_TOKEN` / `BIFURC_RPC_TOKEN_FILE`
#     **do not exist**. The CLI reads `BIFURC_DATA_DIR`, `BIFURC_ENGINE_TOKEN` and
#     `BIFURC_PROXY_BASE_PORT`, and takes `--ws-host` / `--ws-port` / `--token-file` as flags.
#   - the plan's runtime set is "exactly four packages". It is seven, plus `zod` behind the protocol:
#     `@bifurc/protocol`, `archiver`, `js-yaml`, `mkcert`, `simple-git`, `unzipper`, `ws`.
#
# ## What `plan/17` §5 work item 5 added — the image's half of the GUI
#
# `plan/17` §1's rule is that the web GUI's port is **derived, never a setting**: it is the companion `+ 1`.
# The engine cannot own that derivation, because `plan/15`'s model is that the GUI is a **target**, not a
# directory (`EdgeRouterOptions.guiTarget` is a `host:port` string), so a document root inside the engine
# would put a second responsibility behind `--gui-target`. So this image gained three things and lost one:
#
#   - `scripts/docker-entrypoint.mjs` is now the `ENTRYPOINT`. It derives the GUI port **once**, binds a
#     dependency-free static server on it, publishes the resolved companion port to
#     `/tmp/bifurc/companion-port` for the healthcheck, and starts the CLI with `--gui-target`.
#   - `dist/web` — the browser host's bundle (`npm run build:web`, D18) — is copied into the runtime stage,
#     because the base domain serves the GUI and the entrypoint refuses to start without it.
#   - `scripts/docker-healthcheck.mjs` replaces the inline `node -e` probe, so the probe reads the port the
#     entrypoint **actually bound** instead of a number that happened to match the `CMD`.
#   - **the caveat is gone.** The healthcheck comment below used to say a `--ws-port` override had to be
#     paired with a `--health-cmd` override. It no longer does.
#
# The static server is ours rather than a dependency on purpose: the runtime stage is
# `npm prune --omit=dev`, so there is no `express`, no `serve-handler` and no `mime`, and adding one would
# mean shipping a static-file library as a *production* dependency of a package that does not serve files.
#
# ## Reachability — nothing in this image can be published, and that is structural now
#
# Every listener binds loopback, and as of **2026-09-25 that is not a default an operator can override**.
# `plan/05` work item 5's *"Never default to `0.0.0.0`"* is now the weaker half of the rule: the three
# factories refuse a non-loopback address unconditionally, from their own code, and no flag reaches them.
#
#   | listener | binds | refused how |
#   |---|---|---|
#   | the RPC endpoint (`--ws-port`) | `127.0.0.1` | `createWsServer` throws; `--ws-host` no longer exists |
#   | the forward proxy | `127.0.0.1` | `createProxyServer` throws; `--proxy-bind-host` no longer exists |
#   | the edge router | `127.0.0.1` | `createEdgeRouter` throws; `--edge-bind-host` no longer exists |
#   | the web GUI | `127.0.0.1` | `scripts/gui-static-server.mjs` has no other value |
#
# **What replaced the opt-ins is a check, not a credential.** The RPC endpoint used to require a per-start
# token, because `plan/05`'s threat was an unauthenticated *remote* surface. With no remote surface the token
# guarded nothing, so it is gone — `--token-file` and `BIFURC_ENGINE_TOKEN` with it, and the token file the
# host used to write into `/data`. What actually discriminates is the **origin guard on the WebSocket
# upgrade**: a browser is a local process, so a connection from a page the user *visited* arrives on loopback
# too, and the guard refuses any page whose `Origin` is not `localhost` (or a subdomain of it). That is a check
# a secret could never make, because a page cannot be trusted to keep one. See
# `packages/engine/src/lib/net.ts#isAllowedOrigin`.
#
# So the documented way in is still `--network=host` or an SSH tunnel (`plan/10` work item 4) — and now there
# is no other way in at all. **The GUI is not a separate case:** it binds loopback like everything else and is
# reached *through* the edge on the base domain, which is the whole reason `--gui-target` exists.
#
# **A `ports:` mapping cannot work, and neither can `docker run -p`.** Both publish a host port onto a
# container port nothing is listening on, because the listener is on the container's loopback rather than on
# its bridge interface. `docker-compose.yml`'s header records the same thing.

# ── build ────────────────────────────────────────────────────────────────────
FROM node:22-slim AS build

WORKDIR /app

# Manifests first, so a source edit does not re-run the install.
#
# **This list has to be exactly the workspace set `package-lock.json` describes, and `npm ci` is what
# enforces it.** A workspace directory whose manifest is present but which the lockfile does not know is a
# hard `EUSAGE: Missing: @bifurc/<name> from lock file` — the build stops before it compiles anything. All
# five are listed. `packages/gui` is among them even though `build:packages` never builds it: the browser
# bundle aliases `@bifurc/gui` to source, but the *install* still has to agree with the lockfile, and
# `packages/web-client` is what `build:web` resolves through `node_modules` at bundle time.
COPY package.json package-lock.json ./
COPY packages/protocol/package.json packages/protocol/
COPY packages/engine/package.json packages/engine/
COPY packages/client/package.json packages/client/
COPY packages/gui/package.json packages/gui/
COPY packages/web-client/package.json packages/web-client/

# `--ignore-scripts` because the root manifest's devDependencies include Electron, whose postinstall
# downloads a ~100 MB platform binary this image has no use for. It also skips the root `prepare`
# script, which is why `build:packages` is invoked explicitly below.
RUN npm ci --ignore-scripts

COPY packages ./packages

# The browser host's sources, plus the two CSS configs Vite resolves by **walking up** from `web/`.
# Tailwind's `content` globs are relative to the working directory, so both files have to sit at `/app` —
# and its `./renderer/**` glob is simply empty here, which is why the Electron entry is not copied.
#
# **Measured 2026-09-26, because the obvious guess is wrong.** Without them `build:web` still exits 0 **and
# still emits a CSS file** — but it is a third of the size and carries **no Tailwind utilities**: 20,134 bytes
# and zero `w-24` occurrences, against 60,042 bytes and one. So the failure is neither a build error nor a
# missing stylesheet; it is an app that renders **unstyled**, which is worse than both because nothing fails.
COPY web ./web
COPY postcss.config.js tailwind.config.js ./

# Then drop the dev tree. Without this the image carries TypeScript, tsup, vitest, Electron and React.
#
# `build:web` runs after `build:packages` and before the prune, and the order of the first two matters:
# the browser bundle reaches the engine through `node_modules/@bifurc/engine` → `dist/`, because this
# config aliases only `@` and `@bifurc/gui` (unlike Vitest, which aliases the engine to `src`). A
# `build:packages`-only image would have no `dist/web`, and the entrypoint treats that as a broken image
# rather than a degraded mode.
RUN npm run build:packages && npm run build:web && npm prune --omit=dev

# ── runtime ──────────────────────────────────────────────────────────────────
FROM node:22-slim AS runtime

# `git` is a hard requirement — `store/gitStore.ts` and `sync/*` version every config change through it,
# and the host's preflight fails closed without it. `ca-certificates` is for outbound TLS.
RUN apt-get update \
 && apt-get install -y --no-install-recommends git ca-certificates \
 && rm -rf /var/lib/apt/lists/*

# Non-root, and it can be: P3 moved CA installation to the client, so the engine never needs root.
RUN useradd --create-home --uid 10001 bifurc

WORKDIR /app

COPY --from=build /app/node_modules ./node_modules
COPY --from=build /app/package.json ./package.json
COPY --from=build /app/packages/protocol/package.json ./packages/protocol/package.json
COPY --from=build /app/packages/protocol/dist ./packages/protocol/dist
COPY --from=build /app/packages/engine/package.json ./packages/engine/package.json
COPY --from=build /app/packages/engine/dist ./packages/engine/dist

# The web GUI's bundle, and the three scripts that serve it. `dist/web` is what `--gui-target` names; the
# entrypoint refuses to start without `index.html` in it, because a base domain answering 502 is a failure
# with no explanation at the point the user sees it. Copied from the **context** rather than from `build`,
# since nothing in the build stage touches them.
#
# `scripts/gui-static-server.mjs` comes too: the entrypoint imports it, and it has no dependencies to prune.
COPY --from=build /app/dist/web ./dist/web
COPY scripts/docker-entrypoint.mjs scripts/docker-healthcheck.mjs scripts/gui-static-server.mjs ./scripts/

RUN mkdir -p /data && chown -R bifurc:bifurc /data
USER bifurc

# `BIFURC_DATA_DIR` means `--data-dir` is optional, so an operator can override just the flags they care
# about: `docker run … --workspace ws-1`.
#
# `BIFURC_PROXY_BASE_PORT` **sets the forward proxy's port outright** since `plan/17` §1. It is no longer a
# *scan base* — there is no scan, because a browser's proxy setting names this port and a silently moved one is
# a broken browser setting rather than a degraded mode — and it is no longer per-workspace. The name is kept
# because it is documented in `docker-compose.yml` and `DOCKER.md`, and because a container is exactly the case
# an operator wants to set it from the outside. The environment **wins** over the stored file
# (`loadSettings()` layers it on), so a `-e` beats whatever an earlier run left in the volume.
#
# Stated here rather than left to `process.platform`, which resolves to `80` on Windows and `8080` elsewhere.
ENV BIFURC_DATA_DIR=/data \
    BIFURC_PROXY_BASE_PORT=8080

VOLUME ["/data"]

# Documented for an operator reading `docker inspect`. **Corrected for `plan/17` §1**: the retired companion
# port `9271` is gone, and the four the host can hold are the edge (`80`), the forward proxy (`8080`), the
# companion (`4747`) and the web GUI (companion `+ 1`, so `4748`). None is publishable while they bind
# loopback — see the reachability note above.
EXPOSE 80 8080 4747 4748

# A **TCP connect**, not `GET /healthz`: there is no health endpoint yet (`plan/10` work item 6 asks for
# one), and a healthcheck that pretends otherwise would report a healthy container as unhealthy.
#
# **The probe moved off `9271`, which no longer exists**, and it tests the **companion** — the RPC endpoint
# the `CMD` starts — because that is the listener this image exists to expose and the one whose absence means
# "the host is not up". Probing the edge or the forward proxy would be wrong for the opposite reason: both are
# *reported* rather than thrown when they fail to bind (`plan/17` §1), so a container whose edge lost a port
# race would be marked unhealthy while the host was serving RPC perfectly well.
#
# **The coupling this comment used to record is retired.** It read: *"this number is the one the `CMD`
# passes. A `docker run` that overrides `--ws-port` must override the probe with it (`--health-cmd`), or the
# container will report unhealthy while working."* That was true of a hardcoded `4747`. The probe is now
# `scripts/docker-healthcheck.mjs`, which reads the port the **entrypoint actually resolved and bound** out of
# `/tmp/bifurc/companion-port` — so the probe and the listener agree by construction, whatever the port turns
# out to be. `--health-cmd` no longer has to be overridden, and neither does `--ws-port`.
HEALTHCHECK --interval=30s --timeout=3s --start-period=15s --retries=3 \
  CMD node scripts/docker-healthcheck.mjs

# The **entrypoint**, not the executable directly — `plan/17` §5 work item 5. See
# `scripts/docker-entrypoint.mjs` for why the derivation lives there rather than in the engine.
#
# **There is no `CMD` any more**, and each of the three reasons it once had is worth stating.
#
#   - `CMD ["--ws-host", "127.0.0.1"]` named the bind address. `--ws-host` was removed on 2026-09-25 with the
#     bind opt-ins, and it would now be a **fatal unrecognised flag** — the container would refuse to start.
#     Loopback is not something this image states any more; it is the only thing any listener will accept.
#   - it deliberately did **not** carry `--ws-port`. `BIFURC_COMPANION_PORT` is what an operator sets (default
#     `4747` = `DEFAULT_COMPANION_PORT`, `store/appSettings.ts`), and the entrypoint resolves flag →
#     environment → default and **pins `--ws-port` to its own answer**. A `--ws-port 4747` here would
#     **shadow the environment**, because a flag wins, so an operator's `-e BIFURC_COMPANION_PORT=5000` would
#     be silently ignored. With no `CMD` at all there is nothing left to shadow it.
#   - the entrypoint still supplies `--ws-port`, which is what puts the RPC endpoint in WebSocket mode at all —
#     without it the host would use a unix socket or named pipe and the healthcheck would have nothing to
#     connect to. **`--ws-port` is now the only flag that selects that mode**; it used to share the job with
#     `--ws-host`.
#
# **No `--domain`.** It was a required flag until 2026-09-24 and is now gone: the engine is no longer deployable
# in the cloud, so it always answers as `localhost` — which is what this image documents anyway (`--network=host`,
# or an SSH tunnel, so the engine really is on the machine the client is on and `*.localhost` mappings are
# meaningful). Removing it also removes the one way this image could have been made to claim a public name, where
# `*.localhost` would resolve to the *browser's* loopback rather than to this container. See `plan/15` §7 item 9
# and `packages/engine/src/runtime/hostArgs.ts`'s header.
#
# **Arguments after the image name go straight to the entrypoint**, which appends what it resolved — so
# `docker run bifurc:latest --workspace ws-1` and `… --gui-target 5753` both work, and neither needs a `CMD`
# to be replaced first.
ENTRYPOINT ["node", "scripts/docker-entrypoint.mjs"]
