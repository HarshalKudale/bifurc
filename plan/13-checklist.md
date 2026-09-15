# 15 — Master checklist

The literal ordered task list. Phase docs explain *why*; this file is *what to do, in order*.

Tick items as they complete. Do not start a phase until its gate block is fully ticked.

> **Updated 2026-09-14** for the resolved decisions (D1–D9) and the flattened repo. Track C is parked;
> P8 is narrowed; P0 is down to one required spike.

---

## If you do nothing else today, do these four

1. **Run the handler census** and save the raw output — P1 needs it regardless:
   ```bash
   grep -rn "ipcMain.handle" src --include=*.ts > /tmp/handlers.txt
   wc -l /tmp/handlers.txt          # expect 112
   grep -rn "BrowserWindow\|app.getPath\|dialog\.\|shell\." src --include=*.ts | grep -v src/main.ts > /tmp/coupling.txt
   wc -l /tmp/coupling.txt          # expect ~27
   ```
2. **Capture a green baseline** before anything changes:
   ```bash
   npm run test:coverage 2>&1 | tail -20    # record pass counts
   npm run test:e2e 2>&1 | tail -20         # record pass counts
   ```
   Save the numbers in `plan/baseline.md`. Every later phase compares against them.
3. **Read `../File_Ops_Protocol.md` §6.3** — the Firefox trust-store issue. It affects design decisions
   you are about to make, and it is cheap to accommodate now.
4. **Check where `Cleanup_plan.md` actually stands** (D6). It is a hard prerequisite to P2, so its
   remaining size determines when P2 can start. If it is large, decide now whether to do only its
   Phase 1–2 before P2.

*Decisions D1–D9 are already answered — ✅ done 2026-09-14.*

---

## Section 0 — Decisions ✅ DONE

- [x] D1 Ordering — seam first, Tauri deferred → **(a)**
- [x] D2 Engine runtime — Bun / Node SEA / node + JS → **(a), pending P0 spike 2; fallback (c)**
- [x] D3 Transport — socket or ephemeral port → **(b) socket, TCP fallback**
- [x] D4 Linux shell → **keep Electron; revisit later**
- [x] D5 MSIX ownership → **Tauri deferred; Electron stays**
- [x] D6 `Cleanup_plan.md` sequencing → **(a) cleanup first, prerequisite to P2**
- [x] D7 CLI scope → **(a) lifecycle + read-only**
- [x] D8 Web UI parity → **(b) inspection + light editing**
- [x] D9 Versioning → **(a) lockstep**, with the extension exception
- [x] Decision log written in `00-decisions.md`
- [x] Repo flattening recorded (extension extracted, app at root)

**Gate:** ✅ complete. → start P0.

---

## Section 1 — P0 De-risk (1 week)

**Only spike 4 is required.** Spikes 1 and 3 were parked with the Tauri decision (D4/D5); spike 2 is
optional and blocks nothing.

### Spike 4 — Protocol proof (1 week) ✅ REQUIRED

- [ ] Pick 10 representative commands (see the list in `01-phase-0-derisk.md`)
- [ ] Choose schema library — **Zod recommended**
- [ ] Define the 10 schemas
- [ ] Generate TS types
- [ ] Write a typed client
- [ ] Wire those 10 into the renderer behind the existing `window.api` names
- [ ] **Confirm the renderer needed no edits** — this is the whole thesis
- [ ] Record any place it *did* need edits — highest-value output of the spike

### Spike 2 — Bun compile (3 days) 🔸 OPTIONAL

*Run only if you want a single-binary CLI or intend to resume P10. Blocks nothing.*

- [ ] Write `spike/bun-entry.ts` exercising all 8 risky dependencies
- [ ] `bun build --compile`
- [ ] Run with `node_modules` **absent**
- [ ] Verify `createCA`/`createCert` produce a valid PEM pair
- [ ] Verify `archiver` → `unzipper` round-trip
- [ ] Verify `ws` server accepts a connection
- [ ] Verify `simple-git` finds the external `git`
- [ ] Record binary size and cold-start time
- [ ] Record D2

### Spike 1 — MSIX (3 days) ⏸ SKIPPED

*Parked under D5. Run only if the shell migration is revisited.*

- [ ] ~~Confirm Windows Developer Mode~~ — not applicable
- [ ] *(retained in `01-phase-0-derisk.md` if needed later)*

### Spike 3 — WebKitGTK CSS (2 days) ⏸ SKIPPED

*Parked under D5. Run only if the shell migration is revisited.*

- [ ] *(retained in `01-phase-0-derisk.md` if needed later)*

### P0 gate

- [ ] **Spike 4 complete** — renderer-unchanged thesis confirmed or refuted
- [ ] `plan/spike-results.md` written, with skipped spikes recorded as skipped
- [ ] Temporary spike code removed from `main`

---

## Section 2 — P1 Protocol (2–3 weeks) ✅ DONE 2026-09-15

- [x] **Confirm `Cleanup_plan.md` status (D6)** — it gates P2, so know where it stands now

- [x] **Run the classification** — every channel into ENGINE / CLIENT / SPLIT / DROP
- [x] Write `plan/handler-classification.md` (the deliverable, not a by-product)
- [x] Collapse ~36 CRUD channels → 6 generic commands
- [x] Document per-kind CRUD quirks (`rule`/`mock` `unshift`; `mapping` delete cascades to `proxyRules`)
- [x] Fix the command naming convention (`namespace.verb`)
- [x] Write `errors.ts` — error taxonomy with `retryable`
- [x] Write `version.ts` — `PROTOCOL_VERSION`
- [x] Define all command schemas — 89 commands, `packages/protocol/src/commands/`
- [x] Define the event envelope with `seq`
- [x] Define `subscribe` / `unsubscribe`
- [x] Define the `hello` handshake + capability negotiation
- [x] Set up `packages/protocol` with `tsup` (ESM + CJS + `.d.ts`)
- [ ] Add an ESLint rule banning `node:*` imports in `packages/protocol` — deferred to P2 (no ESLint
      config exists in this repo yet); verified manually by grepping the built output instead
- [x] Smoke test: valid and invalid payload per command — 179/179 passing
- [x] **Freeze the protocol.** Create `plan/protocol-changes.md` for any later change

**Gate:** `@bifurc/protocol` builds; smoke server + client round-trip; schemas frozen. ✅

---

## Section 3 — P2 Engine extraction (4–6 weeks) 🟡 IN PROGRESS 2026-09-15

- [x] **`Cleanup_plan.md` Phase 1–2 complete** — prerequisite, not a parallel track (per D6 gate
      assessment in `plan/README.md`; 2.9 still folds into P3 as documented there)
- [x] Build the EventBus (copy `logEmitter`'s pattern) — `src/eventBus.ts`
- [x] Convert broadcast site **#1** — `src/ipc/handlers.ts:24–49` — run the suite
- [x] **Data dir injection** (`appSettings.ts:79`, `workspaceFs.ts:40`) + `paths.ts` — `paths.ts`
      built and unit-tested (`tests/store/paths.test.ts`, 8/8) **and now wired as the primary
      path**; `main.ts` calls `setDataRoot(app.getPath("userData"))` at the top of
      `app.whenReady()`, so existing installs see no path change. The two `*Override` test hooks
      were deliberately kept separate rather than folded in — see the phase doc's work item 4 note.
- [x] Verify what `store/gitStore.ts` actually needs from `electron` — nothing; unused import
      removed
- [x] Convert broadcast site #2 — `coreHandlers.ts:28`
- [x] Convert broadcast site #3 — `entityCrudFactory.ts:29–42`
- [x] Convert broadcast site #4 — `folderHandlers.ts:10`
- [x] Convert broadcast site #5 — `syncHandlers.ts:18`
- [x] Convert broadcast site #6 — `systemHandlers.ts:28` — **resolved as not applicable**:
      on inspection this file's `BrowserWindow` use is `zoom:set`/`shell:setTitleBarOverlay`
      driving actual window chrome (CLIENT-classified per `plan/handler-classification.md`), not
      an engine→client data broadcast. No conversion needed; it stays in the Electron shell when
      the physical move happens.
- [x] Convert broadcast site #7 — `proxy/webhookServer.ts:137` — `src/proxy/` now has **zero**
      Electron imports
- [x] Convert broadcast site #8 — `companion/companionServer.ts:52–83`
- [x] Replace `companion:refresh` with `entity.changed` — internally, at every emission site (the
      wire name survives only in the temporary `src/ipc/eventBridge.ts` shim, by design)
- [x] Break the `coreHandlers` → `main` import cycle
- [x] Remove the `mainWindow` reference from `processSpawner.ts`
- [x] Make engine shutdown idempotent — `src/shutdown.ts` (`shutdownEngine()`, memoised), plus a
      per-entry guard in `processSpawner.stop()` so `stopAll()` is repeatable. `main.ts`'s
      `before-quit` is now one call. Unit-tested (`tests/shutdown.test.ts`, 6/6)
- [x] Split out shell-only handlers (zoom, theme, titlebar, dialogs, openExternal, first-launch) —
      **10 of 15 channels done**: they live in `src/ipc/handlers/clientHandlers.ts` with its own
      `registerClientHandlers()`, unit-tested (`tests/ipc/clientHandlers.test.ts`, 21/21).
      Remaining: the two SPLIT channels (`app:checkUpdate` → P12, `capture:shareJson`) and
      `main.ts`'s tray/window/menu code, which cannot move until `packages/engine` exists.
- [x] Move `tls:installCA` out of the engine — done, into `clientHandlers.ts`
- [ ] Replace the git `dialog.showErrorBox` with `preflight()` — **not done, deliberately**.
      `preflight()` exists and runs in `main.ts`, but the pre-existing git-required block still
      makes its own direct `checkGitInstalled()` call and still shows the dialog, preserving
      current behaviour. Routing it through `preflight()` — and deciding whether
      `data-dir-unwritable` / `port-in-use` / `mkcert-unusable` should become *blocking* — is a
      product decision, left open.
- [x] Move workspace bootstrap out of `main.ts` into the engine — `bootstrapWorkspaces()` in
      `src/startup.ts`; `main.ts` is now a single call and no longer touches
      `workspaceFs`/`gitStore`/`autoSync` directly. Integration-tested with real dirs + real git
      (`tests/integration/workspaceBootstrap.integration.test.ts`, 7/7)
- [x] Replace `registerIpcHandlers()` with the `CommandRegistry` — `src/commands/registry.ts`,
      **~112 commands** across 13 files, covering every `EntityKind` value. Only the P3-bound
      `importExport:*` SPLIT channels remain outside it.
- [ ] Restructure to `packages/*` + `apps/*` workspaces — **partially**: `packages/protocol` is a
      real linked npm workspace (`"workspaces": ["packages/*"]`), built with its own `tsup`
      pipeline and consumed at runtime by `src/commands/registry.ts`. `packages/engine` and the
      file moves are not started.
- [ ] **Split the dependencies** — engine deps out of the flat list — not started
- [ ] Resolve `@/*` aliases at package boundaries — not started
- [ ] `tsup` build for `packages/engine` — not started
- [ ] `git mv` the moved modules (preserve blame) — not started (nothing moved yet)

**Gate:** not green — see the phase doc's acceptance-criteria section for the itemised, honest
status. **Every remaining acceptance criterion is blocked on work item 8** (the physical
`packages/engine` move + dependency split); items 1–7 are otherwise closed apart from the two
items above explicitly left open on product grounds. Verified after the latest changes:
`npm run typecheck` and `npm run build:main` clean; full suite **1615/1616 across 69 files** — the
single failure is the documented sandbox `127.0.0.1:1` connectivity quirk in the P0 spike test,
reproduced identically on the pre-change tree. The 11 e2e specs remain unverified (cannot run in
this sandbox — no desktop session).

---

## Section 4 — P3 File ops (1.5–2.5 weeks)

- [ ] Read `../File_Ops_Protocol.md` in full
- [ ] Build the blob store (`put` / `stat` / `read` / `release`) with tests
- [ ] Blob TTL sweep
- [ ] Size cap enforced **before** allocation
- [ ] Convert `environments-json` exporter **and** importer end-to-end (prove the pattern)
- [ ] Bulk-convert the remaining 30 mechanical files
- [ ] `workspace-zip` exporter (archiver → staged file → blob)
- [ ] `workspace-zip` importer (blob → `unzipper.Open.file()`)
- [ ] Rewire 5 egress channels (dialog-first ordering)
- [ ] Rewire 4 ingress channels (upload once, reference twice)
- [ ] Fix `environments-dotenv.ts:55` name derivation → explicit `filename`
- [ ] Grep for any remaining `filePath.split` / `path.basename(filePath)`
- [ ] Cert: `tls.generate` returns a fingerprint
- [ ] Cert: delete `installCA` from the engine
- [ ] Cert: `removeCert` becomes two-sided
- [ ] Cert: fingerprint drift detection
- [ ] Cert: Firefox warning in the UI
- [ ] Remove all path leaks from protocol types + `renderer/types/window.ts`
- [ ] Update `ImportExportModal.tsx` to carry `blobId` not `filePath`

**Gate:** round-trip green for all 17 formats; no `dialog.*` in the engine; no `filePath` in the protocol.

---

## Section 5 — P4 Transport (3–4 weeks)

- [ ] Define the `Transport` interface
- [ ] Implement `in-process` transport
- [ ] Build the conformance suite skeleton (transport-agnostic)
- [ ] Implement `stdio` transport + framing
- [ ] Generalise `companionServer.ts` into `packages/engine/src/transport/`
- [ ] Enrich the error envelope (`code`, `retryable`)
- [ ] Add the `hello` handshake with version refusal
- [ ] **Auth: token generation, storage (`0600`), validation**
- [ ] **Auth: per-session scope (`read` / `write` / `execute` / `admin`)**
- [ ] Scope enforcement per command class
- [ ] Event delivery with monotonic `seq`
- [ ] Bounded event ring buffer
- [ ] Replay on reconnect (`lastSeq`)
- [ ] `resyncRequired` when replay is impossible
- [ ] `log.entry` coalescing (100 entries / 250 ms)
- [ ] Flow-control window for `log.chunk` / `process.output`
- [ ] Documented drop policy
- [ ] `ws` transport (loopback default; `0.0.0.0` opt-in only)
- [ ] TLS for remote `ws`
- [ ] Rate limiting + per-command timeouts
- [ ] RPC calls in the audit log
- [ ] Verify the **released** companion extension still works (separate repo — cannot be updated atomically)

**Gate:** conformance suite green on all three transports; auth verified; no unauthenticated path.

---

## Section 6 — P5 RPC client (1.5–2 weeks)

- [ ] Write `shape.test-d.ts` **first** — let it fail to compile
- [ ] Implement the client against `in-process`
- [ ] Generate the 129 request/response methods from the protocol
- [ ] Implement the 7 subscription methods with working unsubscribe
- [ ] Compatibility shims in **one** auditable file
- [ ] Client-local methods (dialogs, zoom, theme, titlebar, platform, installCA, first-launch) routed locally
- [ ] Programmatic key-diff script: old `window.api` vs new → zero differences
- [ ] `EngineError` with `code` + `retryable`
- [ ] Idempotency flags; no auto-retry on mutations
- [ ] Subscription multiplexing with reference counting
- [ ] Verify over `stdio` and `ws`

**Gate:** `shape.test-d.ts` compiles; key-diff zero; unsubscribe verified.

---

## Section 7 — P6 Shell seam ✅ SHIP (2–3 weeks)

- [ ] **Baseline re-captured** (35 unit / 11 e2e counts recorded)
- [ ] Build the IPC transport (renderer → main)
- [ ] Route **one** method (`config:get`) through RPC, keep `registerIpcHandlers()` for the rest — suite green?
- [ ] Reimplement `preload.ts` as a client bridge
- [ ] Implement local handlers (dialogs, zoom, theme, titlebar, platform, installCA, first-launch)
- [ ] Engine lifecycle: spawn + ready handshake + timeout
- [ ] Supervision: backoff restart, give up after 3 failures / 60s
- [ ] Shutdown: SIGTERM → 5s → SIGKILL; no orphans
- [ ] Socket-close watchdog so a crashed shell does not orphan the engine
- [ ] Preserve single-instance (one shell → one engine)
- [ ] Add a "connecting to engine" UI state — never block window creation
- [ ] Add the engine binary to `extraResources`; reuse `iconPath()`
- [ ] **Rollback flag** `BIFURC_ENGINE_RPC=0`
- [ ] Verify e2e data-dir isolation still holds (assert engine data dir is in the temp dir)
- [ ] Delete `registerIpcHandlers()` from the default path
- [ ] Run the full suite

**Gate:**
- [ ] 35 unit suites pass (same count as baseline)
- [ ] 11 e2e specs pass, **unmodified**
- [ ] `git diff --stat renderer/` shows only the P3 blob files
- [ ] Key-diff still zero
- [ ] No orphaned engine after a forced kill
- [ ] Packaged NSIS build contains the engine and runs
- [ ] Rollback flag verified
- [ ] **Ship it.** 🎉

> **At this point the project is reversible and the shell is disposable. Everything below is additive.**

---

## Section 8 — Track B: clients (parallel, 7–9 weeks)

### P7 Web UI — inspection + light editing (D8)

- [ ] Browser RPC client over `ws`
- [ ] Connection/auth/version-mismatch/disconnect UX
- [ ] Multi-engine profiles
- [ ] Connection state in the global footer (never silently freeze)
- [ ] Handle `resyncRequired` visibly
- [ ] Replace all 8 dialog call sites with browser equivalents
- [ ] Decide and document the large-export strategy
- [ ] Surface the CA-install limitation with instructions
- [ ] Decide: engine serves the UI (recommended) vs separate static host
- [ ] Playwright e2e on Chromium, Firefox, WebKit
- [ ] *Deferred (D8): full parity — close gaps only after real usage shows where they are*

### P8 CLI — lifecycle + read-only (D7, ~3 weeks)

**In scope for v1:**

- [ ] `engine start | stop | status | logs -f`
- [ ] `remote add | list | use | test`
- [ ] `config get --json`
- [ ] `workspace list | use`, `env list | use`
- [ ] `mocks | requests | rules | mappings list`
- [ ] `tls generate | status | export`
- [ ] `logs tail -f`, `audit list`
- [ ] Attach-before-spawn probe (never two engines on one data dir)
- [ ] Spawn-on-demand over `stdio`
- [ ] `--json` emits the raw protocol result
- [ ] Stable exit codes (0/1/2/3)
- [ ] Ships via npm (single binary deferred with P10)

**Deferred to v2 (D7) — do not build in v1:**

- [ ] *every mutating command: `add` / `rm` / `update` / `send`*
- [ ] *`import` / `export` via the blob layer*
- [ ] *`tls trust` and the Firefox warning*
- [ ] *`config save`, collection runner, gRPC/GraphQL/SOAP execute*

### P9 Docker

- [ ] Run the engine headlessly on the host first
- [ ] Dockerfile: node:22-slim + git + ca-certificates, non-root
- [ ] `/data` volume: config, workspaces, **blobs**
- [ ] Test: blob staging is under the volume
- [ ] `SIGTERM` graceful shutdown
- [ ] RPC on a socket or loopback by default; **not** published
- [ ] **Refuse to start** on a non-loopback bind with no token
- [ ] `/healthz` (leaks nothing)
- [ ] Structured JSON logs on stdout
- [ ] Multi-arch per the P0 decision
- [ ] Document the full CA workflow including Firefox

---

## Section 9 — Track C: shell swap ⏸ PARKED (D1, D4, D5)

> **Do not start.** D4: *"keep it electron right now… Decoupling is the focus right now."*
> D5: *"Tauri is deferred, we keep electron for now until the decoupled engine matures."*
>
> Retained in `11-phase-10-binary-and-11-tauri.md` for when the migration resumes. Parking it removed
> the MSIX and WebKitGTK risks from the critical path entirely, plus ~6–8 engineer-weeks.

### P10 Standalone binary ⏸ de-prioritised

*Nothing on the critical path needs it. Docker uses `node` + JS; the CLI ships via npm.*

- [ ] *Build pipeline per D2 (Bun compile)*
- [ ] *Triple-suffixed filenames*
- [ ] *Clean-VM verification per platform*
- [ ] *CI matrix for all four binaries*

### P11 Tauri shell ⏸ parked

- [ ] *Sidecar spawn + supervisor (Rust)*
- [ ] *Tray, titlebar, zoom, single instance*
- [ ] *CSS fallbacks if spike 3 requires them*
- [ ] *Measure install size + idle RAM vs Electron*
- [ ] *Keep the Electron shell shipping for one release*

---

## Section 10 — P12 Packaging + P13 Hardening (2–3 + 2–3 weeks)

### Packaging — two change points now (P6, P9). No Tauri change point.

- [ ] Engine in `extraResources`; assert it is outside the asar
- [ ] `assert-artifacts` CI job (the icon regression already happened once)
- [ ] NSIS / APPX / AppImage / Docker all publish from CI
- [ ] `app:checkUpdate` moved client-side
- [ ] All artifacts share one version (D9)
- [ ] Store identity (`build.appx`) unchanged by the flattening
- [ ] Grep the workflows for stale `bifurc/` path prefixes
- [ ] ~~MSIX strategy (D5)~~ — not applicable, Electron keeps electron-builder
- [ ] ~~AppImage + WebKitGTK on a clean distro~~ — not applicable while Electron ships

### Hardening

- [ ] Security review with **evidence per item**
- [ ] **Blob path traversal test** (most likely to be missed)
- [ ] Audit every `exec` / `spawn` for injection
- [ ] No unauthenticated path; no "localhost skips auth" shortcut
- [ ] Token `0600`, never logged, rotation works
- [ ] `bifurc doctor`
- [ ] Protocol errors → actionable UI messages
- [ ] Docs: protocol, architecture, docker, troubleshooting, contributing
- [ ] Performance budgets in CI as failing thresholds
- [ ] `typecheck:renderer` clean and gated
- [ ] Coverage thresholds raised
- [ ] `mkId()` widened
- [ ] No guarded e2e specs remain

---

## Continuous — every commit, every phase

- [ ] Full test suite run before every commit
- [ ] One broadcast-site conversion per commit during P2 (never batch)
- [ ] No renderer edits between P1 and P6
- [ ] No `ws` transport merged before auth is complete
- [ ] `plan/protocol-changes.md` updated for any post-freeze protocol change
- [ ] Status table in `README.md` updated at every gate
- [ ] The extension's four commands stay backward-compatible (it is an external repo now)
