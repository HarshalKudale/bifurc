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
- [x] Make engine shutdown idempotent — `packages/engine/src/shutdown.ts` (`shutdownEngine()`, memoised), plus a
      per-entry guard in `processSpawner.stop()` so `stopAll()` is repeatable. `main.ts`'s
      `before-quit` is now one call. Unit-tested (`tests/shutdown.test.ts`, 6/6)
- [x] Split out shell-only handlers (zoom, theme, titlebar, dialogs, openExternal, first-launch) —
      **10 of 15 channels done**: they live in `src/ipc/handlers/clientHandlers.ts` with its own
      `registerClientHandlers()`, unit-tested (`tests/ipc/clientHandlers.test.ts`, 21/21).
      Remaining: the two SPLIT channels (`app:checkUpdate` → P12, `capture:shareJson`) and
      `main.ts`'s tray/window/menu code, which cannot move until `packages/engine` exists.
- [x] Move `tls:installCA` out of the engine — done, into `clientHandlers.ts`. **P3 item 5 finished
      the job**: the channel was in the right place but the *implementation* was still
      `certManager.installCA()` shelling out from inside the engine. It is now
      `certLifecycle.installEngineCa()` → `certTrust.installCA()`, and the engine reports only an
      identity for its CA.
- [ ] Replace the git `dialog.showErrorBox` with `preflight()` — **not done, deliberately**.
      `preflight()` exists and runs in `main.ts`, but the pre-existing git-required block still
      makes its own direct `checkGitInstalled()` call and still shows the dialog, preserving
      current behaviour. Routing it through `preflight()` — and deciding whether
      `data-dir-unwritable` / `port-in-use` / `mkcert-unusable` should become *blocking* — is a
      product decision, left open.
- [x] Move workspace bootstrap out of `main.ts` into the engine — `bootstrapWorkspaces()` in
      `packages/engine/src/startup.ts`; `main.ts` is now a single call and no longer touches
      `workspaceFs`/`gitStore`/`autoSync` directly. Integration-tested with real dirs + real git
      (`tests/integration/workspaceBootstrap.integration.test.ts`, 7/7)
- [x] Replace `registerIpcHandlers()` with the `CommandRegistry` — `src/commands/registry.ts`,
      **~112 commands** across 13 files, covering every `EntityKind` value. Only the P3-bound
      `importExport:*` SPLIT channels remain outside it.
- [x] **`git mv` the moved modules (preserve blame)** — four layers moved, 52 files total.
      **Layer 1** (bottom of the graph): `src/store/` (11 files), `src/lib/` (2),
      `src/subscription/` (1) → `packages/engine/src/`.
      **Layer 2**: `src/proxy/` (19 files), `src/sync/` (8), `src/eventBus.ts` (1) → 28 files.
      These three had to move **together**: `eventBus.ts` imports `@/proxy/logEmitter`,
      `@/proxy/webhookServer` and `@/sync/statusTracker`, while `proxy/**` imports `@/eventBus`.
      Moving any one alone would have left the engine importing *out* of its own package.
      **Layer 3**: `src/applications/` (4 files), `src/companion/` (2), `src/commands/` (2) → 8 files.
      **Layer 4**: `src/startup.ts`, `src/shutdown.ts` → 2 files.
      Done with `git mv`, so blame survives. `src/` is now empty of engine code.
- [x] **`tsup` build for `packages/engine`** — `packages/engine/tsup.config.ts`. Multi-entry with
      the directory structure preserved (`dist/store/config.js`), ESM + CJS + `.d.ts`, mirroring
      the `packages/protocol` pattern. **`bundle: false` is deliberate and load-bearing**:
      `store/paths.ts` holds the resolved data root as module-level state, and bundling would
      inline a private copy into every entry point, so a `setDataRoot()` call through one entry
      would silently stop affecting the modules reached through another. Verified after the build:
      `dist/store/config.js` does `require("./appSettings")` / `require("./workspaceFs")`, and
      `dist/store/paths.js` owns the singleton.
- [x] **Resolve `@/*` aliases at package boundaries** — the engine is alias-free internally (its
      7 `@/store/*` imports became relative); consumers now use `@bifurc/engine/*`. Three
      resolution paths, all deliberate: `tsc` via a new root `tsconfig.json` `paths` entry, vitest
      via `resolve.alias` pointing at the engine **source** (so tests exercise real code and
      coverage can instrument it), and Node at runtime via the package's `exports` map — note
      `tsc-alias` deliberately leaves `@bifurc/engine/*` bare rather than rewriting it to a
      relative path, exactly as it already does for `@bifurc/protocol`. **Layer 1: 254 statements
      across 97 files** rewritten (src 69 files, tests 37 files, renderer **0** — the renderer's `@/`
      alias points at `renderer/`, so it never referenced the moved layer). **Layer 2: 189
      statements across 95 files** (177 import-shaped, plus 12 `vi.mock()` specifiers that an
      import-shaped regex misses entirely). Two layer-2-specific traps, both silent:
      engine-internal imports must be **relative** (23 `@bifurc/engine/store/*` self-imports would
      otherwise resolve through the `exports` map to `dist/` at runtime and load a *second copy* of
      module state), and `vi.mock("@/sync/x")` is a plain call, not a `from` — a stale mock
      specifier silently stops being hermetic because the real module loads instead.
      **Measured, not assumed:** it must be a `resolve.alias` *and* it must be redeclared inside
      every `test.projects` entry. A `resolveId` plugin never fires (the package is externalized
      first), and root-level `resolve.alias` — like root-level `plugins` — is not inherited by
      project runs, which is why a deliberately **bogus** alias target was silently ignored while
      the suite still passed against `dist/`. A `deps.inline` pattern is **not** needed. Before
      this, all 14 engine files reported 0% coverage because tests loaded the built `dist/*.mjs`.
- [x] **Fix the latent workspace-build CI bug** — neither `packages/*` `dist` is committed
      (`dist/` is gitignored repo-wide) and CI only ran `npm ci`, so `npm run typecheck` failed on
      a fresh clone with `TS2307: Cannot find module '@bifurc/protocol'`. Added `build:packages`
      and wired it to `prepare` (which `npm ci` runs) plus `build:main` and `typecheck`, so
      installs are now self-sufficient for both packages.
- [ ] **Split the dependencies** — **done for the engine, pending for the shell.** `packages/engine`
      declares its own runtime deps (`@bifurc/protocol`, `archiver`, `js-yaml`, `mkcert`,
      `simple-git`, `unzipper`, `ws`) and its own
      build devDeps (`tsup`, `typescript`, `@types/node`), and has **no** Electron or renderer
      dependency. Layer 2 added `mkcert` (`proxy/tlsCert.ts`), layer 3 added `ws`
      (`companion/companionServer.ts`), layer 5 added `archiver`, `unzipper` and `js-yaml` when
      `src/ipc/importExport/**` moved into `packages/engine/src/importExport/**`. Everything else the
      engine requires is a Node builtin
      (`child_process`, `crypto`, `events`, `fs`, `http`, `https`, `net`, `os`, `path`, `tls`, `vm`,
      `zlib`).
      **`@bifurc/protocol` was missing until 2026-09-16** — `commands/registry.ts` imported it but the
      manifest did not declare it, so it resolved only because npm hoists it into the root
      `node_modules`. That is invisible in this repo and fatal to a standalone install, which is
      exactly what P9's image is. Found by measuring the engine's `dist` requires against its
      manifest; fixed, with the lockfile regenerated.
      **`js-yaml`, `archiver` and `unzipper` were shell-side until layer 5.** Earlier revisions of
      this checklist correctly listed them as *not* engine dependencies — they were used only by
      `src/ipc/importExport/**` (openapi import, zip export/import), which was then still in the
      shell. That module has now moved, so they are engine dependencies as of 2026-09-16. They stay
      declared in the root `package.json` as well: removing them there changes what electron-builder
      walks, and packaging does not yet follow the package split (`plan/12`).
- [ ] Restructure to `packages/*` + `apps/*` workspaces — **partially**: `packages/protocol` and
      `packages/engine` are both real linked npm workspaces built with their own `tsup` pipelines.
      `apps/*` does not exist yet, and the Electron shell is still the repo root — that switch is
      P6's job.
- [ ] **Packaging follows the new package layout** — see the measured note in `plan/12`: with the
      app's `dist/**` now requiring `@bifurc/protocol` and `@bifurc/engine` at runtime, and
      `build.files` covering only `dist/**/*` + `package.json`, a packaged build cannot resolve
      either specifier. Dev mode and the whole test suite are unaffected. Assigned to P6/P12, not
      fixed here.

**Gate:** ✅ **met, with two items explicitly left open on product grounds.** Work item 8 is **done**:
the package exists, builds and is linked; **all 52 engine files have physically moved** in four
layers; `src/` contains only `ipc/` (the registration layer this phase replaces), `main.ts` and
`preload.ts`; and **`createEngine({ dataDir })` → `{ start(), stop(), status(), registry, bus }`**
is implemented and exercised end to end by `tests/integration/engineSmoke.integration.test.ts` — the
engine starts headless, serves over a real socket, and shuts down cleanly. Items 1–7 are otherwise
closed apart from the two items above explicitly left open on product grounds, **plus one new
decision this work surfaced**: on Windows the store modules resolve to `%LOCALAPPDATA%\Bifurc`
before consulting the data root, so `--data-dir` is not authoritative there — see the finding in
`plan/03` work item 8.

Verified after this change: `npm run typecheck` and `npm run typecheck:packages` clean;
`npm run build:main` clean; full suite **1615/1616 across 69 files** — identical to the baseline,
the single failure being the documented sandbox `127.0.0.1:1` connectivity quirk in the P0 spike
test. The 11 e2e specs remain unverified (cannot run in this sandbox — no desktop session).

---

## Section 4 — P3 File ops (1.5–2.5 weeks) 🟡 IN PROGRESS 2026-09-17

- [x] Read `../File_Ops_Protocol.md` in full
- [x] **Specify the blob primitives in `@bifurc/protocol`** — `blob.put` / `blob.stat` / `blob.read`
      / `blob.release` in `packages/protocol/src/commands/blob.ts`, registered in `COMMANDS` with
      fixtures in `index.test.ts` (187 protocol tests green). This was listed as a P3 **precondition**
      in `plan/04`, but P1 only landed the envelope half (`Capability.BLOB`) and made `export.ts`
      `blobId`-shaped — the commands themselves never existed, so P3 could not start. Added
      2026-09-16. The four tuning values are exported protocol constants
      (`BLOB_INLINE_THRESHOLD_BYTES` 1 MB, `BLOB_MAX_INGRESS_BYTES` 100 MB, `BLOB_TTL_MS` 1 h,
      `BLOB_READ_CHUNK_BYTES` 512 KB) rather than magic numbers, because the client needs them to
      know whether to expect a `blobId` or an inline payload. `BlobRef` is the shared
      `{ blobId } | { inline }` shape that domain results adopt in the conversion pass below.
- [x] **Build the blob store (`put` / `stat` / `read` / `release`) with tests** —
      `packages/engine/src/blob/store.ts`. Layout is `<dataDir>/blobs/<blobId>/{content,meta.json}`,
      a **real file per blob** because `unzipper.Open.file()` will not take a buffer and `archiver`
      pipes to a `WriteStream`. `meta.json` is written **last**, so a `put` that dies mid-write
      leaves a directory that reads as *not found* rather than as truncated content. SHA-256 on
      write. Typed `BlobError` codes (`blob-not-found` / `blob-invalid-id` / `blob-too-large` /
      `blob-size-mismatch` / `blob-invalid-offset`) rather than bare `Error`s, because P4's transport
      has to classify them into the protocol error envelope. `blobContentPath()` is exposed for the
      two zip files, which genuinely need a path.
- [x] **Blob TTL sweep** — `packages/engine/src/blob/sweep.ts`. `sweepBlobs()` reclaims expired
      blobs (by `createdAt`), metadata-less orphan directories (by mtime — a crashed `put`),
      abandoned `.staging/` files and stray files directly under the root. It never creates the
      root and never throws: a per-entry `try`/`catch` plus an `onError` channel, because a sweeper
      that can take the engine down is worse than one that skips a pass.
      `startBlobSweeper()` runs it on an **unref'd** interval (a sixth of the TTL) and returns an
      idempotent stop function. Wired into `createEngine()`'s `start()`/`stop()`.
- [x] **Size cap enforced before allocation** — `assertIngressWithinLimit()`, checked before the
      base64 decode: declared size over the cap, or a base64 string too long to decode under it,
      both reject without allocating. The decoded length must then **equal** the declared size,
      which is the only integrity check available — Node's base64 decoder silently ignores
      unrecognised characters, so without it a truncated payload would stage as a smaller but
      perfectly valid-looking blob.
- [x] **Register the four `blob.*` commands** — `packages/engine/src/blob/commands.ts`
      (`registerBlobCommands`). A function, not an import side effect: `createEngine()` registers
      **no** commands by design, and the consumer registers what it is willing to serve. The shell's
      call site lands with items 3–4.
- [x] **Move `src/ipc/importExport/` into the engine** — 38 files to
      `packages/engine/src/importExport/{exporters,importers,formats}` plus `registry.ts` /
      `types.ts`; the `electron`-importing adapter stays behind as `src/ipc/importExportHandlers.ts`.
      Brings `archiver`, `unzipper` and `js-yaml` across as engine dependencies.
- [x] Convert `environments-json` exporter **and** importer end-to-end (prove the pattern) — 2026-09-17
- [x] Bulk-convert the remaining 30 mechanical files — 2026-09-17. `scripts/p3-convert-importexport.py`
      asserts an exact `(old, new, count)` pair per file and refuses to run if any `fs.` reference
      survives; re-running it is a no-op. 32 files total (16 formats × 2 directions).
- [x] `workspace-zip` exporter (archiver → staged file → blob) — 2026-09-17
- [x] `workspace-zip` importer (blob → `unzipper.Open.file()`) — 2026-09-17
- [x] Fix `environments-dotenv.ts:55` name derivation → explicit `filename` — 2026-09-17, plus the
      `|| "Imported"` fallback for a file named exactly `.env`
- [x] Grep for any remaining `filePath.split` / `path.basename(filePath)` — 2026-09-17. The plan's
      grep is **incomplete**: it does not match `path.extname`, which is what
      `importers/requests-openapi.ts` used, and that one is worse (a staged blob has no extension, so
      every YAML spec would have taken the JSON branch).
- [x] Rewire the `importExport:export` egress channel (dialog-first ordering) — 2026-09-17
- [x] Rewire the `importExport:preflight` / `importExport:import` ingress pair (upload once,
      reference twice) — 2026-09-17
- [x] Rewire the other 4 egress channels (`tls:exportCert`, `runner:exportReport`,
      `capture:shareJson`, `audit:export`) — 2026-09-17. Engine half in
      `packages/engine/src/fileOps/commands.ts`; client half in `src/ipc/fileOpsClient.ts` (the
      shared `call` / `writeArtifact` / `uploadLocalFile` / `pullBlob` / `releaseQuietly` plumbing,
      extracted when the *second* channel needed the blob-chunk loop). The inline-vs-blob decision
      lives once, in `packages/engine/src/blob/publish.ts`. The HTML report renderer moved to
      `packages/engine/src/runner/reportHtml.ts`. **`audit:export` became dialog-first**, which
      also fixed a real cost: it used to run `queryLog({limit: 0})` — the whole `git log` — before
      asking the user where to put the result, so cancelling paid for the full query.
- [x] Rewire the other 3 ingress channels (`tls:importCert`, `tls:importKey`, `grpc:addProto`) —
      2026-09-17. The first two are converted; **`grpc:addProto` needs no change** and the row is
      closed on that finding, not skipped: `ProtoExplorer.tsx:147–152` already reads its file
      through `window.api.openFileDialog()` (which returns `{base64, name}` — content, never a
      path) and sends content. `soap:addWsdl` is the same, and is not even a file picker — the
      WSDL comes from `soapFetchWsdl(url)`. Both already satisfy §5 by a different route, so
      `blob.put` would add two round-trips and a staging store for no boundary gain.
- [x] Remove the remaining path leaks from protocol types (`tls:*`, `runner:exportReport`,
      `capture:shareJson`) — 2026-09-17. All five egress results are now `ArtifactResult`, and both
      TLS ingress params take a `blobId`. `tls:generate`'s `certPath`/`keyPath` stay until item 5's
      fingerprint model replaces them. Three categories are now distinguished in `plan/04` item 6:
      an engine path over the wire (fixed), the client's own chosen path echoed to the renderer
      (not a leak), and an engine path the shell synthesises for the renderer (real, deferred to
      P7 by non-negotiable #3).
      **One extra leak found and fixed:** `tls.importCert` read `blobContentPath(blobId)` directly,
      and that helper validates the id's *shape* but never checks existence — so a swept blob
      produced a raw `ENOENT: … open 'I:\…\blobs\blob_…\content'`, an absolute engine path in a
      protocol result. Now goes through `statBlob()` first, yielding a typed `blob-not-found` with
      a path-free message.
- [x] Cert: `tls.generate` returns a fingerprint — `identifyCert()` parses the DER with
      `crypto.X509Certificate` and reports `sha256:<64 hex>` plus the SHA-1 thumbprint; `tls.certStatus`
      reports the same value. `certPath`/`keyPath` are gone from both results, and the shell
      synthesises them for the renderer (category 3, **P7**).
- [x] Cert: delete `installCA` from the engine — deleted from `proxy/certManager.ts` along with
      `InstallResult`. The engine no longer shells out to anything; the install moved to
      `src/ipc/certTrust.ts`, and the three lifecycle commands moved to
      `packages/engine/src/proxy/certCommands.ts`.
- [x] Cert: `removeCert` becomes two-sided — the engine returns `engineRemoved`; the shell composes
      `clientUntrusted` and `clientUntrustNote`, and either half may succeed alone. **Showing it is P7.**
- [x] Cert: fingerprint drift detection — `AppSettings.tlsTrustedCa` (fingerprint + SHA-1 thumbprint +
      PEM + subject), `summariseTrust()` → `none | trusted | stale`, and a stale entry is un-trusted
      before the new one is installed. **Rendering the warning is P7.**
- [ ] Cert: Firefox warning in the UI — **detection done** (`certTrust.detectFirefox()`, per-platform
      roots, an unlaunched Firefox counted as present) and `firefoxDetected` is on both
      `tls:installCA` and `tls:certStatus` results. The note itself is a renderer change: **P7**.
- [ ] Update `ImportExportModal.tsx` to carry `blobId` not `filePath` — **deferred to P7**: the
      renderer must stay byte-identical through P6, and the shell bridges `filePath` → `blobId`
      in `pendingUploads`, so the wire contract is blob-shaped without touching the renderer.

**Gate:** round-trip green for all 17 formats; no `dialog.*` in the engine; no `filePath` in the protocol.
→ **All three met for every channel that crosses the engine/client file boundary as of
2026-09-17.** `history.list` / `history.diff` still take a `filePath`, correctly — it is a
workspace-relative path the client learned from a previous `history.list` result, never a path on
the user's machine.

### P3 progress notes

**Blob store, sweep and the four `blob.*` commands are done and independently verified**
(`tests/blob/{store,sweep,commands}.test.ts`, real temp data roots, real files). `registerBlobCommands()`
is now **reachable from the shell** — `src/ipc/handlers.ts` calls it alongside
`registerImportExportCommands()`, so the blob layer has a real consumer rather than only a test.

**Item 2 is complete: all 34 files take and return content.** `packages/engine/src/importExport/types.ts`
defines the interface, `registry.ts` wires it, and `filePath` is gone from every signature. The 32
mechanical files were converted by script; `workspace-zip` (both directions) keeps a path-based
signature behind separate `PathExporterFn` / `PathImporterFn` registry slots, because `archiver` pipes
to a `WriteStream` and `unzipper.Open.file()` rejects a buffer. Keeping those in their own slots rather
than behind a legacy adapter makes "the importer interface takes content" true everywhere else and
makes the `workspace-zip`-last ordering enforceable.

**Three production bugs were found and fixed doing it** — all three invisible to the old suite, and
all three recorded in `TESTING.md` §7:

1. **`workspace-zip` export threw on every call.** `import archiver from "archiver"` + `archiver("zip", …)`
   is the v5–v7 API; `archiver` 8 is pure ESM with no default export, so the call was
   `undefined(...)`. `@types/archiver` was pinned at **7**, describing the old API, so `tsc` was
   satisfied. Fixed as `import { ZipArchive } from "archiver"` + `new ZipArchive({ zlib: { level: 6 } })`
   **and** `@types/archiver` `^7.0.0` → `^8.0.0` — the two halves must move together, or the compiler
   keeps endorsing a call shape the runtime rejects.
2. **A YAML OpenAPI spec was previewed with a fabricated item count, and a malformed file was offered
   as importable.** `requests-openapi.ts` had two parsers: `run()` used `loadSpec()`, `preflight()`
   used `JSON.parse` with a `Math.floor(lines / 3)` fallback. `loadSpec()` was `async` (it used
   `await import("js-yaml")`) and `preflight` is synchronous on `ImporterFn`, which is how they
   drifted. Now one synchronous `loadSpec()` serves both.
3. **`environments-dotenv` named the environment `""` for a file called exactly `.env`** — the
   `|| "Imported"` fallback was in the plan's snippet but not in the code.

**One protocol defect fixed:** `ImportCommitParams.collisionStrategy` was frozen as
`["skip", "overwrite", "rename"]` while every real call site uses `["keep", "override", "new"]`, so
`import.commit` would have rejected every real import. Corrected and recorded in
`plan/protocol-changes.md`.

**Item 7 (renderer) is deferred to P7.** `plan/README.md`'s non-negotiable #3 forbids renderer edits
through P6, and `window.api` must stay byte-identical — that is the regression signal for the whole
extraction. The renderer therefore still sends and expects `filePath`; `src/ipc/importExportHandlers.ts`
bridges it to a `blobId` in a `pendingUploads` map, so the **wire** is blob-shaped while the renderer
is untouched. This resolves the contradiction between `plan/04` item 7 and non-negotiable #3 in favour
of the non-negotiable.

**Two things recorded rather than fixed:**

1. **The `import` condition of `@bifurc/engine`'s `exports` map is unloadable by Node.** `tsup`
   emits extensionless relative specifiers in the ESM output (`dist/blob/sweep.mjs` does
   `from "./store"`, `dist/eventBus.mjs` does `from "./sync/statusTracker"`), and Node's ESM
   resolver requires an extension. The CJS build is fine (`require("./store")` resolves) and is
   what `main` points at, so nothing in the shell or the suite is affected — but an ESM consumer
   would get `ERR_MODULE_NOT_FOUND`. Pre-existing since P2 layer 2; assigned to P9/P12.
2. **The blob root follows `dataDir()`, which is *not* authoritative on Windows** — the same
   pre-existing defect `plan/03` records for the workspace store. `paths.dataDir()` honours
   `setDataRoot()` on every platform, so the blob root does too; but `workspaceFs.dataRoot()` checks
   `%LOCALAPPDATA%` first on Windows. A Windows `--data-dir` run would therefore put workspaces in
   `%LOCALAPPDATA%\Bifurc` and blobs in the flag's directory. Harmless for Docker (Linux), which is
   the deployment the blob volume exists for; needs the same product decision already outstanding
   before P8/P9.

**Test-side note.** `tests/integration/importExportHarness.ts` is new: it holds the two client-side
shims (`exportToFile`, `preflightFile`/`importFile`/`sourceFromFile`) that the two format-level suites
use to read and write plain files, so those suites keep testing **codecs** while
`tests/integration/importExportBlob.integration.test.ts` (45 tests) tests the **transport** through the
real command layer. One pre-existing test was rewritten rather than deleted: "reports an error for a
missing file rather than throwing" described a scenario that is no longer expressible, because the
importer no longer sees paths — it now asserts the failure surfaces at the *client's* read, and the
engine-side twin (a `blobId` that does not exist) lives in the blob suite.

**Items 3–4 are now complete: all five egress and all three ingress channels are converted.** The
six channels that are not import/export live in `packages/engine/src/fileOps/commands.ts`
(`registerFileOpsCommands`), and the client half is `src/ipc/fileOpsClient.ts`. Three structural
decisions are worth recording because each removed a class of duplication rather than a single copy:

- **`packages/engine/src/blob/publish.ts` holds the inline-vs-blob decision, once.** All five egress
  channels return the same `ArtifactResult`, so all five must decide the same way. Below
  `BLOB_INLINE_THRESHOLD_BYTES` the artifact never touches the store; above it, it is staged and the
  client pulls it with `blob.read`. `sha256` is computed on **both** branches so a client never has
  to branch on which shape it received to know whether a digest exists.
- **`src/ipc/fileOpsClient.ts` holds the client plumbing, once.** `call()`, `writeArtifact()`,
  `uploadLocalFile()`, `pullBlob()`, `releaseQuietly()`, `mimeFor()`. Extracted when the *second*
  channel needed the blob-chunk loop — the point at which the duplication stopped being
  hypothetical. Five hand-written copies would have been five chances to drop the
  `chunk.length === 0` guard.
- **`packages/engine/src/runner/reportHtml.ts`** takes the HTML report renderer, because rendering a
  report is producing a *generated artifact* and therefore belongs on the engine side of the
  boundary. The dialog and the `writeFileSync` stayed in the shell. It is typed against the
  protocol's `RunReport` rather than `any`, and `completedAt` is optional there, so the duration
  falls back to `startedAt` (0.00s) instead of rendering `NaN`.

**A fourth latent data-loss bug was found by this pass**, and it is the same class as the
`collisionStrategy` defect: `RunReport` in `@bifurc/protocol` was typed from the **HTML renderer's
field usage** rather than from the renderer's real `CollectionRunReport`. `requestId`, `url`,
`testLogs`, `preScriptError`, `postScriptError` and `TestResultEntry.durationMs` were all missing.
`z.object()` **strips unknown keys**, so routing a real report through the schema silently truncated
the **JSON** export — and because the HTML renderer touches only fields the schema happened to have,
the truncation was invisible to the one consumer that existed. Widened, and asserted field by field.

**Three behaviour changes were made deliberately, not incidentally**, and each is a decision a
reviewer should be able to disagree with:

1. `audit:export` is now **dialog-first**, so cancelling no longer pays for a `queryLog({limit: 0})`
   over the workspace's whole `git log`. The cost of the ordering change is that the status message
   now precedes the dialog.
2. `tls:exportCert` is the one channel that is deliberately **not** dialog-first: it asks
   `tls.certStatus` before showing anything, preserving the pre-P3 `fs.existsSync` check. Making a
   user dismiss a save dialog to be told "no CA generated yet" is worse than one extra round-trip
   for a 2 KB file.
3. `tls.importCert` / `tls.importKey` **can now return an `error` string** where they previously
   returned a bare `{ok: false}`. The renderer ignores both shapes identically, so this is additive.

**The three suites that drive the real shell handlers had to be taught the new split.**
`runnerStorage`, `capture` and `auditLog` capture `ipcMain.handle` registrations and call them
directly, which is the right way to test a thin client — but the client now delegates to the engine,
so each `beforeAll` must register the engine half the way `src/ipc/handlers.ts` does. This is a
*fixture* change, not a contract change: the renderer-facing shapes are unchanged, and
`runner:exportReport` / `capture:shareJson` still return `{ok: false}` on cancel and
`{ok: true, filePath}` on success. The failure mode when it is missed is instructive and worth
remembering — the handler returns `{ok:false, error: 'No handler registered for command …'}` and the
file-writing assertions then fail with `ENOENT` on a file that was never written, which reads like a
filesystem bug rather than a registration one.

**Recorded rather than fixed: `tls.importCert` used to leak an engine path on a missing blob.**
`blobContentPath()` validates the id's *shape* but never checks that the blob exists, so reading it
directly turned "the user left the dialog open past `BLOB_TTL_MS`" into a raw
`ENOENT: no such file or directory, open 'I:\…\blobs\blob_…\content'` — an absolute engine-side path
in a protocol result, from a channel whose entire purpose is that no path crosses the boundary.
Fixed by reading through `statBlob()` first, which yields a typed `blob-not-found` with a path-free
message. This is the kind of leak the item-6 grep cannot find, because it is a *runtime* string
rather than a type.



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

#### P9/P12 — the certificate trust store, unverifiable from Windows

> **Recorded, not skipped.** P3 work item 5 built the client-side trust store
> (`src/ipc/certTrust.ts`) on a Windows development machine. Windows is the one platform whose
> behaviour could be **measured** — and measuring it changed the design twice (§5d of `plan/04`). The
> other two are implemented from documentation and asserted only as **command shapes**
> (`tests/ipc/certTrust.test.ts`). These three rows are what remains, and each needs a real machine.

- [ ] **Linux: does `trust anchor <cert>` install without elevation?** This is the difference between
      a clean one-click Linux install and a `pkexec` prompt, and `File_Ops_Protocol.md` §6.2 asks for
      it explicitly. `installPlan()` prefers it and falls through to `pkexec` and then to written
      instructions; the **ordering** is tested, the **elevation** is not.
- [ ] **Linux: does `trust anchor --remove <cert>` actually remove, and is there a probe?**
      `probeTrust()` returns `null` on Linux — it cannot answer — so `uninstallCA()` reports
      `verified: undefined` rather than a `false` that would claim a check nobody performed. Windows
      got a real probe (`certutil -store` exits 17 when the certificate is absent); Linux needs an
      equivalent or the un-trust stays unverified forever.
- [ ] **macOS: the whole path, both directions.** `security add-trusted-cert -d -r trustRoot -k
      <loginKeychain>` to install, and `security delete-certificate -Z <sha1> -t` to remove — where
      the question is specifically whether `-t` clears the **trust settings** and not only the
      keychain entry. Neither command has been run on a Mac.

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
