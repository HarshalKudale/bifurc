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
and that check was a grep for the *method name*. `renderer/panels/AuditLogPanel.tsx:61–62` reaches the
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
   a `requestOrder`. `renderer/lib/useCollectionRunner.ts:67` sends `{requestOrder, delayMs}`, a third
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
