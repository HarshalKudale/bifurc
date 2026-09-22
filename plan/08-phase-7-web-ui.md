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

- Reuse the Vite config (`vite.config.ts`, root `renderer`, alias `@` → `renderer`).
- Add a second entry or a build mode for the web target.
- Output a static bundle; serve from any static host, or have the **engine serve it** — which is
  attractive for Docker, since the engine and UI then ship as one artifact.
- If the engine serves it, the UI is same-origin with the RPC endpoint, which simplifies auth and
  removes CORS entirely. **Recommended for the Docker story.**

---

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
- [ ] All 8 dialog call sites replaced with browser equivalents.
- [ ] Large export path chosen and documented with a stated size limit.
- [ ] Cert install limitation surfaced in the UI with instructions.
- [ ] Multi-engine profiles work.
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
