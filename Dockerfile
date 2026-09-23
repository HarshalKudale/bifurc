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
# `/data` volume holding both the config store and the blob staging directory, and no companion server —
# that is app-level and stays in the desktop app.
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
# ## Reachability — read this before publishing a port
#
# **Nothing this container runs is reachable through `-p`.** Both listening sockets bind loopback:
#
#   - the proxy hardcodes `server.listen(port, "127.0.0.1")`, and there is no setting for it;
#   - the RPC guard refuses a non-loopback bind, and `ws.ts` records that *"nothing in the programme sets
#     `allowNonLoopback`"* — the opt-in exists but no entry point exposes it.
#
# That is deliberate (`plan/05` work item 5: *"Never default to `0.0.0.0`"*), not an oversight, and this
# image does not weaken it. Reach the host with `--network=host` on the machine it runs on, or a sidecar
# or an SSH tunnel — which is what `plan/10` work item 4 recommends. See `DOCKER.md`.

# ── build ────────────────────────────────────────────────────────────────────
FROM node:22-slim AS build

WORKDIR /app

# Manifests first, so a source edit does not re-run the install.
COPY package.json package-lock.json ./
COPY packages/protocol/package.json packages/protocol/
COPY packages/engine/package.json packages/engine/
COPY packages/client/package.json packages/client/
COPY packages/gui/package.json packages/gui/

# `--ignore-scripts` because the root manifest's devDependencies include Electron, whose postinstall
# downloads a ~100 MB platform binary this image has no use for. It also skips the root `prepare`
# script, which is why `build:packages` is invoked explicitly below.
RUN npm ci --ignore-scripts

COPY packages ./packages

# Then drop the dev tree. Without this the image carries TypeScript, tsup, vitest, Electron and React.
RUN npm run build:packages && npm prune --omit=dev

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

RUN mkdir -p /data && chown -R bifurc:bifurc /data
USER bifurc

# `BIFURC_DATA_DIR` means `--data-dir` is optional, so an operator can override just the flags they care
# about: `docker run … --ws-port 0`.
#
# `BIFURC_PROXY_BASE_PORT=8080` states the Linux base rather than leaving it to `process.platform`, and
# it sets the *base* — each further workspace still counts up from it (D16).
ENV BIFURC_DATA_DIR=/data \
    BIFURC_PROXY_BASE_PORT=8080

VOLUME ["/data"]

# Documented for an operator reading `docker inspect`. Neither is publishable while both sockets bind
# loopback — see the reachability note above.
EXPOSE 8080 9271

# A **TCP connect**, not `GET /healthz`: there is no health endpoint yet (`plan/10` work item 6 asks for
# one), and a healthcheck that pretends otherwise would report a healthy container as unhealthy.
HEALTHCHECK --interval=30s --timeout=3s --start-period=15s --retries=3 \
  CMD node -e "const s=require('net').connect(9271,'127.0.0.1');s.on('connect',()=>{s.end();process.exit(0)});s.on('error',()=>process.exit(1))"

# The **executable**, not `host.js`. Loopback is stated rather than assumed, because the guard's default
# and the guard's refusal should not be the same value by accident.
ENTRYPOINT ["node", "packages/engine/dist/runtime/hostCli.js"]
CMD ["--ws-host", "127.0.0.1", "--ws-port", "9271"]
