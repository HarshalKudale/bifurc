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
- `src/ipc/importExport/` has moved. ⬜ **Still in the shell** — this is work item 2's first step.
  Deferred deliberately, not overlooked: the blob store (work item 1) has **no** dependency on it,
  and `plan/04`'s own "How to start" sequences the store first. Moving 39 files and *then* rewriting
  34 of them in the same pass would make a bad change hard to attribute.

---

## Work item 1 — The blob store

> **✅ Implemented 2026-09-16** — `packages/engine/src/blob/{store,sweep,commands}.ts`, with
> `tests/blob/{store,sweep,commands}.test.ts` (88 tests against real temp data roots).
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
> **Deliberately not done here:** moving `src/ipc/importExport/` into the engine. The blob store has
> no dependency on it and the move is item 2's first step, so it is deferred rather than half-done —
> see the Preconditions note below.
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

| Channel | File | Change |
|---|---|---|
| `importExport:export` | `importExport/index.ts:11` | Remove `dialog.showSaveDialog`; return content/blob |
| `tls:exportCert` | `tlsHandlers.ts:23` | Remove dialog + `copyFileSync`; return blob |
| `runner:exportReport` | `runnerHandlers.ts:24` | Remove dialog; return content/blob |
| `capture:shareJson` | `systemHandlers.ts:272` | Remove dialog; return content/blob |
| `audit:export` | `syncHandlers.ts:147` | Remove dialog; return content/blob |

```ts
// target
export.create { kind, format, wsId, filename }
  → { inline: base64 }  |  { blobId, size, mimeType, sha256 }
```

---

## Work item 4 — Rewire the four ingress channels

Upload **once**, reference the blob for both preflight and commit. This also fixes the double-read in
`ImportExportModal.tsx`.

| Channel | File |
|---|---|
| `importExport:preflight` + `importExport:import` | `importExport/index.ts:33, 53` |
| `tls:importCert` | `tlsHandlers.ts:38` |
| `tls:importKey` | `tlsHandlers.ts:50` |
| `grpc:addProto` (for consistency) | `grpcHandlers.ts:62` |

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

---

## Work item 5 — The certificate lifecycle

The genuinely new design work. Full detail in `File_Ops_Protocol.md` §6.

### 5a. Two halves, one fingerprint

```
tls.generate  → { fingerprint: "sha256:ab12…", certBlobId, keyBlobId }
tls.status    → { generated, fingerprint, certBlobId }
tls.removeCert→ { engineRemoved: bool, clientUntrusted: bool }   // two-sided, may partially succeed
```

The client stores the fingerprint it installed and compares on connect. On mismatch: *"the engine's CA
has changed since you trusted it — reinstall."* Without this, a regenerated CA produces opaque TLS
errors with no explanation.

### 5b. Move the install off the engine

Delete `installCA` from `packages/engine/src/proxy/certManager.ts:41`. The client performs it.

| Platform | Command | Privilege |
|---|---|---|
| Windows | `certutil -addstore -user Root <cert>` | none |
| macOS | `security add-trusted-cert -d -r trustRoot -k <loginKeychain> <cert>` | none |
| Linux | `trust anchor <cert>` (preferred) or `sudo update-ca-certificates` | root |

**Verify `trust anchor` works without elevation** (p11-kit user store) — it is the difference between a
clean Linux UX and a `pkexec` prompt. Fall back to `pkexec`, then to written instructions.

### 5c. The Firefox problem

**Firefox does not use the OS trust store** — it ships its own NSS store. Installing via `certutil` or
`security` makes Chrome, Edge and Safari work and **leaves Firefox broken.**

This is a top-tier support burden for a localhost proxy tool, and it gets worse with a remote engine
because users will blame the engine. Handle it deliberately:
- Detect Firefox presence.
- Show an explicit "Firefox needs a separate step" note in the cert UI.
- Optionally offer direct NSS installation.

Do not discover this in issue reports.

---

## Work item 6 — Remove the path leaks

Every one of these must become a blob reference or be dropped. Verified against source:

| Location | Leak |
|---|---|
| `importExport/types.ts` — `ExportResult.filePath` | engine → client |
| `importExport/types.ts` — `PreflightResult.filePath` | engine → client, **and back** |
| `importExport/types.ts` — `ImportRequest.filePath` | client → engine |
| `tlsHandlers.ts:11` — `tls:generate` returns `certPath`, `keyPath` | engine FS layout |
| `tlsHandlers.ts:34` — `tls:exportCert` returns `filePath` | |
| `tlsHandlers.ts:46, 59` — `tls:importCert`/`importKey` return engine `path` | engine FS layout |
| `runnerHandlers.ts:44` — `runner:exportReport` returns `filePath` | |
| `capture:shareJson` returns `filePath` | |
| `renderer/types/window.ts:44, 45, 191` | renderer types encode the above |

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

- [ ] Round-trip test green for **all 17 formats** × export→blob→client-write and client-read→blob→import.
- [ ] `workspace-zip` export and import round-trip through the blob store.
- [ ] No `filePath` in any protocol command or result type.
- [ ] No `dialog.*` anywhere in `packages/engine`.
- [ ] No `filePath.split` / `path.basename(filePath)` name derivation remaining.
- [ ] Blob TTL sweep verified; abandoned blobs are reclaimed.
- [ ] Size cap enforced in `blob.put` with a clear error.
- [ ] Cert: generate → fetch → install → verify fingerprint → regenerate → **drift detected**.
- [ ] Cert: `removeCert` reports partial success when only one half succeeds.
- [ ] Firefox warning present in the cert UI.

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
