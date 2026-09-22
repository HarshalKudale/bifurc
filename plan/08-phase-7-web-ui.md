# 08 — Phase 7: Web UI

> **Updated 2026-09-20 by [`14-workspace-runtime.md`](14-workspace-runtime.md) (D10–D13).** Effort is
> unchanged, but the thing the browser connects to is now a **workspace host**, and the workspace is a
> *selected* resource rather than an implicit global. Two consequences for the items below:
>
> - **Work item 2's "multi-engine profiles" becomes "host + workspace selection"** — the same UI surface,
>   one more level in it. `activeWorkspaceId` is a client/session pointer after P14, not a server setting,
>   so the web UI owns it explicitly rather than inheriting it from `app.json`.
> - **Work item 5's "have the engine serve the UI" becomes "have the host serve the UI"** — still
>   same-origin, still no CORS, still the recommended option for Docker.
>
> Everything else in this phase is unaffected. **Depends on P14.**

**Goal:** run the existing renderer in a browser, connected to a local or remote **workspace host** over
the RPC transport.

**Effort:** 3–4 weeks. **Depends on:** **P14**. **Parallel with:** P9.

> The renderer is already browser-clean — **zero** Node or Electron APIs, no `navigator.userAgent`
> sniffing, and `window.api.platform` is declared but never read (verified). Most of this phase is
> connection UX and the file-handling substitutions, not porting UI.

---

## The architecture this phase targets — D18 (2026-09-22)

> **One UI, two hosts.** The Electron app and the browser app import the **same** UI; the only thing that
> varies is the communication module underneath it. The browser hides host-local affordances (native
> dialogs, tray, titlebar, zoom) and adds remote-host selection. Recorded as **D18** in
> [`00-decisions.md`](00-decisions.md).

The first landing was a framework-free probe page **on purpose** — so that a failure there would be a failure
of the *client* and not of a screen built on top of it. That is still what `apps/web` is. It is **not** the
phase's target, and this section supersedes the "no React" reasoning that landing recorded.

### Why the seam is already there — measured 2026-09-22

The expensive-looking part of a shared UI is "route 240 shell calls through an interface". **That work is
already done, and not by us:**

| Claim | Measurement |
| --- | --- |
| The renderer is shell-free | **0** imports of `electron`, `node:` or `@bifurc/*`. Its only non-relative imports are `react`, `react-dom/client`, `react-resizable-panels`, `react-complex-tree`, `lucide-react`, `@radix-ui/react-switch`, `@lezer/highlight` and the CodeMirror packages |
| It has exactly one shell seam | `window.api` — **240** references across **43** files, **234** of them method calls (the other 6 pass the object itself). That is WI 0's *"119 methods across 234 call sites"* counted the other way round, and the two agree; every other `window.*` is standard DOM: `addEventListener` 11, `removeEventListener` 11, `dispatchEvent` 6, `innerWidth` / `innerHeight` 1 each |
| That seam is already *injected*, not imported | `renderer/types/window.ts` ends in `declare global { interface Window { api: BifurcApi } }` — the UI names the shape and never constructs it |
| Both shells already fill it the same way | `src/preload.ts:61` → `createClient(createIpcTransport(), { local })` → `contextBridge.exposeInMainWorld("api", { ...client })`. `apps/web/src/connect.ts` → `createClient(createBrowserTransport({ url }), { local: createBrowserLocal() })` |

**So the communication module the two hosts differ in is not something this phase must invent.** It is the
`Transport` argument to `createClient`, and both transports already exist. What is missing is that the *UI* is
not yet a package — it is a directory the root Vite config hardcodes as its `root`.

### What that means for the work items

- **Work item 1** (browser client) is unchanged, and still first.
- **Work item 2** gains a shape: the connection/session UI is *shared* UI, so its states are states of the
  shared shell rather than of a browser-only page. Its two auth states still need the `hello` handshake.
- **Work item 5** changes character. It is no longer "build a page"; it is **extract the renderer into a
  package, and give both shells a build that consumes it.** The root `vite.config.ts` hardcodes
  `root: "renderer"`, registers `@vitejs/plugin-react`, and aliases `@` into the renderer tree — so the
  extraction touches both Vite configs and the coverage `include` set.

> **The blast radius is the build, not the code.** The 240 call sites do not change. Two Vite configs, the
> `workspaces` glob, the coverage `include`, CI, and `renderer/types/window.ts`'s global declaration do.

---

## Preconditions

- P6 shipped. Engine is a real, separately-addressable process.
- D8 answered (parity target: full or inspection + light editing).
- P3 complete — the blob flows are what replace native dialogs.

---

## Work item 0 — the opening measurement (2026-09-22)

Re-derived from the tree **before** starting, because this document's prose was written on 2026-09-20 and P14
has since changed what it describes. **Four of its load-bearing claims did not survive.** The method is P14's:
a phase's premises are a claim about the tree, and a claim about the tree is checked by measuring it.

### What holds

- **D8 is answered: (b) inspection + light editing** (`plan/00-decisions.md`). Acceptance criterion 1 has a
  definition to measure against, not a vibe.
- **P6 shipped, and the engine is separately addressable.** Precondition satisfied.
- **P3's blob flows exist** — `blob.put` / `blob.stat` / `blob.read` / `blob.release`, now namespaced per
  workspace by D17.

### What the measurement found

**The renderer's IPC surface is 119 methods across 234 call sites**, in 209 `.ts`/`.tsx` files — and **every
one of the 119 is already on the P5 client surface**: 0 of them lack a key in
`packages/client/src/surface.ts` (144 keys). That is the best news here and it is worth stating as a
*measurement* rather than an assumption: **the adapter this phase must write has no gaps to invent.** It is a
mapping, not a design, and the mapping target already exists and is already the spec.

**25 of the 144 keys are never called by the renderer at all** — and they are almost exactly work item 4's
table: `platform`, `getTheme` / `setTheme`, `getZoomLevel` / `setZoomLevel`, `isFirstLaunch` /
`completeFirstLaunch`, `setTitleBarOverlay`, `pickFilePath` / `pickFolderPath`, `proxyStatus`,
`grpcMockServerStatus`, `gitGetDiff` / `gitGetHistory` / `gitSync`, `onWebhookPayload`. **Item 4 is
corroborated by measurement** — but note what that means in practice: those keys are *unused*, so a browser
build that omits them silently will not fail a single existing test. They need explicit decisions, not
substitutions nobody notices are absent.

**The one wrong claim that costs a work item.** Item 1 says *"`packages/client` already has a `ws` transport.
In the browser, the only new piece is discovery."* A `createWsTransport(url)` does exist — but at
`packages/engine/src/transport/ws.ts:226`, in a file importing `node:crypto`, `node:http`, `node:https`,
`node:net` and the Node `ws` package. **It is Node-only, so a browser cannot load it.** The browser needs its
own `Transport` over the native `WebSocket` API, sharing the framing and the auth shape with the Node one.
**"Discovery only" understates this item by exactly one transport adapter** — and that adapter is the item's
real content.

**Item 3's table names channels that no longer exist.** It lists eight `dialog:*` call sites; a repo-wide grep
for `dialog:` under `renderer/` returns **0**, because P6's dead-channel reduction deleted those channels. The
live surface is `window.api.<method>`, and by that measure there are **16 file/dialog-shaped methods**:
`openFileDialog`, `exportData`, `importData`, `preflightImport`, `exportAudit`, `exportRunnerReport`,
`saveRunnerReport`, `getImportExportFormats`, `listProtoFiles`, `addProtoFile`, `tlsImportCert`,
`tlsImportKey`, `tlsExportCert`, `saveConfig`, `saveRunnerConfig`, `healthbarSaveServices`. The table's
**component** column is still useful; its **channel** column is not — and the criterion "all 8 dialog call
sites replaced" should be re-derived **per method** before it is ticked, not per row.

**There is no web target in the tree.** No `apps/`, no `web/`, no `renderer-web`. Item 5 builds one from
nothing — consistent with the estimate, but worth stating rather than discovering.

### The consequence for sequencing

Item 1 is **not** "discovery". It is a **browser transport adapter plus discovery**, and it is first because
everything else is blocked behind a client that can connect. It is also the only part of this phase that is
testable **without a browser UI** — against a real host over a real socket, which is the shape P14's
instruments already use.

---

## Work item 1 — Browser RPC client

`packages/client` already has a `ws` transport. In the browser, the only new piece is discovery:

```ts
// apps/web/src/connect.ts
const client = createClient(createWsTransport({ url, token }));
await client.hello({ clientName: "web", clientVersion: VERSION });
```

The browser client uses the **same** `createClient` and the same shims. No divergence.

> **Corrected 2026-09-22 — see work item 0.** The transport named above is **Node-only**: `ws.ts` imports
> `node:crypto` / `node:http` / `node:https` / `node:net` and the `ws` package, so a browser cannot load it.
> The browser needs its own `Transport` over the native `WebSocket` API. "The only new piece is discovery"
> understates this item by one adapter.

### The placement decision (2026-09-22)

**The adapter is a new package, `@bifurc/web-client`.** Decided by the owner, on the grounds that the
programme's point is decoupled packages. Three placements were on the table; the other two were rejected
on measured grounds rather than taste:

- **`packages/engine/src/transport/wsBrowser.ts`** — where every other transport lives, but `engine` is a
  Node package (`ws`, `simple-git`, `mkcert`, `archiver`) and a browser bundle reaching into it is the
  opposite of the goal.
- **`packages/client/src/browser.ts`** — the smallest surface for the app, but `@bifurc/engine` is only a
  **devDependency** of the client and its engine import is **type-only** (`index.ts:39`), so using the
  shared session at runtime would mean promoting a Node package into a browser bundle.

**The measurement that made the new package viable.** The browser needs exactly one thing at *runtime*:
`createClientSession`. `Transport` and `EngineEvent` are **type-only** imports, erased at build, and cost a
browser bundle nothing — which is already how `@bifurc/client` uses them. So the problem reduced to where
`session.ts` can live.

**`types.ts` and `session.ts` moved into `@bifurc/protocol`** as `src/wire/`. Two facts decided the
destination, and both were checked rather than assumed:

1. `@bifurc/protocol` already declares itself "environment-agnostic: no Node built-ins, importable from
   Node, Bun, and a **browser bundle**", and enforces it with `platform: "neutral"` in its `tsup.config.ts`
   — *"fail loudly if a future command schema accidentally pulls one in."* The build is the instrument, so
   the move cannot silently regress.
2. `framing.ts` did **not** move. It is length-prefix framing for byte streams, `Buffer`-heavy throughout,
   and a WebSocket needs none of it — `ws.ts` sends a bare message because the protocol frames natively.
   Moving it would have been cargo-culting a "wire trio" that is really a pair plus a stream codec.

**Two changes were required for `session.ts` to load outside Node**, both small and both type-level:

- `randomUUID` came from `node:crypto`; it is now `globalThis.crypto.randomUUID()`, present in every
  browser and in Node from 19 on. Verified present on this machine's Node 22.22.2.
- `ClientChannel.encode`/`write` were typed against `Buffer`, a Node global. They are now `Uint8Array`.
  `Buffer` extends `Uint8Array`, so every existing channel still satisfies the contract — the widening is
  backward-compatible, not a break.

**The browser sends *text* frames, deliberately.** `encode()` still returns bytes because `stdio` and
`socket` need a length prefix; but the engine's server reads every frame as text
(`JSON.parse(raw.toString())`), and `@bifurc/engine`'s own `ws` client sends text. Sending binary would
work — `ws` emits a `Buffer` for binary and `.toString()` decodes it — but it would exercise a server path
no existing test covers. Text keeps the browser and the Node client byte-identical on the wire.

**Engine keeps thin re-exports** at `transport/types.ts` and `transport/session.ts`, so the ~10 existing
importers — including the shell's `ipc/rpcBridge.ts` and `ipcTransport.ts` — keep working unchanged.
Repointing them at `@bifurc/protocol` is a follow-up; a second import path for the same declarations is a
wart, not a defect.

**One thing this did *not* fix, recorded because the move made it visible:** `ws.ts`'s client does **not**
use `createClientSession`. It still carries its own copy of the whole state machine — pending, control,
listeners, teardown, deliver — which is exactly the "third copy where similar becomes drifted" that
`session.ts` was extracted to prevent. It is a refactor candidate for whoever next touches `ws.ts`.

---

### The build record (2026-09-22) — what WI 1 actually landed

Two halves in `packages/web-client`, both against **real** I/O and with no mocks:

- **`channel.ts` / `createBrowserTransport`** — a `ClientChannel` over the native `WebSocket`, letting
  `createClientSession` own correlation, refcounting and teardown. Writing a second session would have been
  the **third** copy `session.ts` exists to prevent. It sends **text** frames, matching the engine's server
  and its own `ws` client. 8 cases against a real `createWsServer`.
- **`local.ts` / `createBrowserLocal`** — the **16 required `RequiredClientLocal` members**, without which
  `createClient` cannot be called at all (it *requires* `local`). Declared with `satisfies`, never `as`. 16
  jsdom cases. Of the sixteen, **9 do the desktop's real work and 7 report a truthful failure**: 2 **throw**
  (`pickFilePath`, `pickFolderPath` — a browser file input yields a `File` whose name is a bare name with no
  directory, so the desktop's reusable-path contract cannot be honoured, and a synthetic path would fail one
  step later instead of here) and 5 **refuse structurally** (`setTitleBarOverlay`; `tlsInstallCA` →
  `needsManualInstall` + instructions, which is a *supported outcome* rather than an error; `tlsRemoveCert`;
  `tlsCertStatus`; `checkUpdate` → `ok:false` **with the version still reported**, so "I cannot answer this"
  cannot be misread as "you are up to date"). `openExternal` is an **http/https allowlist plus
  `noopener,noreferrer`**: the URL comes from a possibly remote engine, and `window.open("javascript:…")`
  would run script in a context the user trusts.

**The wire machinery moved as a precondition.** `types.ts` + `session.ts` → `@bifurc/protocol/src/wire/`,
because protocol is the package that already enforces `platform: "neutral"` and is therefore the only one
whose build fails loudly if a Node built-in creeps in. `framing.ts` **stayed in the engine** — it is
length-prefix framing for byte streams and a WebSocket needs none of it; moving it would have been
cargo-culting a "wire trio" that is really a pair plus a stream codec. Engine keeps thin re-exports, so the
shell is untouched. `@bifurc/client` was *already* `platform: "neutral"` — only the transport was missing.

**The `ws.ts` third copy is gone (WI 1b).** `createWsTransport`'s client is now a `ClientChannel` +
`createClientSession`: ~180 lines of duplicated state machine deleted, leaving only what is per-transport
(the `ws` library's `.on()` API; the `headers` option, where a remote deployment puts a bearer credential;
the ten-second connect timeout; and the close code). The conformance suite (57 cases) passed unchanged, which
is the evidence the two copies agreed.

Completing that extraction exposed **two defects that were already on `HEAD` and green in `npm test`**.
Neither is a P7 regression; both are recorded in `TRAPS.md`:

1. A **type error** left by the `Buffer` → `Uint8Array` widening. The widening was justified as safe because
   `Buffer extends Uint8Array` — true of every *implementation*, false of a *consumer*: `session.test.ts`
   stored into a `Buffer[]`. `npm test` runs vitest, **and vitest does not typecheck**, so it shipped for a
   full commit. **Run `tsc -p packages/engine/tsconfig.json` explicitly after any type-level change.**
2. An **uncaught exception** on the connect-timeout path. `release()` detached every listener and *then*
   called `close()`; on a socket still `CONNECTING`, `ws.close()` aborts the handshake and emits `error` on
   the **next tick**, and an `error` event with no listener **throws** — outside the `try`, so it escaped as
   an uncaught exception rather than failing one session. Pre-existing: the pre-refactor copy had the same
   order, and nothing had ever exercised the timeout, so nothing had ever emitted it.

**Still open after WI 1:** no web target exists (`apps/`) — that is item 5, built from nothing. Two
capabilities have no browser equivalent and are **recorded rather than worked around**: `tlsRemoveCert`'s
engine half is unreachable from any non-shell `ClientLocal` (the hook is injected *without* the transport, so
the gap is in the injection point, not in the browser file), and CA install is manual by nature (item 4).

### The two defects the suite found (2026-09-22) — a connect that never settles, and a message that lies

The first full-suite run after `apps/web` landed reported **one** failure, and a **test timeout is different
evidence from a failed assertion**: it says something never settled. That narrowed it in one step.
`packages/web-client/tests/browserTransport.test.ts` → *"reports a connect failure distinctly from a dropped
session"* died on vitest's 15 s limit rather than on an expectation. (An earlier report put the failure in
`apps/web/tests/connect.test.ts` as a wrong *message*; that did not reproduce, and reading the file it cannot:
its fatal is only produced with `open === false`, which needs a socket that never opened, while its `probe()`
— a round trip — cannot succeed without one. Recorded as unexplained rather than as fixed.)

**Defect 1 — nothing bounded the connect.** `ensureReady()` armed no timer, and a peer that accepts the TCP
connection and then never answers the upgrade produces no `open`, no `error` and no `close`, so the promise
never settled and every request behind it waited forever. That is the "hangs rather than rejects" failure
`plan/05` forbids, arriving through the one door `createClientSession` cannot see.

**The mechanism, measured rather than argued.** A probe against the platform's own `WebSocket` — Node 22's,
i.e. undici's — shows that on a **failed connect it fires `error` and then no `close` at all**, with
`readyState` still `CONNECTING` seconds later. Three forms behave identically: a port that was bound and
released, a listener that refuses the upgrade, and a listener that destroys the socket on arrival.

That is a property of the **host**, not of the protocol. `ws` — which the engine's channel uses — does emit
`close` after `error`, and a spec-compliant browser does too, so `ws.ts`'s *"`ws` always emits `close` after
`error`, so the teardown and the rejection both happen there"* is true where it is written and **false here**.
The consequence is that **that handler cannot be copied**: silencing `onerror` and letting `close` report — the
`ws.ts` shape — would leave a wrong address reported by **nothing** on this host. The first attempt at this
fix took exactly that shape, which is what made the trap visible.

**Defect 2 — the message did not depend on whether the connection had ever been established.** Both handlers
called `fail()` unconditionally, and `fail()` is idempotent and first-wins, so which message a caller saw was
decided by **which event fired first** rather than by whether the socket ever opened. On a host where `error`
precedes `close` for an established connection that drops abnormally, that announces a dropped session as
`Could not connect` — the message item 2's UI answers with "retry from scratch" rather than "reconnect".

So the two naive positions — report always, or never — are wrong in opposite directions, and the shape that
holds is a guard:

- **`error`, only while the socket has never opened** (`if (open) return`). On this host that is the only
  signal a failed connect has.
- **`close`, for the established case**, where it says `Session closed: …`.
- **A connect timeout** for the third case — a peer that accepts the TCP connection and then answers nothing.
  A refusal fires `error`; silence fires nothing at all. `DEFAULT_CONNECT_TIMEOUT_MS = 10_000`, the engine's
  number and the engine's wording so the two clients report a slow engine identically, overridable via
  `connectTimeoutMs`, armed with the connect and disarmed on `open`, on `close` and in `releaseSocket()`.

Three cases, each guarding one path, and each shown to fail without it:

| Case | Guards | Without the fix |
|---|---|---|
| dead address (bound, then released) | that a connect failure settles at all | hung on vitest's 15 s limit |
| a peer that accepts and never answers | the connect timeout | hangs |
| a listener that refuses the upgrade | the `error` handler | falls back to the 10 s timeout, wrong message |

The last one exists *because of* the mutation discipline rather than beside it: silencing `onerror` (M9) left
the dead-address case **passing** through its disjunction, so without it that handler would have been
unguarded. The dead-address case also asserts the **distinction** rather than one spelling of it —
`/Could not connect|Timed out after/` and **not** `/Session closed/` — because both spellings mean "the engine
was never reached", and the second is the message work item 2 answers with "reconnect".

## Work item 2 — Connection and session UX

This is genuinely new surface that has no Electron equivalent.

| State | UI |
|---|---|
| No engine configured | Onboarding: enter a URL, or "connect to local engine" |
| Connecting | Progress state; surface the handshake result |
| Version mismatch | Clear, actionable error — never a generic failure |
| Unauthorised | Token entry; explain where to find it |
| Connected | Normal app; show engine identity + version in the footer |
| Disconnected | Banner + automatic reconnect with backoff; do not silently freeze |
| `resyncRequired` | Visible indicator + automatic state re-fetch |

**Multi-engine** — the web UI should support more than one engine profile (local, staging, a colleague's
Docker instance). This is the main capability the web UI has that Electron does not.

**Do not let the UI look frozen when the engine drops.** The most common complaint about browser-based
tools against a remote backend is a UI that silently stops updating. Wire connection state into the
existing global footer component.

---

## Work item 3 — Replace native dialogs with blob flows

Every `dialog:*` call site becomes a browser file input or a download.

> **Corrected 2026-09-22 — see work item 0.** The `dialog:*` channels in the table below **no longer exist**;
> P6's dead-channel reduction removed them, and a grep for `dialog:` under `renderer/` returns 0. The live
> surface is `window.api.<method>`, where **16** methods are file/dialog-shaped rather than 8. Keep the table
> for its component column; re-derive the channel column before ticking the criterion.

| Renderer location | Current | Replacement |
|---|---|---|
| `components/modals/ImportExportModal.tsx` | `exportData` → engine dialog | `export.create` → blob → `<a download>` |
| `components/modals/ImportExportModal.tsx` | `preflightImport` → engine dialog | `<input type="file">` → `blob.put` → `import.preflight` |
| `components/common/BinaryViewer.tsx:58` | `openFileDialog()` | file input |
| `components/editor/MultipartEditor.tsx:67` | `openFileDialog()` | file input |
| `components/grpc/ProtoExplorer.tsx:147` | `openFileDialog()` | file input |
| Runner report export | `runner:exportReport` dialog | blob → download |
| Audit export | `audit:export` dialog | blob → download |
| Capture share | `capture:shareJson` dialog | blob → download |

**Constraint:** browsers cannot stream a download without buffering it in memory, and large exports
(workspace zip, big HAR) will be a problem. Mitigations:
- Use the engine's blob URL with a `Content-Disposition` header if the engine can serve HTTP, or
- Chunked `blob.read` into a `Blob` and trigger the download, or
- Accept the memory cost and document the limit.

Pick one deliberately; do not discover it when a user exports a 500 MB workspace.

**Certificate install is the hard case.** A browser cannot install a CA into the OS trust store. The web
UI can only:
- Offer the cert as a download,
- Show per-platform instructions,
- Show the fingerprint so the user can verify.

That is a genuine capability gap versus the desktop shell. **Say so in the UI** rather than leaving
users stuck.

---

## Work item 4 — Substitutions for shell-only capabilities

| Electron-only | Web replacement |
|---|---|
| `window.api.platform` | `navigator.platform` / UA-CH, or better: ask the engine nothing and infer locally |
| `setTitleBarOverlay` | N/A — browser chrome |
| `getZoomLevel` / `setZoomLevel` | Browser zoom; the renderer never used these (verified) |
| `getTheme` / `setTheme` | Read from engine config; apply locally |
| `isFirstLaunch` / `completeFirstLaunch` | Engine config flag, not shell state |
| `shell:openExternal` | `window.open` |
| Tray / always-on | **Not available.** Document it. |

---

## Work item 5 — Build and host

- Reuse the Vite config (`vite.config.ts`, root `renderer`, alias `@` → `renderer`). **Under D18 this is no
  longer "reuse" — see §"The architecture this phase targets": the shared UI gets its own config, and each
  host's config adds only its own entry.**
- Add a second entry or a build mode for the web target.
- Output a static bundle; serve from any static host, or have the **engine serve it** — which is
  attractive for Docker, since the engine and UI then ship as one artifact.
- If the engine serves it, the UI is same-origin with the RPC endpoint, which simplifies auth and
  removes CORS entirely. **Recommended for the Docker story.**

---

### The first landing (2026-09-22) — `apps/web`

A new workspace app, `apps/web`, that does exactly one thing:
`createClient(createBrowserTransport({ url }), { local: createBrowserLocal() })` against a real engine, and
report the result. No framework, no UI — so that a failure here is a failure of the **client** and not of a
screen built on top of it. `?engine=ws://host:port` selects the endpoint, defaulting to
`ws://127.0.0.1:8080` (D16's win32 base), so one bundle points at a local, a Docker-hosted or a remote
engine without a rebuild.

The split is deliberate: `src/connect.ts` holds the composition and **no DOM**; `src/main.ts` holds the DOM
and no decisions. That is what lets a Node-side suite drive the composition against a real engine over a
real socket — the closest such a suite can get to "it works in a browser".

**On "reuse the Vite config" — corrected by D18.** The *tooling* is reused (Vite 8, the same output shape as
`dist/renderer`, `base: "./"`), but `apps/web` has its **own** `vite.config.ts`, and that was the right call
*for this landing*: the root config sets `root: "renderer"`, registers `@vitejs/plugin-react`, and aliases `@`
into the renderer tree — every one of which would silently apply to a probe page that has no React.

What was wrong was the reasoning's reach. It read as a statement about the phase, and under **D18** the
opposite is true: `apps/web` will import the shared UI *including* its React components. The split that
matters is therefore not "desktop config vs web config" but **"shared-UI config vs per-host config"** — the
React plugin and the `@` alias move to the shared UI's config, and each host's config adds only what its own
entry needs. See §"The architecture this phase targets".

**The engine-serving option stays open but is not taken.** `base: "./"` is what makes the output servable
from a subpath, which is what "have the engine serve it" needs; nothing here wires that up.

**Three defects this increment found, none of them in the new code:**

1. **`build:packages` never built `@bifurc/web-client`.** It listed protocol, engine and client only — so the
   new package's `dist/` was stale from the moment `local.ts` was added, and since a browser bundler
   resolves `@bifurc/web-client` to `dist/`, `createBrowserLocal` **did not exist** there. Fixed by adding it
   to `build:packages` and `typecheck:packages`, adding `apps/*` to `workspaces`, and adding
   `apps/**/*.test.ts` to the vitest `unit` include — without which the app's suite would never have run,
   because a glob that looks like it covers everything is how a suite goes silently unexecuted. `build:web`
   is deliberately **not** part of `npm run build`: the desktop build should not build the web target.
2. **`channel.ts` reported a dropped session as "Could not connect", and never bounded the connect.**
   `onerror` and `onclose` both called `fail()` unconditionally, and `fail()` is idempotent — so whichever
   fires first wins, and a browser fires `error` **then** `close` when an *established* connection drops
   abnormally, which makes the message a function of **event order** rather than of whether the socket ever
   opened. `onerror` now reports **only while the socket has never opened** (`if (open) return`), and
   `onclose` distinguishes the two cases the same way. Separately, `ensureReady()` armed no timer at all, so
   a peer that accepts the TCP connection and then answers nothing left the promise unsettled forever;
   `DEFAULT_CONNECT_TIMEOUT_MS` now covers it. Both are written up in §"The two defects the suite found".
3. **A docblock that *documents* `@vitest-environment` is itself that directive.** See `TRAPS.md`.

**Verified:** `tsc` clean on all five packages/apps; the bundle contains **0 `node:` built-ins, 0
`require(`, 0 `process.env`** and pulls in no `ws` package, while using `WebSocket`, `localStorage` and
`document`. The two `Buffer`/`electron` textual matches in the bundle are a feature-detected
`globalThis.Buffer ?? atob` fallback and protocol *strings* (`"electron-shell"`, surface descriptions) —
checked rather than assumed. What is **not** verified here is a real browser: that is item 6, and the owner
runs it.

## Work item 6 — Browser-specific testing

- Playwright against a real browser + a real engine (not Electron).
- Cover: connect, handshake failure, auth failure, disconnect/reconnect, `resyncRequired`, one blob
  upload, one blob download, one large export.
- Test on Chromium, Firefox and WebKit. Firefox matters here for a second reason: it does not use the OS
  trust store, so proxy testing through the UI will behave differently.

---

## Acceptance criteria

- [ ] Web UI reaches the D8 parity target in a browser.
- [ ] Connects to a local engine and to a remote engine.
- [ ] Handshake failure, auth failure and version mismatch all produce actionable UI, not generic errors.
- [ ] Disconnect/reconnect works; the UI never silently freezes.
- [ ] `resyncRequired` handled visibly.
- [ ] All 8 dialog call sites replaced with browser equivalents. **Re-derive before ticking — the number is
      stale.** WI 0 measured **16** file/dialog-shaped `window.api` methods, and a repo-wide grep for
      `dialog:` under `renderer/` returns **0** because P6's dead-channel reduction deleted those channels.
      Tick this **per method**, not per row of item 3's table.
- [ ] Large export path chosen and documented with a stated size limit.
- [ ] Cert install limitation surfaced in the UI with instructions.
- [ ] Multi-engine profiles work. **Now "host + workspace selection"** — renamed by the P14 note at the top of
      this document (D10–D13), and under **D18** it is *shared* UI rather than browser-only.
- [ ] E2E coverage on Chromium, Firefox and WebKit.

---

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| Silent UI freeze on disconnect | **High** | Connection state in the global footer; explicit reconnect test |
| Large exports blow browser memory | Medium | Decide the strategy in item 3; test with a real workspace |
| Users expect CA install to work in a browser | **High** | Explicit UI copy; this is a genuine gap, not a bug |
| CORS/proxy configuration burden when hosted separately | Medium | Have the engine serve the UI — same-origin, no CORS |
| Divergent client code between web and Electron | Medium | Both use `packages/client`; review for forked shims |
