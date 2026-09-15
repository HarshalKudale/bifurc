# Bifurc — UI / Engine Decoupling Assessment

**Scope:** Separate the Electron UI from the backend engine so that engine and UI communicate over a
JSON-RPC-style transport, enabling a future **headless engine + webview UI + CLI**, and a
**Docker-deployable engine** that remote clients can attach to.

**Baseline:** `bifurc-monorepo` v0.3.1 — Electron 42.3.0, React 18, Vite 8, TypeScript 5.4, electron-builder 26.8.1.

---

## 1. Executive summary

**Verdict: feasible, and meaningfully cheaper than a typical Electron decoupling** — because three
properties of the current codebase already do most of the hard work (§3). You are not designing an
RPC layer from scratch; you are **generalizing one that already ships** (`companionServer.ts`).

| | |
|---|---|
| **Headline estimate** | **~40–45 engineer-weeks** (range 32–52 with contingency) |
| **Calendar** | **3–5 months** with 2–3 engineers |
| **Minimum viable decoupling** | **~8–10 engineer-weeks** to insert the seam and ship a zero-regression Electron release |
| **Highest-risk item** | MSIX/APPX packaging of a split shell + engine (§7.1) |
| **Highest-value shortcut** | The preload bridge is already a flat `window.api` facade — reimplement it as an RPC client and **all 207 renderer call sites keep working unchanged** (§3.1) |

**Recommended sequencing:** strangle the seam first. Land the engine extraction + shell bridge as a
**lockstep release with zero user-visible change**, prove it in production, *then* add web UI, CLI and
Docker in parallel. Do not attempt all four clients before the seam is proven.

---

## 2. What is actually in the codebase today

Measured, not estimated:

| Area | Files | LOC | Notes |
|---|---:|---:|---|
| `src/ipc/` | 53 | 4,763 | Handler layer + import/export |
| `src/proxy/` | 19 | 2,567 | The actual product: proxy, TLS, mocks, scripts |
| `src/store/` | 10 | 1,278 | Config, workspace FS, git store |
| `src/sync/` | 8 | 1,060 | git-based sync, audit, status tracker |
| `src/applications/` | 4 | 854 | Child-process spawner |
| `src/companion/` | 2 | 309 | **Existing WS RPC server** |
| `src/lib/` | 2 | 140 | |
| `src/subscription/` | 1 | 21 | Stub gates (always `allowed: true`) |
| `src/main.ts` | 1 | 336 | Shell: window, tray, menu, lifecycle |
| `src/preload.ts` | 1 | 197 | The IPC facade |
| **Main process total** | **~101** | **~11,525** | |
| `renderer/` | 209 | 27,910 | React UI |
| `tests/` | 35 | 9,636 | Unit + integration |
| `e2e/` | 11 specs | — | Playwright driving `_electron.launch` |

**IPC surface area:**

| Metric | Count |
|---|---:|
| `ipcMain.handle` (request/response) | **112** |
| `ipcMain.on` (fire-and-forget) | **0** |
| Main → renderer push events | **7** |
| `ipcRenderer.invoke` wrappers in preload | **136** |
| `ipcRenderer.on` subscriptions in preload | **7** |
| `window.api.*` call sites in renderer | **207** across 43 files |

The push events are: `sync:status`, `sync:entityStatus`, `log:entry`, `log:chunk`, `server:error`,
`companion:refresh`, `webhook:payload`.

**Handler classification** (this drives the whole estimate):

| Class | Count | Disposition |
|---|---:|---|
| Generic entity CRUD (factory-generated) | ~36 | Collapse into ~6 generic RPC methods |
| Protocol ops (GraphQL / gRPC / SOAP / TLS / runner / audit / sync / healthbar / webhook) | ~50 | Move to engine, mostly 1:1 |
| Shell-only (zoom, theme, titlebar, `openExternal`, update check, first-launch) | ~14 | **Stay in the client**, never cross the wire |
| Dialog-mediated file I/O (pick file/folder, save report, import/export, TLS cert import) | ~12 | **Needs semantic redesign** for remote — see §4.3 |

So: of 112 handlers, roughly **85 become engine commands**, **14 stay client-side**, and **12 need a
new design** rather than a port.

---

## 3. Why this is cheaper than it looks — four existing assets

### 3.1 The renderer is already platform-clean

`renderer/` contains **zero** Node or Electron API usage. No `require`, no `fs`, no `path`, no
`electron`, no `__dirname`. The only `process.*` reference is `process.platform`, exposed once through
preload.

Everything the UI knows about the backend is the flat object literal in `src/preload.ts`. That means:

> **Rewrite `preload.ts` as an RPC client that reconstructs the same `window.api` shape, and the
> 27,910 LOC of renderer code does not change at all.**

This is the single largest cost saving in the project. The renderer's "web readiness" is already done.

### 3.2 A working JSON-RPC transport already exists

`src/companion/companionServer.ts` is a localhost-only WebSocket server implementing exactly the
protocol you described:

```
Client → Server: { id: string, action: string, payload: any }
Server → Client: { id: string, ok: boolean, data?: any, error?: string }
```

It is bound to `127.0.0.1`, gated by an `ALLOWED_ACTIONS` allowlist (`src/companion/allowedActions.ts`),
and is **already consumed in production** by the `bifurc-extension` browser extension
(`background.js` → `ws://127.0.0.1:9271`). Port is configurable via `cfg.companionPort`.

The work is therefore **widening an existing, shipping protocol** (4 allowed actions → ~85), not
inventing a transport. That removes the largest category of design risk.

### 3.3 The engine is already almost Electron-free

Electron imports leak into the engine in only **three categories**, across **10 files**:

| Category | Files | Sites |
|---|---|---|
| `BrowserWindow.getAllWindows()` for broadcasting | `ipc/handlers.ts`, `coreHandlers.ts`, `entityCrudFactory.ts`, `folderHandlers.ts`, `syncHandlers.ts`, `systemHandlers.ts`, `proxy/webhookServer.ts`, `companion/companionServer.ts` | ~15 |
| `app.getPath("userData")` | `store/appSettings.ts`, `store/workspaceFs.ts` | 2 |
| `dialog.*` / `shell.*` (shell concerns) | `runnerHandlers.ts`, `syncHandlers.ts`, `systemHandlers.ts`, `tlsHandlers.ts`, `importExport/index.ts` | ~10 |

**`src/proxy/` — the actual product — has exactly one Electron import** (`webhookServer.ts`, purely to
broadcast payloads). The proxy core, TLS handling, mock engine, script executor and service discovery
are already pure Node.

There is also already a correct pattern in-repo to copy: `src/proxy/logEmitter.ts` is a proper
`EventEmitter` that the IPC layer subscribes to. Generalizing it into an engine-wide event bus is a
mechanical refactor, not a redesign.

### 3.4 CRUD is already factory-generated

`src/ipc/handlers/entityCrudFactory.ts` already generates `add`/`update`/`delete` for ~11 entity kinds
from one parameterized function (`registerEntityCrudHandlers`). The 36 CRUD handlers are therefore
**~6 generic RPC methods** in the new protocol, not 36 hand-written ones.

---

## 4. The real coupling points — where the work actually is

### 4.1 The broadcast pattern (primary architectural knot)

Eight files push events straight into Electron windows:

```ts
BrowserWindow.getAllWindows().forEach((w) => {
  if (!w.isDestroyed()) w.webContents.send("log:entry", entry);
});
```

The engine currently knows about the UI. This must invert: the engine emits on an event bus, and the
transport layer subscribes and fans out to *its own* connected clients. Fixing this is ~15 call sites
but touches the sync, log, webhook and process-spawner paths — it is the item most likely to surface
subtle regressions (missed events, duplicate delivery, ordering).

**One genuine design problem here:** `applications/processSpawner.ts` holds a `mainWindow: BrowserWindow`
reference (`setMainWindow`) to stream child-process output. A headless engine has no window. This must
become a per-subscription event stream keyed by process id.

### 4.2 Data directory injection

`store/appSettings.ts:79` and `store/workspaceFs.ts:40` both call `app.getPath("userData")`. Replace with
an injected base directory resolved at engine startup (`--data-dir`, `BIFURC_DATA_DIR`, or XDG/LOCALAPPDATA
defaults). Trivial code change, but **mandatory** for CLI and Docker.

### 4.3 File dialogs — the one thing that does not port

This is the subtlest part of the whole project and deserves an explicit decision.

Today the engine calls `dialog.showOpenDialog` / `showSaveDialog` and then reads/writes the chosen path
directly (`importExport/index.ts`, `tlsHandlers.ts`, `runnerHandlers.ts`, `syncHandlers.ts`,
`systemHandlers.ts`).

- **Local engine + local UI:** fine. Client picks the path, sends it to the engine, engine touches disk.
- **Remote engine (Docker) + browser UI:** *broken by design.* A path picker on the user's machine is
  meaningless to a container's filesystem.

You must choose a model:

| Option | Mechanism | Cost | Trade-off |
|---|---|---|---|
| **A. Path-passthrough** | Client sends absolute path; engine reads it | Low | Only works when client and engine share a filesystem |
| **B. Upload/download** | Client streams bytes over RPC; engine writes to a managed staging dir | Medium | Works everywhere; needs size limits, streaming, progress |
| **C. Engine-side browser** | Engine exposes a sandboxed directory-listing API; UI browses the *engine's* FS | Medium | Right for Docker volumes; poor for local "import my Postman file" |

**Recommendation:** support **A for local/embedded** and **B for remote**, with the transport declaring
its capability. Do not try to force one model. Budget 1–2 weeks for this alone, and treat it as a
protocol-level concern rather than a handler-level one.

### 4.4 Shell → engine import (circular dependency)

`src/ipc/handlers/coreHandlers.ts:20` imports `updateTrayMenu` from `@/main`. The handler layer reaching
back into the shell entry point is a circular dependency and an immediate blocker for extraction. Invert
it: the shell subscribes to an engine `settings:changed` event.

### 4.5 Build & module boundaries

- Path aliases: `tsc` + `tsc-alias` with `@/*` → `src/*`, CommonJS output. Real package boundaries mean
  aliases must be resolved per package (or replaced with proper package imports).
- electron-builder currently bundles `files: ["dist/**/*"]` — main and renderer together. Splitting means
  the shell must ship the engine as a dependency or an `extraResources` payload.
- No `tsup`/`esbuild` config exists for producing a publishable library artifact. Needs adding.

### 4.6 Security — genuinely new work

The companion server's security model today is "bind to 127.0.0.1 + 4-action allowlist". For a remote
engine you need, none of which exists yet:

- Authentication (shared token / mTLS) — the engine exposes **mock servers, TLS CA generation, script
  execution and arbitrary child-process spawning**. An unauthenticated remote RPC surface is a remote
  code execution hole.
- TLS on the RPC channel.
- Per-command authorization, not just a coarse allowlist.
- Rate limiting and audit of RPC calls.

Treat this as a first-class workstream, not a hardening afterthought. Note that the engine already
writes an audit log (`audit:list`, `audit:diff`, `audit:export`) — extend it to cover RPC calls.

### 4.7 Version skew

Once shell, web UI, CLI and Docker engine are separate artifacts, you need a protocol version handshake
and a compatibility policy. Otherwise a new UI against an old remote engine fails in confusing ways.
This is **ongoing maintenance cost**, not one-time — plan to keep all artifacts in **lockstep version**
initially and only relax later.

---

## 5. Target architecture

```
┌──────────────────────────────────────────────────────────────────────┐
│  CLIENTS                                                             │
│                                                                      │
│  Electron shell          Web UI              CLI                     │
│  (thin host)             (browser)           (terminal)              │
│  ├ renderer (unchanged)  ├ renderer (reused) ├ command tree          │
│  ├ preload → RPC client  ├ RPC client (ws)   ├ RPC client (stdio/ws) │
│  └ shell-only IPC:       └ connection/auth   └ output formatting     │
│    zoom, theme,            + file upload                             │
│    titlebar, dialogs                                                 │
└───────────────┬──────────────────┬───────────────────┬──────────────┘
                │                  │                   │
      embedded / spawned      WebSocket           stdio / WebSocket
                │                  │                   │
        ┌───────┴──────────────────┴───────────────────┴───────┐
        │  TRANSPORT LAYER                                      │
        │  JSON-RPC 2.0  ·  auth  ·  version negotiation        │
        │  event fan-out  ·  reconnect + resync  ·  backpressure│
        └───────────────────────┬───────────────────────────────┘
                                │
        ┌───────────────────────┴───────────────────────────────┐
        │  @bifurc/engine   (pure Node, zero Electron)          │
        │                                                       │
        │  EventBus ── proxy server · TLS/mkcert · mock engine  │
        │              script executor · workspace FS · git sync │
        │              process spawner · webhook server · audit  │
        │                                                       │
        │  CommandRegistry  (~85 commands, schema-validated)     │
        │  data dir injected · headless startup path            │
        └───────────────────────────────────────────────────────┘

        ┌───────────────────────────────────────────────────────┐
        │  @bifurc/protocol   (shared)                          │
        │  command/event schemas · generated TS types · version  │
        └───────────────────────────────────────────────────────┘
```

**Proposed package layout:**

```
packages/
  protocol/     # schemas, generated types, version const  — no runtime deps
  engine/       # pure Node engine + CommandRegistry + EventBus
  client/       # typed RPC client (ws + stdio transports), implements Window["api"] shape
  cli/          # command tree over client/
apps/
  desktop/      # Electron shell: main.ts, preload bridge, tray, packaging configs
  web/          # Vite build of the existing renderer + browser RPC client
  companion/    # browser extension (already an RPC consumer — minimal change)
```

---

## 6. Workstream breakdown and estimates

Estimates in **engineer-weeks** (one senior engineer, familiar with the codebase). Ranges reflect
uncertainty, not padding.

| # | Workstream | Low | High | Notes |
|---|---|---:|---:|---|
| 1 | **Protocol design & freeze** | 2.0 | 3.0 | Classify 112 handlers; define command/event schemas; error taxonomy; version handshake; capability negotiation; auth model. Deliverable: `@bifurc/protocol`. |
| 2 | **Engine extraction** | 4.0 | 6.0 | De-Electron the 10 files; introduce `EventBus`; inject data dir; split shell-only handlers; fix `coreHandlers → main` cycle; package build config. |
| 3 | **File-I/O semantics redesign** | 1.0 | 2.0 | The §4.3 decision: local passthrough vs remote upload/download. Includes streaming, size limits, progress. |
| 4 | **Transport layer** | 3.0 | 4.0 | Generalize `companionServer` into a full JSON-RPC server; stdio transport; auth + TLS; reconnect; **event resync after disconnect**; backpressure for `log:entry`/`log:chunk`. |
| 5 | **Typed RPC client** | 1.5 | 2.0 | Reconstruct the `window.api` shape so renderer is untouched; ws + stdio; retry/timeout. |
| 6 | **Electron shell as thin host** | 2.0 | 3.0 | IPC↔RPC bridge; keep the 14 shell-only handlers local; engine lifecycle (embed/spawn/supervise); single-instance; tray; first-launch. |
| 7 | **Web UI** | 3.0 | 4.0 | Renderer is reusable; add connection/auth/multi-engine UX, replace native dialogs with upload/download, source `platform` from the engine instead of `process.platform`, static hosting. |
| 8 | **CLI** | 3.0 | 5.0 | Scope-dependent. Read-only + engine lifecycle is ~3w; full CRUD parity across all protocols is ~5w+. |
| 9 | **Packaging & distribution** | 3.0 | 5.0 | Four targets (§7) + Dockerfile + release workflow rework + MSIX validation. |
| 10 | **Tests & migration** | 3.0 | 4.0 | Rework `tests/ipc/handlers.test.ts` (currently mocks `ipcMain`); add transport conformance suite; web + CLI e2e tracks. |
| 11 | **Security, observability, docs** | 2.0 | 3.0 | Auth, per-command authorization, RPC audit, structured logging, protocol docs, migration guide. |
| | **Subtotal** | **27.5** | **41.0** | |
| | **Contingency (25%)** | 7.0 | 10.0 | MSIX surprises, protocol churn, remote file-I/O edge cases |
| | **Total** | **34.5** | **51.0** | **~40–45 engineer-weeks central estimate** |

**Calendar:** with 2 engineers ~4–5 months; with 3 engineers ~3–4 months. Workstreams 4/5, 7/8 and 9/10
parallelize well; 1, 2, 3 and 6 are on the critical path and largely sequential.

### Minimum viable decoupling — 8–10 engineer-weeks

If you want the *option* before the *payoff*:

1. Protocol design, scoped to the existing handler surface (2–3 w)
2. Engine extraction + `EventBus` (4–6 w)
3. Shell bridge, keeping `window.api` byte-identical (2–3 w)

**Ships as a normal lockstep Electron release with no user-visible change.** The engine is now a real
seam. Web UI, CLI, Docker and the packaging split then become additive, independently schedulable work
against a proven interface. This is the path I would take.

---

## 7. Packaging analysis

### 7.1 Current state

| Target | Configured where | Artifact |
|---|---|---|
| Windows NSIS | `bifurc/package.json` → `build.win.target` | `Bifurc.Setup.exe` |
| Windows APPX (MS Store) | `build.appx` with Store identity | `Bifurc.Setup.appx` |
| Linux AppImage | **CI only** — `.github/workflows/release.yml:117` (`--linux AppImage`); no `linux` block in `package.json` | `*.AppImage` |
| Docker | **does not exist** | — |

Store identity is declared inline: `identityName: HarshalKudale.Bifurc`,
`publisher: CN=C594A390-264F-4DE0-ABF9-524B641E479D`, `applicationId: Bifurc`, with logos in
`build/appx/`. `extraResources` ships `tray-icon.png` and `icon.png`.

Release flow: `prepare` → `build-windows` (windows-latest) + `build-linux` (ubuntu-latest) →
`publish-release` via `softprops/action-gh-release`. Both jobs run `npm ci` and `npm run package`.

### 7.2 How each target hosts a split engine

| Target | Engine hosting | Difficulty | Key constraint |
|---|---|---|---|
| **NSIS** | Ship engine as `extraResources`; spawn as supervised child process on loopback. Optionally register a Windows service. | **Low** | None significant — full freedom. |
| **APPX / MSIX** | Must ship engine **inside** the package (an MSIX is a single immutable container). Spawn as child, or run **in-process**. | **High** | See §7.3 |
| **AppImage** | Bundle engine inside the image; spawn as child; write data to `$XDG_CONFIG_HOME`/`~/.config`. | **Medium** | AppImage is a read-only squashfs mounted at a **random path each run** — no absolute paths baked in, no self-writes. Engine must resolve all paths at runtime. |
| **Docker** | Native. Engine *is* the image. | **Medium** | Needs headless mode, git + mkcert in the image, volume for data, auth, port exposure. |

### 7.3 The MSIX problem (highest-risk packaging item)

An MSIX/APPX is a single immutable, signed container with one declared entry point. It does not
naturally accommodate "install a separate daemon and connect to it". Two viable approaches:

**Option 1 — In-package child process (preferred if it validates).**
Ship the engine inside the package and spawn it as a child of the shell. Electron apps are packaged as
**full-trust desktop bridge** apps, which are *not* subject to the UWP/WinRT loopback restriction — so a
loopback WebSocket to `127.0.0.1` should work, and child-process spawning is permitted for full-trust
apps. **However:** the child inherits the package identity and container, and its writable paths are
virtualized/redirected. This must be **empirically validated before committing to it** — I would make
this the first spike of the project.

**Option 2 — Embedded in-process engine (safe fallback).**
The shell links the engine as a library and runs it in-process, with the RPC transport short-circuited
to an in-memory adapter. Store build gets a working app; remote/web/CLI still work everywhere else.

**Recommendation:** design the transport so **embedded and remote are the same interface**, and let the
packaging target choose. This is exactly why workstream 4 should define a transport abstraction rather
than a single WebSocket implementation. Cost: ~1 extra week. Benefit: the MSIX question stops being a
release blocker.

### 7.4 Docker image

New artifact, and the one with the most hidden requirements:

- **Headless startup.** `src/main.ts:255` currently calls `checkGitInstalled()` and, if git is missing,
  shows an Electron `dialog.showErrorBox` then `app.quit()`. This must become a headless-friendly
  startup check with a structured error — the engine cannot depend on a dialog to report a fatal
  condition.
- **Base image must include `git`** (hard requirement — config versioning and sync depend on it).
  Cert generation itself is fine: `mkcert` v3.2.0 is **pure JS** (`node-forge`, no bundled native
  binary), so it compiles and containerises cleanly. What is *not* portable is CA **installation** —
  see §7.7.
- **Volumes:** data dir must be a declared volume, or all config/workspaces vanish on restart.
- **Networking:** the proxy port must be exposed and configurable; the RPC port should **not** be
  publicly exposed by default.
- **Auth is not optional.** Exposing this engine means exposing mock servers, TLS CA generation, script
  execution and child-process spawning.
- Consider a `/healthz` endpoint — you already have health-check machinery (`healthbar:*`).
- Decide **amd64-only or multi-arch** (`linux/arm64` matters for Apple Silicon and Graviton).

### 7.7 CA trust is a client-side concern, not an engine one

`installCA` (`src/proxy/certManager.ts:41`) shells out to platform tools:

| Platform | Command |
|---|---|
| Windows | `certutil -addstore -user Root <cert>` |
| macOS | `security add-trusted-cert -d -r trustRoot -k <loginKeychain> <cert>` |
| Linux | `sudo update-ca-certificates` |

This works today because engine and browser share a machine. With a remote engine it breaks: the CA
must be trusted by **the browser the user is actually proxying through**, which is on the *client*. A
CA installed inside a container does nothing for the user's Chrome.

The engine should **generate** the CA and hand it to the client; the client (or the user, guided by the
client) performs the trust install. Note the Linux path already needs `sudo` — which is another reason
it cannot live inside a container.

### 7.5 Packaging work items

| Item | Effort |
|---|---|
| Restructure `package.json` → workspaces; per-package builds; resolve `@/` aliases at package boundaries | 1 w |
| electron-builder reconfig: shell as app, engine as `extraResources`/dependency; `asarUnpack` for native bins | 1 w |
| MSIX spike + implementation (§7.3) | 1–2 w |
| Dockerfile + compose sample + healthcheck + volume docs | 0.5 w |
| AppImage: runtime path resolution, bundled engine, `$XDG` data dir | 0.5 w |
| Release workflow rework: artifact matrix, version lockstep, engine/shell versioning | 1 w |

### 7.6 Versioning and auto-update

`app:checkUpdate` (`systemHandlers.ts:85`) fetches GitHub Releases and selects an asset by
`process.platform`. With split artifacts this becomes ambiguous — is it updating the shell or the engine?

**Recommendation:** keep shell, engine, protocol and web UI on a **single lockstep version** initially.
Let the shell own its own update; the engine is updated as part of the shell's payload (NSIS/APPX/AppImage)
or via the container tag (Docker). Independent engine updates only make sense once a remote-engine user
base exists — defer that, and note that `process.platform` will be unavailable in a browser context, so
this handler must move behind the RPC boundary or be resolved client-side.

---

## 8. Recommended phasing

| Phase | Content | Exit criteria |
|---|---|---|
| **0 — De-risk** (2–3 w) | Protocol sketch; **MSIX loopback/child-process spike**; generalize `companionServer` to ~10 real commands as proof | MSIX question answered; transport proven with real commands |
| **1 — Engine** (4–6 w) | Workstreams 2 + 3 | Engine package runs headless; zero Electron imports; starts, serves, shuts down cleanly |
| **2 — Seam** (2–3 w) | Workstreams 4 + 5 + 6 | **Electron release ships with `window.api` unchanged; 35 unit + 11 e2e suites green** |
| **3 — Clients** (4–5 w, parallel) | Workstreams 7 + 8 | Web UI reaches parity on a browser; CLI covers engine lifecycle + core CRUD |
| **4 — Distribution** (3–4 w) | Workstream 9 | NSIS + APPX + AppImage + Docker all publish from CI |
| **5 — Harden** (2–3 w) | Workstreams 10 + 11 | Transport conformance suite; auth; protocol docs |

Phase 2 is the milestone that matters. Everything before it is reversible; everything after it is
additive.

---

## 9. Risk register

| Risk | Impact | Likelihood | Mitigation |
|---|---|---|---|
| MSIX cannot spawn/supervise the engine as expected | High | Medium | Phase 0 spike; embedded in-process fallback (§7.3) |
| Remote file dialogs are semantically wrong | High | **High** | Decide §4.3 model in Phase 0; capability negotiation in the protocol |
| Event loss/duplication during disconnect & reconnect | Medium | High | Event sequence numbers + resync-on-reconnect; conformance tests |
| `log:entry` / `log:chunk` volume overwhelms a remote transport | Medium | Medium | Backpressure, coalescing, sampling; these are high-frequency today |
| Protocol churn causes repeated client rework | Medium | High | Freeze schemas in Phase 0; generate types; version handshake |
| Unauthenticated remote engine = RCE | **Critical** | Medium | Auth + per-command authorization before *any* remote transport ships |
| CA trust-store install is host-scoped, not engine-scoped | Medium | **High** | `installCA` shells out to `certutil` / `security` / `update-ca-certificates`. For a remote engine the CA must be trusted by the *client's* browser. Redesign as a client-side step (§7.7) |
| Test suite rework larger than expected (`tests/ipc/handlers.test.ts` mocks `ipcMain`) | Low | High | Budget 3–4 w; convert to RPC-level tests against an in-process transport |
| Scope creep into the `Cleanup_plan.md` refactor | Medium | Medium | Do **not** mix the two. Land cleanup first or defer it — see §10 |

---

## 10. Anti-patterns to avoid

1. **Do not mix this with `Cleanup_plan.md`.** That plan already touches ~110 files across 5 phases
   (splitting `handlers.ts` from 1,909 lines, extracting `entityCrudFactory`, consolidating panels).
   Doing both at once makes regressions untraceable. Land the cleanup first, *or* land the seam first —
   not simultaneously. There is useful overlap (splitting `handlers.ts` helps extraction), so sequencing
   cleanup → decoupling is slightly better.
2. **Do not change the renderer in Phase 2.** Any renderer edit during the seam phase destroys your
   best regression signal. Keep `window.api` byte-identical.
3. **Do not design a bespoke RPC.** You already have a working `{id, action, payload}` shape consumed by
   a shipping extension. Standardize on JSON-RPC 2.0 for ecosystem tooling, but keep the action
   namespace you already use (`mock:add`, `request:add`, `folder:add`, `config:get`).
4. **Do not port all 112 handlers 1:1.** ~36 CRUD handlers collapse to ~6 generic commands; ~14 are
   shell-only and should never cross the wire. Porting blindly triples the protocol surface and the test
   matrix.
5. **Do not ship a remote transport before auth.** The engine spawns processes and executes scripts.
6. **Do not version shell and engine independently at first.** Lockstep until a remote-engine user base
   actually exists.

---

## 11. Decisions needed before starting

1. **CLI scope** — engine lifecycle + read-only inspection (~3 w), or full CRUD parity across all
   protocols (~5 w+)? This is the single largest swing in the estimate.
2. **File I/O model** for remote engines — passthrough, upload/download, or engine-side browser (§4.3)?
3. **MSIX strategy** — validate in-package child process, or commit to embedded in-process fallback (§7.3)?
4. **Docker architecture** — amd64 only, or multi-arch including `linux/arm64`?
5. **Cleanup ordering** — `Cleanup_plan.md` before, after, or interleaved with decoupling (§10.1)?
6. **Web UI parity target** — must it reach full parity with Electron, or is it an inspection/light-edit
   surface? Affects workstream 7 substantially.

---

*Assessment based on the repository state at `bifurc-monorepo` v0.3.1. All counts measured directly
from source. Estimates are in engineer-weeks for a senior engineer familiar with this codebase.*
