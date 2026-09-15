# 08 — Phase 7: Web UI

**Goal:** run the existing renderer in a browser, connected to a local or remote engine over the RPC
transport.

**Effort:** 3–4 weeks. **Depends on:** P6. **Parallel with:** P8, P9.

> The renderer is already browser-clean — **zero** Node or Electron APIs, no `navigator.userAgent`
> sniffing, and `window.api.platform` is declared but never read (verified). Most of this phase is
> connection UX and the file-handling substitutions, not porting UI.

---

## Preconditions

- P6 shipped. Engine is a real, separately-addressable process.
- D8 answered (parity target: full or inspection + light editing).
- P3 complete — the blob flows are what replace native dialogs.

---

## Work item 1 — Browser RPC client

`packages/client` already has a `ws` transport. In the browser, the only new piece is discovery:

```ts
// apps/web/src/connect.ts
const client = createClient(createWsTransport({ url, token }));
await client.hello({ clientName: "web", clientVersion: VERSION });
```

The browser client uses the **same** `createClient` and the same shims. No divergence.

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
