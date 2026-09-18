# Protocol changes

The protocol is **frozen at the end of P1**. Any change after that must be recorded here with a
justification, because every change costs a multiple of what it would have cost in P1.

**Rule:** if you are about to change a command name, a payload shape, an error code, or an event
envelope — write the entry first, then make the change.

---

## Template

```markdown
### YYYY-MM-DD — <short title>

**Phase:** P< n >
**Type:** breaking | additive | fix
**Change:** what changed
**Why it could not wait:**
**Affected:** commands / events / clients
**Migration:** what each client must do
**Approved by:**
```

---

## Change log

### 2026-09-15 — Protocol frozen at v1.0.0 (89 commands)

**Phase:** P1
**Type:** n/a — this is the freeze point, not a change
**Change:** `@bifurc/protocol` (`packages/protocol/`) built and frozen at `PROTOCOL_VERSION = "1.0.0"`,
covering all 80 ENGINE + 14 SPLIT (engine-half) commands enumerated in
`plan/handler-classification.md`, collapsed per work item 2 to 89 wire commands (36 generated CRUD
channels → the 6 generic `entity.*` commands). Full command list, params schemas, error taxonomy,
event envelope (with `seq`), `hello` handshake, and `subscribe`/`unsubscribe` are in
`packages/protocol/src/`.
**Why it could not wait:** n/a — this entry exists so every subsequent row has a clear baseline to
diff against.
**Affected:** the entire command surface, for the first time.
**Migration:** none yet — P2 onward consume this package; no client has been rewired to it yet
(that starts at P2/P5/P6).
**Approved by:** this session, per `plan/02-phase-1-protocol.md` acceptance criteria (all met — see
"Gate" note below).

*No changes since freeze.*

---

### 2026-09-17 — `import.commit`'s `collisionStrategy` enum corrected; `filePath` removed from the `export.*` / `import.*` schemas

**Phase:** P3 (work item 2 — the conversion pass)
**Type:** fix (the enum) + breaking-in-theory/empty-in-practice (the `filePath` removals)
**Change:** two things in `packages/protocol/src/commands/export.ts`.

1. `ImportCommitParams.collisionStrategy` was frozen as
   `z.enum(["skip", "overwrite", "rename"])`. The real local type
   (`packages/engine/src/importExport/types.ts` — `CollisionStrategy`) and **every** real call site,
   including all four collision tests in `tests/integration/importExport.integration.test.ts`, use
   `["keep", "override", "new"]`. As frozen, the schema would have rejected every real import with
   an "Invalid payload" error — not a subtle mismatch, a total failure of the feature. Corrected to
   `z.enum(["keep", "override", "new"])`.
2. `filePath` is gone from `ImportPreflightParams`, `ImportCommitParams`, `ExportCreateResult` and
   `ImportPreflightResult`, and `ImportPreflightParams.blobId` / `ImportCommitParams.blobId` are now
   **required** rather than optional. This is `plan/04` work item 6 and its acceptance criterion
   "no `filePath` in any protocol command or result type" (`File_Ops_Protocol.md` §8).

**Why it could not wait:**
- (1) is a **defect in the frozen schema**, not a preference. It could not be honoured by any
  implementation, so "freezing" it protected nothing. It is recorded here rather than quietly
  patched because the freeze rule is explicit.
- (2) could not wait for P4 because P3 is the phase that implements the blob layer these schemas
  describe; leaving a `filePath` branch in the wire contract would mean P4 has to remove a field
  that was never served.

**Why this is not a `PROTOCOL_VERSION` bump.** Both schemas describe commands that **no
implementation has ever registered**. `import.commit`, `import.preflight` and `export.create` were
declared in P1 and left unimplemented — `packages/engine/src/importExport/registry.ts` is where they
land, and P3 is the phase that lands them. A client cannot depend on an enum that only ever
rejected, and the only in-repo client (`renderer/types/ipc.ts`, mirrored by hand) already sends
`keep`/`override`/`new`. The change makes the schema accept what the only client sends, and removes
fields no client ever read.

**Affected:** `export.create`, `import.preflight`, `import.commit` — schemas only. No command name,
error code or event envelope changes. The four frozen companion-extension commands
(`mock:add`, `request:add`, `folder:add`, `config:get`) are **untouched**.

**Migration:** none required — no client has been wired to these three commands yet. P4's transport
and P5's client consume the corrected shapes from the start.

**Approved by:** this session. Note for P4: the enum's values are now the same strings as
`CollisionStrategy`, so the transport no longer needs a translation table between the wire and the
engine's own type — that translation table was the thing the mismatch would have forced.

---

### 2026-09-17 — the four remaining artifact commands adopt `BlobRef`; the two cert ingress params take a `blobId`

**Phase:** P3 (work items 3–4 — the rest of the conversion pass)
**Type:** breaking in shape, **empty in practice** — none of these six commands has ever been registered
**Change:** five things, all in `packages/protocol/src/commands/`.

1. **A shared result type.** `blob.ts` gains `ArtifactResult`, and `ArtifactMeta` for its metadata
   half:

   ```ts
   export type ArtifactMeta = { suggestedName: string; size: number; mimeType: string; sha256: string };
   export type ArtifactResult =
     | ({ ok: true } & ArtifactMeta & BlobRef)
     | { ok: false; error: string; canceled?: boolean };
   ```

2. **`ExportCreateResult` becomes an alias of it.** The shape is *identical* to what was already
   there (`{ok:true; suggestedName; size; mimeType; sha256} & BlobRef`); this is a de-duplication,
   not a wire change. `export.create` is unaffected.

3. **Four results adopt it**, replacing `{ ok: boolean; content?: string; suggestedFilename?: string;
   error?: string }`:
   `TlsExportCertResult`, `RunnerExportReportResult`, `AuditExportResult`, `CaptureShareJsonResult`.
   `content: string` → `inline: base64 | blobId`, plus `size`, `mimeType`, `sha256`, and
   `suggestedFilename` → `suggestedName`.

4. **`TlsImportCertParams` and `TlsImportKeyParams`** change from `{ content: string }` to
   `{ blobId: string }`.

5. **`RunResult` / `RunReport` widened** with the fields the renderer's real
   `RunnerRequestResult` / `CollectionRunReport` already carry and the frozen schema dropped:
   `requestId`, `url`, `testLogs`, `preScriptError`, `postScriptError`, and `durationMs` on a test
   entry. All optional, all additive.

**Why it could not wait:**

- (3) is not a preference: `blob.ts` has said since P1 that "domain commands that produce an artifact
  (`export.create`, `runner.exportReport`, `audit.export`, `capture.shareJson`) return this shape",
  and `File_Ops_Protocol.md` §4 names all five egress channels with the target
  `{ inline: base64 } | { blobId, size, mimeType }`. The four interfaces were the *pre*-conversion
  declarations that `blob.ts`'s own comment says are "adopted during P3's conversion pass". Leaving
  them as `content: string` would mean two mechanisms for the same thing, and `content` has no size
  bound — `audit.export` with `limit: 0` returns the **entire** audit log, which is exactly the
  payload `BLOB_INLINE_THRESHOLD_BYTES` exists to route around.
- (4) follows from (3): if egress is `BlobRef`-shaped then ingress is `blob.put` + `blobId`, per
  `File_Ops_Protocol.md` §5. A `content: string` ingress param would bypass `blob.put`'s size cap
  and its three integrity checks.
- (5) is a **latent data-loss bug**, not a tidy-up. `z.object()` strips unknown keys, so routing the
  real report through the frozen schema would have silently truncated the JSON export — the HTML
  renderer happens to use only fields the schema had, which is precisely why this would have shipped
  unnoticed. Same class as the `collisionStrategy` defect: a frozen schema that describes a shape no
  caller has.

**Why this is not a `PROTOCOL_VERSION` bump.** Same argument as the entry above, and it is worth
stating plainly: **none of these six commands has ever been registered in a `CommandRegistry`.** They
were declared in P1 and left unimplemented; the shell performed the work inline in
`tlsHandlers.ts` / `runnerHandlers.ts` / `syncHandlers.ts` / `systemHandlers.ts` instead. A client
cannot depend on a payload no server has ever accepted. The only in-repo client (`renderer/types/ipc.ts`,
mirrored by hand) does not name any of these commands — it calls the `ipcMain` **channels**, whose
renderer-facing return shapes are **unchanged** by this entry and stay byte-identical per
`README.md` non-negotiable #3. The shell bridges channel → command, exactly as it already does for
the other ~112.

**Affected:** `tls.exportCert`, `tls.importCert`, `tls.importKey`, `runner.exportReport`,
`audit.export`, `capture.shareJson` — schemas only. No command name, error code or event envelope
changes. The four frozen companion-extension commands (`mock:add`, `request:add`, `folder:add`,
`config:get`) are **untouched**. `export.create` / `import.preflight` / `import.commit` are
**unchanged** (see item 2 — alias only).

**Migration:** none required — no client has been wired to these six commands. Note for the shell
(P3 item 3–4, done in the same session): the `filename` the client picks in the save dialog is passed
to the command so `suggestedName` can echo it, and the client writes `inline` or pulls `blob.read`,
then releases — identical to the `importExport:export` path.

**Approved by:** this session.

---

### 2026-09-17 — the certificate lifecycle: `tls.generate` / `tls.certStatus` report a fingerprint instead of paths; `tls.removeCert` becomes two-sided

**Phase:** P3 (work item 5 — the certificate lifecycle)
**Type:** breaking for three result shapes; **empty in practice** (see below) + `fix` for one
**Change:** three things, all in `packages/protocol/src/commands/tls.ts`.

1. **`TlsGenerateResult` loses `certPath` / `keyPath`, gains `fingerprint`.**

   ```ts
   // before                                  // after
   { ok: boolean; certPath?: string;          { ok: boolean;
     keyPath?: string; error?: string }         fingerprint?: string; error?: string }
   ```

2. **`TlsCertStatusResult` loses `certPath` / `keyPath`, gains `fingerprint`.**

   ```ts
   // before                                  // after
   { generated: boolean;                      { generated: boolean;
     certPath: string | null;                   fingerprint: string | null }
     keyPath: string | null }
   ```

   `fingerprint` is `sha256:<64 lowercase hex>` over the certificate's DER — the same string
   browsers and the OS show — computed with `crypto.X509Certificate#fingerprint256`. It is `null`
   in two distinct situations, and the distinction is deliberate: **no CA** (`generated: false`) and
   **the file on disk is not a certificate** (`generated: true, fingerprint: null`). The second is
   the state the pre-P3 code had no way to report; see the `fix` note below.

3. **`TlsRemoveCertResult` becomes two-sided.** `{ ok: boolean }` →
   `{ ok: boolean; engineRemoved: boolean }`. The engine reports only **its own half** — it cannot
   know whether the client's OS trust store was cleaned. `clientUntrusted` is composed by the shell
   and never crosses the wire, because only the client has a trust store
   (`File_Ops_Protocol.md` §6.1).

**Why it could not wait:** P3 is the phase that owns the certificate lifecycle
(`File_Ops_Protocol.md` §6), and the whole point of item 5 is that the two halves live on different
machines. A result type that hands back an engine filesystem path cannot survive P4's transport: the
path is meaningless on the client, and `File_Ops_Protocol.md` §8 lists it as a boundary violation.
The fingerprint is the replacement, and it is the *only* thing that makes CA drift detectable — a
regenerated CA otherwise produces opaque TLS errors with no explanation.

**Why this is not a `PROTOCOL_VERSION` bump.** This is the honest version of the argument, and it is
weaker than the two entries above, so it is stated plainly rather than by precedent:

- The two prior entries could say *"no implementation has ever registered these commands."* **That
  is not true here.** `tls.generate`, `tls.certStatus` and `tls.removeCert` **are** registered — in
  `src/ipc/handlers/tlsHandlers.ts`, the shell, which has been serving them since P2.
- What is true is that **the shell is their only consumer in the tree**, it is updated in the same
  change, and it is the *server* for these commands rather than a client of them. The renderer is
  not a protocol client: it calls the `ipcMain` **channels**, and every one of those return shapes
  is **unchanged** by this entry (`README.md` non-negotiable #3). The shell bridges channel →
  command exactly as it already does for the other ~112.
- No wire client exists yet. P4 lands the transport; P5/P6 land the first real client. A version
  bump is a lockstep signal, and there is no one to signal.

**Affected:** `tls.generate`, `tls.certStatus`, `tls.removeCert` — result types only. No command
name, no params schema, no error code, no event envelope changes. `tls.exportCert` /
`tls.importCert` / `tls.importKey` are **untouched** (they were converted in items 3–4). The four
frozen companion-extension commands (`mock:add`, `request:add`, `folder:add`, `config:get`) are
**untouched**.

**Migration:** the shell's `tls:generate` / `tls:certStatus` handlers synthesise `certPath` /
`keyPath` from `appDataDir()` for the renderer, exactly as `tls:importCert` already does — this is
category 3 in `plan/04` item 6 (an engine FS layout detail the shell computes), and it is removed in
P7, not here. Note that `tls:certStatus`'s renderer-facing shape is preserved even though **no
renderer code calls it**: `renderer/types/window.ts:188` declares it, and non-negotiable #3 is about
the surface, not about whether something currently reads it.

**A `fix` rode along: the corrupt-CA state is now reportable.** `packages/engine/src/fileOps/commands.ts`
records that `tls.importCert` does not validate what it imports — the pre-P3 handler was
`copyFileSync` and accepted anything — so *"a bad `ca-cert.pem` surfaces as an opaque TLS failure
later instead of as a clear error at import time."* That gap is **not** closed by rejecting bad
input at import: the import contract deliberately preserves arbitrary bytes (asserted by
`tests/integration/fileOps.integration.test.ts`, including a deliberately non-ASCII payload), and
narrowing it would be a second, unrequested behaviour change. It is closed **at status time**
instead: `getCertStatus()` now reads the certificate and returns `generated: true, fingerprint:
null` for a file that is not a certificate. Same information, at the moment a UI asks, without
touching the import path. The pre-existing fixtures that write `"cert"` as the file body become the
corrupt-cert case for free.

**Approved by:** this session, after an explicit scoping question. The mechanism half of item 5 was
chosen — engine fingerprint + `tls.status`, `installCA` moved to the client, `uninstallCA` added,
drift and Firefox detection — with the renderer-facing acceptance criteria left for P7.

---


## Guidance

### Breaking changes

Require a `PROTOCOL_VERSION` major bump. Every client must be updated in lockstep (D9).

Examples: renaming a command, removing a field, changing a payload type, changing an error code's meaning.

### Additive changes

Minor bump. Clients must tolerate their absence — this is what capability negotiation is for
(`02-phase-1-protocol.md` item 7).

Examples: a new command, a new optional field, a new capability string.

### Do not do this

- Rename a command for aesthetics.
- Change a payload shape to be "cleaner" while clients are in flight.
- Add a command that duplicates an existing one because the name is inconvenient.
- Change the error envelope's structure.

### Before adding any command, check

1. Does an existing command already do this with a different payload?
2. Should this be a `kind` discriminator on `entity.*` instead of its own command? (This is how 36 CRUD
   channels became 6.)
3. Does the payload contain a filesystem path? It must not (`File_Ops_Protocol.md` §8).
4. Is there a schema, and does the engine validate against it?
5. Is it in the right scope class?
