# 11 — Phase 10: Standalone engine binary ⏸ DE-PRIORITISED

> ## Scope change — 2026-09-14, following D1/D2/D4/D5
>
> **This phase is de-prioritised and P11 below it is parked.** It exists to serve a Tauri sidecar, and
> Tauri is deferred (D4/D5).
>
> | Consumer | Still needs a standalone binary? |
> |---|---|
> | Tauri shell (P11) | Yes — but P11 is parked |
> | Docker (P9) | **No** — the Dockerfile runs `node` + JS. A compiled binary buys nothing there. |
> | CLI (P8) | **No** — ships via npm, or as a single binary later if you want one |
> | Electron shell (P6) | **No** — spawns the engine from `extraResources` |
>
> **Nothing on the critical path depends on this phase.** Run it if you want a single-binary CLI, or
> when the shell migration resumes. Do not let it delay P1–P9.
>
> D2 (`bun build --compile`, pending the P0 spike) remains the intended approach and is unchanged.

**Goal:** a single executable per platform that runs the engine with no `node_modules` and no runtime
installed.

**Effort:** 1–2 weeks. **Depends on:** P2 (engine package). **Blocks:** P11 (Tauri sidecar) — parked.

> This phase exists because a Tauri shell needs a sidecar binary, not a Node project. P0 spike 2 answers
> D2 — this phase executes that answer.

---

## Preconditions

- `packages/engine` builds and runs headlessly (P2 gate).
- P0 spike 2 resolved: D2 answered with measured binary size and cold-start time.

---

## Work item 1 — Build pipeline

Per D2. If the spike chose Bun:

```bash
bun build packages/engine/src/index.ts \
  --compile \
  --target=bun-<platform>-<arch> \
  --outfile dist/engine-<triple>
```

If it chose Node SEA or `node` + JS, the pipeline differs but the interface is the same.

### Target triples

Tauri's sidecar mechanism requires the binary filename to carry a target triple suffix:

```
engine-x86_64-pc-windows-msvc.exe
engine-x86_64-apple-darwin
engine-aarch64-apple-darwin
engine-x86_64-unknown-linux-gnu
```

Name them this way from the start — retrofitting the naming later means touching the Tauri config, the
build scripts and CI at once.

---

## Work item 2 — What must not break in a compiled binary

The P0 spike validated the dependency set, but verify each of these explicitly in the real engine, not a
toy entry point:

| Concern | Why | Check |
|---|---|---|
| `mkcert` `createCA` / `createCert` | Pure JS (`node-forge`), but a bundler can still mangle it | Generate a CA and verify the PEM parses |
| `unzipper.Open.file()` | Needs a real path; must resolve at runtime | Round-trip a workspace zip |
| `archiver` → `WriteStream` | Streams to a path | Same round-trip |
| `simple-git` | Spawns the external `git` binary | Assert git is found and version-parsed |
| `ws` | Native-ish; some bundlers break it | Server accepts a connection |
| `node:child_process` in `processSpawner` | Spawns arbitrary user commands | Spawn a trivial child |
| `crypto` for token + checksums | Must be the real implementation | Generate a token, hash a blob |
| Dynamic `require` | **The classic compiled-binary failure** | `grep -rn "require(" packages/engine/src` and eliminate or convert |

**Dynamic `require` is the most likely breakage.** Check for it before the first build, not after.

---

## Work item 3 — Runtime dependencies that remain external

The binary does not remove these; document them as requirements:

| Dependency | Required? | Consequence if absent |
|---|---|---|
| `git` | **Required** | Config versioning and sync do not work. Preflight fails. |
| `certutil` / `security` / `trust` | Only for CA trust | Not needed in the engine anymore (moved to the client in P3). **The engine no longer needs root.** |
| A writable data dir | **Required** | Cannot start |

The preflight check from P2 reports these clearly, which matters more now that there is no `npm install`
to hint at what is missing.

---

## Work item 4 — Verify on a clean machine

**Do not verify on your dev machine.** It has Node, npm and `node_modules` available, which can mask a
missing dependency.

Verification matrix — each cell is a fresh VM or container with nothing pre-installed:

| Platform | Test |
|---|---|
| Windows 11 (no Node) | Start engine, connect via CLI, run a proxy request |
| macOS (no Node) | Same |
| Ubuntu 24.04 (git only) | Same |
| Debian stable (git only) | Same |

For each: record binary size, cold-start time, and peak RSS.

---

## Work item 5 — Build in CI

Add a matrix job producing all four binaries as release artifacts. The P12 release workflow consumes them.

```yaml
strategy:
  matrix:
    include:
      - { os: windows-latest, triple: x86_64-pc-windows-msvc }
      - { os: macos-14,       triple: aarch64-apple-darwin }
      - { os: ubuntu-latest,  triple: x86_64-unknown-linux-gnu }
```

Note macOS is built on `macos-14` (arm64) — cross-compiling to x86_64 macOS from arm64 is possible but
requires the right toolchain. Build natively where you can.

---

## Acceptance criteria

- [ ] One binary per platform, correctly triple-suffixed for Tauri.
- [ ] Runs with **no `node_modules` present** on a machine without Node installed.
- [ ] CA generation produces a valid PEM pair.
- [ ] Workspace zip export and import round-trip.
- [ ] Proxy serves a real request end to end.
- [ ] `ws` server accepts a connection.
- [ ] Child process spawn works (`applications` feature).
- [ ] No dynamic `require` remains in the engine.
- [ ] Missing `git` → clear preflight error, non-zero exit, no crash.
- [ ] Binary size and cold-start recorded for each platform.
- [ ] CI matrix builds all four on every release tag.

---

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| A dependency breaks only in the compiled binary | **High** | Verify the real engine, not a toy entry point; test on a clean VM |
| Dynamic `require` survives to the build and fails at runtime | High | Grep before building |
| Verifying on a dev machine masks a missing dependency | **High** | Clean VM/container per platform, no Node installed |
| macOS x86_64 cross-compile problems | Medium | Build natively on the right runner |
| Binary size balloons past what Tauri's installer can absorb | Low | Measure at the gate; Node SEA is larger than Bun |

---

# 12 — Phase 11: Tauri shell ⏸ PARKED

> ## PARKED — 2026-09-14, per D1, D4 and D5
>
> **Do not start this phase.** The decisions were explicit:
>
> - **D4:** *"keep it electron right now we will see about the shell migration later. Decoupling is the
>   focus right now."*
> - **D5:** *"Tauri is deferred, we keep electron for now until the decoupled engine matures."*
>
> **This document is retained intact as the plan of record for when the migration resumes.** Everything
> in it was verified against source and remains accurate. It is parked, not abandoned.
>
> ### Why parking it is a good trade
>
> Parking P11 removes **two of the three highest-risk items** from the programme:
>
> | Risk | Before | Now |
> |---|---|---|
> | MSIX cannot spawn a supervised engine | High, on the critical path | **Removed** — Electron stays, electron-builder's APPX already works |
> | WebKitGTK `oklch()` / `color-mix()` failures | High, blocked Linux | **Removed** — Electron ships Chromium |
> | Bun compile against the real deps | Medium | **Downgraded** — blocks nothing |
>
> Cost: roughly **6–8 engineer-weeks** and the install-size win. Benefit: a materially lower-risk
> critical path, and a decision that is now *reversible* rather than load-bearing.
>
> ### What makes resuming cheap
>
> The decoupling work in P1–P9 is not wasted by parking this — it is the **precondition** for it. After
> P6 the shell is disposable, and the RPC client built in P5 is the same client Tauri uses. Resuming
> means running P0 spikes 1 and 3 (their methods are already written), then P10, then this.
>
> ### Before starting, if you do resume
>
> 1. Re-read D4/D5 in `00-decisions.md` and confirm the decision has actually changed.
> 2. Run P0 spike 3 (WebKitGTK) and spike 1 (MSIX) — both are preconditions.
> 3. Re-measure the Electron baseline (`baseline.md`) so the size win is provable, not assumed.

**Goal:** replace the Electron shell with a Tauri v2 shell that reaches feature parity, at a fraction of
the install size.

**Effort:** 3–4 weeks. **Depends on:** P5 (client), P10 (binary). **Status: parked.**

> Everything up to and including P9 already delivers the decoupled engine, web UI, CLI and Docker image.
> This phase is only about the desktop shell.

---

## Preconditions

- P0 spike 3 resolved: D4 answered (WebKitGTK CSS compatibility per distro).
- P6 shipped and stable for at least one release cycle.
- P10 gate green: binaries exist and are triple-suffixed.

---

## Work item 1 — Sidecar lifecycle

Tauri v2 supports external binaries via `tauri.conf.json` → `bundle.externalBin`. But **Tauri's sidecar
does not auto-restart**, so you port the supervisor from P6 item 2:

```
1. Resolve data dir (Tauri path API → app data dir)
2. Pick a socket path or ephemeral port
3. Spawn sidecar:  engine --data-dir <p> --transport <t> --token <random>
4. Wait for ready handshake with timeout
5. Connect the RPC client, send `hello`
6. Supervise: restart with backoff; give up after 3 failures in 60s
7. On exit: SIGTERM → wait 5s → SIGKILL
8. Socket-close watchdog so a crashed shell does not orphan the engine
```

Reuse the P6 logic conceptually — but it is a **rewrite in Rust**, not a port. Budget for that.

---

## Work item 2 — Feature parity checklist

Every item must have a Rust implementation or a documented decision to drop it.

| Electron capability | Source | Tauri approach |
|---|---|---|
| Tray icon + menu | `main.ts:61–116` | Tauri `tray-icon` feature. Menu is 3 items (Open / minimizeToTray checkbox / Quit) — small. |
| Minimize-to-tray on close | `main.ts:229` | Window close handler + config-backed flag |
| Tray menu reflects config | `updateTrayMenu()` | Subscribe to `settings.changed` on the bus |
| Hidden titlebar + overlay theming | `main.ts:176–181`, `syncTitleBarOverlay()` | `decorations: false` + custom drag regions. **Different mechanism — a rewrite.** |
| DPI-derived default zoom | `computeDefaultZoomForDisplay()` | Webview zoom. **Inconsistent across WebView2 / WKWebView / WebKitGTK — expect per-platform work.** |
| `Ctrl+=/-/0` zoom shortcuts | `main.ts:200–225` | Rust key handler + persisted level |
| Single instance | `main.ts:239` | `tauri-plugin-single-instance` |
| First launch | `app:isFirstLaunch` | Engine config flag |
| Native dialogs | `dialog:*` | `tauri-plugin-dialog` |
| `shell.openExternal` | `systemHandlers.ts:206` | `tauri-plugin-opener` |
| Auto-update | `app:checkUpdate` | `tauri-plugin-updater` — different mechanism, rewrite |
| CA install | `tls:installCA` | Rust command shelling out to `certutil` / `security` / `trust` |

---

## Work item 3 — Webview compatibility

This is the phase's main technical risk, and why P0 spike 3 exists.

`oklch()` in 7 renderer files, `color-mix()` in 2 — including `tokens.css`, `styles.css`,
`codemirrorTheme.ts` and `TitleBar.tsx`.

| Webview | Platform | Status |
|---|---|---|
| WebView2 (Chromium) | Windows | Fine |
| WKWebView | macOS | Fine (Safari 15.4+ / 16.2+) |
| WebKitGTK | Linux | **Depends on distro version** — see the P0 matrix |

If spike 3 showed failures on distros you support, add `@supports` fallbacks with hex equivalents for
the affected files (1–2 weeks, budgeted here).

---

## Work item 4 — Performance expectations, stated honestly

Set expectations internally before this phase, or it will be judged on the wrong axis.

| | Electron | Tauri |
|---|---|---|
| Install size | ~150–250 MB | **~10–20 MB** |
| Idle RAM | ~120–200 MB | **~40–80 MB** |
| UI runtime perf | Chromium | **≈ parity on Windows** (WebView2 *is* Chromium); possible Linux regression |
| Backend RAM | Node | **Unchanged** |

The win is **size and memory**, not speed. On Windows you are unbundling Chromium, not removing it.

---

## Work item 5 — Keep the Electron shell alive

Do not delete the Electron shell in this phase. Run both for at least one release.

- The Electron shell is the fallback if the Tauri shell has a platform-specific problem.
- Under D5, it may also remain the **Microsoft Store** artifact, since Tauri has no MSIX target.
- Deleting it is a separate, later decision made on data.

---

## Acceptance criteria

- [ ] Tauri shell reaches parity on every item in work item 2, or has a documented drop.
- [ ] Sidecar spawns, handshakes, and supervises with backoff.
- [ ] No orphaned engine after a forced shell kill.
- [ ] Tray, minimize-to-tray and the config-backed checkbox all work.
- [ ] Titlebar theming matches the design system on all three platforms.
- [ ] Zoom behaves consistently, or the per-platform differences are documented.
- [ ] Install size and idle RAM measured and reported against the Electron baseline.
- [ ] CSS renders correctly on every distro in the P0 matrix.
- [ ] Bundle produced for Windows, macOS and Linux.
- [ ] The Electron shell still builds and ships.

---

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| WebKitGTK CSS failures on older Linux | **High** | P0 spike 3; `@supports` fallbacks; D4 permits a per-platform split |
| Zoom/DPI inconsistency across webviews | **High** | Treat as a known difference; test each platform explicitly |
| Titlebar rewrite diverges visually | Medium | Compare side by side against the Electron build |
| Sidecar lifecycle bugs orphan engines | Medium | Port the P6 supervisor including the socket watchdog |
| Scope creep into removing the Electron shell | Medium | Hard rule: both shells ship for one release |
| Judged as "faster" and found to be "same speed" | Medium | State the expectation table before starting |
