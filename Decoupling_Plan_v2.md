# Decoupling Plan v2 — Headless engine + Tauri shell

**Supersedes:** §6 (workstreams 6–9), §7 (packaging) and §8 (phasing) of `Decoupling_Assessment.md`.
Everything else in v1 — protocol design, engine extraction, file-I/O redesign, transport, tests,
security — is **unchanged** and still applies.

**Question addressed:** can the end state be *headless Node/Bun engine + Tauri (or webview) UI*, with
the desktop app spawning the engine, and is that more efficient than the v1 plan?

---

## 1. Verdict

**The end state you describe is correct and worth building.** Engine as a standalone binary, UI as a
thin webview wrapper, three clients on one protocol — that is the right architecture.

**But one premise needs correcting.** You said *"the bridge is what needs to be replaced."* That is
half true, and the half that is false is where the schedule risk lives:

| | True? |
|---|---|
| The **data bridge** (`preload.ts` → RPC client) is small | **True.** Renderer is platform-clean; 207 call sites don't change. |
| The **shell** is just a bridge | **False.** `main.ts` is 336 lines of real desktop behaviour: tray menu with config-backed checkbox state, titlebar overlay theming, DPI-derived default zoom, zoom keyboard shortcuts with persistence, minimize-to-tray-on-close, single-instance focus, first-launch flow, git preflight with an error dialog. Rewriting that in Rust is ~3–4 weeks, not a swap. |

**And the efficiency you're looking for comes from *ordering*, not from choosing Tauri earlier.**

Once the engine is decoupled, **the shell becomes disposable** — that is the entire point of the
exercise. So:

> Doing the decoupling **first** turns the Tauri migration into a ~10–14 week *isolated, deferrable,
> reversible* project. Doing Tauri **first** (or concurrently) entangles three independent unknowns:
> engine extraction **+** webview compatibility **+** a custom MSIX build.

**Recommendation: build the seam, keep Electron as the shell for one release cycle, then swap to Tauri
as a separate project.** You lose nothing — the RPC client you write for Electron is the same one Tauri
uses — and you de-risk the entire programme.

---

## 2. What I verified in the codebase

### Confirmations (things that make your plan cheaper)

| Finding | Evidence |
|---|---|
| **Renderer is 100% platform-agnostic** | `renderer/types/window.ts:193` declares `platform: string`, but it is **never read anywhere** in the renderer. No `navigator.userAgent` sniffing. Zero platform branches. Fully portable to any webview. |
| **Tray menu is trivial** | `main.ts:78` — three items: Open, a `minimizeToTray` checkbox, Quit. Small Rust rewrite, not a subsystem. |
| **Zoom is shell-only** | `renderer/` has **zero** references to `zoomLevel`. It is entirely a `main.ts` + `systemHandlers.ts` concern, so the renderer doesn't care how zoom is implemented. |
| **Engine has no asset-path coupling** | `__dirname` / `process.resourcesPath` appear **only** in `main.ts` (lines 38–39, 171, 184 — all shell). No engine module resolves assets from disk. **This is the key enabler for compiling the engine to a standalone binary** — nothing will break on missing `node_modules`. |
| **Cert generation is pure JS** | `mkcert` v3.2.0 depends only on `node-forge` + `commander`. No bundled native binary, no `spawn`. *(This corrects a claim in v1.)* Compiles and containerises cleanly. |

### Risks (things that make it more expensive than it looks)

| Finding | Evidence |
|---|---|
| **WebKitGTK CSS compatibility** | `oklch()` in **7 files**, `color-mix()` in **2** — including `renderer/tokens.css`, `styles.css` and `lib/codemirrorTheme.ts`. On Windows (WebView2 = Chromium) and macOS (WKWebView) this is fine. On **Linux (WebKitGTK)** support depends on the distro's WebKit version — older LTS releases will render the entire design system wrong. |
| **MSIX is not a first-class Tauri target** | Tauri v2's Windows bundler produces **NSIS + MSI (WiX)**. There is no `appx`/`msix` target. Your Microsoft Store artifact would have to be authored separately from a Tauri layout — *more* custom work than today, where electron-builder does it for you. |
| **Zoom / DPI behaviour** | `computeDefaultZoomForDisplay()` + `Ctrl+=/-/0` shortcuts with persistence (`main.ts:139–225`) depend on `webContents.setZoomLevel`, which is uniform in Electron. Webview zoom is inconsistent across WebView2 / WKWebView / WebKitGTK. Expect to re-solve this per platform. |
| **Titlebar is a rewrite, not a port** | `titleBarStyle: "hidden"` + `titleBarOverlay: { color, symbolColor, height }` synced to theme, with `syncTitleBarOverlay()` compensating for zoom. Tauri uses `decorations: false` + drag regions — different mechanism. `TitleBar.tsx` also uses `oklch`/`color-mix`, so it's in the compat blast radius. |
| **CA trust is host-scoped** | `certManager.ts:41` shells out: `certutil -addstore -user Root` (Win), `security add-trusted-cert` (macOS), `sudo update-ca-certificates` (Linux). For a remote engine the CA must be trusted by the **client's browser**, not the container. See v1 §7.7. |

---

## 3. The performance claim — what you actually get

Worth being precise, because the win is real but narrower than "no Electron bloat":

| | Electron (today) | Tauri v2 |
|---|---|---|
| Installer / install size | ~150–250 MB | **~10–20 MB** — biggest win |
| Idle RAM | ~120–200 MB | **~40–80 MB** — real win |
| UI runtime performance | Chromium | **≈ parity on Windows** (WebView2 *is* Chromium); **possible regression on Linux** (WebKitGTK) |
| Backend RAM | Node process | **unchanged** — the engine is still Node/Bun |

So: a large **disk** win, a solid **idle-memory** win, and **no guaranteed UI performance win** —
possibly a Linux regression. Also note that on Windows you are not removing Chromium, you are
*unbundling* it: WebView2 is still Chromium, just a shared system runtime (pre-installed on Win11,
usually present on Win10 via Edge, and declarable as an MSIX dependency).

Frame it internally as **"smaller and lighter"**, not **"faster"**.

---

## 4. New workstream: standalone engine binary

This does not exist in v1 and is a prerequisite for your step 3.

| Approach | Size | Risk | Notes |
|---|---|---|---|
| **`bun build --compile`** | ~50–90 MB | Medium | Best startup time. Needs a spike: `ws`, `simple-git`, `archiver`, `unzipper` under Bun. |
| **Node 20+ SEA** | ~80–110 MB | Low–Medium | Embeds Node. Well-supported, larger. |
| **Ship `node` + bundled JS** | ~45–60 MB extra | **Very low** | Most boring, most reliable. No compile step, no compat questions. |

The dependency surface is actually compile-friendly: `mkcert` (pure JS), `node-forge`, `ws`,
`archiver`, `unzipper` are all pure JS. The only subprocess users are `simple-git` (spawns `git`) and
`certManager` (spawns `certutil`/`security`/`sudo`) — both fine in a compiled binary, both requiring
the external tool to exist at runtime.

**Recommendation:** spike Bun compile in Phase 0. Fall back to shipping `node` + JS if it fights you —
that is a legitimate choice and costs only installer size.

### Spawn / handshake model (your step 3)

```
Tauri shell starts
  → picks an ephemeral port (or unix socket / named pipe on Windows)
  → spawns engine sidecar with --port=N --token=<random> --data-dir=<path>
  → engine prints "ready" on stdout (or writes a handshake file)
  → shell connects over WebSocket, authenticates with token
  → shell supervises: restart on crash with backoff
  → on quit: SIGTERM engine, wait, SIGKILL
```

Three things to get right, none of which exist today:

1. **Port allocation.** `cfg.companionPort ?? 9271` is a fixed default. A spawned engine must take an
   ephemeral port and report it back, or use a unix domain socket (macOS/Linux) / named pipe (Windows)
   to avoid collisions entirely. Prefer the socket — it also sidesteps the MSIX loopback question.
2. **Handshake.** The shell must know the engine is ready and authenticated before showing UI.
3. **Supervision.** Tauri's sidecar does **not** auto-restart. You need a supervisor loop with backoff,
   plus crash reporting into the UI.

---

## 5. Revised estimate

### 5.1 Workstreams

Unchanged from v1 (the bulk):

| # | Workstream | Low | High |
|---|---|---:|---:|
| 1 | Protocol design & freeze | 2.0 | 3.0 |
| 2 | Engine extraction | 4.0 | 6.0 |
| 3 | File-I/O semantics redesign | 1.0 | 2.0 |
| 4 | Transport layer | 3.0 | 4.0 |
| 5 | Typed RPC client | 1.5 | 2.0 |
| 10 | Tests & migration | 3.0 | 4.0 |
| 11 | Security, observability, docs | 2.0 | 3.0 |
| | **Subtotal (unchanged)** | **16.5** | **24.0** |

Changed or new:

| # | Workstream | v1 | **v2** | Why |
|---|---|---|---:|---|
| 6a | Electron shell as thin host (interim) | 2.0–3.0 | **2.0–3.0** | unchanged — you still need this bridge release |
| 6b | **Standalone engine binary** | — | **1.0–2.0** | new — §4 |
| 6c | **Tauri shell rewrite (Rust)** | — | **3.0–4.0** | new — tray, window, titlebar, zoom, single-instance, lifecycle |
| 6d | **Webview compatibility work** | — | **1.0–2.0** | new — `oklch`/`color-mix` fallbacks, titlebar, zoom/DPI per platform |
| 7 | Web UI | 3.0–4.0 | **3.0–4.0** | unchanged (renderer now shared with Tauri) |
| 8 | CLI | 3.0–5.0 | **3.0–5.0** | unchanged |
| 9 | Packaging & distribution | 3.0–5.0 | **5.0–6.5** | MSIX becomes custom-built from a Tauri layout (§2) |
| | **Subtotal (changed/new)** | | **18.0** | **26.5** |

| | Low | High |
|---|---:|---:|
| Subtotal | 34.5 | 50.5 |
| Contingency (25%) | 8.6 | 12.6 |
| **Total — full end state** | **43** | **63** |

**Central estimate: ~45–55 engineer-weeks**, versus **~40–45** for the v1 Electron-only path.

**So the Tauri migration costs roughly +10 engineer-weeks over keeping Electron.** That is a fair price
for a ~10× smaller installer — but only if you *choose* it, not if it sneaks in as an assumed freebie.

### 5.2 The number that actually matters

The total is not the useful figure. The **tier** structure is:

| Tier | Content | Effort | Cumulative |
|---|---|---:|---:|
| **1 — Seam** | Protocol, engine extraction, file-I/O design, Electron thin host. Ships as a normal release, **zero user-visible change**. | **10–14 w** | 10–14 w |
| **2 — Headless clients** | Transport, RPC client, web UI, CLI, Docker. Still Electron on the desktop. | **12–17 w** | 22–31 w |
| **3 — Tauri swap** | Standalone binary, Rust shell, webview compat, Tauri bundling, custom MSIX. | **10–14 w** | 32–45 w |

**Tier 3 is fully deferrable.** You can ship Tier 1 and Tier 2 — decoupled engine, web UI, CLI, Docker
image, all working — and decide on Tauri a year later without having made a single decision that
constrains you. That is the efficiency you were looking for.

---

## 6. Shell options compared

| Option | Shell work | Install size | Idle RAM | MSIX | New risk |
|---|---:|---|---|---|---|
| **Keep Electron** | ~0 — already works | ~150–250 MB | ~120–200 MB | Works today | None |
| **Tauri v2** | 7–11 w | ~10–20 MB | ~40–80 MB | Custom build | WebKitGTK CSS, zoom/DPI |
| **Webview only** (no shell) | ~1 w | n/a | n/a | n/a | Loses tray + always-on — you said you want these |

Note that **all three consume the identical RPC client**. Whichever you pick, the engine work is the
same, so the decision is genuinely reversible — provided you do Tier 1 first.

---

## 7. Recommended sequencing

```
Phase 0 — De-risk (2–3 w)   ← start here
  ├ protocol sketch + schema generation
  ├ MSIX loopback / child-process spike          (v1 §7.3)
  ├ Bun compile spike on the real dependency set (§4)
  └ WebKitGTK CSS check: does the design system survive? (§2)
        → run tokens.css + a CodeMirror instance in WebKitGTK 2.40 and 2.44

Phase 1 — Engine (4–6 w)
  └ v1 workstream 2 + 3, unchanged

Phase 2 — Seam (4–6 w)      ← SHIP THIS. Zero user-visible change.
  └ Electron shell stays; preload becomes an RPC client; window.api unchanged;
    35 unit suites + 11 e2e suites must stay green

Phase 3 — Headless clients (12–17 w, parallelisable)
  └ transport, web UI, CLI, Docker. Electron still the desktop shell.

  ── everything above is on the critical path. everything below is optional. ──

Phase 4 — Tauri swap (10–14 w)   ← defer freely
  └ standalone binary, Rust shell, webview compat, Tauri bundling, custom MSIX
```

**Phase 0's WebKitGTK check is the one that could change your mind**, so do it early and cheaply. If
`oklch()`/`color-mix()` don't survive WebKitGTK on your target distros, Tier 3 grows by a few weeks of
fallback work — or you decide Tauri is a Windows/macOS-only shell and keep Electron on Linux, which is
a perfectly reasonable split.

---

## 8. Corrections to v1

| v1 claim | Corrected |
|---|---|
| "`mkcert` bundles platform binaries — verify Linux availability and multi-arch" | **Wrong.** `mkcert` v3.2.0 is pure JS (`node-forge`). No native binary. Cert *generation* is portable. |
| "Highest risk: MSIX hosting" | Still true, but the risk **changes shape** under Tauri: it's no longer "can we spawn a child from a package" but "we must author MSIX by hand because Tauri has no MSIX target." |
| — | **New risk not in v1:** WebKitGTK CSS compatibility for the `oklch`/`color-mix` design system on Linux. |
| — | **New risk not in v1:** CA trust install is host-scoped, so it cannot run in the engine for remote clients (v1 §7.7 added). |

---

## 9. Decisions this creates

1. **Ordering** — confirm Tier 1 ships with Electron intact, and Tauri is deferred to a separate
   project. (Strongly recommended.)
2. **Engine binary** — Bun compile, Node SEA, or ship `node` + JS? (§4)
3. **Transport for spawned engines** — ephemeral TCP port, or unix socket / named pipe? (Socket
   recommended; it also sidesteps the MSIX loopback question.)
4. **Linux shell** — Tauri everywhere, or Tauri on Windows/macOS and Electron on Linux, if WebKitGTK
   fails the Phase 0 check?
5. **MSIX ownership** — who authors and signs the Store artifact once Tauri's bundler stops producing it?
6. **Bun vs Node for the engine generally** — worth deciding once, since it affects the compile target,
   the Docker base image, and the CLI runtime.

---

*v2 assessment based on the same repository state as v1 (`bifurc-monorepo` v0.3.1). New findings in §2
were measured directly from source; the `mkcert` correction was verified against
`node_modules/mkcert@3.2.0`.*
