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
rejected, and the only in-repo client (`packages/gui/src/types/ipc.ts`, mirrored by hand) already sends
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
cannot depend on a payload no server has ever accepted. The only in-repo client (`packages/gui/src/types/ipc.ts`,
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
renderer code calls it**: `packages/gui/src/types/window.ts:188` declares it, and non-negotiable #3 is about
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

### 2026-09-19 — `AuditListParams` gains `filePath` / `fromTs` / `toTs`

**Phase:** P6 (step 3d — unblocking the four pinned keys)
**Type:** additive (three new optional fields on a `.strict()` object)
**Change:** one schema, in `packages/protocol/src/commands/audit.ts`.

```ts
export const AuditListParams = z.object({
  workspaceId: z.string().optional(),
  entity:      z.string().optional(),
  action:      z.string().optional(),
  entityId:    z.string().optional(),
+ filePath:    z.string().optional(),   // workspace-relative, e.g. "mocks/root/mock_abc.json"
+ fromTs:      z.number().optional(),   // inclusive, epoch ms
+ toTs:        z.number().optional(),   // inclusive, epoch ms
  search:      z.string().optional(),
  limit:       z.number().int().optional(),
  offset:      z.number().int().optional(),
}).strict();
```

The three fields are exactly the members `QueryLogOptions` (`packages/engine/src/store/gitStore.ts`) has
and this schema lacked.

**Why it could not wait:** `audit.list` was the single `NARROWED` entry in the step-3b ratchet, and the
category's own definition is *"a decision, not a move"* — so the decision was taken. But the stronger
reason is that the narrowing turned out to be **live, not latent**: the ratchet's justification was
"nothing currently passes those — `auditList` exists on `window.api` and no renderer code calls it",
and that check was a grep for the *method name*. `packages/gui/src/panels/AuditLogPanel.tsx:61–62` reaches the
same command through `window.api` and sets `fromTs`/`toTs` whenever its date filter is used. Because the
schema is `.strict()`, that difference is a `BAD_REQUEST` rather than an ignored filter — so routing
`audit.list` before this change would have broken the audit panel's date range, with every unit test in
the repo still green (they drive the handler, not the panel).

**Why this is not a `PROTOCOL_VERSION` bump.** Same argument as the three entries above, and it holds
for the same measurable reason: **`audit.list` has never been registered in a `CommandRegistry`.**
`tests/ipc/handlers.test.ts`'s ratchet asserts that directly, and `plan/07` §5b recorded it. Its only
server has been the shell's `audit:list` `ipcMain` body, whose renderer-facing shape is **unchanged** by
this entry (`README.md` non-negotiable #3) — it forwards the options object to `queryLog`, which has
accepted all three fields all along. A client cannot depend on a payload no server has ever accepted.

**Affected:** `audit.list` — params only. No command name, no result type, no error code, no event
envelope changes. The four frozen companion-extension commands (`mock:add`, `request:add`,
`folder:add`, `config:get`) are **untouched**. `audit.diff`, `audit.export` and `history.*` are
untouched — `HistoryListParams` already declares `filePath`, and it is required there for a different
reason (a history query is always *about* one file).

**Migration:** none required. The engine half lands in
`packages/engine/src/store/auditCommands.ts` (`registerAuditCommands`), registered from
`src/ipc/handlers.ts` alongside the other `register*Commands` modules.

**A regression guard rides along**, because this entry's own reasoning is why it is needed:
`tests/ipc/handlers.test.ts` now parses the exact payload `AuditLogPanel.tsx` builds and asserts it
succeeds, *and* that a misspelt `fromTS` still fails. The ratchet's other direction asserts that each
`BLOCKED` command is *still* blocked, so a widening cannot go unnoticed there; but `audit.list` has left
the list entirely, so without that test the three fields could be dropped again and the only symptom
would be the date filter quietly returning everything.

**Approved by:** this session, on the user's explicit decision to unblock the four pinned keys rather
than defer them to P12.

---

### 2026-09-19 — `RunnerConfigSchema` becomes a free-form record; the two `runner.*Config` commands unblock

**Phase:** P6 (step 3d — unblocking the four pinned keys, key 2)
**Type:** fix — the frozen schema described a payload no caller sends
**Change:** one schema, in `packages/protocol/src/commands/runner.ts`.

```ts
-export const RunnerConfigSchema = z.object({
-  requestOrder: z.array(z.string()),
-  delayMs: z.number(),
-});
+export const RunnerConfigSchema = z.record(z.string(), z.unknown());
```

This is the shape `entity.ts` already uses for the same situation, and the same call `AppConfig`
(`config.ts`) and `ApplicationConfig` (`application.ts`) make: a large, evolving, **caller-owned** blob
stays `z.record` at the wire boundary, and the domain layer owns validation. `RunnerLoadConfigResult`
is `z.infer<typeof RunnerConfigSchema> | null`, so it widens with it.

**Why it could not wait:** `runner.saveConfig` / `runner.loadConfig` were the two `BLOCKED` entries in
the step-3b ratchet, and their block was asserted *directly* — parsing the real config through the
frozen schema had to fail — precisely so that widening the schema would fail that test and announce
they had become movable rather than leaving them filed as "unimplemented" forever. That is what
happened. But the frozen schema was wrong **twice over**, and only one half was ever visible:

1. It **required** `requestOrder`, so it rejected what real callers save.
   `tests/integration/runnerStorage.integration.test.ts:207` round-trips
   `{delayMs: 250, stopOnFailure: true, iterations: 3}` and line 218 saves `{delayMs: 0}` — neither has
   a `requestOrder`. `packages/gui/src/lib/useCollectionRunner.ts:67` sends `{requestOrder, delayMs}`, a third
   distinct subset. The shell's own `runner:saveConfig` body takes `config: any` and does
   `JSON.stringify(config, null, 2)` with no validation at all, so there was never a fixed shape to
   describe.
2. It **stripped** everything else. `z.object()` drops unknown keys, so even a payload that *had*
   passed would have reached disk without `stopOnFailure` or `iterations`. This half is the more
   dangerous one and the one the ratchet could not express: a dropped field is silent data loss in
   saved run configuration, not a validation error, and nothing reports it.

**Why this is not a `PROTOCOL_VERSION` bump.** Same argument as the entry above, and for the same
measurable reason: **neither `runner.saveConfig` nor `runner.loadConfig` has ever been registered in a
`CommandRegistry`** — the ratchet in `tests/ipc/handlers.test.ts` asserted that directly until this
entry retired it. Their only server has been the shell's `runner:saveConfig` / `runner:loadConfig`
`ipcMain` bodies, whose renderer-facing shape is **unchanged** by this entry (`README.md`
non-negotiable #3): they take `(wsId, folderId, config)` and return `{ok}` / the parsed config, and
`JSON.stringify` has never filtered a key. A client cannot depend on a payload no server has ever
accepted, and no server has ever *rejected* a config for shape reasons either.

**Affected:** `runner.saveConfig` params and `RunnerLoadConfigResult` only. No command name, no error
code, no event envelope. `runner.saveReport`, `runner.exportReport`, `runner.getHistory` and
`runner.listFolderIds` are untouched.

**Migration:** none required. The engine half lands in
`packages/engine/src/runner/runnerCommands.ts` (`registerRunnerCommands`), registered from
`src/ipc/handlers.ts` alongside the other `register*Commands` modules.

**A consequence worth recording, because it was found rather than assumed:** widening the schema
**silently invalidated a conformance fixture.** `COMMAND_FIXTURES["runner.saveConfig"].invalid` was
`config: {}` — a near-miss only because the old schema *required* `requestOrder`, and an empty record is
a valid record under `z.record`. The every-command matrix in
`packages/engine/tests/conformance/protocol.conformance.ts:927` asserts each fixture's `invalid` payload
is rejected with `BAD_REQUEST`, across all four transports, so this would have failed four suites for a
reason with no obvious connection to the change. It is now `config: 42` — a **non-record**, which is the
one thing the widening still rejects, and the payloads that also work (`[]`, `null`, `"str"`) were
checked against the built schema rather than assumed. The lesson generalises: `z.record` widens a
schema's *acceptance*, so every `invalid` fixture targeting that field must be re-derived, not trusted.

**Regression guards.** The retired `BLOCKED` assertion is replaced in `tests/ipc/handlers.test.ts` by
one asserting the widening is **accepted AND preserved** (`toEqual` on the whole config, which is the
assertion the block could never make — it only ever proved rejection), plus
`tests/integration/runnerStorage.integration.test.ts` now drives both commands **through the registry**
rather than only through the shell channels, because the channels are self-contained `ipcMain.handle`
bodies and would have kept passing if the engine implementation were wrong.

**Approved by:** this session, on the user's explicit decision to unblock the four pinned keys rather
than defer them to P12.

---

### 2026-09-20 — `app.checkUpdate` deleted; the update check is client-local

**Phase:** P6 (step 3f)
**Type:** breaking (empty in practice — see "Why this is not a `PROTOCOL_VERSION` bump")
**Change:** `app.checkUpdate` removed from `COMMANDS` in `packages/protocol/src/commands/index.ts`
(**93 → 92**). `AppCheckUpdateParams` / `AppCheckUpdateResult` are deleted from `commands/misc.ts`, the
row is deleted from `commands/fixtures.ts`, and `"app.checkUpdate": "read"` is deleted from
`packages/engine/src/transport/auth/scopes.ts`. Three consumers move with it, and none of them changes
behaviour:

- `@bifurc/client`'s `surface.ts` reclassifies the `checkUpdate` key `t("app.checkUpdate")` →
  `local(...)` — the classification counts move **transport 73 → 72, local 13 → 14**, and this is the
  surface's last `SPLIT`, so the inventory is now 144 of 144 with none remaining.
- `@bifurc/client`'s `index.ts` delegates to `local.checkUpdate()` instead of `call("checkUpdate", {})`.
- `src/preload.ts` loses its **last** override. `contextBridge.exposeInMainWorld("api", { ...client })`
  now has no exceptions for the first time.

**Why it could not wait:** the command was **declared but never served**, and the gap between those two
facts was already producing a wrong answer. No engine module ever registered `app.checkUpdate` —
`packages/engine/src/proxy/serverCommands.ts` carries a section titled "What is deliberately absent:
`app.checkUpdate`". But `@bifurc/client` classified the key `{kind: "transport"}`, and `commandFor()`
resolves that through `SURFACE`, so `client.checkUpdate()` sent `app.checkUpdate` over the transport and
would have been answered `UNKNOWN_COMMAND`. The Electron shell never hit that path only because
`src/preload.ts` **overrode** the key with a local channel (`app:checkUpdate`,
`src/ipc/handlers/systemHandlers.ts:27`), which does the fetch in the shell. The command was therefore
dead on the wire and shadowed in the shell — and the pin that kept it visible was load-bearing.

**The silent failure the pin was protecting against.** `AppCheckUpdateResult` carries `tagName`; the
renderer's `UpdateCheckResult` reads `hasUpdate` / `currentVersion` / `latestVersion` / `releaseUrl`.
`@bifurc/client` bridged the two with `as ReturnType<BifurcApi["checkUpdate"]>` — a cast in the
direction that fails **silently**. Routing it as-is would have left `hasUpdate: undefined`, which is
falsy, so the About panel would have rendered "you're up to date" forever and nothing anywhere would
have raised an error. Resolving the split removes the cast rather than honouring it.

**Why the split resolves to the client, not the engine.** `currentVersion` is the version of the thing
the *user* is running, and `downloadUrl` must be the asset for the *user's* platform. An engine can
produce neither correctly: its own version only coincides under D9 lockstep, and matching its own
`process.platform` would hand a Windows user the AppImage. Both are properties of the client. There was
no engine half to contract with, so the honest resolution was deletion rather than implementation.

**Why this is not a `PROTOCOL_VERSION` bump.** The same argument as the 2026-09-17 entry: the schema
described a command **no implementation has ever registered**, so no deployed client could depend on
it. `PROTOCOL_VERSION` stays `"1.0.0"`. Note the shape of the exception, though, because it is narrower
than it looks. This is the second time the freeze rule has been discharged by "nothing was ever
served", and the first time the command in question was one a **client did route**. The distinguishing
question is not "was it implemented" but "could any deployed client have observed it on the wire", and
here the answer is no only because the shell's preload override shadowed the routed path. Had the
override not existed, this would have been a real breaking change and owed a major bump.

**Affected:** `app.checkUpdate` (removed). The surface key `checkUpdate` **survives** — it is
reclassified, not deleted — so `window.api` stays byte-identical and non-negotiable #3 is untouched.
`@bifurc/client` gains `checkUpdate` as the fourteenth `ClientLocal` member. No events, no other
commands. `plan/handler-classification.md`'s row 92 records this channel as `SPLIT`; that file is the
frozen P1 measurement and is deliberately **not** retro-edited — this entry is the delta.

**Migration:** none. `@bifurc/client`'s `checkUpdate` is unchanged for callers — same key, same result
shape, now served by `local.checkUpdate()` from the runtime the client is actually running on. The
shell's `app:checkUpdate` channel is unchanged, so the packaged app behaves identically.

**Recorded after the fact, and that is itself the finding.** The rule at the top of this file says
"write the entry first, then make the change". This one was written the same session but **after** the
commits (`3c984fe`, `7387f33`), because the deletion was made under the same decision and the log
obligation was missed at commit time. The change was correct; the bookkeeping was not. It is recorded
here rather than silently backfilled so that the next post-freeze change is not the third one caught
late.

**Approved by:** this session, on the user's explicit decision among the three designs audited in
`plan/07` §5b — **B, reclassify as local** — chosen over A (add an engine command) and C (a pure
preload function with no IPC).

---

### 2026-09-21 — ten workspace-scoped commands gain an optional `workspaceId` (P14 WI 1 / D13)

**Phase:** P14 (work item 1, the protocol-shape half)
**Type:** **additive** (minor bump; no `PROTOCOL_VERSION` major)
**Change:** `workspaceId?: string` is added to the params of ten commands, all of them addressed to a server
that a host may be running once per workspace:

| File | Commands |
|---|---|
| `commands/server.ts` | `server.status`, `server.start`, `server.stop`, `server.restart`, `proxy.status` |
| `commands/webhook.ts` | `webhook.registerActive`, `webhook.unregisterActive`, `webhookServer.start`, `webhookServer.stop`, `webhookServer.status` |

The field is **optional**, and that is the entire compatibility story: omitted means *the active workspace*,
which is what the single-workspace desktop has always meant by these commands. A host member's client always
sends one, because for it there is no meaningful default — D13 puts N workspaces behind **one** RPC endpoint,
so an unaddressed `server.status` would be ambiguous between them. The two `*StatusResult` shapes add an
optional `workspaceId` echo so a multiplexed client can match a response to its request rather than assuming
FIFO.

**`services.discover` is deliberately NOT scoped** — it lists the ports this *host* is listening on, which is
a property of the host and not of any workspace. That omission is asserted in `index.test.ts` so it is a
decision rather than something a later reader has to guess at.

**Why this is a schema change and not a widening, in the plan's terms.** All ten were
`z.object({}).strict()`. `.strict()` turns an *unknown* key into a rejection, so adding a parameter does not
merely relax a check — it changes which keys are known. This is the case `plan/14` §12.2 flagged as "WI 1's
change invalidates that command's fixtures", and the instruction was to **re-derive them, not trust them**.

**The fixtures were re-derived, and the outcome is worth recording because it is the opposite of the
expectation.** The five `server.*` / `proxy.status` rows and the two `webhookServer` rows used
`invalid: { extra: true }` — a generic unknown-key near-miss. Those still reject, because `extra` is still
unknown; but the generic form no longer tests anything specific to *this* change. They were replaced with a
near-miss on the new field (`{ workspaceId: 42 }`), which is a strictly better assertion: a missing optional
field is valid by construction, so "reject a missing one" is not a test, whereas "reject a wrong-typed one"
exercises the type check the parameter just gained. **Nothing needed re-deriving in the sense the plan
anticipated** — no fixture became accidentally-valid — and that is itself the finding: the hazard was a
fixture whose invalid payload *is* the newly allowed key, and none of the ten was written that way.

`packages/protocol/src/commands/index.test.ts` also gained a relationship test the per-command pair cannot
express: for each of the ten, `workspaceId` present **is accepted**, absent **is still accepted**, and a
non-string **is rejected**. Both directions of the first two are load-bearing — asserting only "present is
accepted" would pass if someone made the field *required* and broke every desktop caller, which is the single
most likely way this change goes wrong.

**Verified:** `packages/protocol/src/commands/index.test.ts` **185 → 216** cases (**+31**). The figure was
taken by enumerating the run's own test names rather than by counting edits, because the first attempt at this
number was wrong by five and the error was invisible from the diff:

| Kind | Count |
| --- | --- |
| Generated per-command pairs (92 commands × 2) | 184 |
| The coverage invariant (`it` over `COMMAND_FIXTURES`) | 1 |
| `services.discover` is deliberately NOT scoped | 1 |
| Scoped block — three `it.each(SCOPED)` of 10 | 30 |
| **Total** | **216** |

The prior count is therefore **185**, not the 190 an earlier revision of this entry carried. **What went wrong:
the earlier figure subtracted only the 30 `it.each` cases from 216 and then attributed the residual 26 to the
change, so the baseline absorbed the coverage invariant and the non-scoping assertion — two cases that existed
before the change and still exist after it.** The +5 that `plan/14` recorded as "unaccounted" was never
unaccounted; it was a baseline miscalculation, and the reconciliation was being chased because the arithmetic
was done from memory instead of from the enumerable run. **Enumerate the run (`--reporter=json` +
`assertionResults[].title`), classify by name, and only then subtract** — a suite whose cases are *generated*
has no reliable hand-count, and this is the second time in P14 that a count written into prose drifted from the
tree.

No command was added or removed, so `COMMANDS` is still 92 and the per-command generator still emits 2 cases
each. The extensions' four frozen commands
(`EXTENSION_FROZEN_COMMANDS` — `mock:add`, `request:add`, `folder:add`, `config:get`) are **untouched**:
none of them is in the ten.

**Approved by:** `plan/14` D13, taken before P14 began. This entry records the landing, not a new decision.

---

### 2026-09-21 — the four `blob.*` commands gain an optional `workspaceId` (P14 WI 1 / D17)

**Phase:** P14 (work item 1, the blob half)
**Type:** **additive** (minor bump; no `PROTOCOL_VERSION` major)
**Change:** `workspaceId?: string` is added to the params of `blob.put`, `blob.stat`, `blob.read` and
`blob.release`. Engine-side, the store resolves `<dataDir>/blobs/<workspaceId>/…` and the TTL sweep walks
only that namespace; omitted means the host-wide root.

**Why this is a change at all.** `plan/14` §12.1 audited the pre-pivot assumption that blob staging was
*already* per-workspace and measured it **false** — `blobRoot()` was `<dataDir>/blobs` (`blob/store.ts`) and
the four params carried no `workspaceId` (this file), pinned by `tests/blob/store.test.ts:62` for the
Docker-volume requirement. D17 decided the namespace on 2026-09-20. Without it a `blobId` from workspace A
resolves for workspace B, and B's sweep can collect A's in-flight transfer — the failure mode is a
cross-workspace read and a vanishing upload, neither of which is visible to a single-workspace test.

**A namespace, not a move.** The root stays under `dataDir()`, so `plan/04`'s Docker requirement holds
unchanged: `path.relative(dataDir(), blobRoot(wsId))` is `blobs/<wsId>` and never starts with `..`. Asserted
both ways in `tests/blob/store.test.ts`.

**The fixtures were re-derived, and here the hazard the plan anticipated IS live — unlike the ten.** All four
are `.strict()`, and the module header in `blob.ts` says so. The four `valid` rows now supply `workspaceId`,
so the new field is exercised on the happy path. The `invalid` rows were checked individually rather than
assumed: `blob.put`'s is a missing `filename`, `blob.read`'s a negative `offset`, and `blob.stat` /
`blob.release` use `{}` — which stays invalid because `blobId` is required, i.e. **for a reason unrelated to
scope**. That last pair is worth naming: an `{}`-shaped `invalid` row keeps passing after this change while
testing nothing about the new field, which is exactly the "silently stops testing rejection" pattern the D13
entry described. Each row was verified to fail on a required-or-typed field and never on scope.

**`.merge()` ordering is a live trap and was measured, not assumed.** `WorkspaceScope` now lives in
`commands/scope.ts` because three command files need it. Zod is **v4 (4.6.5)**, and the two orderings
disagree:

```
Base.strict().merge(Scope).safeParse({ bogus: 1 }).success → true    ← unknown keys SILENTLY allowed
Base.merge(Scope).strict().safeParse({ bogus: 1 }).success → false   ← correct
```

So `merge` returns a fresh object schema and does not carry strictness forward. Every consumer therefore
writes `Base.merge(WorkspaceScope).strict()`. Getting this wrong would have quietly widened four `.strict()`
commands to permissive ones while every "the valid fixture parses" test still passed.

**Not scoped here, deliberately:** the five egress commands that stage artifacts above
`BLOB_INLINE_THRESHOLD_BYTES` (`export.create`, `tls.exportCert`, `runner.exportReport`, `capture.shareJson`,
`audit.export`) do **not** gain `workspaceId`. D17 names the four `blob.*` params, and extending the field to
the egress set is a separate protocol change with its own fixtures and its own entry — recorded in
`blob/publish.ts`'s header so the omission is visible from a call site that reads as complete.

**The landing found a real bug, and it is the one this entry should be read for.** The first integration run
went from **0 → 30 failures**, all `blob-not-found` on round trips that had passed minutes earlier. Cause:
`export.create` had been handed `TEST_WS` while the test client's own `blob.put` / `blob.read` /
`blob.release` had not, so **the engine's export was staged into a namespace its own requester could not
read** — a latent defect a host-wide root had been concealing, since with one namespace every id resolved
regardless of who asked. Fixed in four rounds (30 → 22 → 4 → 3 → 1 → 0) by scoping the *client*, then three
engine call sites the first substitution pass missed, then `publishArtifact`'s missing fourth argument. **The
fix is always to scope the caller, never to relax the store:** making `blobRoot()` fall back to the shared
root on a miss would have cleared all 30 failures and destroyed the property D17 exists to create.

**The plan's hazard note named the wrong place.** `plan/14` WI 1 predicted the `blob.*` `invalid` fixtures
would need re-deriving. They did not — the fixtures were fine. The exposure was entirely in the *call sites*,
which the note did not mention. A hazard note records where its author was looking, not where the bug is.

**The follow-up audit, and what it left standing.** A grep for store call sites that pass **no** id — the
question a find-replace cannot answer — returned exactly two files, both now explicable rather than open:

- `packages/engine/src/fileOps/commands.ts:139-140` (`tlsImport`'s `statBlob(blobId)` /
  `blobContentPath(blobId)`) — **deliberately unscoped, and this is a decision rather than an omission.**
  The CA is one per *machine*, not per workspace: `caCertPath()` / `caKeyPath()` resolve under
  `appDataDir()` (`proxy/certManager.ts`), which on Windows is `%LOCALAPPDATA%/Bifurc` and unreachable from
  the workspace data root by that module's own recorded decision; `tls.exportCert` reads the same paths; and
  `proxy/server.ts`'s `ownsCA` flag exists because N workspaces share that one mkcert root. A CA import is a
  transfer between a client and **the machine's** CA — the workspace is not a participant — so a
  `workspaceId` here would let a client stage under A and import into the shared CA, i.e. the same
  half-threading the integration failures had just exposed, arriving this time on purpose. Recorded in the
  function's own docblock.
- The five egress commands above — WI 8's work, as stated.

**Verified:** `packages/protocol/src/commands/index.test.ts` **216 → 228** cases (**+12**), decomposed by
enumerating the run rather than by counting edits: 184 generated pairs (92 commands x 2, unchanged — no
command was added), 1 coverage invariant, 1 `services.discover` non-scoping assertion, and the scoped
`it.each` block at **42** (three of fourteen scoped commands, up from ten). `tests/blob/store.test.ts`
**58 → 63** (five D17 cases, all asserting on **identity** per the acceptance criterion) and the blob suites
**94/94** overall; `tests/integration/importExportBlob.integration.test.ts` back to **45/45**.

**Approved by:** `plan/14` D17, taken 2026-09-20 after §12.1 measured the "already per-workspace" claim
false. This entry records the landing, not a new decision.

---

### 2026-09-21 — three of the five egress commands gain an optional `workspaceId` (P14 WI 8)

**Phase:** P14 (work item 8, the egress half of D17)
**Type:** **additive** (minor bump; no `PROTOCOL_VERSION` major)
**Change:** `workspaceId?: string` is added to the params of `runner.exportReport`, `capture.shareJson` and
`audit.export`. All three are `.strict()` and now merge the shared `WorkspaceScope` in the correct order
(`Base.merge(WorkspaceScope).strict()` — see the D17 entry's measured `.merge()` trap).

**Why: the blob half.** These commands return an `ArtifactResult`, and above
`BLOB_INLINE_THRESHOLD_BYTES` (1 MiB) that is a **staged blob** which the client then reads with
`blob.read` **scoped to its own workspace**. D17 gave the four `blob.*` params the namespace and
`export.create` passed its id to `publishArtifact`; these three did not, so the engine staged host-wide
while a scoped client read elsewhere — **the engine's own export unreadable by the client that requested
it**, which is the exact failure `importExportBlob.integration.test.ts` caught for `export.create`. WI 8
is that fix applied to the rest of the set, and `blob/publish.ts` now carries the whole table.

**Why: `audit.export` had a second, worse problem.** Its handler resolved the workspace itself as
`loadConfig().activeWorkspaceId`, reasoning that "the engine can already look up" the id. **That reasoning
was desktop-only.** `plan/14` §2.3 demotes `activeWorkspaceId` to the *desktop's UI pointer*, so on a host
serving N workspaces an `audit.export` from workspace B returned the **active** workspace's log — the
caller could not ask for the log it meant. Same class as WI 1's `server.start { workspaceId }` being
accepted and ignored. The handler is now `params.workspaceId ?? cfg.activeWorkspaceId`, so the desktop's
behaviour is unchanged.

**`tls.exportCert` is deliberately NOT scoped, and it is a decision rather than an omission.** The CA is
one per **machine**, not per workspace — `caCertPath()` resolves under `appDataDir()`, and
`proxy/server.ts`'s `ownsCA` exists because N workspaces share that one mkcert root — so a `workspaceId`
there would be a field that means nothing, which is *worse* than an absent one because it would read as
though the export were workspace-scoped. **And it cannot reach the blob branch anyway, measured rather
than assumed:** a `ca-cert.pem` is **1,240 bytes** against a **1,048,576-byte** threshold, ~845× short, and
a PEM will not grow three orders of magnitude. The same reasoning already kept `tls.importCert` /
`tls.importKey` unscoped; all three now say so in `fileOps/commands.ts`.

**This is a contract-consistency fix, not a live bug fix — and the distinction governs how it is
verified.** Today's only client (`packages/client/src/index.ts`) sends **no** `workspaceId` on
`blob.put` / `blob.read` / `blob.release` (lines 284, 298, 474, 496); it relies on the host-wide default,
which is what the unscoped egress commands produced — so the two matched and nothing failed. The
inconsistency bites the **first client that scopes its blob reads** — P7's web UI, or D15's CLI — which
would find `export.create`'s output and not the other three's. That is why the fix lands before that
client exists, and why the test below is written as an **identity** assertion rather than as "a blobId
came back".

**Fixtures re-derived, per the D17 precedent.** All three `valid` rows now supply `workspaceId`, so the
new field is exercised on the happy path. Each `invalid` row was checked individually to fail on a
**required-or-typed** field and never on scope: `runner.exportReport` → the `format` enum (`"pdf"`),
`audit.export` → the enum (`"xml"`), `capture.shareJson` → `{}`, invalid because `entries` is required.
The hazard is a fixture whose invalid payload *is* the newly allowed key; none was written that way here.

**One assertion was strengthened rather than merely extended.** The `it.each(SCOPED)` case
`"%s rejects a non-string workspaceId"` used to parse `{ workspaceId: 42 }` **alone** — which a command
with any required field rejects for *those fields being missing*, so `expect(success).toBe(false)` passed
whatever happened to the `workspaceId`'s type. It is now `{ ...valid, workspaceId: 42 }`, leaving the type
as the only possible error, plus an assertion that the failure's `path` includes `workspaceId`.

**Verified:** `packages/protocol/src/commands/index.test.ts` **228 → 237** (**+9** = the three new scoped
commands × the three `it.each` cases each). `tests/integration/workspaceCommandScope.integration.test.ts`
**8 → 9**: a new case forces the blob branch with a `capture.shareJson` payload just over the threshold —
chosen because its `entries` come from the client, so the size is the test's to set — and asserts the
artifact is readable **scoped to B** and **throws under A**. It guards its own premise (`blobId` defined,
`inline` undefined) so it cannot pass without crossing the threshold.

**Mutation-checked, and both mutations were fully identified rather than waved at.** (1) Widening
`WorkspaceScope.workspaceId` to accept a number → **24 failures**, which resolve exactly as **17** × the
type case (the scoped set) plus **7** × `"— rejects an invalid payload"` for the D13 commands whose
`invalid` fixture *is* `{ workspaceId: 42 }` — two independent mechanisms catching it. (2) Dropping
`params.workspaceId` from `captureShareJson` → **exactly 1** failure, the new integration case.

**Approved by:** `plan/14` D17 (which named the egress set as a follow-on change with its own fixtures and
its own entry) and WI 8. This entry records the landing, not a new decision.

---

### 2026-09-29 — four `*Saved` commands: execute a saved request entity by id

**Phase:** post-P18 housekeeping — the engine-side prerequisite for the parked CLI (P8)
**Type:** additive
**Change:** four new commands, in a new file `packages/protocol/src/commands/saved.ts`:

| command | counterpart of | entity kind it loads |
|---|---|---|
| `request.replaySaved` | `request.replay` | `requests` |
| `graphql.executeSaved` | `graphql.execute` | `graphqlRequests` |
| `soap.executeSaved` | `soap.execute` | `soapRequests` |
| `grpc.executeSaved` | `grpc.execute` | `grpcRequests` |

All four take the same params — `{ workspaceId: string, id: string }`, `.strict()` — and nothing else, and
each returns exactly what the command it delegates to returns. **No existing command, payload, error code
or event changed.** The protocol goes 89 → **93** commands.

**Why it could not wait:** it is not urgent — nothing is broken — but it is *cheap now and expensive
later*, and that asymmetry is the whole argument. Today no client depends on the new shape, and the change
is four additive names plus one engine module that alters no existing file's behaviour. The first consumer
that needs it (D15's CLI: `bifurc requests send <id>`) would otherwise arrive with a client already built
against the payload-passing commands, and the fix would then be a protocol change *and* a client change at
once.

**The problem, in one sentence:** every request-executing command takes the **whole request** — correct for
an editor, since an unsaved tab has nothing else to send, and wrong for every other consumer, which has an
*id*. The renderer is the only caller today, so the engine has never had to serve the id case; a second
consumer would have to load the entity itself and re-send every field, which puts the stored shape in the
client's hands — exactly the coupling the engine exists to remove.

**Checklist (`## Guidance`, "Before adding any command"):**

1. *Does an existing command already do this with a different payload?* — **Yes, deliberately, and that is
   this entry's central point.** The payload difference *is* the feature: `{method, url, headers, body}`
   versus `{workspaceId, id}`. The originals cannot be reshaped — the editor genuinely has an unsaved state
   to send — so a counterpart is the only shape that serves both. This is the one place the checklist's
   rule is answered by adding a near-duplicate, and the reason is recorded rather than left to be
   rediscovered.
2. *Should this be a `kind` discriminator on `entity.*`?* — **No.** `entity.*` is CRUD *over* the entity;
   these *execute* it, which is a different scope class (`write`/`read` versus `execute`). Four commands
   rather than one `entity.execute{kind}` because the kind is fixed by the name — a single command would
   accept `kind: "grpcRequests"` for a REST replay, which is an answer about neither.
3. *Does the payload contain a filesystem path?* — No: `{workspaceId, id}`.
4. *Is there a schema, and does the engine validate against it?* — Yes. `SavedEntityRef` is declared once
   and reused by all four, `.strict()` on the outermost call; the registry validates every inbound payload
   against it.
5. *Is it in the right scope class?* — `execute`, the same as the four commands they delegate to. A
   *weaker* scope would have been a widening: a counterpart delegates through `registry.invoke()`, which
   does **not** re-check scopes (the check is in the transport decorator), so a `read`-scoped session could
   otherwise reach `graphql.execute` by the saved-entity door.

**Two things the counterparts deliberately do not do**, recorded in `saved.ts`'s header so the omission
reads as a decision: they do **not** resolve `{{variables}}` against the active environment, and they do
**not** run the pre/post/test scripts. Both are the renderer's half today and neither has ever been part of
`request.replay` — the collection runner drives scripts through `script.execute` *around* the replay. A
caller that wants either must ask for it.

**Four near-misses were considered and left out**, on the same rule — a counterpart is only meaningful when
the command's *entire* input is derivable from a saved entity: `graphql.introspect` and `soap.fetchWsdl`
(the GUI drives both from a typed-in address, and `wsdls` stores the fetched document, not a target),
`grpc.reflect` (typed-in address), `script.execute` (a script is a *field* of a request entity, and the
command also needs the response context), and `healthbar.checkUrl` (healthbar services are not a CRUD kind
and have no id). Listed in `saved.ts`, so a reader can tell an omission from an oversight.

**One engine module, not four.** The four are one capability — *load the entity the caller named and hand
it to the command that runs it* — and the module reads only the entity (`workspace/`) and dispatches only
through the `CommandRegistry` (`kernel/`), naming its targets as strings and importing no feature module at
all. It lives at `packages/engine/src/workspace/entities/savedRequestCommands.ts`, beside `entity.load`,
whose job it composes. Splitting it per protocol would not distribute any behaviour; it would put four
copies of one body in four directories.

**Affected:** commands — four new names. Clients — **none required to change.** `@bifurc/client`'s
`window.api` surface is deliberately untouched (it is frozen and byte-identical, and no renderer call site
is migrated here), so the two totality tables that *do* track the protocol were the only client-side edits:
`packages/engine/src/transport/auth/scopes.ts` (four `execute` rows) and `packages/client/src/retry.ts`
(four non-idempotent rows). Both assert total coverage of `COMMANDS` and throw at import time otherwise, so
adding a command without them is a load failure rather than a silent gap.

**Migration:** none. Existing clients keep calling the existing commands unchanged; a consumer that wants
the id shape calls the new name.

**Verified:** `packages/protocol/src/commands/index.test.ts` **241 → 249** (+8 = 4 commands × the 2 generated
cases each — the per-command pair, not the `workspaceId` block, which covers a fixed 17-command list and was
not added to). The unit project total moved with it, **2,281 → 2,289**, which is the whole of the change:
no other unit test's count changed. `tests/integration/protocolExecution.integration.test.ts` **41 → 54**
(+13), driving all four
against **real** HTTP servers: the REST case asserts the bytes the upstream received — including that the
stored *text* body arrives as text, which is the group's one real transformation — and each command's
`NOT_FOUND` case is what proves the load half runs at all, since the gRPC target is a stub that ignores
every parameter. `tests/integration/workspaceHost.integration.test.ts`'s shell↔engine drift guard needed a
row in its hand-maintained path table for the new owner and passes with it. Full unit project green;
`typecheck:packages` and the root `tsc --noEmit` clean. (Incidental: the three "93-entry table" mentions in
the protocol source, which read 93 while the real count was 89, are now accurate.)

**Approved by:** the owner's direction, 2026-09-29 — *"if request is saved then it is on disk and UI should
only be passing request's entity id where backend loads the entity and does the request … CLI is out of
scope now but we can at least make engine work correctly for saved entities."* This entry records the
landing, not a new decision.

> **⚠ Amended 2026-09-30 for the REST case** — `request.replaySaved` becomes **`request.send`** with a
> widened payload; see the entry below. The other three names are unchanged *for now*: they are renamed one
> at a time as each protocol's pipeline lands (`plan/19` §8 trap 1 — one commit per protocol).

---


### 2026-09-30 — `request.send`: the REST pipeline, and the id form's payload widens

**Phase:** `plan/19` WI 3, first protocol (D24 / D27)
**Type:** **breaking in name only** — and why that is not a `PROTOCOL_VERSION` bump is below
**Change:** `request.replaySaved` → **`request.send`**, and its params widen from `{workspaceId, id}` to a
union of two complete forms:

| input form | payload | who uses it |
|---|---|---|
| **id** | `{ workspaceId, id }` | a saved tab holding no unsaved changes, and the CLI |
| **inline** | `{ workspaceId, request, scripts }` | a draft tab, and any caller that has not saved yet |

`request` is `{method, url, headers, body}` with **`body` as text**; `scripts` is `{pre?, post?, test?}`.
The result widens too: the response (`status` / `headers` / `body` base64 — the fields `request.replay`
already returns) **plus** what the pipeline did on the way through — the request as actually sent, the test
results and logs, and the two script error strings.

**Why the name changes instead of a fifth command being added.** §5 option B was chosen for the pipeline
surface (D27), and B's cost — as §5 itself says — is that it leaves the `*Saved` family as a near-duplicate.
That cost is avoidable here and *only* here: **the `*Saved` family has never been committed.** No client has
ever seen `request.replaySaved`, so there is nothing to migrate. Adding `request.send` beside it would have
created the near-duplicate §5 warns about, in the one change that had the opportunity to avoid it.

**Why this is not a `PROTOCOL_VERSION` bump.** `## Guidance` lists "rename a command" as breaking, and it
is — for a **released** name. This one has never been released: it exists only in the protocol source,
`PROTOCOL_VERSION` stays `"1.0.0"`, and the command **count is unchanged at 93** (one name replaced by one
name). The honest framing is that this corrects an unreleased landing. It gets its own entry rather than an
edit into the 2026-09-29 one because the *reasoning* differs: that entry argued for adding an id-shaped
command; this one argues for the pipeline, and the rename is a consequence of it.

**Why the id form carries the body as text while `request.replay` stays base64.** §5's sub-decision. The
pipeline's input *is* the stored entity — `SavedRequest.body` is text, and all five importers store text —
so carrying base64 would have meant the caller encoding, the engine decoding, and the blank-means-empty rule
re-derived at both ends. The raw primitive keeps base64 because binary safety is a wire concern and
`request.replay` predates this. The pipeline's REST `send` closure encodes **once**, in the one place that
talks to `request.replay`.

**Checklist (`## Guidance`, "Before adding any command"):**

1. *Does an existing command already do this with a different payload?* — **It replaces one.** The inline
   form is `request.replay` plus resolution and scripts; the id form is the 2026-09-29 counterpart. One name
   covers both because D24 makes them **the same pipeline** with two sources of request, and two names for
   that would be the duplication this checklist item exists to prevent.
2. *Should this be a `kind` discriminator on `entity.*`?* — **No**, for the 2026-09-29 reason: `entity.*` is
   CRUD *over* the entity, while this *executes* it — a different scope class.
3. *Does the payload contain a filesystem path?* — No. `{workspaceId, id}` or `{workspaceId, request,
   scripts}`.
4. *Is there a schema, and does the engine validate against it?* — Yes: a `z.union` of two `.strict()`
   objects, so a payload that is neither form is rejected rather than half-read.
5. *Is it in the right scope class?* — `execute`, unchanged and the same as the `request.replay` it
   invokes. A pipeline that resolved and ran scripts at `read` scope would be a widening.

**The union is the one thing to be careful about, and it is not option C.** §5 rejected option C —
folding the id form into `request.replay` — because *"one command with two mutually-exclusive input modes is
a schema nothing can validate honestly"*. That objection does not carry over, and the distinction is worth
stating rather than assuming: option C would have put two modes on the **raw primitive**, whose whole value
is being one thing; `request.send` is a *new* command whose two forms are each a complete description of a
request, and `z.union` validates them exhaustively. What must not happen is a payload that is *partly* one
form — `{workspaceId, id, scripts}` — and `.strict()` on **both** members is what turns that into a
rejection rather than a silent half-read.

**Affected:** commands — `request.replaySaved` → `request.send`; count unchanged at 93. Clients —
`@bifurc/client`'s `replayRequestSaved` → `requestSend` (a **rename**, so the surface count is unchanged),
and `packages/gui/src/types/window.ts` likewise. Renderer — `useRestActions` stops resolving and
orchestrating; `savedEntitySend.ts` shrinks to `!isDraft && !hasLocalChanges`, which is `plan/19` §7's
predicted end state and the reason its two domain terms disappear.

**Migration:** none. The only caller is this repository's REST tab, migrated in the same change — which is
`plan/19` §8 trap 1's *"one commit per protocol: the engine starts and the renderer stops"*, not an option.

**Verified:** `packages/protocol/src/commands/index.test.ts` **249** — unchanged, and that is the point: one
name replaced by one name. The generated pair now exercises the union's **inline** member as `valid`, and as
`invalid` a payload that is **partly both** forms — the rejection a two-member union introduces, and the
thing `.strict()` on both members exists for.

`tests/integration/protocolExecution.integration.test.ts` **54 → 54**: the block's cases were *rewritten*
rather than added to. Call sites renamed, and the case that pinned *"forwards the stored values literally —
no `{{variable}}` resolution"* **inverted** — because that boundary is precisely what this entry moves. It now
asserts that a stored `{{apiToken}}` resolves against the workspace's `__global__` environment, which is
`plan/19` §10's acceptance criterion stated end to end.

**The first integration run failed, and the failure is worth recording.** `No handler registered for command
"request.send"` for **7** cases: `protocolExecution.integration.test.ts` registers handler groups by calling
each `register*Commands()` by name, and it had never heard of `registerPipelineCommands`. **A new
`register*Commands` owner needs its call site added there as well as in the shell** — the same *"a command's
registration is not the whole of its dependency"* shape `registerAll.ts` warns about, one consumer over.

Unit and integration both green apart from the standing environment failures; `plan/19` §11 records the
totals.

---


### 2026-09-30 — `graphql.send`: the second protocol, and what "resolve every field" actually means

**Phase:** `plan/19` WI 3, second protocol (D23 / D24 / D27)
**Type:** **breaking in name only** — same argument as `request.send` above, and it holds for the same
reason: the name it replaces was never released
**Change:** `graphql.executeSaved` → **`graphql.send`**, params widened to a union:

| input form | payload | who uses it |
|---|---|---|
| **id** | `{ workspaceId, id }` | a saved tab with no unsaved changes, and the CLI |
| **inline** | `{ workspaceId, request }` | a draft tab, and any caller that has not saved yet |

`request` is `{endpointUrl, headers, query, variables, operationName}` — **the stored entity's own shape**,
not `graphql.execute`'s. The inline form carries what the *editor* holds, and the editor holds `endpointUrl`;
the `endpointUrl` → `url` rename stays the engine's business, exactly as it already is for the id form.
Command count stays **93** — one name replaced by one name.

**No `scripts` field, and that is a decision rather than an omission.** `request.send` has one; this command
does not, because the script sandbox's context *is* a `{method, url, headers, body}` request
(`mocks/scriptExecutor.ts`'s `IpcScriptOpts`) and a GraphQL request is not one. A `scripts` object nothing
could run would be a field that lies about what the command does. It arrives when the sandbox grows a
protocol-neutral context — the same change that would let this command use `runRequestPipeline`.

**⚠ This entry fixes a latent inconsistency, and the gate is what documented it.** Each non-REST tab resolved
*some* of its fields in the renderer:

| tab | fields the gate's token scan treats as resolvable | fields `handleSend` actually resolved |
|---|---|---|
| GraphQL | `endpointUrl`, `query`, `variables`, `operationName`, `headers` | `endpointUrl`, `variables`, `headers` |
| SOAP | `endpointUrl`, `soapAction`, `body`, `headers` | `endpointUrl`, `body`, `headers` |
| gRPC | `serverAddress`, `serviceName`, `methodName`, `requestBody`, `metadata` | `serverAddress`, `requestBody`, `metadata` |

**The gate was written from intent and the tab from practice, and the gate is the better spec.** A
`{{token}}` in a GraphQL `query` — or a SOAP `soapAction`, or a gRPC `serviceName` — has therefore never
resolved, in *either* path: the gate only ever chose between two ways of sending the same unresolved field.
Engine-side resolution removes the disagreement by construction. **The engine resolves every field**, which is
the only form under which `usesVariables` can leave the gate at all (`plan/19` §7); resolving the narrower set
would have preserved the bug *and* kept the term forever.

**Checklist (`## Guidance`, "Before adding any command"):** the same five answers as `request.send` above —
it replaces an existing name rather than being added beside it; it is not an `entity.*` kind discriminator;
it carries no filesystem path; it is a `.strict()`-on-both-members union; and it keeps the `execute` scope of
the `graphql.execute` it invokes.

**Verified:** `packages/protocol/src/commands/index.test.ts` **249** — unchanged again, one name replaced by
one name. `graphql.send`'s generated pair exercises the union's **inline** member and, as `invalid`, a payload
that is partly both forms.

`tests/integration/protocolExecution.integration.test.ts` **54 → 55**: the `graphql.executeSaved` cases were
renamed, and **one case was added** — *"graphql.send resolves EVERY field — including the query and the
operation name"* — because this migration changes behaviour rather than only moving it. It writes a
`graphqlRequests` entity whose endpoint, header, query, variables **and** operation name each contain
`{{apiToken}}`, and asserts the upstream received all five resolved. That is the assertion for the table above:
a `{{token}}` in a query never resolved before, in **either** path.

Unit and integration green apart from the standing environment failures; `plan/19` §11 records the totals.

---


### 2026-09-30 — `soap.send`: the third protocol, and the simplest shape of the three

**Phase:** `plan/19` WI 3, third protocol (D23 / D24 / D27)
**Type:** **breaking in name only** — the same argument as `request.send` and `graphql.send` above
**Change:** `soap.executeSaved` → **`soap.send`**, params widened to a union:

| input form | payload | who uses it |
|---|---|---|
| **id** | `{ workspaceId, id }` | a saved tab with no unsaved changes, and the CLI |
| **inline** | `{ workspaceId, request }` | a draft tab, and any caller that has not saved yet |

`request` is `{endpointUrl, soapAction, headers, body}` — **the stored entity's shape, which for SOAP is also
`soap.execute`'s.** All four fields are already named identically on both sides, so unlike GraphQL there is no
rename for the engine to perform and this is the simplest of the three unions. Command count stays **93**.

**No `scripts` member**, for the reason recorded in the `graphql.send` entry above: the sandbox's context is a
`{method, url, headers, body}` request and a SOAP request is not one.

**The same class of latent under-resolution, one field wide.** The tab resolved `endpointUrl`, `headers` and
`body` but **not `soapAction`**, while the Send gate's token scan treated all four as resolvable — so a
`{{token}}` in a SOAPAction never resolved in *either* path. `soap.send` resolves all four. The full argument
is in the `graphql.send` entry and applies unchanged; this one is recorded separately because the *field* is
different, and a reader looking for "was `soapAction` ever resolved?" should find the answer here rather than
infer it from a GraphQL entry.

**Checklist (`## Guidance`, "Before adding any command"):** the same five answers as `request.send` — it
replaces an existing name rather than being added beside it, it is not an `entity.*` kind discriminator, it
carries no filesystem path, it is a `.strict()`-on-both-members union, and it keeps the `execute` scope of the
`soap.execute` it invokes.

**Verified:** `packages/protocol/src/commands/index.test.ts` **249** — unchanged again, one name replaced by
one name. `tests/integration/protocolExecution.integration.test.ts` **55 → 56**: the `soap.executeSaved`
cases renamed, plus **one added** — *"soap.send resolves EVERY field — including the SOAPAction"* — asserting
that a `{{token}}` in the endpoint, the SOAPAction, a header **and** the body all arrived at the upstream
resolved. That is the assertion for the under-resolution this entry fixes.

Unit and integration green apart from the standing environment failures; `plan/19` §11 records the totals.

### 2026-09-30 — `grpc.send`: the fourth protocol, and the end of the `*Saved` family

**Phase:** `plan/19` WI 3, fourth and last protocol (D23 / D24 / D27)
**Type:** **breaking in name only** — the same argument as the three entries above
**Change:** `grpc.executeSaved` → **`grpc.send`**, params widened to a union:

| input form | payload | who uses it |
|---|---|---|
| **id** | `{ workspaceId, id }` | a saved tab with no unsaved changes, and the CLI |
| **inline** | `{ workspaceId, request }` | a draft tab, and any caller that has not saved yet |

`request` is `{serverAddress, serviceName, methodName, requestBody, metadata, protoFileId, useReflection}` — the
stored entity's shape, and `grpc.execute`'s, so like SOAP there is no rename. **`protoFileId` and
`useReflection` are required rather than optional**, because the editor always has both: a proto file may be
`null` and reflection may be off, but neither is *absent*, and optionality would invite a caller to omit a
decision rather than make one. Command count stays **93**.

**No `scripts` member**, for the reason recorded in the `graphql.send` and `soap.send` entries.

**⚠ `protoFileId` is deliberately NOT resolved.** Every other field is — including `serviceName` and
`methodName`, which the tab never resolved while the Send gate treated them as resolvable — but `protoFileId`
names a stored file rather than being template text, and the gate never scanned it. Resolving it would be a new
behaviour nothing asked for.

**⚠ This entry closes the family.** With all four protocols migrated,
`packages/engine/src/workspace/entities/savedRequestCommands.ts` — the module that held four load-and-dispatch
bodies — is **deleted**, and its registrations go with it from `registerAll.ts`, `src/ipc/handlers.ts` and both
integration harnesses. Its *"one module, not four"* argument had already narrowed three times (one member grew
a pipeline, two grew resolution); the fourth made it empty.

**Verified:** the resolution is **not observable for this protocol, and the gap is recorded rather than papered
over** — see `grpc/commands.ts`'s header. `grpc.execute` is a `wontfix` stub that ignores every parameter, so
an integration case could assert only the stub's message; replacing it with an echo needs a monkey-patched
registry, because `CommandRegistry.register()` throws on a duplicate. The `NOT_FOUND` case is what proves the
load half. `packages/protocol/src/commands/index.test.ts` **249** unchanged; `protocolExecution`
**56 → 56** (renamed, not added to). `plan/19` §11 records the totals.

**Checklist (`## Guidance`, "Before adding any command"):** the same five answers as `request.send` — it
replaces an existing name, it is not an `entity.*` kind discriminator, it carries no filesystem path, it is a
`.strict()`-on-both-members union, and it keeps the `execute` scope of the `grpc.execute` it invokes.

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
