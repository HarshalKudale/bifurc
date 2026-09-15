# 10 — Phase 9: Docker

**Goal:** a container image that runs the engine headlessly, survives restarts, authenticates clients,
and can be attached to from the web UI or CLI.

**Effort:** 1–2 weeks. **Depends on:** P2 (headless engine), P3 (blob store on the volume), P4 (transport
+ auth). **Parallel with:** P7, P8.

---

## Preconditions

- Engine starts from a bare Node/Bun process with `--data-dir` (P2 gate).
- Blob store lives under `dataDir()` (P3 gate).
- Auth implemented and verified (P4 gate).

> **Do not build this before P4.** A container exposes the engine's mock servers, TLS CA generation,
> script execution and child-process spawning. An unauthenticated image is a remote code execution
> vulnerability with a published port.

---

## Work item 1 — Headless startup path

`src/main.ts:255` currently hard-requires git and reports failure through an Electron dialog:

```ts
const hasGit = await checkGitInstalled();
if (!hasGit) {
  dialog.showErrorBox("Git required", "...");   // ← no dialog in a container
  app.quit();
  return;
}
```

Replaced in P2 item 6 with `preflight(): Promise<StartupCheck[]>`. The container entrypoint:

```ts
const checks = await preflight();
const fatal = checks.filter((c) => !c.ok);
if (fatal.length) {
  for (const c of fatal) console.error(`[fatal] ${c.code}: ${c.message}`, c.hint ?? "");
  process.exit(1);                      // non-zero, message on stderr
}
```

**Git is a hard requirement** — config versioning and sync depend on it (`store/gitStore.ts`). It must be
in the base image, not assumed.

---

## Work item 2 — Dockerfile

```dockerfile
FROM node:22-slim

# git is required by the engine; ca-certificates for outbound TLS
RUN apt-get update \
 && apt-get install -y --no-install-recommends git ca-certificates \
 && rm -rf /var/lib/apt/lists/*

# non-root
RUN useradd -m -u 10001 bifurc

WORKDIR /app
COPY packages/engine/dist ./engine
COPY package.json ./

RUN mkdir -p /data && chown -R bifurc:bifurc /data
USER bifurc

ENV BIFURC_DATA_DIR=/data \
    BIFURC_RPC_HOST=0.0.0.0 \
    BIFURC_RPC_PORT=9271

VOLUME ["/data"]
EXPOSE 8080 9271

HEALTHCHECK --interval=30s --timeout=3s --start-period=10s \
  CMD node -e "fetch('http://127.0.0.1:9271/healthz').then(r=>process.exit(r.ok?0:1)).catch(()=>process.exit(1))"

ENTRYPOINT ["node", "engine/index.js"]
```

Notes:
- **`mkcert` is pure JS** (`node-forge`) — no extra binary needed for cert generation. Verified.
- **Non-root by default.** The engine no longer installs CAs (that moved to the client in P3), so it does
  not need root. That is a direct benefit of the P3 decision.
- **`/data` is the volume.** Both the config store and the blob staging directory live under it.

---

## Work item 3 — Volume layout and persistence

```
/data/
  app.json                 # app settings
  data/                    # workspaces, entities, git repos
  blobs/                   # P3 staging — MUST be on the volume
  engine.token             # auth token (0600)
```

**The blob directory is the subtle one.** If it is outside the volume, an in-flight export is lost when
the container restarts, and the client gets a confusing error about a missing blob. Assert in a test that
the blob root is under `BIFURC_DATA_DIR`.

Document the volume in the image README:

```bash
docker run -d --name bifurc \
  -p 8080:8080 \
  -p 9271:9271 \
  -v bifurc-data:/data \
  -e BIFURC_RPC_TOKEN="$(openssl rand -base64 32)" \
  bifurc:latest
```

---

## Work item 4 — Networking and exposure

| Port | Purpose | Default exposure |
|---|---|---|
| `8080` | The proxy itself — the reason the tool exists | **Publish this** |
| `9271` | RPC control channel | **Do not publish by default** |

The RPC port is an admin surface. Publishing it exposes script execution and process spawning.

Recommendation: bind the RPC channel to a **unix socket inside the container** by default, and reach it
via `docker exec`, a sidecar, or an SSH tunnel. If a TCP RPC port must be published, require an explicit
opt-in and document that TLS + token are mandatory.

**The proxy port has its own consideration:** users proxying browser traffic to a container need the CA
trusted by *their* browser, and they need to point their browser at the container. Both are client-side
concerns (P3) — document the workflow end to end, because it is not obvious.

---

## Work item 5 — Authentication

Non-negotiable for a published image.

```
BIFURC_RPC_TOKEN         # or read from a mounted secret file
BIFURC_RPC_TOKEN_FILE    # preferred for orchestrators
```

- If neither is set and the RPC port is bound to anything other than loopback → **refuse to start** with
  a clear message. Fail closed, never open.
- Log the token fingerprint at startup, never the token.
- Support token rotation without a restart (a new `hello` with the new token succeeds; the old one stops
  working).

---

## Work item 6 — Health and observability

- `GET /healthz` on the RPC port — unauthenticated, returns `{ status, version, uptime }` only. No
  config, no state, no version of anything sensitive.
- Structured JSON logs to stdout (containers collect stdout; do not write log files).
- Respect `SIGTERM`: graceful shutdown, stop the proxy, close RPC connections, flush the audit log.
- Add `docker compose` example with a named volume.

---

## Work item 7 — Architecture

Per the decision made in P0/`00-decisions.md`:

| Option | Cost |
|---|---|
| `linux/amd64` only | Simplest; works for most CI and cloud |
| `linux/amd64` + `linux/arm64` | Needed for Apple Silicon dev and Graviton |

Use `docker buildx` with a multi-arch manifest if you go multi-arch. Note that **`git` in the base image
is the only arch-sensitive dependency** — everything else is JS.

---

## How to start — the first three things

1. **Run the engine headlessly on the host first** (`node packages/engine/dist/index.js --data-dir /tmp/x`).
   Confirm it starts, serves, and shuts down on `SIGTERM`. If it does not, the problem is in P2, not the
   Dockerfile.
2. **Write the Dockerfile with git + the volume, and nothing else.** No healthcheck, no compose, no
   multi-arch. Get `docker run` working with a CLI client attached.
3. **Then add auth and make it fail closed.** Verify the container refuses to start with a
   non-loopback RPC bind and no token.

---

## Acceptance criteria

- [ ] Image builds and runs; engine starts headlessly with no Electron.
- [ ] `SIGTERM` shuts down gracefully; `SIGKILL` leaves no corrupt state.
- [ ] `/data` volume persists config, workspaces **and blobs** across `docker restart`.
- [ ] Blob staging verified to be under the volume.
- [ ] Missing git → clear non-zero exit with a message, not a crash.
- [ ] RPC port bound to loopback or a socket by default; not published.
- [ ] Container **refuses to start** with a non-loopback RPC bind and no token.
- [ ] Auth works from the CLI; wrong token is rejected.
- [ ] `/healthz` responds and leaks nothing sensitive.
- [ ] Logs are structured JSON on stdout.
- [ ] Runs as non-root.
- [ ] `docker compose` example in the README, with the CA workflow documented end to end.
- [ ] Architecture(s) built and published per the P0 decision.

---

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| Shipping before auth is complete | Medium | Hard gate; the acceptance criterion is explicit |
| Blob store placed outside the volume | **High** | Assert the blob root is under `BIFURC_DATA_DIR` in a test |
| Users publish the RPC port and expose an RCE surface | **High** | Loopback/socket by default; refuse to start unauthenticated on a public bind; document loudly |
| Git missing from the base image | Low | It is in the Dockerfile; the preflight check catches it |
| CA workflow is confusing to end users | **High** | Document the full flow: generate on engine → export → install in the *client's* browser → Firefox needs its own step |
| Orphaned engine processes in the container after a proxy crash | Low | Single process; `SIGTERM` handling tested |
