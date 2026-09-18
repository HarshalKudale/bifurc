# 04 — Phase 3: File operations and the blob layer

**Goal:** implement the engine/client mediation for every file and system operation, per
`File_Ops_Protocol.md`. The engine becomes the authority for generated artifacts; the client becomes the
authority for the user's machine.

**Effort:** 1.5–2.5 weeks. **Depends on:** P1 (blob primitives in the protocol), P2 (engine structure).
**Blocks:** P7 (web UI needs blob flows), P9 (Docker needs blob staging on the volume).

> **Read `File_Ops_Protocol.md` first.** This document is the execution plan for that design.

---

## Preconditions

- `blob.put` / `blob.stat` / `blob.read` / `blob.release` specified in `@bifurc/protocol`.
  ✅ **Met 2026-09-16** — `packages/protocol/src/commands/blob.ts`. This precondition was **not**
  actually satisfied by P1: P1 landed only the envelope half (`Capability.BLOB`) and made
  `export.ts` `blobId`-shaped, leaving the four commands undefined, so this phase could not start.
  The tuning values (`BLOB_INLINE_THRESHOLD_BYTES`, `BLOB_MAX_INGRESS_BYTES`, `BLOB_TTL_MS`,
  `BLOB_READ_CHUNK_BYTES`) are exported constants so the client and the engine agree on them, per
  work item 1's "not magic numbers in two places".
- Engine has an injected data dir (P2 item 4) — the blob store lives under it. ✅
- `src/ipc/importExport/` has moved. ✅ **Met 2026-09-16 (layer 5).** 38 files now live in
  `packages/engine/src/importExport/{exporters,importers,formats}` plus `registry.ts` / `types.ts`.
  The shell-side adapter was deliberately left behind as `src/ipc/importExportHandlers.ts` — it is
  the **only** file in that directory that imports `electron`, so it is the seam, not part of the
  layer. `index.ts` was renamed rather than kept, so that `src/ipc/` reads as one flat set of
  handlers.   This also brings `archiver`, `unzipper` and `js-yaml` across as engine dependencies;
  nothing in `src/` or `renderer/` references them any more.
  ✅ **Conversion done 2026-09-17** — the 34-file interface conversion below (work item 2) is
  complete, and the `importExport` half of items 3–4 with it. See work item 2 for the three
  production bugs it uncovered.

---

## Work item 1 — The blob store

> **✅ Implemented 2026-09-16** — `packages/engine/src/blob/{store,sweep,commands}.ts`, with
> `tests/blob/{store,sweep,commands}.test.ts` (89 tests against real temp data roots: 58 + 20 + 11).
>
> **Layout:** `<dataDir>/blobs/<blobId>/content` + `meta.json`, plus `<dataDir>/blobs/.staging/`
> for in-flight writes. `<blobId>` is a **directory** and the bytes are a **real file** inside it,
> which is what the table below demands — `unzipper.Open.file()` will not take a buffer and
> `archiver` pipes to a `WriteStream`. Metadata lives beside the content rather than in a sidecar
> index, so `release` is one recursive delete and there is no second structure to keep in sync.
>
> **`meta.json` is written last.** Every read path gates on it, so a `put` that dies mid-write
> leaves a directory that reads as *not found* rather than as truncated content. The orphan is
> reclaimed by the sweep. A poor man's atomic write, with no rename dance.
>
> **Errors are typed.** `BlobError` carries a `code` (`blob-not-found`, `blob-invalid-id`,
> `blob-too-large`, `blob-size-mismatch`, `blob-invalid-offset`) because P4's transport has to
> translate them into the protocol error envelope and cannot pattern-match a message string. The
> four `blob.*` result types carry no error field, so the envelope is the only place an error can
> live — handlers therefore throw rather than returning `{ok:false}`.
>
> **The size cap is checked before allocating, and there are three rules, in increasing cost:**
> the declared size must fit `BLOB_MAX_INGRESS_BYTES`; the base64 string must be short enough to
> *possibly* decode under it (a 1 GB string decodes to ~750 MB — the exact OOM being prevented);
> and then the decoded length must **equal** the declared size. That last one is the only integrity
> check available: Node's base64 decoder silently ignores unrecognised characters, so without it a
> truncated payload stages as a *smaller but perfectly valid* blob. The pre-decode rules are
> extracted as `assertIngressWithinLimit()` so the bound is testable without allocating 140 MB.
>
> **The lease slides on read.** `statBlob` reports `ttlRemainingMs` but does not extend it; `readBlob`
> does, on every read that returns bytes. A 100 MB export pulled at the default 512 KB chunk size is
> ~200 round-trips, and without this a slow client could have its blob swept out from under it
> mid-transfer. A zero-byte read at the end does not extend it, so an idle client cannot hold a blob
> alive forever by polling.
>
> **Path traversal is defended twice:** `blobId` must match `^blob_[0-9a-f]{32}$` (which cannot
> express `..`, a separator, or user-chosen text at all), and `blobDir()` then asserts the resolved
> path is a direct child of the root. P13 lists the traversal test as the most likely thing to be
> missed; it is in `store.test.ts` against 15 hostile ids.
>
> **`createStaging()`** covers the one producer that cannot hand over a `Buffer` — the workspace zip
> exporter pipes `archiver` into a `WriteStream` and never holds the archive in memory. The caller
> writes to `handle.path` and calls `commit()` (hash by streaming, then `rename` into place — same
> volume by construction, so a 200 MB archive moves in constant time) or `discard()`. An abandoned
> handle is reclaimed by the sweep even if the process dies first.
>
> **The sweep** (`sweep.ts`) reclaims expired blobs by `createdAt`, metadata-less orphans by mtime
> (a crashed `put`), abandoned staged files, and stray files directly under the root. It never
> creates the root and never throws — a per-entry `try`/`catch` with an `onError` channel, because a
> sweeper that can take the engine down is worse than one that skips a pass.
> `startBlobSweeper()` runs it on an **unref'd** interval (a sixth of the TTL) and returns an
> idempotent stop function; `createEngine()` starts it in `start()` and stops it in `stop()`.
> The unref matters: an interval that kept the event loop alive would mean an engine, CLI or
> container that never exits unless someone remembers to stop it.
>
> **`registerBlobCommands(registry)`** binds the four frozen protocol commands to the store. It is a
> function rather than an import side effect because `createEngine()` registers **no** commands by
> design — the consumer registers what it is willing to serve. The shell's call site lands with
> items 3–4, when `importExport:*` becomes the first real consumer of a staged blob.
>
> **Deliberately not done in *this item*:** moving `src/ipc/importExport/` into the engine. The blob
> store has no dependency on it and the move is item 2's first step, so it was left to that step
> rather than half-done here — it landed separately as layer 5, on the same day. See the
> Preconditions note above and work item 2 below.
>
> **Two findings, recorded not fixed:**
>
> 1. **`@bifurc/engine`'s `import` export condition is unloadable by Node.** `tsup` emits
>    extensionless relative specifiers in the ESM output (`dist/blob/sweep.mjs` → `from "./store"`;
>    `dist/eventBus.mjs` → `from "./sync/statusTracker"`), and Node's ESM resolver requires an
>    extension. CJS is fine and is what `main` points at, so neither the shell nor the suite is
>    affected; an ESM consumer would get `ERR_MODULE_NOT_FOUND`. Pre-existing since P2 layer 2 —
>    assigned to P9/P12.
> 2. **The blob root follows `dataDir()`, which is not authoritative on Windows.** `paths.dataDir()`
>    honours `setDataRoot()` on every platform, so the blob root does too — but on Windows
>    `workspaceFs.dataRoot()` checks `%LOCALAPPDATA%` first. A Windows `--data-dir` run would split
>    workspaces and blobs across two directories. Harmless for the Linux/Docker case the blob volume
>    exists for; needs the same product decision already outstanding before P8/P9 (see `plan/03`).

```
packages/engine/src/blob/
  store.ts        # put / stat / read / release
  sweep.ts        # TTL cleanup
```

Design constraints, all of which come from real code:

| Constraint | Why |
|---|---|
| **Real directory**, `<dataDir>/blobs/<id>` | `unzipper.Open.file()` will not accept a buffer (`importers/workspace-zip.ts:41`). `archiver` pipes to a `WriteStream` (`exporters/workspace-zip.ts:15`). |
| **On the data volume** | In Docker, staging outside the volume loses in-flight transfers on restart. |
| **TTL ~1 hour + periodic sweep** | A Docker volume accumulates garbage forever otherwise. `blob.release` is the fast path; the sweep is the safety net. |
| **Inline below ~1 MB** | Matches the existing cap in `dialog:openFile` (`systemHandlers.ts:238`). Saves a round-trip for the common JSON export. |
| **Max size enforced in `blob.put` before allocating** | Suggest 100 MB for ingress. Do not let a client OOM the engine. |
| **SHA-256 on write** | Enables integrity checks and is needed for the cert fingerprint (§item 5). |

Record the threshold and the max as protocol constants, not magic numbers in two places.

---

## Work item 2 — Convert the 34 exporter/importer files

> **✅ Done 2026-09-17.** All 34 files take and return **content**; `filePath` is gone from the
> interface, from the registry, and from `@bifurc/protocol`. The shell is the client half
> (`src/ipc/importExportHandlers.ts`) and `window.api` is byte-identical, so
> `plan/README.md`'s non-negotiable #3 holds and work item 7 is deferred to P7.
>
> ### What the new interface is
>
> `packages/engine/src/importExport/types.ts` is the definition; `registry.ts` is the wiring.
>
> ```ts
> interface ExporterFn { run(wsId: string): Promise<ExportResult> }
> // ExportResult = { ok: true; content: string; suggestedName: string } | { ok: false; error: string }
>
> interface ImporterFn {
>   preflight(wsId: string, source: ImportSource): PreflightResult   // sync — see below
>   run(wsId: string, source: ImportSource, strategy: CollisionStrategy): Promise<ImportResult>
> }
> // ImportSource = { content: string; filename: string }
> ```
>
> Both result unions are discriminated on `ok`. The old shape made every field optional, so
> `{ ok: true }` alone type-checked and the compiler could not tell a complete result from a
> truncated one.
>
> **`filename` is not decoration.** Two importers genuinely need it, and both were deriving it from
> a path before: `environments-dotenv` names the new environment after it, and `requests-openapi`
> picks YAML vs JSON by its extension.
>
> ### The 32 mechanical conversions
>
> Done by `scripts/p3-convert-importexport.py`, which asserts an exact `(old, new, count)` pair for
> every file and refuses to run if any `fs.` reference survives. Re-running it is a no-op. The
> script exists because a hand conversion of 32 files across 64 edit sites is exactly the kind of
> change where one missed call site produces a *runtime* failure the compiler cannot see — tests are
> not typechecked (`tsconfig.json` has `include: ["src/**/*"]`).
>
> Three things the scripted pass had to be taught, each of which was a real defect first:
>
> 1. **`convert_exporter()` was defined and never called.** The script ran green and converted only
>    the importers. Symptom: ~14 `typecheck` errors of the form
>    `Type '(wsId: string, filePath: string) => …' is not assignable to '(wsId: string) => …'`.
> 2. **Idempotency was checked against the wrong anchor.** The "already applied?" test was
>    `new in text`, and the inserted helper itself contained the anchor string, so every re-run
>    appended another copy of `dotenvEnvName` — surfacing as `TS2393: Duplicate function
>    implementation`. The check is now `found == 0 and (not new or new in text)`.
> 3. **An anchor that is a substring of its own replacement.** `"export function preflight("`
>    matched inside the block inserted *before* it, so the fixed idempotency rule then inserted a
>    second copy. Deleted by hand.
>
> ### The two non-mechanical files, and the bug in one of them
>
> `workspace-zip` (both directions) keeps a path-based signature behind `PathExporterFn` /
> `PathImporterFn`. It is not a matter of taste: `archiver` pipes into a `WriteStream` and never
> holds the archive in memory, and `unzipper.Open.file()` rejects a buffer outright. The paths are
> engine-local — a `createStaging()` handle for export, `blobContentPath()` for import — and never
> cross the wire. Keeping them in separate registry slots rather than behind a `LegacyImporter`
> adapter makes "the importer interface takes content" true everywhere else, and makes the
> `workspace-zip`-last ordering enforceable.
>
> **Converting this file surfaced a production bug that had nothing to do with the interface.**
> It opened with `import archiver from "archiver"` and called `archiver("zip", {…})` — the v5–v7
> API. `archiver` 8 is pure ESM with **no default export**, so `require("archiver").default` is
> `undefined` and every ZIP export threw `TypeError: archiver is not a function`. It stayed hidden
> because `@types/archiver` was pinned at **7**, which describes the default-export API — the
> compiler was satisfied — and because no test had ever executed the exporter. Fixed as
> `import { ZipArchive } from "archiver"` + `new ZipArchive({ zlib: { level: 6 } })`, with
> `@types/archiver` bumped `^7.0.0` → `^8.0.0`. **The two halves must move together**: fixing only
> the import leaves the compiler endorsing a call shape the runtime rejects. See `TESTING.md` §7.
>
> ### Two path-derivation sites, one of which `plan/04` does not name
>
> The plan says to "grep for any other `filePath.split` / `path.basename(filePath)` before
> declaring this done". That grep finds `environments-dotenv` and **misses
> `importers/requests-openapi.ts`**, which used `path.extname(filePath)`. It is the worse of the
> two: a staged blob's content file is named `content` and has no extension at all, so under the
> blob layer every YAML spec would have silently taken the JSON branch.
>
> Both are fixed, and `environments-dotenv` also gained a latent-bug fix: a file named exactly
> `.env` strips to the empty string, so the new environment was unnamed. The `|| "Imported"`
> fallback was already in the plan's snippet but not in the code.
>
> ### A third bug, found by the test that was supposed to be about the filename
>
> `importers/requests-openapi.ts` had **two parsers**. `run()` used `loadSpec()`, which chose
> JSON vs YAML and parsed properly; `preflight()` tried `JSON.parse` and, on failure, returned
> `Math.floor(lines containing ":" / 3)` as `itemCount`. So a YAML spec's count was fabricated, and
> *any* text that failed `JSON.parse` came back `ok: true` — a truncated file was offered to the
> user as importable. `loadSpec()` was `async` (it used `await import("js-yaml")`) and `preflight`
> is a **synchronous** method on `ImporterFn`, which is how the two drifted apart. Fixed with a
> static `js-yaml` import, a synchronous `loadSpec()`, and one parser serving both halves — which
> is what `ImporterFn`'s contract already claimed they were.
>
> ### Step 1 history — the move, 2026-09-16 (layer 5)
>
> The files were moved first and the interface converted second, as separate verifiable steps, so
> that a bad result could be attributed to one of them.
>
> **What the move cost, and what it caught.** The rewriter reported 170 specifier rewrites across
> 45 files and **one** unresolved target, all of which were expected. What it *missed* was a bare
> side-effect import — `import "./registry";` in `importExportHandlers.ts`, which has no `from`
> clause and so matched none of the script's five patterns. It surfaced as a hard
> `Cannot find module './registry'` because a test imports that file directly; in an untested file
> the same miss would have been silent. The script now has a sixth pass, and re-running it is a
> no-op (0 rewrites, 0 unresolved), which is the check that it is complete rather than merely quiet.
>
> **`coverage.exclude` was re-checked, not relocated.** `src/ipc/importExport/types.ts` was excluded
> as type-only; the moved `packages/engine/src/importExport/types.ts` was re-verified — all ten
> exports are `type` / `interface` declarations with no runtime values — so the exclusion is correct
> and now points at the new path. (Contrast layer 3, where the same kind of entry was hiding live
> code and had to be *deleted*.)
>
> **Root dependencies left in place on purpose.** `archiver`, `unzipper` and `js-yaml` are still
> declared in the root `package.json` even though the engine now owns them. Removing them changes
> what electron-builder walks, and packaging does not yet follow the package split (`plan/12`) —
> so that removal belongs with the packaging fix, not here. Duplicated declarations are harmless;
> a prematurely narrowed dependency list is not.

**32 of these are mechanical.** The interface changes from a path to content:

```ts
// before
export async function run(wsId: string, filePath: string): Promise<ExportResult> {
  fs.writeFileSync(filePath, JSON.stringify(payload, null, 2), "utf-8");
  return { ok: true, filePath };
}

// after
export async function run(wsId: string): Promise<{ content: string; suggestedName: string }> {
  return { content: JSON.stringify(payload, null, 2), suggestedName: "mocks-export.json" };
}
```

### The mechanical list (32)

| Kind | Files |
|---|---|
| environments | `environments-dotenv`, `environments-json`, `environments-postman` |
| mappings | `mappings-json` |
| mocks | `mocks-json`, `mocks-postman`, `mocks-wiremock` |
| proxy rules | `proxyrules-json` |
| requests | `requests-curl`, `requests-har`, `requests-insomnia`, `requests-openapi`, `requests-postman` |
| webhooks | `webhooks-json` |
| websockets | `websockets-json` |
| workspace | `workspace-json` |

…each in both `exporters/` and `importers/`.

### The two non-mechanical files

`workspace-zip` (both directions) — stream-based, must stage through the blob store:

```ts
// exporter: archiver → staged file → blob
const out = fs.createWriteStream(blobPath);
archive.pipe(out);

// importer: blob staged file → unzipper
const directory = await unzipper.Open.file(blobPath);
```

Do these two **last**, after the mechanical 32 have proven the new interface.

### The name-derivation bug — fix it here

```ts
// importers/environments-dotenv.ts:55
const name = filePath.split(/[/\\]/).pop()?.replace(/\.env.*$/, "") ?? "Imported";
```

Under a remote engine this either produces garbage or leaks a fragment of the user's local path into an
entity name. Becomes an explicit `filename` parameter supplied by the client. **Grep for any other
`filePath.split` / `path.basename(filePath)` before declaring this done.**

---

## Work item 3 — Rewire the five egress channels

Client shows the dialog first, then calls the engine. Fail fast on cancel — never build a 200 MB
workspace zip the user then abandons.

> **✅ Done 2026-09-17.** All five egress channels are converted. The last four landed together with
> work item 4's remaining ingress pair, because they share one contract and one publisher.
>
> `importExport:export` came first and alone. `src/ipc/importExportHandlers.ts` shows the dialog
> **first**, then calls `export.create`, then either decodes the inline bytes or pulls the blob with
> the `blob.read` chunk loop, then writes the file itself. `window.api` and every renderer file are
> untouched, which is what keeps `plan/README.md`'s non-negotiable #3 intact through P6. The channel
> name is still `importExport:export`; `@bifurc/protocol` names the *command* `export.create` and the
> two are bridged in `src/ipc/handlers.ts`, exactly like the other ~112. It was done then rather than
> with items 5–6 because `plan/04`'s own "How to start" step 2 asks for one pair converted
> end-to-end so that the interface change, the blob wiring and the client save are proven together —
> and because `registerBlobCommands()` had been written in item 1 with nothing calling it.
>
> `src/ipc/fileOpsClient.ts` is the shared client plumbing — `call()`, `writeArtifact()`,
> `uploadLocalFile()`, `pullBlob()`, `releaseQuietly()`, `mimeFor()`. It was extracted when the
> *second* channel needed the blob-chunk loop, which is the first moment the duplication was real
> rather than hypothetical: converting five channels by hand would have produced five copies of a
> loop where one copy keeps the `chunk.length === 0` guard and four do not.
>
> **`blob/publish.ts` is the one place the inline/blob decision is made.** All five channels return
> the same `ArtifactResult`, so all five must make the same decision the same way. Below
> `BLOB_INLINE_THRESHOLD_BYTES` the artifact never touches the store — the common case (a small JSON
> export, a 2 KB certificate) is one round-trip — and above it the artifact is staged and the client
> pulls it with `blob.read`. `sha256` is computed on **both** branches, so a client never has to
> branch on which shape it received to know whether a digest exists.
>
> **The dialog now precedes the work on every channel, and that fixed a real cost.** `audit:export`
> used to run `queryLog({limit: 0})` — the whole `git log` of the workspace — and only then ask the
> user where to put it. Cancelling paid for the full query. It is now dialog-first like the rest.
> Two consequences are worth stating because they are behaviour changes, not refactors: the status
> messages for cert/audit/report now precede the save dialog, and `tls.importCert`/`tls.importKey`
> can now return an `error` string where they previously returned a bare `{ok:false}`.
>
> **One channel is deliberately not dialog-first:** `tls:exportCert` asks the engine
> `tls.certStatus` before showing anything, preserving the pre-P3 `fs.existsSync` check. Making a
> user dismiss a save dialog to be told "no CA generated yet" is worse than one extra round-trip for
> a 2 KB file.
>
> **`grpc:addProto` / `soap:addWsdl` need no change at all** (see work item 4).

| Channel | File | Change | Status |
|---|---|---|---|
| `importExport:export` | `importExportHandlers.ts` | Remove `dialog.showSaveDialog`; return content/blob | ✅ |
| `tls:exportCert` | `tlsHandlers.ts:23` | Remove dialog + `copyFileSync`; return blob | ✅ |
| `runner:exportReport` | `runnerHandlers.ts:24` | Remove dialog; return content/blob | ✅ |
| `capture:shareJson` | `systemHandlers.ts:272` | Remove dialog; return content/blob | ✅ |
| `audit:export` | `syncHandlers.ts:147` | Remove dialog; return content/blob | ✅ |

```ts
// target
export.create { kind, format, wsId, filename }
  → { inline: base64 }  |  { blobId, size, mimeType, sha256 }
```

The engine half is `packages/engine/src/fileOps/commands.ts`
(`registerFileOpsCommands(registry)`), and the HTML report renderer moved to
`packages/engine/src/runner/reportHtml.ts` — the rendering is a *generated artifact*, so it belongs
on the engine side; the dialog and the `writeFileSync` did not move.

---

## Work item 4 — Rewire the four ingress channels

Upload **once**, reference the blob for both preflight and commit. This also fixes the double-read in
`ImportExportModal.tsx`.

> **✅ Done 2026-09-17.** All ingress channels are converted.
>
> `src/ipc/importExportHandlers.ts` shows the open dialog, uploads with `blob.put`, calls
> `import.preflight`, remembers the `blobId` against the renderer's `filePath`-identified request,
> and on commit calls `import.commit` then `blob.release`. The renderer keeps sending a `filePath`
> because it cannot change before P7; the shell bridges that to a `blobId` in `pendingUploads`, so
> the *wire* contract is blob-shaped even though the renderer's is not yet.
>
> **"Upload once, reference twice" is now a tested property**, not just a design intent:
> `importExportBlob.integration.test.ts` asserts that `import.preflight` and `import.commit` both
> succeed against a single staged blob, and that the engine does **not** auto-release after commit
> (so a retry after a failed commit does not need a re-upload — the client releases, per
> `File_Ops_Protocol.md` §5).
>
> **`tls:importCert` / `tls:importKey` are persisted imports, not transform-and-discard.** The
> engine copies the staged bytes into `appDataDir()` and the bytes live on; the client still
> releases, because the staged copy has served its purpose. Two details worth recording:
>
> - **The destination is resolved through `appDataDir()`, not `dataDir()`.** They are the same
>   directory on Linux/macOS, but on Windows `appDataDir()` is `%LOCALAPPDATA%/Bifurc` and does not
>   follow `setDataRoot()` — and `generateCA()` / `tls.removeCert` both use it. A third answer here
>   would make the certificate invisible to whichever half looked in the other directory. The
>   divergence is pre-existing and recorded; it is not introduced by this conversion.
> - **The bytes are copied as bytes.** The pre-P3 handler was `fs.copyFileSync`; reading a PEM as
>   UTF-8 and writing it straight back would silently normalise anything non-ASCII. A PEM is ASCII,
>   so it would not matter today — but there is no reason to introduce the difference.
>
> **`grpc:addProto` and `soap:addWsdl` need no change, and this is a deliberate finding rather than
> an omission.** The plan groups them with the ingress channels on the grounds that they read a
> client file. Neither does, and the check is worth recording because it is the reason the row is
> closed rather than converted:
>
> - `ProtoExplorer.tsx:147–152` calls `window.api.openFileDialog()`, which returns `{base64, name}`
>   — content and filename, never a path — decodes it, and sends `{name, content, parsedServices}`.
> - `WsdlExplorer.tsx:143–150` is not a file picker at all: it calls `soapFetchWsdl(url)` and sends
>   the `content` that comes back.
>
> Both are therefore already the §5 contract — **content on the wire, the client owning the
> filesystem** — reached by a different route. Converting them to `blob.put` would push a few
> kilobytes of WSDL/proto through a staging store, add two round-trips and a release, and change a
> working contract for no boundary gain. The row is closed as **no change needed**.
>
> One protocol defect was fixed on the way through — see the collision-strategy note below.

| Channel | File | Status |
|---|---|---|
| `importExport:preflight` + `importExport:import` | `importExportHandlers.ts` | ✅ |
| `tls:importCert` | `tlsHandlers.ts:38` | ✅ |
| `tls:importKey` | `tlsHandlers.ts:50` | ✅ |
| `grpc:addProto` | `grpcHandlers.ts` | ✅ no change needed — already content-shaped (see note above) |
| `soap:addWsdl` | `soapHandlers.ts` | ✅ no change needed — content fetched from a URL, not a file (see note above) |

```ts
client: blob.put { filename, mimeType, data } → blobId
client: import.preflight { kind, format, wsId, blobId } → { itemCount, collisionIds }
        [user resolves collisions]
client: import.commit { kind, format, wsId, blobId, collisionStrategy } → { imported, skipped }
client: blob.release { blobId }        // imports are transform-and-discard
```

**Note the distinction:** imports (Postman, OpenAPI, HAR, curl, dotenv) are transform-and-discard — the
source file never persists on the engine, so release immediately. WSDL, proto files and certificates
**do** become persisted entities and are copied into the workspace store.

**One protocol defect fixed on the way through.** `ImportCommitParams.collisionStrategy` was frozen
as `["skip", "overwrite", "rename"]` while every real call site uses `["keep", "override", "new"]`.
As written, `import.commit` would have rejected every real import. Corrected in
`packages/protocol/src/commands/export.ts` and recorded in `plan/protocol-changes.md`; both the
acceptance of the three real names and the rejection of the three retired ones are asserted.

---

## Work item 5 — The certificate lifecycle

The genuinely new design work. Full detail in `File_Ops_Protocol.md` §6.

> **🟢 2026-09-17 — the mechanism half is done. The rendering half is P7.**
>
> **Done and verified.** The fingerprint (`identifyCert()` — SHA-256 over the DER, via
> `crypto.X509Certificate`); `tls.generate` / `tls.certStatus` reporting it instead of paths;
> `installCA` **deleted from the engine**; a client-side trust store with install, un-trust and
> post-command **verification**; the Linux fallback chain; Firefox detection; and the drift record
> with the stale-entry replacement.
>
> **Every new field is additive.** `window.api` is byte-identical through P6
> (`README.md` non-negotiable #3) — `renderer/types/window.ts` and `preload.ts` are untouched, and
> the renderer reads none of `fingerprint`, `trustState`, `trustedFingerprint`, `firefoxDetected`,
> `alreadyTrusted`, `verified` or `clientUntrusted`.
>
> **Not done, deliberately:** the three acceptance criteria that are UI-visible — the drift warning
> rendered, the Firefox note rendered, and the two-sided `removeCert` result surfaced. Those need
> renderer edits, which P1–P6 forbid, so they belong to P7 alongside item 7. The *values* they need
> are already on the wire.
>
> **Two facts were measured against a real `certutil` rather than assumed, and both changed the
> design** — see §5d. Neither is in the original plan, and one of them is the difference between a
> truthful un-trust result and a false one.

### 5a. Two halves, one fingerprint

```
tls.generate  → { ok, fingerprint: "sha256:ab12…" }
tls.certStatus→ { generated, fingerprint }
tls.removeCert→ { ok, engineRemoved }        // the engine's half only
```

The client composes its own half and never sends it: `tls:certStatus` adds
`trustedFingerprint` + `trustState`, `tls:removeCert` adds `clientUntrusted`.

The client stores the fingerprint it installed and compares on connect. On mismatch: *"the engine's CA
has changed since you trusted it — reinstall."* Without this, a regenerated CA produces opaque TLS
errors with no explanation.

**`certBlobId` / `keyBlobId` are deliberately not implemented**, although the sketch above and
`File_Ops_Protocol.md` §6.1 both name them. Two reasons, and the first is structural: the blob store
is a **transient** staging area whose sweeper deletes anything past `BLOB_TTL_MS` (1 h), while the CA
is a **durable** entity with a ten-year validity — they are opposites. Second, the engine needs a real
file on disk: the proxy's TLS server is handed a path (`tlsCaCertPath`) and `mkcert` writes PEM. A
durable blob would mean two sources of truth for one certificate, and the egress channel that actually
needs to move the bytes (`tls.exportCert` → `ArtifactResult`) already does. Recorded here because a
sketch in the plan is still the spec until it is amended in writing.

The client's record (`AppSettings.tlsTrustedCa`) carries the **PEM** and not only the fingerprint.
That is not redundancy: once the engine regenerates, its copy of the trusted certificate is gone, and
`certutil` / `security` / `trust` will not remove a certificate you cannot hand them.

### 5b. Move the install off the engine

✅ **Done.** `installCA` is deleted from `packages/engine/src/proxy/certManager.ts`, and
`InstallResult` with it. The engine no longer shells out to anything.

| Piece | Was | Now |
|---|---|---|
| the install | `certManager.installCA()` — engine shells out | `src/ipc/certTrust.ts#installCA()` |
| the un-trust | *did not exist* | `src/ipc/certTrust.ts#uninstallCA()` |
| the composition | — | `src/ipc/certLifecycle.ts` |
| the channel | `clientHandlers.ts` (unchanged — still CLIENT-classified) | same |
| the three lifecycle commands | `src/ipc/handlers/tlsHandlers.ts` (the shell serving engine commands) | `packages/engine/src/proxy/certCommands.ts` |

| Platform | Command | Privilege |
|---|---|---|
| Windows | `certutil -addstore -user Root <cert>` | none |
| macOS | `security add-trusted-cert -d -r trustRoot -k <loginKeychain> <cert>` | none |
| Linux | `trust anchor <cert>` (preferred) or `sudo update-ca-certificates` | root |

**Verify `trust anchor` works without elevation** (p11-kit user store) — it is the difference between a
clean Linux UX and a `pkexec` prompt. Fall back to `pkexec`, then to written instructions.

✅ **The chain is implemented in that order** — `trust anchor` → `pkexec trust anchor` → written
instructions — and `pkexec` is only offered when `trust` exists, because `pkexec trust anchor` elevates
the same binary rather than substituting for it. The *elevation* question cannot be answered from a
Windows development environment; it is recorded as an unchecked P9/P12 task in `plan/13-checklist.md`.

### 5c. The Firefox problem

**Firefox does not use the OS trust store** — it ships its own NSS store. Installing via `certutil` or
`security` makes Chrome, Edge and Safari work and **leaves Firefox broken.**

This is a top-tier support burden for a localhost proxy tool, and it gets worse with a remote engine
because users will blame the engine. Handle it deliberately:
- Detect Firefox presence.
- Show an explicit "Firefox needs a separate step" note in the cert UI.
- Optionally offer direct NSS installation.

Do not discover this in issue reports.

✅ **Detection implemented** (`certTrust.detectFirefox()`, per-platform profile and binary paths). The
UI note is P7; the flag it needs (`firefoxDetected`) is already on both `tls:installCA`'s and
`tls:certStatus`'s results.

An unlaunched Firefox is counted as **present**: a missing profiles directory is not evidence of
absence, and a freshly-installed Firefox with no profiles yet belongs to exactly the user who is about
to hit this.

### 5d. Two measured facts that changed the design

Both were measured against the real `certutil` on Windows 11 during item 5, not inferred from
documentation. Neither is in the original plan.

**1. `certutil -delstore` exits 0 whether or not it deleted anything.**

```
> certutil -delstore -user Root "00112233445566778899AABBCCDDEEFF00112233"
Root "Trusted Root Certification Authorities"
CertUtil: -delstore command completed successfully.
exit=0
```

There is no such certificate in the store. So the delete command's exit status **cannot** be the basis
of a "removed" claim — a result built on it would tell the user their CA had been un-trusted when it
had not, which is a security-relevant lie rather than an ordinary bug.

**2. `certutil -store -user Root <thumbprint>` does discriminate.**

```
> certutil -store -user Root "00112233445566778899AABBCCDDEEFF00112233"
CertUtil: -store command FAILED: 0x80090011 (-2146893807 NTE_NOT_FOUND)
exit=17
```

Exit 0 when present, **17** (`NTE_NOT_FOUND`) when absent. That is the verification primitive.

**What changed as a result.** `probeTrust()` is a separate concept from `run()` — "did the command
succeed?" and "what does the store say?" are different questions, and a non-zero exit is the *expected
answer* half the time. `TrustResult.verified` is a separate field from `TrustResult.ok`, and
`uninstallCA()` re-queries the store after deleting rather than trusting the exit status. `probeTrust()`
returns `true | false | null`, where `null` means *this platform cannot answer* — Linux has no probe
that could be verified here, and `verified: undefined` is the honest report rather than a `false` that
claims a check nobody performed. `tests/ipc/certTrust.test.ts` pins both directions.

A third, smaller finding: querying by **serial number** does not behave the same way (`certutil -store
-user Root 3531333934` prints *"Cannot find the certificate and private key for decryption"* and exits
**0** even for a certificate that is present), so the probe uses the SHA-1 thumbprint.

### 5e. The import-validation gap, closed at status time instead

`packages/engine/src/fileOps/commands.ts` recorded that `tls.importCert` does not validate what it
imports — the pre-P3 handler was `copyFileSync` and accepted anything — and assigned the fix to item 5.

Item 5 **did not** close it by rejecting bad input at import. The import contract deliberately
preserves arbitrary bytes, asserted by `tests/integration/fileOps.integration.test.ts` including a
deliberately non-ASCII payload, and narrowing it would be a second unrequested behaviour change. It is
closed at **status time** instead: `getCertStatus()` reads the certificate and reports
`generated: true, fingerprint: null` for a file that is not a certificate. Same information, at the
moment a UI asks, without touching the import path.

---

## Work item 6 — Remove the path leaks

Every one of these must become a blob reference or be dropped. Verified against source.

> **🟡 2026-09-17 — every engine-side leak is gone. What remains is the renderer-facing shell
> shape, and it has to stay until P7.**
>
> The acceptance criterion "no `filePath` in any protocol command or result type" is **met**: no
> command that crosses the engine/client file boundary takes or returns a path. The distinction the
> table below now draws is between three different things that all used to be called "a path":
>
> 1. **An engine filesystem path travelling over the wire** — gone from all five egress and all
>    three ingress channels. This is the boundary violation, and it is fixed.
> 2. **The client's own chosen path echoed back to the renderer.** `runner:exportReport` and
>    `capture:shareJson` return `{ok: true, filePath}` where `filePath` is what the *user* typed into
>    the save dialog. That is the client's own information, not the engine's, and it is not a leak —
>    it is how the renderer shows "saved to …". The engine never sees it.
> 3. **An engine path the shell *synthesises* for the renderer.** `tls:importCert` returns
>    `path.join(appDataDir(), "ca-cert.pem")` so `TlsSettingsSection.tsx:37` can put it in
>    `config.tlsCaCertPath`. The engine deliberately reports no path at all; the shell computes this
>    one. It is still an engine FS layout detail reaching the renderer, so it is a genuine leak —
>    but `window.api` is byte-identical through P6 (`README.md` non-negotiable #3), so it stays, and
>    work item 7 is where it stops being a path.
>
> The three `importExport` leaks are gone entirely — that channel had no renderer contract to
> preserve because the shell bridges `filePath` → `blobId` in `pendingUploads`.

| Location | Leak | Status |
|---|---|---|
| `importExport/types.ts` — `ExportResult.filePath` | engine → client | ✅ replaced by `{ content, suggestedName }` |
| `importExport/types.ts` — `PreflightResult.filePath` | engine → client, **and back** | ✅ dropped — the client already knows where its own file is |
| `importExport/types.ts` — `ImportRequest.filePath` | client → engine | ✅ replaced by `{ content, filename }` |
| `tlsHandlers.ts:11` — `tls:generate` returns `certPath`, `keyPath` | engine FS layout | ✅ the engine returns `{ ok, fingerprint }`; the shell synthesises both paths from `caCertPath()` (category 3) — **P7** |
| `tlsHandlers.ts` — `tls:exportCert` | engine → client | ✅ the engine returns `ArtifactResult` with no path; the shell returns the user's own `filePath` (category 2) |
| `tlsHandlers.ts` — `tls:importCert` / `tls:importKey` | engine FS layout | 🟡 engine returns `{ok}` only; the shell synthesises the path for the renderer (category 3) — **P7** |
| `runnerHandlers.ts` — `runner:exportReport` | engine → client | ✅ the engine returns `ArtifactResult`; the shell returns the user's own `filePath` (category 2) |
| `capture:shareJson` | engine → client | ✅ same — the engine returns `ArtifactResult`; the shell returns the user's own `filePath` (category 2) |
| `audit:export` | engine → client | ✅ the engine returns `ArtifactResult`; the shell returns the user's own `filePath` (category 2) |
| `renderer/types/window.ts:44, 45, 191` | renderer types encode the above | ⬜ deferred to P7 — the renderer must stay byte-identical through P6 |

**One more path leak was found and fixed while converting the ingress pair**, and it is the kind
that would have been easy to leave: `tls.importCert` read `blobContentPath(blobId)` directly, and
`blobContentPath()` validates the id's *shape* but never checks that the blob exists. So a swept or
released blob produced a raw `ENOENT: no such file or directory, open 'I:\…\blobs\blob_…\content'`
— an absolute engine path, in a protocol result, from a channel whose whole point is that no path
crosses the boundary. Fixed by going through `statBlob()` first, which converts it into a typed
`blob-not-found` with a path-free message. Asserted in
`tests/integration/fileOps.integration.test.ts`.

---

## Work item 7 — Renderer changes

Minimal, but not zero:

- `renderer/types/window.ts` — drop `filePath` from `ExportRequest`/`PreflightRequest`/`ImportRequest`
  result types; add the blob shapes.
- `renderer/components/modals/ImportExportModal.tsx:100–117` — stop parking `res.filePath` in state;
  carry `blobId` instead.
- Certificate UI — add the fingerprint display, the install step, the Firefox note, and the "CA changed,
  reinstall" warning.

**Everything else in the renderer is untouched.**

---

## How to start — the first three things

1. **Build the blob store with tests**, before touching any exporter. It is the foundation, and it has
   clear invariants (TTL, size cap, checksum, release).
2. **Convert one mechanical exporter and one mechanical importer end-to-end** (`environments-json` is the
   simplest pair). Prove the interface change, the blob wiring, and the client save all work together.
3. **Then bulk-convert the remaining 30** with the proven pattern. `workspace-zip` last.

---

## Acceptance criteria

Status as of **2026-09-17**. Items 1–6 are done; **item 5 is done as a mechanism and open as a UI**,
which is the only substantive work left in P3 and is bounded by non-negotiable #3. Item 7 is P7.

- [x] Round-trip test green for **all 17 formats** × export→blob→client-write and client-read→blob→import.
      — `tests/integration/importExportBlob.integration.test.ts` sweeps all 17 through the real
      command layer (`export.create` → `blob.put` → `import.preflight` → `import.commit` →
      `blob.release`), asserting the artifact's size and SHA-256 at every hop. The two
      format-level suites own codec fidelity through `importExportHarness.ts`.
- [x] `workspace-zip` export and import round-trip through the blob store. — Proven, and it is what
      found the `archiver` 8 default-import bug: this is the first test that ever executed the
      exporter. The archive is asserted to start with `PK` and to be non-empty.
- [x] **All five egress channels and all three ingress channels converted.** — `tls:exportCert`,
      `runner:exportReport`, `capture:shareJson` and `audit:export` joined `importExport:export`;
      `tls:importCert` and `tls:importKey` joined the `importExport` ingress pair.
      `grpc:addProto` / `soap:addWsdl` are closed as *no change needed* — see work item 4.
      New coverage is `tests/integration/fileOps.integration.test.ts` (29 tests), and the three
      suites that drive the real shell handlers (`runnerStorage`, `capture`, `auditLog`) were
      updated to register the engine half the way `src/ipc/handlers.ts` does.
- [x] No `filePath` in any protocol command or result type. — With one qualification:
      `history.list` / `history.diff` in `commands/audit.ts` still take a `filePath`, and should.
      That is a **workspace-relative** path the client learned from a previous `history.list`
      result, not a path on the user's machine — it never describes a client filesystem location.
      Every command that crosses the engine/client *file* boundary is `ArtifactResult`/`blobId`-shaped.
- [x] No `dialog.*` anywhere in `packages/engine`. — The remaining matches are prose in comments.
- [x] No `filePath.split` / `path.basename(filePath)` name derivation remaining. — In the engine.
      `src/ipc/handlers/clientHandlers.ts`, `src/ipc/importExportHandlers.ts` and
      `src/ipc/fileOpsClient.ts` still call `path.basename`/`path.extname`, which is correct and
      required: the **shell is the client**, and naming the user's own file is precisely its job.
- [x] Blob TTL sweep verified; abandoned blobs are reclaimed. (Work item 1, `tests/blob/sweep.test.ts`.)
- [x] Size cap enforced in `blob.put` with a clear error. (Work item 1, `blob-too-large`.)
- [ ] Cert: generate → fetch → install → verify fingerprint → regenerate → **drift detected**. (Item 5.)
      — **Mechanism done; the loop is not tested end-to-end, and should not be.** `identifyCert()`
      produces the fingerprint, `tls.certStatus` reports it, `summariseTrust()` classifies
      `none | trusted | stale` from `(record, engineFingerprint)`, and
      `tests/ipc/certLifecycle.test.ts` pins the comparison in all four directions including "the
      engine has no CA at all". What is *not* covered is the loop against a real OS trust store: doing
      that from a test run would mean mutating the developer's certificate store, which a suite must
      not do. The install/un-trust command shapes are asserted exactly instead
      (`tests/ipc/certTrust.test.ts`), and the loop needs a manual pass on each platform — see the
      unchecked P9/P12 rows in `plan/13-checklist.md`.
- [ ] Cert: `removeCert` reports partial success when only one half succeeds. (Item 5.)
      — **Both halves report separately and the partial paths are tested.** The engine returns
      `engineRemoved`; the shell composes `clientUntrusted` and `clientUntrustNote`, and keeps the
      drift record when the un-trust fails so the certificate can still be removed later. What remains
      is the UI that *shows* it: P7.
- [ ] Firefox warning present in the cert UI. (Item 5.)
      — **Detection done** (`detectFirefox()`, per-platform roots, unlaunched Firefox counted as
      present) and the flag is already on both results. The note itself is a renderer change: P7.

---

## Rollback

The blob layer is additive; the exporter interface change is not. Convert in small commits, one format
family at a time, so a bad change is revertible to a single family. Keep the old path-based interface
available behind a flag until the bulk conversion is proven.

---

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| Blob store placed outside the data volume, breaking Docker | Medium | Assert the blob root is under `dataDir()` in a test |
| `workspace-zip` streaming proves harder than expected | Medium | Do it last; it is the only genuinely non-mechanical case |
| Firefox trust issue ships unnoticed | **High** | Explicit UI note + a manual test step in the acceptance criteria |
| Size limits set too low for real HAR exports | Medium | Measure a real capture export before picking the number |
| Blob garbage accumulates in long-running Docker deployments | Medium | TTL sweep with a test that asserts reclamation |
