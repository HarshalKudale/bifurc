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
- [x] **Packaging follows the new package layout** — **measured 2026-09-20: it already does, and the
      note this item carried was stale.** `plan/12`'s premise came from the 2026-09-10 asar, which
      predates both `@bifurc` dependencies — `node_modules/@bifurc` was absent because nothing depended
      on it, not because electron-builder drops workspace links. Against the current tree,
      electron-builder 26.8.1 detects the workspace root and copies
      `node_modules/@bifurc/{engine,protocol,client}` into the asar **with their `package.json`**, so
      **44 of 44** of the required specifiers resolve inside the archive. `build.files` needed no
      change, and neither option `plan/12` offered was required. **What was actually broken was
      `dist/`:** `tsc` never deletes, so 92 of 116 `dist/*.js` were compiled copies of modules P2 had
      moved into `packages/engine` — 447 KB shipped inside the asar, one of them requiring a module
      that no longer exists under any name. Fixed by `scripts/clean-main-dist.mjs` on every
      `build:main`; guarded by `scripts/assert-packaged.mjs`, run by `release.yml` before upload.
      Detail and the three-way verification: `plan/07` work item 4.

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

## Section 7 — P6 Shell seam ✅ SHIP (2–3 weeks) 🟡 IN PROGRESS

- [x] **Baseline re-captured** — unit: 97 files / 2,418 before step 2; **101 / 2,503 — 2,462 passed**
      after step 3b-3. **Current, after step 3f: 101 / 2,507 — 2,466 passed.** e2e: 11 specs / 46 tests is
      the figure to compare against, **not yet captured** (needs a normal terminal — see `plan/07`
      Preconditions)
- [x] Build the IPC transport (renderer → main) — `src/ipcTransport.ts` + `src/ipc/rpcContract.ts`
- [x] Route **one** method (`config:get`) through RPC, keep `registerIpcHandlers()` for the rest —
      step 2, suite green
- [x] Reimplement `preload.ts` as a client bridge — steps 3b-1, 3b-3, 3d and 3f: **all 144 keys** are
      `{ ...client }`, with **no overrides left**. The last one, `app.checkUpdate`, was removed
      2026-09-20, when step 3f resolved the surface's final `SPLIT` by deleting the protocol command and
      reclassifying the key as client-local (`plan/07` §5b; `plan/protocol-changes.md`). The channel
      survives as a local one — see the `registerIpcHandlers()` item below
- [x] Implement local handlers (dialogs, zoom, theme, titlebar, platform, installCA, first-launch) —
      `src/ipc/handlers/clientHandlers.ts`, plus the three filesystem hooks (`pickSavePath`,
      `writeArtifact`, `readArtifactFile`)
- [x] Delete `eventBridge.ts` (step 3c) — the event half of the legacy path is gone; `registerIpcHandlers()`
      still runs, and `src/ipc/importExportHandlers.ts` is dead but not yet deleted
- [ ] Engine lifecycle: spawn + ready handshake + timeout — **deliberately deferred**; the engine is
      in-process behind a proven seam, and the process split is a change of transport behind it
- [ ] Supervision: backoff restart, give up after 3 failures / 60s — as above
- [ ] Shutdown: SIGTERM → 5s → SIGKILL; no orphans — as above
- [ ] Socket-close watchdog so a crashed shell does not orphan the engine — as above
- [x] Preserve single-instance (one shell → one engine) — `app.requestSingleInstanceLock()` untouched
- [ ] Add a "connecting to engine" UI state — never block window creation — **renderer work, forbidden
      in P1–P6** (non-negotiable #1); needs a decision, since it is in the plan's own work item 3
- [ ] Add the engine binary to `extraResources`; reuse `iconPath()` — **a process-split concern, and
      deferred with it.** While the engine is in-process it is an ordinary npm package inside the asar,
      so there is no binary to ship. The *resolution* half of work item 4 is done instead:
      `scripts/clean-main-dist.mjs` + `scripts/assert-packaged.mjs` + a `release.yml` step — see
      `plan/07` work item 4.
- [x] **Rollback flag** `BIFURC_ENGINE_RPC=0` — **decided 2026-09-20: dropped, not implemented.** The
      plan assumed the preload's old channel table would survive as the fallback branch; step 3b-1
      deleted it, and step 3 re-pointed `registerIpcHandlers()`'s bodies at the registry, so after that
      there was exactly **one** path and nothing for a flag to select between. Writing it now would be
      dead code that *reads* as a safety net — worse than none. **`git revert` is the single rollback
      layer**, and it is complete: the phase is a bounded set of commits, `renderer/` is untouched by all
      of them, and the seam is reachable only through `src/main.ts`. The flag returns with the process
      split, where two transports genuinely coexist — see `plan/07` work item 6 and Rollback.
- [x] Verify e2e data-dir isolation still holds (assert engine data dir is in the temp dir) — work item 5.
      **Done 2026-09-20, assertion installed; the e2e run itself is still uncaptured (next item).** The
      plan's premise named one input; there are **two**, and on Windows `%LOCALAPPDATA%` wins —
      `workspaceFs.dataRoot()` and `appSettings.settingsPath()` short-circuit on it *before* consulting the
      `setDataRoot(app.getPath("userData"))` that `src/main.ts:256` supplies. So the `electronApp` fixture
      now probes the app process for `userData`, `%LOCALAPPDATA%` and `platform`, and requires each to be
      inside its temp dir. It asserts **containment of the inputs**, not a re-derived path: both consumers
      only `path.join` relative segments onto one of the two, so input containment implies path
      containment, and a re-derivation would be a copy that can drift. Placed in the fixture *setup* so a
      new spec cannot forget it. Scope: this is the **shell wiring**; the engine's resolution order is
      covered by `tests/store/paths.test.ts` + `tests/store/workspaceFs.test.ts`. **Unset `%LOCALAPPDATA%`
      on win32 is an error, not a skip** — the engine would read the developer's real profile. Verified:
      `npm run typecheck:e2e` EXIT=0, plus a throwaway script against the engine's real built resolvers —
      under the fixture env `dataRoot()`/`dataDir()`/`wsDir()`/`platformDefaultDataDir()` all land in temp,
      and with the override removed `dataRoot()` escapes to `C:\Users\wrath\AppData\Local\Bifurc\data` and
      is rejected, which is what makes the guard non-vacuous. The 11 specs are **untouched** (`git status`:
      only `e2e/fixtures/electronApp.ts`). Also measured: `dataRoot()` and `dataDir()` resolve to *different*
      directories (`<temp>\Bifurc\data` vs `<temp>\electron-userdata`) — the `plan/03` work item 8 defect,
      deliberately untouched.
- [ ] **Reduce** `registerIpcHandlers()` to the local-handler entry point — *(reworded 2026-09-20; it read
      "Delete `registerIpcHandlers()` from the default path")*. **It is a reduction, not a deletion, and
      that is the correction that resolving `checkUpdate` produced.** The key blocker is gone:
      `checkUpdate` was the last `SPLIT` and was resolved on 2026-09-20 by **deleting `app.checkUpdate`
      from the protocol** — the whole operation is client-local (`plan/07` §5b) — so all 92 protocol
      commands are registered. **Step 3d (2026-09-19) had already retired the other three by widening the
      schemas they were pinned on, not by moving them** — `listAudit` (`NARROWED`, `fcd6d0d`) and
      `saveRunnerConfig` / `loadRunnerConfig` (`BLOCKED`). But `app:checkUpdate` remains a **local**
      channel — the shell owns `app.getVersion()` and the platform match — and local channels are not
      legacy, so something must still register them. The second blocker was `plan/07` §5c: 51 engine
      commands were registered at module scope in nine shell files, so deleting the shell's handler layer
      answered `UNKNOWN_COMMAND` for all of them on a containerised engine (P9) — and the ratchet could
      not see any of them. **§5c is complete and statically guarded**, so this item is unblocked on both
      counts. See step 3e.
      **But "unblocked" is not "safe to start", and an audit on 2026-09-20 (`plan/07` §5b-bis) says why.**
      The 132 legacy channels are **not uniformly dead**: 9 of them are *superseded shell halves* that own
      a `dialog.*` / `app.getVersion()` step the client now performs through a different channel. Those 9
      twins are now **confirmed one by one** (`plan/07` §5b-bis, "The 9 twins, verified per key"): eight
      equivalent or better, and one — `audit:export` — a **regression**, because `exportAudit` took
      `artifactToFile`'s artifact-first default and so undid P3's dialog-first ordering for the one egress
      artifact with no size ceiling. Fixed, and the `packages/client/src/local.ts` docblock that sanctioned
      it corrected in the same pass.       **The `tls` shape decision landed on 2026-09-20 — both keys are `local`** (the `tlsInstallCA`
      precedent, so the whole trust-store surface uses one mechanism). They had to move because each is
      **two-sided** and the routed form delivered the engine half alone: `tls:certStatus` is **latent**
      (P7 renders the drift warning and the Firefox note; nothing reads those fields yet), while
      `tls:removeCert` was **live** — `TlsSettingsSection.tsx:57` calls it, so the "remove certificate"
      button deleted the engine's keypair and left the OS trust store trusting the CA. `surface.ts` now
      classifies both `local` (`transport` 72 → 70, `local` 14 → 16, total 144 unchanged), `ClientLocal`
      gained the two members, and `certLifecycle.readCertStatus()` was extracted so the eight-field
      composition has one implementation rather than living in a channel body the routed twin could
      half-fill. **Consequence for this item: class 2 grows 9 → 11** — the two superseded `tlsHandlers.ts`
      channels join the list, and both are unreachable today because `window.api` is `{ ...client }`.
      `src/ipc/fileOpsClient.ts` **survives** the reduction, being a live shared library on two live
      paths. The audit also corrects this item's own wording: `registerClientHandlers()` is *already* the
      local-handler entry point (`src/main.ts:294`), so `registerIpcHandlers()` reduces to the
      **engine-registration + event-wiring + bridge** entry point rather than "to the local-handler entry
      point".
      **Starting the pass found two things that change its shape (`plan/07` §5b-ter):** the 48 generated
      channels cannot go without **splitting `entityCrudFactory.ts` first** — `registerEntityCrudHandlers`
      also calls `registerCrudKind()`, which populates the map the four `entity.*` commands dispatch
      through, so deleting it breaks 16 kinds with the misleading *"is not routed through the
      CommandRegistry yet"* — and the pass is a **test migration**, not a deletion: **eight** integration
      suites register handler groups by hand and drive the channels directly (`foldersAndWorkspaces` 45
      invocations, `protocolExecution` 62). So the unit is the **handler group** — delete its channels,
      re-point the suites that drove them, verify, commit.
      **Progress 2026-09-20 — stage 3b is COMPLETE: 57 of its 57 channels done** (`plan/07` §5b-sexies,
      §5b-septies). **Slice 1 (9 channels):** the nine protocol-client passthroughs
      (`graphql:introspect|execute`, `soap:fetchWsdl|execute`, the five `grpc:*`) are deleted from the three
      handler files, and `protocolExecution`'s 20 call sites are re-pointed at `commandRegistry.invoke()`
      through an explicit `invokeCommand(command, params)` helper. **Slice 2 (48 channels):**
      `entityCrudFactory.ts` is **deleted**, its five callers now call the engine's `registerCrudKind()` /
      `registerSimpleKind()` directly, and the five `ipc*` fields are gone from `CrudFactoryOpts` /
      `SimpleEntityOpts`. The **split-then-delete ordering §5b-ter predicted was necessary, not optional** —
      the wrapper also did the container insert, so deleting it outright would have broken all 16 kinds with
      the same misleading *"is not routed through the CommandRegistry yet"* message.
      **Reconciliation: 101 files / 2,496 tests / 2,455 passed / 40 skipped / 1 failed.** The delta from
      stage 3a's checkpoint (2,514 / 2,473 / 40 / 1) is **exactly −18 tests, and the mechanism is named**:
      `handlers.test.ts` generates **one `it` per `expectedChannels` entry**, so removing the 18 deleted
      channel names removed their 18 generated tests — the correct outcome, since the channels no longer
      exist and the list exists to assert their presence. Slice 1 cost **zero** tests (its 19 cases were
      re-pointed); slice 2 costs 18 (its cases are generated).
      **Still open after stage 3b:** the three files are named for something they no longer do
      (`*Handlers.ts` now register *kinds*), and `registerIpcHandlers()`'s own reduction. Collateral repairs
      across the two slices: `tests/spike/protocolPoc.test.ts` had been deleted without a record (restored —
      it is P0's runtime evidence and the baseline's documented 1 failure, and its retired-channel fixture
      now covers 4 of the 10 channels it drives), and the tree had been left mid-slice-1 with 19 cases
      failing `No handler registered`.
- [x] Step 3d — unblock the pinned keys (`plan/07` *Step 3d*, `plan/protocol-changes.md` ×2) — three of
      the four were **schema** problems rather than routing problems; `NARROWED` and `BLOCKED` are
      retired rather than left empty. Also fixed a consequence the ratchet could not see: the widening
      made `COMMAND_FIXTURES["runner.saveConfig"].invalid` (`config: {}`) *valid*, which would have
      failed the conformance suite on all four transports
- [x] Step 3e — the §5c pass: move the **51** shell-registered engine commands into `packages/engine`
      (`plan/07` §5c-i … §5c-vii). **DONE — seven slices, all suite-verified, and
      `grep -rn '^commandRegistry.register(' src/` now returns nothing.** The vitest outage cleared
      before slice 3, which is how slice 1's real defect was found (`plan/07` §5c-iii).
      **slice 1** — the five §5c named (`config.get`, `env.setActive`, `workspace.setActive`,
      `entity.load`, `entity.setEnabled`) → `store/coreCommands.ts` behind `registerCoreCommands()`;
      `syncEnabledSet`'s three copies collapsed into `store/enabledSet.ts`. It had added its
      `registerCoreCommands()` call to `settingsMutations`' **`beforeEach`**, which throws
      `Command "config.get" is already registered.` on the second test — 29 of 30 tests dead — because
      the registry is a singleton with no `clear()` while `register()` throws on duplicates by design.
      Fixed in `0160188` by moving it to `beforeAll`, where the other four suites already had it.
      **slice 2** — `runner.getHistory` / `runner.listFolderIds` → `runner/runnerCommands.ts`, so
      `registerRunnerCommands()` owns the whole `runner.*` namespace; needed **no** suite edits
      (`runnerStorage.integration.test.ts` already called it). **slice 3** (`cfc727e`) — the four
      `folder.*` → `store/folderCommands.ts`; the channels *delegate*, so the two hand-registering
      suites (`foldersAndWorkspaces`, `protocolPoc`) needed the call. **slice 4** (`2e8174d`) — the
      nine protocol-client commands → `protocols/{graphql,soap,grpc}Commands.ts`; `graphql.*`/`soap.*`
      used `require("http")` *inside the body*, which does not exist in the engine's **ESM** output, so
      they were converted to static builtin imports and checked at the artifact
      (`grep -c 'require(' dist/protocols/*.mjs` → 0). Slice 4 is mutation-checked: dropping
      `registerGrpcCommands()` fails 4 tests with `No handler registered`. **slice 5** (`cd7afad`) —
      the ten `application.*` → `applications/applicationCommands.ts`; the cleanest slice of the pass
      because **nothing had to travel** (every dependency was already engine-side, so the shell file
      was a pure adapter), and its private `generateId()` was byte-identical to `store/config.ts`'s
      exported one. Mutation-checked: dropping `registerApplicationCommands()` fails 11 of 12 tests.
      **slice 6** (`4c6d8ab`) — the largest by count, the 17 `sync.*` / `git.*` / `entity.publish` /
      `folder.publish` / `entity.restore` / `audit.diff` / `history.*` → `sync/syncCommands.ts`; also
      "nothing had to travel", but with a **local `sanitizeDirName` that must not be collapsed** into
      `store/workspaceFs`'s same-named export (different pattern, different replacement, no fallback —
      collapsing would silently change the published git path). Its mutation check is the most
      informative of the pass: dropping `registerSyncCommands()` fails the ratchet with **exactly the
      17** names and two `history:list` tests with `No handler registered`, while
      `gitRemote.integration.test.ts` stays **24/24 green** — a suite that registers the engine half
      itself is **structurally blind to the wiring**, so its green is not evidence about it.
      **§5c's count was 5; the real number was 51** across nine shell files. **slice 7** (`b48c65e`) —
      the last four, `entity.create`/`update`/`delete`/`list` → `store/entityCrudCommands.ts`. It is
      **structurally unlike 1–6**: not a file of commands but a **factory split across the seam**, whose
      four commands dispatch through per-kind options that the six *shell* files populate — so the
      dependency ran the wrong way and moving the registrations alone would have left them consulting an
      **empty** map, each throwing `"<kind>" is not routed through the CommandRegistry yet`, the same
      message a genuinely unregistered kind throws. Fixed by inverting the population direction: the
      containers went to the engine, the shell calls `registerCrudKind()`/`registerSimpleKind()` and
      keeps the channels, and **both factory signatures are unchanged, so none of the six caller files
      needed an edit**. Its mutation check is the sharpest of the pass — dropping the call gives **52
      failures, all in `handlers.test.ts`** (the ratchet with exactly the four names, plus 51 from the
      per-kind channels that delegate into them), while the hand-registering `entityConflictScope` stays
      **9/9 green**. The predicted affected set was 7 suites, derived from the factory's callers; the
      measured set is **5** (`entityConflictScope`, `foldersAndWorkspaces`, `settingsMutations`,
      `protocolPoc`, `protocolExecution`) — `gitRemote`/`gitSync` never imported anything reaching the
      factory. **Derive the affected set from the registration calls a suite makes, not from the files
      it mentions.** Verified after every slice from 3 onward: `npm run typecheck` EXIT=0 and the full
      suite at **101 files / 2,508 tests — 2,467 passed, 40 skipped, 1 failed** (the documented
      environmental `soap.execute`), i.e. the recorded baseline exactly, so **seven slices produced no
      test-count delta**.
      **This unblocked step 3 on the §5c side.** *(Superseded 2026-09-20 by step 3f: the sentence that
      stood here — "`registerIpcHandlers()` still cannot be deleted, because `app.checkUpdate`
      (SPLIT → P12) needs a channel" — was right about the blocker and wrong about the fix.
      `app.checkUpdate` is gone from the protocol, but `app:checkUpdate` remains a **local** channel
      because the shell owns `app.getVersion()` and the platform match, so the function is **reduced**
      to the local-handler entry point rather than deleted. `src/ipc/importExportHandlers.ts` is still
      dead and still dies with it.)*
      **The heuristic that predicts a slice's difficulty is not the command count but how much of the
      body is the shell's own** — slice 5 (10 commands) was the easiest, slice 1 (5 commands) the
      costliest, and slice 7 (4 commands) the most structurally awkward.
- [x] Extend the ratchet so it can see shell-registered engine commands — today it asks only "is this
      command registered *in this process*", and in this process the shell had registered all 51, so
      §5c was invisible to the mechanism built to catch exactly this class. **Done 2026-09-20: the
      static guard is `tests/ipc/handlers.test.ts` → *"registers nothing at module scope — the §5c
      guard"*.** It asserts the architectural rule rather than the historical bug: **registration goes
      through `register*Commands(registry)`, so the `commandRegistry` singleton is never registered
      directly** — which is what makes the engine containerisable, since a consumer supplying its own
      registry can only see commands registered *explicitly* against it. It reads every `.ts` under
      `src/` and `packages/engine/src/` via `import.meta.glob(..., { query: "?raw" })` and requires
      `/^\s*commandRegistry\.register\(/m` to match nothing.
      **Strictly stronger than the grep the pass itself used.** The guard matches a *statement* — a line
      whose first non-whitespace content is the call — so it catches module scope at column 0 **and** a
      module-scope call wrapped in a top-level `if`/`try` block. Verified by mutation that the column-0
      `grep -rn '^commandRegistry.register(' src/` the §5c pass relied on **misses** the indented
      variant, while this catches it. Two shapes it does not catch, stated in the test rather than
      implied: a call preceded by other code on the same line, and an indirect alias
      (`const r = commandRegistry; r.register(...)`).
      **The anti-vacuity guards are the part worth keeping.** A wrong glob returns `{}`, so the
      assertions would pass against an empty string — a guard that reads as coverage while covering
      nothing. So the test asserts the reader found >20 files in each tree **and** a positive control:
      `^\s*registry\.register\(` matches >30 times in the engine, proving the reader sees real
      registration code and a zero for the singleton form means "not written that way" rather than
      "content never loaded".
      **Mutation-checked both ways**: a module-scope registration in a scratch `src/` file fails the
      guard naming that file; the same call indented inside a top-level `if` also fails it; deleting the
      file restores green. Full suite **101 files / 2,509 tests — 2,468 passed, 40 skipped, 1 failed**
      (the documented environmental `soap.execute`) — **+1 over the baseline, which is this test and
      nothing else**.
      **But the ratchet is sharp on the *move*, which §5c did not anticipate** (`plan/07` §5c-v): once
      a command's only registration is an explicit call, forgetting that call makes it unregistered
      and the ratchet fails — it caught exactly that in slice 5 (all ten `application.*`), in slice 6
      (exactly the 17 `sync.*`/`git.*`/`history.*`) and in slice 7 (exactly the four `entity.*`).
      **Corollary measured in slice 6 (`plan/07` §5c-vi): a suite that registers the engine half itself
      is structurally blind to the wiring.** `gitRemote.integration.test.ts` stayed 24/24 green with the
      call site commented out, and `entityConflictScope` stayed 9/9 in slice 7. Only the ratchet and the
      delegating channels can fail loudly, so a hand-registering suite's green must never be quoted as
      verification of a call site.
- [x] Run the full suite — after every step, with the delta measured against the previous baseline

**Gate:**
- [x] 101 unit suites pass (same count as baseline) — **met on every step**; the current figure is **101
      files / 2,507 tests — 2,466 passed, 40 skipped, 1 documented `soap.execute` flake**, re-measured on
      the committed state after step 3f. That is the baseline plus §5c-viii's static guard, minus the
      **two** tests `packages/protocol/src/commands/index.test.ts` generates per command for the deleted
      `app.checkUpdate` (`accepts a valid payload` / `rejects an invalid payload`) — a delta accounted
      for by mechanism, not by count
- [ ] 11 e2e specs pass, **unmodified** — **carried as an owner-run verification.** Still the real gate
      for 3b-1 and 3b-3, because the unit suite tests handlers rather than the preload. Re-attempted
      2026-09-20 from the agent shell and it reproduced the documented environmental failure **exactly**
      (46 failed / 0 passed, every test dying at `firstWindow()`), so what is open is the shell's
      inability to host a GUI process, not the seam — see `plan/07` Preconditions for the three controls
- [x] `git diff --stat renderer/` shows only the P3 blob files — **empty** as of step 3b-3
- [x] Key-diff still zero — `packages/client/tests/surface.test.ts`, 144 keys both directions
- [x] Packaged bundle resolves the engine from inside the asar and runs — *(reworded 2026-09-20; the
      original read "Packaged NSIS build contains the engine and runs")* **Measured and CI-guarded**:
      44 of 44 `@bifurc/...` specifiers resolve inside the archive, `scripts/assert-packaged.mjs` runs in
      `release.yml` before upload, and the guard is proven non-vacuous in four directions. **The NSIS
      installer and a real packaged launch are still unverified** — carried to P12.
- [x] Rollback flag — **resolved as a decision: dropped** (see the item above). Not a gate any more.
- [x] No orphaned engine after a forced kill — **moved to the process split**, where it is criterion 7's
      relocated home (`plan/07` work item 2). An in-process engine cannot be orphaned. **Re-scoped again
      2026-09-20 by D14:** the process split is downgraded, so this criterion now belongs to the
      **workspace host** (`plan/14` §7) rather than to P6.
- [x] **Ship it.** 🎉 — **reached 2026-09-20: P6 closed in scope**, with the e2e run carried as an
      owner-run verification and NSIS + real packaged launch carried to P12. `plan/07`'s Acceptance
      criteria carries the re-scope, what moved, and why.

> **At this point the project is reversible and the shell is disposable. Everything below is additive.**

---

## Section 7.5 — P14 Workspace runtime 🆕 2026-09-20 (3–4 weeks)

**Full plan: [`14-workspace-runtime.md`](14-workspace-runtime.md). Read it before P7, P8 or P9.**

The pivot: the deployable unit is a **workspace**, not the engine (D10). Servers become per-workspace;
the companion server stays app-level.

> **⚠ Read §12 of `plan/14` first — the reconciliation with the completed phases (measured 2026-09-20).**
> P0–P6 are done, and they were done under the pre-pivot assumptions, so the pivot's target architecture is
> a claim *about work already in the tree*. Six phases audited; **one turned out to be wrong**: §2.2's claim
> that blob staging is "already per-workspace" is false — `blobRoot()` is `<dataDir>/blobs` and the four
> `blob.*` commands carry no `workspaceId`, so blob staging is a **third app-level thing** this plan's split
> table has no row for. §12.1 carries the measurement, the three costs under N > 1 and two fix shapes.

- [x] **DECIDED 2026-09-20 (D17) — blob staging is namespaced per workspace.** `blobRoot(wsId)` =
      `<dataDir>/blobs/<wsId>/`, `workspaceId` added to the four `blob.*` params, sweeper becomes
      per-runtime. Owner's call, made after §12.1 measured §2.2's "already per-workspace" claim false. The
      rejected shape — declare it app-level like the companion server — is recorded in §12.1, not deleted.
      **The root stays under `dataDir()`** (the Docker-volume requirement, pinned by
      `tests/blob/store.test.ts:62`), so it is a *namespace*, not a move. Breaking protocol change → lands
      with WI 1 and WI 8, and the `blob.*` `invalid` fixtures must be **re-derived**, not trusted
- [ ] **WI 0 — the measurement gate.** Inventory every reader of `AppConfig.port` / `.webhookPort` /
      `.activeWorkspaceId` / `isRunning()` and every module-level mutable in `proxy/`. **Count it and
      write the number down** — the phase's estimate rests on it being small.
      *Partly pre-answered by §12.2: `packages/engine/src/index.ts:210-259` holds nearly every reader —
      `preflight()` over three global ports, `startServer(settings.port)`, `startCompanionServer(...)`, the
      global `waitUntilReady(isRunning, getServerError)`, and a `status()` returning one `proxyPort`.*
- [ ] **WI 1 — port model + protocol shape (D16).** Host-local `runtime.json` at the data root mapping
      `wsId → { preferredPort?, resolvedProxyPort?, resolvedWebhookPort? }` (**not** `workspace.json` — it
      is git-tracked and a port would sync between machines). Allocator per D16: per-workspace override
      (fails loudly if taken) → environment **base port** → contiguous scan upward. Base = `80` on win32,
      `8080` elsewhere, overridable via `BIFURC_PROXY_BASE_PORT`. **Keep the platform branch to one
      constant** — if per-platform logic appears anywhere else in the allocator, D16 is mis-implemented.
      Driven by the real `listen()`, never by a probe. Skip a reserved-port set (companion, RPC, gRPC
      `9102`, other runtimes' ports). `workspaceId` added to `server.*` and `webhookServer.*`;
      `server.status` returns the **resolved** port
- [ ] **WI 2 — de-singletonize the webhook receiver first, then the proxy.** Readiness must become
      per-instance, not a global `isRunning()`. **Leave `store/paths.ts` and `processSpawner.ts` alone**
- [ ] **WI 3 — `WorkspaceRuntime` + host entry point** inside `packages/engine` (`runtime/host.ts`), not
      a new package — `bundle: false` makes a second package a module-state duplication hazard
- [ ] **WI 4 — sever `activeWorkspaceId` from the servers.** `config.save` is **split, not deleted** —
      the companion-restart half stays
- [ ] **WI 5 — preflight + port allocation.** `preflight()`'s port check must become **non-fatal for the
      base port** (a busy 80 is the normal second-workspace case) and stay fatal only for a per-workspace
      override; `checkPortInUse` is for *reporting*, never for allocating. Reuse
      `applications/portUtils.ts`; do not duplicate it. Log the resolved base port once at startup
- [ ] **WI 6 — the deployable host artifact**: one container = one workspace, one volume = that workspace.
      The image sets **`BIFURC_PROXY_BASE_PORT=8080`** — which is what Linux derives anyway, so it *states*
      the published port rather than changing it
- [ ] **WI 7 — the desktop adopts it**, with **zero user-visible change** and `window.api` byte-identical
- [ ] **WI 8 — protocol + compat record** in `plan/protocol-changes.md`; fixtures **re-derived**; the
      extension's four frozen commands asserted unchanged **by name**
- [ ] Two workspaces serve their own rules simultaneously on different ports, with no cross-talk
- [ ] **The D16 sequence holds per environment:** on `win32` 80 → 81 → 82; elsewhere 8080 → 8081 → 8082.
      Assert the sequence, and assert it for **both** bases
- [ ] **The platform branch is one constant** — assert base `80` under a simulated `win32` and `8080`
      otherwise, and that nothing else in the allocation path branches on platform
- [ ] **A per-workspace override fails loudly**, naming the next free port — never silently moves
- [ ] **`EACCES` advances rather than killing the runtime** — an override below 1024 on a platform that
      forbids it still comes up *(defensive: the default path never attempts a privileged port)*
- [ ] **The resolved port survives a restart:** free the base port, restart the host, and the workspace that
      incremented is **still** on its incremented port
- [ ] **No reserved port is ever assigned** — assert 9271 (companion) and 9102 (gRPC) are never handed out;
      the second workspace's webhook skips 9102 and lands on 9103
- [ ] `preflight()` does **not** fail on a busy **base** port, and **does** fail on a busy per-workspace
      override
- [ ] A workspace host starts with **no Electron and no companion server** — assert the companion port is
      *not* listening
- [ ] `store/paths.ts` still has exactly **one** data root in the process

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

### P8 CLI ⏸ **PARKED 2026-09-20 (D15)** — was: lifecycle + read-only (D7, ~3 weeks)

> **Deferred, not deleted.** Two shapes are recorded in [`14-workspace-runtime.md`](14-workspace-runtime.md)
> §7: (a) a CLI executable inside the Electron app talking to the app's engine over the socket transport,
> or (b) an npm package speaking `@bifurc/client` against a local app, a local host or a remote host.
> **The decision waits on P14 + P9**, because it depends on whether the workspace host becomes a common
> deployment. The list below is kept as the v1 scope that was agreed, for whoever picks it up.

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

### P9 Docker ⚠ **REWRITTEN 2026-09-20** — now "a workspace host in a container", not the engine

> **Read [`14-workspace-runtime.md`](14-workspace-runtime.md) before starting this.** D10 changes what the
> image contains: the host composes one `WorkspaceRuntime` per workspace and **does not start the companion
> server**. Documented deployment becomes **one container = one workspace, one volume = that workspace's
> `data/` + `blobs/`**. The port table in `plan/10` §4 needs N rows (one proxy port per hosted workspace),
> not 2. Effort is roughly unchanged; the artifact and the volume layout are not. **Depends on P14.**
>
> The list below is still the right checklist for the *host* — read "the engine" as "the workspace host"
> throughout.

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

- [ ] Engine in `extraResources`; assert it is outside the asar — waits on the process split
- [ ] `assert-artifacts` CI job (the icon regression already happened once) — **partly landed
      2026-09-20**: `scripts/assert-packaged.mjs` asserts a packaged bundle resolves every `@bifurc/...`
      specifier its compiled output requires, and `release.yml` runs it against `release/win-unpacked`
      before upload. Still to do: the engine-*binary* assertion (needs the process split) and the Linux
      path (`linux-unpacked/resources/app.asar`).
- [ ] NSIS / APPX / AppImage / Docker all publish from CI
- [x] `app:checkUpdate` moved client-side — **done 2026-09-20, ahead of P12** (step 3f). The whole
      operation is client-local, so the protocol no longer declares `app.checkUpdate` at all and the key
      is the fourteenth `ClientLocal` member — see `plan/07` §5b and `plan/protocol-changes.md`. Note
      what this item got wrong as a P12 concern: there was never an engine half to move, and the shell
      channel survives deliberately as a *local* one, so nothing about it was packaging-shaped
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
