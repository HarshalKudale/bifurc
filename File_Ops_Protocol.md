# File & System Operations Protocol

**Companion to:** `Decoupling_Plan_v2.md` §5 workstream 3 ("File-I/O semantics redesign").
**Decision recorded:** the engine is the **authority for generated artifacts**; the client is the
**authority for the user's machine**. The client mediates every operation that touches the user's
filesystem or OS trust store.

**Verdict: this is the right model, and it is cheaper than the v1 estimate implied** — because the
import/export subsystem is already *string-in / string-out*, and the correct client-side read pattern
already exists in three call sites. It is a generalisation job, not a redesign. The genuinely new
design work is the certificate lifecycle (§6).

---

## 1. The model in one line

```
Engine   owns: its data dir, generated artifacts, the CA keypair, workspace files
Client   owns: the user's filesystem, native dialogs, the OS trust store, the browser's trust

Every crossing is bytes over RPC, never a path.
```

---

## 2. What I verified — the codebase is closer to this than expected

### 2.1 The import/export interface is already string-in / string-out

This is the single biggest finding. `src/ipc/importExport/registry.ts`:

```ts
export interface ExporterFn { run(wsId: string, filePath: string): Promise<ExportResult>; }
export interface ImporterFn {
  preflight(wsId: string, filePath: string): PreflightResult;
  run(wsId: string, filePath: string, strategy: CollisionStrategy): Promise<ImportResult>;
}
```

And **every** implementation is the same shape:

```ts
// exporters/*.ts — 17 files
fs.writeFileSync(filePath, JSON.stringify(payload, null, 2), "utf-8");
return { ok: true, filePath };

// importers/*.ts — 17 files
const data = JSON.parse(fs.readFileSync(filePath, "utf-8"));
```

**`filePath` is a pure transport detail.** These functions build a string and write it, or read a
string and parse it. Nothing else about the path matters.

Consequence: the interface change is

```ts
run(wsId: string): Promise<{ content: string; suggestedName: string }>
preflight(wsId: string, content: string): PreflightResult
run(wsId: string, content: string, strategy: CollisionStrategy): Promise<ImportResult>
```

— and **32 of the 34 files** change in the same mechanical one-line way.

**The two exceptions** are `workspace-zip` (both directions), which are genuinely stream-based:

```ts
// exporter — archiver
const output = fs.createWriteStream(filePath);
archive.pipe(output);

// importer — unzipper requires a real file
const directory = await unzipper.Open.file(filePath);
```

`unzipper.Open.file()` will not take a buffer. This is why the engine needs a **real staging
directory**, not an in-memory blob store — see §3.2.

### 2.2 The correct client-side read pattern already exists

`systemHandlers.ts:232` — `dialog:openFile` — already does exactly the right thing:

```ts
const { filePaths, canceled } = await dialog.showOpenDialog(win, {...});
const buffer = fs.readFileSync(filePath);
return { name: path.basename(filePath), size: stat.size,
         base64: buffer.toString("base64"), mimeType: mimeMap[ext] ?? "application/octet-stream" };
```

It even enforces a **1 MB limit**. And it is already consumed in three places:
`BinaryViewer.tsx:58`, `MultipartEditor.tsx:67`, `ProtoExplorer.tsx:147`.

So the model you're describing is **already the established pattern in this codebase for binary
payloads** — it simply hasn't been applied to the import/export/cert flows. That is a strong signal
this will land cleanly.

### 2.3 The actual blocker: a filesystem path round-trips through React state

`renderer/components/modals/ImportExportModal.tsx`:

```ts
// line 100 — engine opens the dialog and reads the file
const res = await window.api.preflightImport(req);

// line 108 — the path is parked in renderer state
dispatch({ type: "collision", filePath: res.filePath!, itemCount: res.itemCount ?? 0, ... });

// line 110 — or passed straight back
await applyImport(res.filePath!, "keep");

// line 117 — and sent back to the engine
const req: ImportRequest = { kind, format, wsId, filePath, collisionStrategy: strategy };
```

Two problems, one of which is a latent bug today:

1. **For a remote engine, `res.filePath` is meaningless** — it is a path on the engine's filesystem,
   and it is being echoed back as if it were the client's.
2. **The file is read twice.** `preflight` reads it, then `importer.run()` reads it again from the same
   path. A staged-blob model fixes this as a side effect: upload once, reference twice.

---

## 3. The blob layer

Three new protocol primitives, orthogonal to the domain commands.

### 3.1 Primitives

```
// ── Ingress: client → engine ─────────────────────────────────────────────
blob.put    { filename, mimeType, size, data: base64 }   // or chunked stream
            ← { blobId, sha256 }

// ── Egress: engine → client ──────────────────────────────────────────────
blob.stat   { blobId }
            ← { size, mimeType, sha256, filename, ttlRemainingMs }

blob.read   { blobId, offset?, length? }
            ← { data: base64, eof }                       // chunked, resumable

blob.release{ blobId }                                    // client is done
            ← { ok }
```

### 3.2 Design rules

| Rule | Rationale |
|---|---|
| **Staging is a real directory**, `<dataDir>/blobs/<id>` | `unzipper.Open.file()` and `archiver` need real paths. Do not use an in-memory store. |
| **Blobs live on the data volume** | In Docker, a blob store outside the volume loses in-flight transfers on restart. |
| **TTL ~1 hour + periodic sweep** | A Docker volume will otherwise accumulate garbage forever. `blob.release` is the fast path; the sweep is the safety net. |
| **Inline below a threshold** | Artifacts under ~1 MB (matches the existing `dialog:openFile` limit) return `{ inline: base64 }` instead of a `blobId`. One round-trip for the common case. |
| **The client supplies `filename`** | The engine must never derive a display name from a path — see §8. |
| **Client shows the dialog first** | Fail fast on cancel. Do not make the engine build a 200 MB workspace zip the user then cancels. |

---

## 4. Egress flows (engine → client)

**Current** (e.g. `importExport:export`, `importExport/index.ts:11`):

```
client: invoke("importExport:export", {kind, format, wsId})
engine: dialog.showSaveDialog()  → filePath          ← engine owns the dialog (wrong)
        exporter.run(wsId, filePath)                 ← writes to the user's disk (wrong)
        return { ok, filePath }
```

**Target:**

```
client: dialog.showSaveDialog()  → localPath          ← client owns the dialog
client: invoke("export.create", { kind, format, wsId, filename })
engine: exporter.run(wsId) → { content, suggestedName }
        returns { inline: base64 }  |  { blobId, size, mimeType }
client: writes bytes → localPath                       ← client owns the write
```

Applies to five channels: `importExport:export`, `tls:exportCert`, `runner:exportReport`,
`capture:shareJson`, `audit:export`.

---

## 5. Ingress flows (client → engine)

```
client: dialog.showOpenDialog() → localPath
client: reads bytes, enforces size limit
client: invoke("blob.put", { filename, mimeType, data })  → blobId    ← uploaded ONCE

client: invoke("import.preflight", { kind, format, wsId, blobId })
        ← { itemCount, collisionIds }
        [user resolves collisions]
client: invoke("import.commit", { kind, format, wsId, blobId, collisionStrategy })
        ← { imported, skipped }
```

Note the upload-once property fixes the double-read in §2.3.

Applies to: `importExport:preflight` + `importExport:import`, `tls:importCert`, `tls:importKey`,
and `grpc:addProto` / `soap:addWsdl` where the client currently reads the file itself.

**Imports are transform-and-discard, not store.** A Postman collection is parsed into entities and the
source file never needs to persist on the engine — so `blob.release` immediately after commit. Contrast
with WSDL / proto / cert, which *do* become persisted entities and are copied into the workspace store.

---

## 6. The certificate lifecycle — where the real design work is

Certs are **not** just "fetch a file". There are two halves that live on different machines and can
silently drift apart.

### 6.1 Two halves, and the drift problem

```
ENGINE half                          CLIENT half
─────────────                        ────────────
generate CA keypair                  fetch cert bytes
store in dataDir                     install into OS trust store
serve cert + fingerprint             report trust status
delete on removeCert                 un-trust on removeCert
```

Today `tlsHandlers.ts` fuses both halves into engine-side handlers, which is why `tls:installCA` shells
out to the OS from inside the engine (`certManager.ts:41`).

**The drift problem:** a user can generate a CA on engine A, install it on client B, then regenerate on
engine A — and the client still trusts the *old* CA. Symptom: opaque TLS errors with no explanation.

**Mitigation — carry a fingerprint:**

```
tls.status → { generated: bool, fingerprint: "sha256:ab12…", certBlobId }
```

The client stores the fingerprint it installed and compares on connect. On mismatch: "the engine's CA
has changed since you trusted it — reinstall." This is cheap and removes a whole class of confusing
bug reports.

### 6.2 Privilege matrix for install

| Platform | Command | Privilege | Notes |
|---|---|---|---|
| Windows | `certutil -addstore -user Root <cert>` | **none** | User store — no elevation. Cleanest path. |
| macOS | `security add-trusted-cert -d -r trustRoot -k <loginKeychain> <cert>` | **none** | Login keychain. May prompt for keychain password. |
| Linux | `sudo update-ca-certificates` | **root** | The outlier. |

Linux needs a different approach in a GUI client. Options, in preference order:

1. `trust anchor <cert>` (p11-kit) — often works for the user store without root. **Verify this.**
2. `pkexec` — triggers a polkit graphical auth prompt. Acceptable UX.
3. Fall back to: write the cert to a known location, show exact instructions, open the containing
   folder. Ugly but never fails.

### 6.3 The Firefox gotcha — flag this in the UI

**Firefox does not use the OS trust store.** It ships its own NSS store. Installing the CA via
`certutil` (Windows) or `security` (macOS) makes Chrome, Edge and Safari trust it — **Firefox will
still fail.**

For a localhost proxy tool this is a top-tier support burden, and it gets *worse* with a remote engine
because users will blame the engine. Two mitigations:

- Show an explicit "Firefox needs a separate step" note in the cert UI.
- Optionally detect Firefox and offer to install into its NSS store directly.

This is worth handling deliberately rather than discovering it in issue reports.

---

## 7. Channel migration table

| Channel | Today | Target | Class |
|---|---|---|---|
| `importExport:export` | dialog + write, returns `filePath` | `export.create` → blob; client saves | Egress |
| `importExport:preflight` | dialog + read, returns `filePath` | `import.preflight(blobId)` | Ingress |
| `importExport:import` | takes `filePath` | `import.commit(blobId, strategy)` | Ingress |
| `importExport:formats` | pure data | unchanged | — |
| `tls:generate` | returns `certPath`, `keyPath` | `tls.generate` → fingerprint + blobIds | Egress |
| `tls:installCA` | **engine shells out to OS** | **client-side** | Privileged |
| `tls:exportCert` | dialog + `copyFileSync` | blob; client saves | Egress |
| `tls:certStatus` | engine file existence check | engine status + client fingerprint compare | Split |
| `tls:importCert` | dialog + copy into `appDataDir` | `blob.put` → engine stores | Ingress |
| `tls:importKey` | dialog + copy into `appDataDir` | `blob.put` → engine stores | Ingress |
| `tls:removeCert` | engine `unlinkSync` | engine unlink **+ client un-trust** | Split |
| `runner:saveReport` | writes into engine workspace | **unchanged** — engine-side persistence | — |
| `runner:exportReport` | dialog + write | blob; client saves | Egress |
| `audit:export` | dialog + write | blob; client saves | Egress |
| `capture:shareJson` | dialog + write | blob; client saves | Egress |
| `dialog:openFile` | returns `{name,size,base64,mimeType}` | **client-side entirely** | Client |
| `dialog:pickFilePath` | returns path | **client-side entirely** | Client |
| `dialog:pickFolderPath` | returns path | **client-side entirely** | Client |
| `shell:openExternal` | `shell.openExternal(url)` | **client-side entirely** | Client |
| `app:checkUpdate` | uses `app.getVersion()`, `process.platform` | **split** — shell version (client) vs engine version (engine) | Split |
| `soap:fetchWsdl` | HTTP fetch | unchanged — network, not filesystem | — |
| `grpc:addProto` | client reads file, sends object | `blob.put` for consistency | Ingress |

**Totals:** 5 egress, 4 ingress, 1 privileged, 3 split, 4 client-only, 2 unchanged, plus 34
exporter/importer files.

---

## 8. Path leaks to remove

Every one of these exposes a filesystem path to the other side and must become a blob reference or be
dropped:

| Location | Leak |
|---|---|
| `importExport/types.ts` — `ExportResult.filePath` | engine → client |
| `importExport/types.ts` — `PreflightResult.filePath` | engine → client, **and back** |
| `importExport/types.ts` — `ImportRequest.filePath` | client → engine |
| `tlsHandlers.ts:11` — `tls:generate` returns `certPath`, `keyPath` | engine FS layout |
| `tlsHandlers.ts:34` — `tls:exportCert` returns `filePath` | client path echoed by engine |
| `tlsHandlers.ts:46` — `tls:importCert` returns engine `path` | engine FS layout |
| `tlsHandlers.ts:59` — `tls:importKey` returns engine `path` | engine FS layout |
| `runnerHandlers.ts:44` — `runner:exportReport` returns `filePath` | client path echoed by engine |
| `capture:shareJson` returns `filePath` | client path echoed by engine |
| `renderer/types/window.ts:44,45,191` | renderer types encode the above |

And one behavioural leak — the only place a **display name is derived from a path**:

```ts
// src/ipc/importExport/importers/environments-dotenv.ts:55
const name = filePath.split(/[/\\]/).pop()?.replace(/\.env.*$/, "") ?? "Imported";
```

Under a remote engine this either produces a garbage name or leaks a fragment of the user's local path
into an entity name. Must become an explicit `filename` parameter supplied by the client.

---

## 9. Gotchas worth writing down

1. **`unzipper.Open.file()` needs a real path.** Forces a real staging directory. Do not attempt an
   in-memory-only blob store.
2. **`archiver` pipes to a `WriteStream`.** Stage to disk, then serve. Or refactor to collect into a
   buffer for small workspaces — but the staging path is simpler and handles growth.
3. **Docker volume.** Blob staging must live on the mounted volume.
4. **Size limits.** `dialog:openFile` already caps at 1 MB. Define a protocol-level max (suggest 100 MB
   for imports, generous for exports) and enforce it in `blob.put` before allocating.
5. **`tls:removeCert` is now a two-sided operation.** Removing engine-side without un-trusting
   client-side leaves the client trusting a CA the engine no longer has. Both halves must run, and the
   result must report if only one succeeded.
6. **Export while the engine is remote and the user cancels.** With dialog-first ordering this is free.
   Without it, you've built an artifact for nothing.
7. **`runner:saveReport` is correctly engine-side** — it writes to the workspace store for history, and
   `runner:getHistory` reads it back. Don't accidentally move it to the client during this refactor.

---

## 10. Effort

| Item | Effort |
|---|---:|
| Blob layer (put/stat/read/release, staging, TTL sweep, size limits) | 3–4 d |
| 34 exporter/importer interface changes (32 mechanical, 2 stream-based) | 2–3 d |
| 5 egress channels rewired to dialog-first + blob | 2 d |
| 4 ingress channels rewired to upload-once | 2 d |
| Cert lifecycle: two halves, fingerprint drift, un-trust, platform install | 3–5 d |
| Renderer changes (`ImportExportModal`, `window.ts` types, cert UI, Firefox note) | 2–3 d |
| Tests (blob layer, cert fingerprint drift, zip round-trip, size limits) | 2–3 d |
| **Total** | **1.5–2.5 weeks** |

**This revises v1's workstream 3 from 1–2 weeks to 1.5–2.5 weeks.** The extra time is the blob lifecycle
and the certificate two-halves work — neither of which was visible before reading the cert handlers.
Tier 1 absorbs it: **10–15 weeks** rather than 10–14.

The good news is that the mechanical bulk (34 files) is genuinely mechanical, and the pattern is
already proven in this codebase by `dialog:openFile`.

---

*Based on `bifurc-monorepo` v0.3.1. All interface shapes, counts and line references verified against
source.*
