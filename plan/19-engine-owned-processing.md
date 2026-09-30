# P19 — The UI is a client: move processing into the engine

**Goal:** the renderer holds **no domain logic and no domain state**. It renders what the engine did and
sends what the user typed. Environment resolution, `{{random.*}}`, pre/post/test scripts, collection-run
execution, cURL parsing and every future protocol module (MQTT and friends) live in the engine.

**Effort:** ~2 weeks for WIs 1–5 and 7; WI 6 (WebSocket) is its own 1–2 weeks and can land separately.
**Depends on:** nothing outstanding — the engine extraction (P2–P6) is complete, which is what makes this
a *move* rather than a rewrite. **Blocks:** P8 (the CLI cannot own a collection runner or a run's state).
**Parallel with:** P18 (different files entirely).

---

## 0. Why this document exists now

The owner's direction, 2026-09-29:

> *"UI is just a client and it should not own any engine processing and state … resolving environments,
> resolving `random.*` should be part of Engine. … Pre post scripts and tests and runner executions are
> supposed to be done in engine in proper sandbox. UI only shows what engine is doing … so UI just sends
> the folder id to our collection runner cause if later down the line if we add cli we want engine to keep
> track of running stuff not the UI. … Moving heavy lifting logic to engine makes UI very light and
> optimized while having single point of logical maintainance. … Once we start introducing new modules for
> request tab like MQTT we want our engine do the work."*

**This was written after two changes landed in this direction without a plan**, and both are addressed
explicitly rather than retro-fitted:

| Landed 2026-09-29 | Disposition |
|---|---|
| The four `*Saved` execute commands (`plan/protocol-changes.md`) | **On the target path — keep as-is.** "Name a saved entity by id" is the shape the CLI needs and the shape the UI should use for a saved tab. |
| The four tabs' Send gate (`packages/gui/src/lib/savedEntitySend.ts`) | **Transitional.** Two of its four terms exist *only* because resolution and scripts are in the renderer. WI 3 deletes those two and the file survives as pure **UI state** (`isDraft` / `hasLocalChanges`). §7 is the explicit accounting — nothing here may be quietly left behind. |

---

## 1. The finding: the duplication is already here, and it is load-bearing

This is not a hypothetical tidiness argument. The renderer already contains **five re-implementations of
engine logic**, and a sixth piece of engine knowledge (the gate) that exists solely because of the split.

| # | Renderer copy | Engine authority | Notes |
|---|---|---|---|
| 1 | `packages/gui/src/lib/resolveVars.ts` (`resolveVars:5`, `resolveHeaders:31`, `mergeEnvVars:22`) | `packages/engine/src/mocks/mockHandler.ts:13` | **Two copies already.** The engine's is private to mocks. |
| 2 | `packages/gui/src/lib/randomizer.ts` (190 lines, 31 tokens) | `packages/engine/src/utils/randomizer.ts` (116 lines, 31 tokens) | Data pools are **identical today** and **nothing pins them equal** — they agree by discipline, not by construction. |
| 3 | `packages/gui/src/lib/collectionRunner.ts:94` `executeOneRequest` — resolve → pre → replay → post → test | **does not exist** | A second copy of the *same pipeline* as #4, below. |
| 4 | `packages/gui/src/components/rest/useRestActions.ts:25` `handleSend` — resolve → pre → replay → post → test | **does not exist** | Same five steps, different call site. |
| 5 | `packages/gui/src/lib/runnerReport.ts:25` `generateHtmlReport` | `packages/engine/src/runner/reportHtml.ts:20` | The engine already renders the report; the renderer has a second renderer. |
| — | `packages/gui/src/lib/curlParser.ts` vs `packages/engine/src/importexport/importers/requests-curl.ts:14`; `packages/gui/src/lib/utils.ts:127,135` vs `workspaceFs` | | Same pattern, smaller stakes. |

> **⚠ The line numbers above were measured 2026-09-29 and will rot.** They are here so the first reader can
> verify the finding in one jump, not as a durable index — re-derive them before citing them anywhere else.

Plus the consequence the owner spotted directly:

> **The gate is evidence, not a solution.** `canSendByEntityId` refuses the id path when a field contains
> `{{…}}` or the request has scripts. Both conditions are facts about **where the work runs**, not about the
> request. Once the engine resolves and runs the pipeline, neither can make the stored entity differ from
> what the user sees, and both terms become dead weight. **A gate that has to encode "the client and the
> engine would disagree here" is a bug report about the architecture.**

---

## 2. What "the UI is a client" means, concretely

One test, applied to every line in `packages/gui/src`:

> **Would this line be different if the UI were a terminal, a web page, or a test harness?**
> If yes, it is engine work.

That is the whole rule, and it is deliberately not "does it feel heavy". It classifies the census cleanly:

| Renderer concern | Verdict |
|---|---|
| `resolveVars` / `resolveHeaders` / `mergeEnvVars` / randomizers | **Engine.** A CLI resolving `{{host}}` differently from the app would be a bug in one of them. |
| pre/post/test script orchestration | **Engine.** The *sandbox* is already engine-side (`mocks/scriptExecutor.ts`); only the *orchestration* is in the renderer. |
| collection run loop, run state, progress, cancellation | **Engine.** The CLI must be able to start a run and attach to it. |
| cURL parsing, capture→entity construction | **Engine.** Pure parsing of a wire format the engine already parses elsewhere. |
| report HTML/JSON rendering | **Engine.** Already is — the renderer copy is the duplicate. |
| base64↔text, `tryFormat`, `statusColor`, `contentTypeToMode`, layout, strings | **Presentation.** Keep. |
| open tabs, drafts, split orientation, zoom, theme | **UI state.** Keep — it is about *this window*, not about the workspace. |
| "is this tab saved, and does the editor differ from disk?" | **UI state.** Keep. This is what the gate reduces to (§7). |

---

## 3. Decisions

**D22 — the renderer holds no domain logic, and a source-level ratchet enforces it.** §1's table is
*pre-existing* drift, which means it grew while nobody was looking. The rule is only worth writing down if
something fails when it is broken — so WI 1 lands a test that scans `packages/gui/src` for domain logic and
fails on anything not in a **shrinking** allowlist. This is `tests/engine/layout.test.ts`'s idiom applied to
a second boundary, and it is the reason this phase cannot decay the way the last one did.
*Approved by:* the owner's direction above.

**D23 — resolution is engine-owned, and the engine resolves against its own environment state.** The
renderer must not send a resolved environment, and must not send a pre-resolved request. The engine already
owns the environments (`environments` entity kind, `activeEnvironmentId`, the synthetic `__global__`), so it
is already the authority — the renderer is currently *reconstructing* the answer with `mergeEnvVars`
(`App.tsx:288`) and passing it back. **The merge semantics (active overrides global) must move with it**, or
shadowed keys resolve differently engine-side.
*Approved by:* the owner's direction above.

**D24 — one pipeline, and both input forms run it.** `resolve → pre-script → send → post-script → test`
becomes **one engine function**. The inline form and the id form differ only in where the request comes from.
Today the renderer has two copies of this pipeline (§1 rows 3 and 4); after WI 3 it has none.
*Approved by:* the owner's direction above.

**D25 — the collection runner executes in the engine and reports progress by event.** The UI sends a
**folder id** and renders events. Run state (running, current index, results, cancellation) is engine state
for the lifetime of the run, not React state.
*Approved by:* the owner's direction above.

**D26 — a new protocol module lands engine-first.** MQTT, SSE, and whatever follows must be usable from the
CLI on the day it is added. WI 7 makes that a written contract plus a ratchet, so "engine does the work" is
the default rather than a convention each contributor has to remember.
*Approved by:* the owner's direction above.

**D27 — the pipeline's command surface: one new `*.send` per protocol, taking an `id` OR an inline request
(DECIDED 2026-09-29 by the owner — §5 option B).** `request.send { workspaceId, id }` for a saved tab, and
`{ workspaceId, request, scripts }` for a draft. **This is not §5's recommendation**, which was option A;
the owner chose B. §5's stated objection to B therefore has to be settled as part of WI 3:

> **⚠ B leaves yesterday's four `*Saved` commands as near-duplicates — §5 says so in as many words, and the
> owner settled it on 2026-09-29: they are REPLACED.** `request.replaySaved` → `request.send` with a widened
> payload union, and likewise for `graphql` / `soap` / `grpc`. **The replacement is a rename, not a
> migration**, because that family has never been committed — `plan/protocol-changes.md` documents work that
> is still unstaged. So B costs no fifth command name and no second client method, and it deletes
> `savedRequestCommands.ts`'s `encodeBody()` step rather than leaving it beside the pipeline's text body.
>
> **Two consequences worth naming.** (1) The `*Saved` family never had boundary-ratchet entries — it lives
> engine-side — so replacing it removes no allowlist entry, and §6's WI 3 schedule is unaffected. (2)
> `packages/gui/src/lib/savedEntitySend.ts`'s id-vs-payload decision now chooses between `*.send`'s **two
> input forms** rather than between two commands, which is what §7 predicted: after the shrink it is
> `!isDraft && !hasLocalChanges`.
>
> **The body is carried as TEXT** (§5's sub-decision, unchanged by the choice above). **The response keeps
> base64 on the wire** — the renderer's `b64ToText` stays, and stays presentation.

**D28 — the randomizer catalogue is a pinned UI copy, not a wire command (DECIDED 2026-09-29 by the
owner).** *"random.\* types can be kept in UI as auto complete but actual value resolver still stays in
engine. UI just has copy of list of random.\* attributes."* So the split is **list vs resolver**, not
renderer vs engine: `RANDOMIZER_TOKENS` now lives in `packages/engine/src/utils/randomizer.ts` **as the
authority**, the renderer keeps its copy for `RandomizerHint.tsx`'s autocomplete, and
**`tests/gui/randomizerCatalogue.test.ts` pins them equal** — including that every catalogued key has a
matching `case` in the generator. **The pin is not optional:** a copy with nothing checking it is exactly
§1 row 2's "two data pools pinned equal by nothing", so the test is what makes the copy legitimate rather
than a new instance of the defect this phase opens with. The boundary ratchet's `randomizer` construct was
narrowed to the **generator** accordingly — flagging `RANDOMIZER_TOKENS` there would contradict D28.

**D29 — a run's lifetime is the ENGINE's, not the client's (DECIDED 2026-09-30 by the owner).** A client
disconnect and a workspace switch **do not** cancel a run; `runner.status` lets a reconnecting client attach to
one already in flight. A **host restart loses it** — there is no durable run record, and the `.runs/` files stay
what they have always been: a *report* of a finished run, written by `runner.saveReport`, not live state. This
is §8 trap 5's requirement — *"a disconnected client must not cancel the run … and 'cancel on disconnect' is the
default most implementations reach for"*. Persisting the run so a restart could resume it was considered and
**rejected**: resuming would have to decide whether to re-send the requests the run had already sent, and that
is a question with no safe default.

**D30 — one run per `{workspaceId, folderId}`; a second is REJECTED, and two different folders may run at once
(DECIDED 2026-09-30 by the owner).** A run executes one folder's ordered requests, so the folder *is* the run's
identity and that is the lock's granularity: `runner.start` for a folder already running fails with an error
naming it. §8 trap 6 is why this is a decision rather than a discovery — *"concurrency is undefined today
because the renderer made it impossible … prevented by there being one browser tab."* The per-folder policy is a
**superset** of that behaviour (which allowed zero concurrent runs), so nothing regresses. A workspace-wide lock
was rejected because it forbids two independent collections for a reason the UI never had; a queue was rejected
because it needs ordering semantics and a way to report "queued" that this document has no use for yet.

---

## 4. The target architecture

```
renderer (no domain logic)
   │  request.send { workspaceId, id }                     ← saved tab, and the CLI
   │  request.send { workspaceId, request, scripts }        ← draft tab only
   ▼
engine  ── resolve (env + randomizers, from engine state)
          ── pre-script        ┐
          ── send (replayRequest / graphql / soap / grpc)
          ── post-script       ├─ the ONE pipeline (WI 3)
          ── test-script       ┘
          ── return { request, response, tests, logs, errors }
   │
   └── runner.start { workspaceId, folderId } → events → runner.cancel / runner.status   (WI 4)
```

Three properties this buys, each of which the owner named:

1. **Single point of maintenance.** One resolver, one randomizer, one pipeline, one report renderer.
2. **A light UI.** The four tabs and the runner lose their pipelines; the runner's `useCollectionRunner`
   loses `running`/`progress`/`results`/`report` and gains a subscription.
3. **A CLI that works.** `bifurc requests send <id>` and a run the engine keeps executing after the client
   disconnects are only possible if the engine owns the pipeline and the run.

---

## 5. The command surface — options, and the recommendation

**Constraint:** `request.replay` (base64 body, no resolution, no scripts) must survive as the **raw
primitive** — the proxy's own path uses `replayRequest` directly, and the CLI may want an unresolved replay.
So the pipeline is additive.

| Option | Shape | Verdict |
|---|---|---|
| **A. Extend the existing pairs** (recommended) | `request.replay` keeps its contract; `request.replaySaved` becomes *load + full pipeline*; the inline form gains optional `scripts` and stops being pre-resolved | **Least new surface, and it retires the gate's domain terms.** The two commands already exist and the UI already switches between them; what changes is how much the engine does on the way through. |
| B. One new `*.send` per protocol with `id` \| inline | `request.send { workspaceId, id }` / `{ workspaceId, request, scripts }` | Cleanest naming, but a **third** name per protocol, and it leaves `*Saved` (added yesterday) as a near-duplicate — the exact thing `plan/protocol-changes.md`'s checklist warns about. |
| C. Fold the id form into the raw command (`id?`) | `request.replay { workspaceId, id?, method?, url?, … }` | **Rejected.** One command with two mutually-exclusive input modes is a schema nothing can validate honestly, and it breaks the raw primitive the proxy-adjacent paths rely on. |

**Recommendation: A**, with these two sub-decisions recorded because they are where the real cost sits:

- **The pipeline command carries the body as TEXT, not base64.** `SavedRequest.body` is stored as text, and
  the pipeline's input *is* the stored entity. This deletes the `encodeBody()` step that exists in
  `savedRequestCommands.ts` **only** because the raw command wants base64 — one fewer transformation and one
  fewer place for the blank-means-empty rule to be re-derived.
- **The response keeps base64 on the wire.** Binary safety is a wire concern; the engine decodes internally
  for the scripts. The renderer's `b64ToText` stays, and stays presentation.

---

## 6. Work items

Each item is independently shippable, and **each owes a deletion** — the project's rule that a move is not
finished until the old copy is gone. "Allowlist −N" is the boundary ratchet from WI 1, which is what makes
the deletions checkable rather than remembered.

### WI 1 — the boundary ratchet
`tests/gui/boundary.test.ts`: scan `packages/gui/src` for the domain constructs of §1 (resolver, randomizer,
pipeline, curl parser, report renderer, path helpers) and fail on any file not in an **`ALLOWED`** map that
can only shrink — the same shape and the same staleness assertion as `tests/engine/layout.test.ts`. Entries
carry the WI that will delete them. **Lands first, on purpose**: it is what stops the migration from adding a
seventh duplicate while it runs.

### WI 2 — resolution and randomizers move to the engine
One engine module (candidate home: `packages/engine/src/utils/resolve.ts`, extracted **down** per §5.1 since
`mocks/` and the new pipeline both need it). Move the **global+active merge** with it (D23). Delete
`gui/lib/resolveVars.ts` and `gui/lib/randomizer.ts`; make `mockHandler.ts` stop carrying its private copy.
Expose the token **catalogue** separately from the generator — `RandomizerHint.tsx` legitimately needs the
list for autocomplete, and deleting it with the generator would take a working UI affordance with it.
**Allowlist −2.** Delta: the `{{…}}` term in the gate becomes unnecessary (deleted in WI 3).

### WI 3 — the request pipeline moves to the engine

**Split into two halves.** §8 trap 1 forbids landing "the engine starts and the renderer stops" separately
*per protocol*, but the engine's side of it is protocol-independent and is not the same amount of work as
rewiring four tabs. So:

- **Engine half — LANDED.** `packages/engine/src/pipeline/runPipeline.ts` is D24's one function:
  `resolve → pre-script → send → post-script → test`, with `send` **injected** so the file knows no
  protocol. `packages/engine/src/workspace/envState.ts` (new) supplies the merged environment —
  `__global__` overridden by the active one — which is the D23 / §8-trap-2 requirement that the merge
  travel with the *call*; it reads the two things it needs directly rather than through
  `loadWorkspaceDispatchSlice()`, which **writes** `enabled.json` as a side effect of reading an
  environment. `utils/resolve.ts` gained `mergeEnvVars`.
  `tests/pipeline/runPipeline.test.ts` (15 cases) asserts both of §10's named traps: a **shadowed key**
  resolves to the active value while a global-only key stays reachable, and the text/base64 boundary
  (§8 trap 4) holds in both directions.
- **Renderer half — REST LANDED 2026-09-30; the other three are next, one at a time** (§8 trap 1: *"One
  commit per protocol: the engine starts and the renderer stops."*). Per protocol: add its `*.send` command
  (D27's union payload), rewire its tab, then delete that tab's renderer-side pipeline. `script.execute`
  **stays** — the pipeline calls the same sandbox (`mocks/scriptExecutor.ts`) directly, and a CLI may still
  want one script on its own; what moves is the orchestration, not the command.

**REST's landing, concretely** (the wire argument is `plan/protocol-changes.md`'s 2026-09-30 entry):

- `request.replaySaved` → **`request.send`**, a union of `{workspaceId, id}` and `{workspaceId, request,
  scripts}`, returning the pipeline's full result. The name it replaced was never released, so this is a
  rename and the command count stays **93**.
- `packages/engine/src/pipeline/commands.ts` (new) drives it. It lives with the pipeline rather than beside
  the other three counterparts, because a command that calls `pipeline/` is a `workspace/ → pipeline/` edge
  and `workspace/`'s §6 row is `kernel/` + `utils/` — **so the ratchet forced a placement, not merely a
  permission.** `pipeline`'s contract gained `kernel/` for it, and its `ORPHANS_ALLOWED` entry was deleted.
- `useRestActions` assembles the request and calls `window.api.requestSend`; it no longer imports
  `resolveVars`, `runPreScript`, `runPostScript` or `runTestScript`, and `RestTab` passes `false` for the
  gate's two domain terms. **The allowlist went 9 → 8** — the first entry this ratchet has retired.
- **`hasExecutableScripts` is deleted from the gate entirely** — §7 predicted it and the migration is what
  made it possible: it was `true` for REST alone, so REST's change made it `false` for all four tabs, and a
  constant inside that gate reads as a check being performed when none is. `usesVariables` stays until the
  last of the three remaining tabs migrates.

**The deletions this item owes, corrected against the measurement:**

| allowlist entry | deleted when |
|---|---|
| ~~`components/rest/useRestActions.ts#pipeline`~~ | **DELETED 2026-09-30** — REST's `*.send` landed |
| `lib/collectionRunner.ts#pipeline` | **WI 4** — it is the second orchestrator, and the loop wrapping it is WI 4's |
| `lib/scriptRunner.ts#pipelineStep`, `lib/testRunner.ts#pipelineStep` | **WI 4** — their *last* caller is `collectionRunner`, so WI 3 removed one caller and WI 4 removes the other |
| `lib/resolveVars.ts#resolver`, `lib/randomizer.ts#randomizer` | the last tab stops resolving — after **all four** protocols |

**§6's "Allowlist −1" for this item is wrong in both directions, and the measurement is the correction.**
The suite counts *constructs present*, so REST removes **one** entry — the orchestration — not three: the
step wrappers outlive it because `collectionRunner` still calls them, which §6's table had attributed to
this item.

#### The three non-REST protocols: what each migration actually is

REST is the only one with a *pipeline* in the renderer, so the other three are simpler — but the
simplification is a different **shape**, recorded here so each of the three is not re-derived:

- **They have no scripts.** Their tabs store `preScript` / `postScript` and their `handleSend` never reads
  them, so `scripts` is not part of their wire form at all. The script sandbox's context *is* a
  `{method, url, headers, body}` request (`mocks/scriptExecutor.ts`'s `IpcScriptOpts`), which a GraphQL or
  gRPC request is not — so a `scripts` field would be a field that lies. It arrives when the sandbox grows a
  protocol-neutral context, which is the same change that would let these commands use `runRequestPipeline`.
- **They therefore do not call `runRequestPipeline`.** That function's `PipelineRequest` view exists for the
  three script steps, and a protocol with no scripts would have to fabricate one. Their `*.send` handlers are
  *load-or-inline → resolve every field against the merged env → dispatch the raw command* — D24's pipeline
  with the script steps empty and no invented shape. **Their home is their own feature module**
  (`graphql/commands.ts`, `soap/commands.ts`, `grpc/commands.ts`), whose §6 rows already grant
  `kernel/` + `utils/` + `workspace/`: no new row, no sideways grant.
- **⚠ The engine must resolve EVERY field, not the subset the tab resolved — and this is what makes the three
  worth doing at all.** Each tab resolved *fewer* fields than the gate's token scan treats as resolvable:

  | tab | gate scan (the intent) | `handleSend` actually resolved (the practice) |
  |---|---|---|
  | GraphQL | `endpointUrl`, `query`, `variables`, `operationName`, `headers` | `endpointUrl`, `variables`, `headers` |
  | SOAP | `endpointUrl`, `soapAction`, `body`, `headers` | `endpointUrl`, `body`, `headers` |
  | gRPC | `serverAddress`, `serviceName`, `methodName`, `requestBody`, `metadata` | `serverAddress`, `requestBody`, `metadata` |

  A `{{token}}` in a GraphQL `query`, a SOAP `soapAction` or a gRPC `serviceName` has therefore **never
  resolved**, in *either* path — the gate only ever chose between two ways of sending the same unresolved
  field. So an engine that resolved only what the tab did would preserve the bug **and** keep
  `usesVariables` in the gate forever, because a token in one of those fields would still make the stored
  entity differ from the screen. **The gate's scan is the specification of "every field", and of the two it
  is the better spec** — the gate was written from intent, the tab from what someone remembered to resolve.
  Resolving all of them is what lets §7's end state arrive.

#### WI 3 is COMPLETE — all four protocols landed 2026-09-30

The family's shared module, `workspace/entities/savedRequestCommands.ts`, is **deleted**, and so are its
registrations in `registerAll.ts`, `src/ipc/handlers.ts` and both integration harnesses. §7's end state
arrived with it: `savedEntitySend.ts` is now `!isDraft && !hasLocalChanges`, **both** domain terms deleted
rather than weakened, and `tests/renderer/savedEntitySend.test.ts` pins each departure so neither can return
unnoticed. `containsVariableToken` is deleted with them — it had exactly one consumer, the four gates.

**⚠ But the allowlist did NOT reach `{}`, and the reason is not WI 3's.** Two entries §6 attributed to "the
last tab stops resolving" **outlive the four tabs**, because resolution is not only the tabs' business:

| entry | still present because |
|---|---|
| `lib/resolveVars.ts#resolver` | `useWebSocket.ts` resolves the url and the headers (**WI 6**), and `HealthBarPanel.tsx` resolves a service url (WI 5's "remaining renderer processing"). `App.tsx:288`'s `mergeEnvVars` also still runs, for the panes' active-env hint. |
| `lib/randomizer.ts#randomizer` | the generator is only reachable *through* `resolveVars`, so it goes with the resolver — i.e. with WI 6. |

**So the resolver and randomizer entries belong to WI 6, not to WI 3**, where §6's table put them. The
correction matters now that WI 3 is closed: a reader looking for the remaining debt would otherwise conclude
those two entries had been forgotten. The `pipeline` / `pipelineStep` entries are unchanged and still WI 4's —
`collectionRunner` is the second orchestrator and the loop wrapping it is that item's.

**A gap in the layout ratchet was closed while adding `pipeline/`, and it matters more than this item.**
`tests/engine/layout.test.ts` checked that every module named in `CONTRACT` is a real directory, but not
that every real directory is named — and `contractViolations()` returns `false` for an unknown module, so
a new directory's imports were **never checked**. Adding a directory was therefore the one silent way out
of the layering rule. The converse assertion now exists (six assertions, was five), verified by planting a
`bogus/` directory.

### WI 4 — the collection runner moves to the engine

**Engine half — LANDED 2026-09-30.** `packages/engine/src/runner/runCollection.ts` is the loop, and it is thin
because WI 3 exists: the whole per-request body is **one `registry.invoke("request.send", {workspaceId, id})`**.
The renderer's version could not be that thin — it had to do the five steps itself, which is why §1 counted it
as a second copy of the pipeline rather than as a runner.

`packages/engine/src/runner/runRegistry.ts` is the **run state**, and it is where **D29** and **D30** live:
module-level by design (D29 — no persistence, no teardown hook, and *nothing in the file can observe a
disconnect*), keyed by `{workspaceId, folderId}` (D30 — a second run of the same folder is refused while one is
in flight, two different folders run side by side, and a *finished* run does not block a new one because the
snapshot has to outlive the run for `runner.status`).

`tests/runner/runRegistry.test.ts` (12 cases) drives both directly, including the three that make the decisions
checkable rather than asserted: `startRun` returns **before** the loop finishes, `cancelRun` is the **only**
lever that stops a run, and the two halves of D30 in both directions.

**⚠ Two config fields stay un-honoured, and that is deliberate.** `runner.json` carries `stopOnFailure` and
`iterations`, and the renderer's `runCollection` read **neither** — a folder configured to stop on failure ran
to the end anyway. Implementing them here would be a behaviour change smuggled in under a move, and each is a
real decision (does a failed *test* count as a failure, or only a failed *send*?) that this item is not the
place to make. Recorded in `runCollection.ts` so the omission reads as one.

**Still to come for this item:** the wire — `runner.start` / `runner.cancel` / `runner.status` plus the progress
events — and the renderer half. **⚠ The engine half is safe to land first, unlike WI 3's**, and the reason is
worth stating: trap 1 exists because two resolvers *double-apply* a value, whereas two run loops do not
interfere — the renderer's is only reachable from the UI, and nothing routes to it but the UI's own button.
There is no window in which a request is processed twice.

**Renderer half — LANDED 2026-09-30.** `useCollectionRunner` is a **subscription**: `runnerStatus` on mount
(so a tab opened mid-run attaches — D29 visible in the UI), `runnerStart` / `runnerCancel` for the two
buttons, and the two events for everything else. **⚠ The hook still holds React state, and the distinction
that matters is what it is *used* for**: nothing in it decides anything — the cancel is a request to the
engine, and whether a run is going comes from the engine's answer. It is a mirror, not the authority, and that
is the §2 test applied to it (*would this differ if the UI were a terminal?* — the state would not).

**⚠ A flaw the tests caught, and it was in the hook rather than in the test.** The first version only
*updated* a snapshot it already had (`prev ? … : prev`), so a `runner.finished` event arriving with no snapshot
was dropped — which is exactly the disconnected-client case D29 exists for. Both handlers now **establish**
state when there is none, and that is what `total` riding on every progress event was for. (A progress event
carries no start time, so that branch's `startedAt` is a guess; recorded in the hook rather than papered over.)

**The four deletions this item owed, all landed:** `lib/collectionRunner.ts`, `lib/scriptRunner.ts`,
`lib/testRunner.ts`, and `runnerReport.ts`'s HTML renderer — **the allowlist went 8 → 4**.
`runnerReport.ts` itself survives with the two *storage* wrappers, which is the `file#construct` keying doing
its job: the file stayed, the construct the ratchet tracked did not. `script.execute` stays — trap 7 — and the
renderer's two deleted test files' subjects are `tests/runner/runRegistry.test.ts`'s now.

**The wire and the renderer half — the original spec, unchanged:**
`runner.start { workspaceId, folderId }` → engine loads the folder's ordered requests, runs the WI 3 pipeline
per request with the configured delay, and emits progress; `runner.cancel`, `runner.status`. New event names
in `packages/protocol/src/events.ts` (the list is total, so this is a deliberate protocol addition) plus the
matching client subscriptions — which **grows the client surface again**, so it is a recorded change of the
same kind as yesterday's, not an accident. Delete `collectionRunner.ts`'s loop, `useCollectionRunner`'s
`running`/`progress`/`results`/`report` state, and `runnerReport.ts`'s HTML renderer. **Allowlist −1.**

### WI 5 — the remaining renderer processing

Four sub-items, and they are independent — each owes its own deletion:

1. **cURL parsing → `request.parseCurl`, and `gui/lib/curlParser.ts` deleted. — LANDED 2026-09-30.** The
   parser is `packages/engine/src/utils/curl.ts`, shared with the cURL importer, which had been carrying a
   **third** copy whose own comment said so. **Allowlist −1.**
2. **Capture→entity construction → the engine. — LANDED 2026-09-30.** `capture.buildRequest` /
   `capture.buildMock` replace `reqToHeadersBody` / `buildMockInitial`; the construction is
   `packages/engine/src/utils/captureEntities.ts` and `captureUtils.ts` keeps only its formatting half.
   **⚠ Three things it turned up that the plan did not anticipate:** the hop-by-hop list was a **third**
   copy of the same five names (now imported, not moved — and the name is a lie for it, recorded); the
   mock path needed a **content-type predicate**, which is a third **pinned** renderer copy; and the
   base64 decode had to be **ported rather than copied**, because `atob` throws where `Buffer.from` does
   not. **Allowlist unchanged** — neither function was one of §1's constructs.
3. **The `gui/lib/utils.ts` path helpers → deleted. — LANDED 2026-09-30, both halves.** The engine half is
   `workspace/entityPaths.ts` plus `forEachCrudKind()`, which build `entity id → relPath` and
   `folder id → directory prefix` by walking the **kind registry** (so a new kind appears for free); they
   ride `sync.getEntityStatus`'s result as `EntityStatusPayload.paths`. The renderer half is
   `lib/entityPaths.tsx` (a lookup + context) and eleven call sites. **⚠ They do not ride `config.get`, and
   the check is `config.save`:** it hands its payload straight to `saveConfig()`, so a computed field on an
   entity would be **persisted to disk and read back stale**. **⚠ No protocol change** — the result type is
   hand-mirrored, not declared in `@bifurc/protocol`. **⚠ One caller takes the lookup as an argument rather
   than from the context, and it is not an oversight:** `useEntityGitActions` is a hook, so it runs in
   `App`'s body *above* the provider; see §11. **Allowlist 3 → 2.**
4. **`HealthBarPanel.tsx`'s resolution → the engine. — LANDED 2026-09-30.** §6's WI 3 completion note already
   attributes this here rather than to WI 6: it is the *other* renderer resolver user besides `useWebSocket`,
   and moving it is WI 6's prerequisite rather than its content. It took a **new read command**,
   `env.resolveText { workspaceId, text }` — the engine owns resolution, so the panel can no longer compute
   the url it is about to probe, and *display* is a need the pipeline cannot serve. **⚠ The allowlist does
   not move:** its entry is keyed on the resolver's **definition**, and `useWebSocket.ts` still imports it.
   **⚠ The near-miss this avoided:** letting `healthbar.checkUrl` return the url it resolved would have left
   the card showing a raw `{{host}}/health` until the first check — and a service with auto-refresh off may
   never be checked at all.

**Body-mode sniffing needs no change, and that is a measurement rather than an omission.** §6 asked for the
content type to come from the engine's response instead of being re-derived. WI 3's migration already did it:
`useRestActions` reads `res.headers["content-type"]` — the engine's response — and `APPLY_CURL` reads the
headers the engine's parser returned. There is no second derivation left to delete.

**⚠ §6's "Allowlist −2" is right and the handover note's "−3" is not, and the difference is the same class of
error §11 keeps catching.** Four entries existed when WI 5 opened: `resolveVars.ts#resolver`,
`randomizer.ts#randomizer`, `curlParser.ts#curlParser`, `utils.ts#pathHelpers`. WI 5 can retire exactly **two**
(items 1 and 3). The other two belong to **WI 6**, and item 4 does not change that: moving `HealthBarPanel`'s
resolution leaves `useWebSocket.ts` as the renderer's last resolver *user*, so the **definition** in
`lib/resolveVars.ts` — which is what the ratchet keys on — survives until WI 6. A −3 would require WI 5 to
delete `lib/resolveVars.ts` while `useWebSocket` still imports it. So the measured WI 5 is **−1 so far** (item
1), with a further −1 when item 3 lands.

**Item 1's detail, because two of its decisions are load-bearing.** The command returns the parse
**unfiltered**: `SKIP_CURL_HEADERS` is the *caller's* policy (the tab drops hop-by-hop headers on import, the
importer drops them inside its own pipeline), so a parser that filtered would be choosing for both. That
leaves the renderer needing the list for one read-side job the engine cannot do for it — filtering a **stored**
mock's `capturedHeaders` on load — so the renderer keeps a **pinned copy**, exactly as D28 allows for
`RANDOMIZER_TOKENS`, and `tests/gui/curlHeaderSkip.test.ts` is the pin. The boundary ratchet's `curlParser`
`detect` was **narrowed to the parser** accordingly; narrowing a pattern is the one edit to that suite that
loosens it, so it ships with the pin rather than instead of it.

### WI 6 — WebSocket to the engine *(separate; may be its own phase)* — **LANDED 2026-09-30**

`useWebSocket.ts` held a module-level `Map` of live sockets and did the protocol work in the renderer, so a
socket died with the window and a CLI could not open one. Engine-side connection registry + commands +
subscription events. **Not on the critical path for WIs 1–5**, and the largest single item here — which is why
it is called out rather than folded in.

**What landed.** `packages/engine/src/websocket/` — `wsRegistry.ts` (the sockets and the four decisions) and
`wsCommands.ts` (the six `ws.*` commands and the bus binding) — replacing the reserved `index.ts` placeholder.
The wire is six commands plus two events (`event.ws.status`, `event.ws.message`); `env.getMerged` came with it
and is the smaller half of the same item. `useWebSocket.ts` is now a **mirror** over `window.api`.
**Both ratchets reached `{}`** — see §9.

**The four decisions `Protocol_Module_Contract.md` §4 requires, made rather than discovered:**

| decision | here |
|---|---|
| **Scope** | `connect` / `disconnect` / `send` are `execute`; `status` / `list` are `read`; `clearMessages` is `write`. The split is what lets a reconnecting client ask *what happened* without being granted the right to open or close anything. |
| **Lifetime (D29)** | A transport disconnect and a workspace switch do **not** close a socket; a host restart loses every one. `disconnectSocket()` is the only lever and `ws.disconnect` its only caller — **there is no disconnect hook in the file**, deliberately, which is §8 trap 5's requirement stated as code. |
| **Concurrency** | One socket per `{workspaceId, socketId}`; a second connect **replaces**. See below — this is deliberately *not* D30's refusal. |
| **Event completeness** | `ws.status` returns the whole snapshot **including the retained message log**, so a client that attached mid-stream renders the backlog rather than only what happens next. That is the `total`-rides-every-event rule in its socket form. |

**⚠ Why the concurrency policy differs from D30, and it is a difference rather than a slip.** D30 refused a
second run of the same folder. The distinction is the kind of thing each command makes: a **run** is a unit of
work, so a duplicate is effort nobody asked for twice; a **socket** is a channel, so a second connect on a live
key is a **reconnect** — the renderer did exactly that ("close any existing connection for this tab"), and
refusing would make a broken socket unrecoverable without a separate disconnect first. The replacement is
atomic in the sense that matters: the old socket is marked superseded *before* it is closed, so its close event
cannot emit a `disconnected` that arrives after the new `connecting`. That is what the registry's `generation`
counter exists for, and the failure it prevents is an intermittently wrong-coloured dot rather than an error.

**⚠ D29 in the UI, and it is the one place the migration changes user-visible behaviour.**
`useWebSocket` has **no disconnect-on-unmount**, because an unmount cannot tell "the user closed this tab" from
"the window is going away". `WebSocketsPanel` issues the explicit `wsDisconnect` from the tab-close handler,
which *is* a gesture. The consequence, recorded rather than hidden: a tab that disappears without being closed
leaves its socket running — which is the point, and reopening that tab re-attaches and shows the retained
transcript. The connection-limit banner reports the engine's live count, so those sockets are visible rather
than mysterious.

**⚠ Two behaviour changes that are improvements rather than ports**, both unreachable before because the
*browser* API could not express them: the browser `WebSocket` constructor **cannot send headers at all** (the
renderer's own comment said the resolved headers were "stored, not injected"), so a `{{TOKEN}}` in an
Authorization header went from *resolved and discarded* to *resolved and sent*; and binary frames are now
detected with the client's `isBinary` rather than `typeof ev.data === "string"`, which was a proxy for the
frame's opcode rather than a reading of it.

**⚠ And one bug the tests caught, in the registry rather than in a test.** A `ws.disconnect` issued while the
socket is still CONNECTING aborts the handshake, and `ws` reports that as `close(1006)` — abnormal. Read by
close code alone, a user pressing Disconnect mid-connect would have been shown *"Connection closed (code
1006)"*. So `closeRequested` — which the `error` handler already consulted — is consulted by the `close`
handler too: **a close the caller asked for is clean whatever code the client reports**, because the code
describes how the *handshake* ended and the flag describes who ended it.

**The two ratchets, and what emptied them:**

- **`tests/gui/protocolModule.test.ts` 1 → 0.** The entry was `hooks/useWebSocket.ts#websocket`, and the
  construction is gone: the renderer contains none of the seven patterns.
  **⚠ Emptying the allowlist removed this file's vacuity guard, and a replacement was required.** While the
  allowlist held one entry, its staleness assertion also proved the scanner worked — it passed only if the scan
  found that live violation. With `{}` there is no entry whose liveness depends on a hit, so a `detect` regex
  that silently stopped matching would leave every assertion green while measuring nothing. The guard is now
  explicit: `each detect pattern still matches a real construction` runs every pattern against a sample that
  must match **and one that must not**, with the table checked in both directions against `CONSTRUCTS`.
- **`tests/gui/boundary.test.ts` 2 → 0.** `lib/resolveVars.ts` is **deleted**; `lib/randomizer.ts` survives with
  its catalogue only, which is the `file#construct` keying doing its job (the WI 4 `runnerReport.ts` outcome).
  The two entries were §6's mis-attribution to WI 3 — see §6's WI 3 completion note — and they could not retire
  a moment earlier because the ratchet keys on the resolver's **definition**.

**⚠ And a factual error in `protocolModule.test.ts`'s own docblock, found by this change and corrected.**
It claimed *"a comment mentioning `new WebSocket` in a design note is not a violation"*. The scan reads the
file as **text**, so a comment that quotes a construction **is** one — and it bit immediately: the new
`useWebSocket.ts` docblock described what it used to do by writing the constructor out, and the suite failed on
it. The docblock was reworded rather than the scanner loosened, because **a false positive is safe and a false
negative is not**: a scan that errs toward flagging prose is better than one that strips comments and can
therefore hide a real construction on the same line as a `//` inside a string literal.

**`env.getMerged` and `App.tsx`.** The merge (`__global__` overridden by the active environment) is D23's, and
`App.tsx` was still computing it on every render and threading the result down as `activeEnv` for
`EnvVarHint`'s autocomplete. It is now a read. **This is the D28 list-vs-logic split, not a new rule**: the UI
may hold the *list* it offers the user, and the *resolver* is never the UI's — which is exactly why
`lib/resolveVars.ts` could be deleted outright rather than trimmed, and why the catalogue in
`lib/randomizer.ts` stays.

**Still open, recorded rather than fixed:** `packages/gui/src/lib/blocks.ts` holds the same class of
construction as WI 5 item 2 — it builds a `mocks` entity from something the user did — and **no ratchet sees
it**, because it is not one of §1's constructs. It was found while doing WI 5's capture work and is unchanged
by WI 6. See the note under WI 5 item 2.

### WI 7 — the extensibility contract for new protocol modules — **LANDED 2026-09-30**

Two deliverables, both written **before** MQTT rather than after it:

- **`Protocol_Module_Contract.md`** (repo root, alongside `File_Ops_Protocol.md` and `Engine_Module_Plan.md`)
  — the checklist a new protocol module must satisfy to be engine-first: the command set, the `*.send` union
  (D24/D27), the entity kind, the events, the module directory and its `CONTRACT` row, the two registration
  call sites, and the four decisions a module with long-lived state owes (scope, lifetime D29, concurrency
  D30, event completeness). It is **derived from the four migrations rather than from taste** — every rule in
  it is something this codebase got wrong once, and the §5 anti-pattern table names the cost of each.
- **`tests/gui/protocolModule.test.ts`** — the ratchet that keeps it true. It asks the complementary question
  to `tests/gui/boundary.test.ts`: the boundary ratchet asks *"is engine logic implemented in the renderer?"*
  (by name, for §1's constructs); this one asks ***"does a tab speak the wire itself?"*** (by construction
  site — `new WebSocket`, `new EventSource`, `http.request`, `net.connect`, `dgram.createSocket`,
  `XMLHttpRequest`, a bare `fetch(`).

**⚠ Why a second ratchet file rather than a seventh construct in the first.** The two hold different
categories — engine *logic* vs engine *I/O* — and their entries retire for unrelated reasons. Folding them
together would give one allowlist whose staleness assertion is unreadable, because an entry's owner would no
longer be one work item.

**The allowlist opened at one entry — `hooks/useWebSocket.ts#websocket`, owned by WI 6 — and is now `{}`.**
The `websocket/` engine module was already reserved (§9) with its `CONTRACT` row written, so WI 6's landing
point existed and the item arrived under the layering rule rather than outside it. **The suite was
self-verifying in the direction that mattered:** its staleness assertion passed only if the scan actually
found that construction, so a `detect` pattern that matched nothing failed the run rather than passing
vacuously — which is the failure mode a name-based scan is most prone to.

**⚠ And emptying the allowlist destroyed that property, so WI 6 replaced it with an explicit control.** With
no entry left, nothing's liveness depends on a hit — every assertion could go green while the scanner measured
nothing. `each detect pattern still matches a real construction` now runs each pattern against a sample that
must match **and one that must not**, with the control table checked in both directions. **The lesson is
general: a ratchet whose allowlist is empty needs a positive control, or it is not a ratchet.** §9's own
`boundary.test.ts` has the same shape now, and it is the one gap this item found in its predecessor.

**⚠ And the docblock's "a comment mentioning the construction is not a violation" was half wrong**, corrected
in WI 6: the scan reads the file as **text**, so a comment that quotes a construction *is* flagged. That is
left as-is deliberately — a false positive is safe and a false negative is not — but it is worth knowing
before writing a design note in the renderer.

**What it does not catch, recorded rather than papered over:** detection is by construction *site*, so a
transport reached through an alias, or a socket dependency added to `packages/gui/package.json`, is invisible.
The stronger form is a dependency assertion on that package, and it belongs with the first module that needs
one — §6 of the contract says so rather than pretending the scan is complete.

---

## 7. The gate: exactly what happens to it

`packages/gui/src/lib/savedEntitySend.ts` is not deleted. It **shrinks to UI state**, and the file's
docblock loses the domain reasoning with it. The accounting, term by term:

| Term | After WI 3 | Why |
|---|---|---|
| `isDraft` | **stays** | A draft has no entity on disk. This is a fact about the tab, not about the request. |
| `hasLocalChanges` | **stays** | Autosave is debounced; "the file matches the editor" is real UI state. |
| `usesVariables` | **deleted** | The engine resolves. A stored `{{host}}` is no longer a reason the disk and the screen disagree. |
| `hasExecutableScripts` | **deleted** | The engine runs them. The stored entity *is* the complete description of the request, scripts included. |

So the end state is `!isDraft && !hasLocalChanges` — a question the UI can answer about itself. The
`hasExecutableScripts` asymmetry (REST passes `true`, the other three `false`) also disappears, because it
was a fact about which tabs happened to wire scripts up, not about protocols.

---

## 8. Traps this phase must respect

1. **⚠ Do not resolve in both places, even briefly.** A resolved value can itself contain `{{…}}`, so an
   engine that resolves while the renderer still does will double-resolve for a *shadowed* case and look
   correct in every test that does not use one. **One commit per protocol: the engine starts and the renderer
   stops.**
2. **⚠ `mergeEnvVars` is not in the engine.** `App.tsx:288` merges global + active before resolving, and the
   engine's `mockHandler` has no equivalent — it resolves against a single env. Moving resolution without
   moving the merge changes which value a shadowed key resolves to. The merge is part of WI 2, not a detail.
3. **⚠ `{{random.*}}` is non-deterministic, so a re-executed pipeline mints new values.** Today the renderer
   resolves once and a transport retry re-sends *those bytes*; engine-side resolution means a retry would
   re-mint. It happens to be safe **only because the retry policy already classifies these commands as
   non-idempotent** (`packages/client/src/retry.ts`) — so the classification is now load-bearing for a second
   reason, and a future "make `request.*` retryable" change would silently start sending different data.
4. **⚠ Two body encodings will coexist.** `request.replay` takes base64; the pipeline takes text. Documented
   in §5, but it is the kind of pair that gets mixed up in a refactor — assert it in a test rather than in
   prose.
5. **⚠ Engine-owned run state needs a lifetime policy, and it is a decision, not an implementation detail.**
   What happens to a run when the workspace switches, when the host restarts, when the client disconnects
   mid-run? The CLI's whole reason for wanting this is that **a disconnected client must not cancel the run** —
   so "cancel on disconnect" is the wrong default, and it is the default most implementations reach for.
6. **⚠ Concurrency is undefined today because the renderer made it impossible.** Two runs of one folder, or
   runs of two folders, were prevented by there being one browser tab. The engine needs an explicit policy
   (one run per workspace? a queue? rejection?) — decide it in WI 4 rather than discovering it.
7. **⚠ `script.execute` must not be deleted with the orchestration.** It is the sandbox's entry point and the
   pipeline's dependency. WI 3 removes the *orchestration* from the renderer, not the command.
8. **⚠ The randomizer catalogue is UI-read data.** `RandomizerHint.tsx` reads `RANDOMIZER_TOKENS`. Deleting
   the renderer's randomizer must not take the autocomplete list with it — split the catalogue from the
   generator in WI 2.
9. **⚠ The boundary ratchet's allowlist must be able to shrink to `{}`.** If an entry cannot be deleted by the
   WI that claims it, the entry is wrong — the same staleness assertion that forced the engine-layout
   cleanup's last 22 violations.

---

## 9. Acceptance criteria

**All met as of 2026-09-30 (WI 6).** Each line records *how* it is met rather than only that it is, because the
phase's own rule is that a claim without a measurement is the thing this document exists to avoid.

- [x] `tests/gui/boundary.test.ts` passes with an **empty** allowlist; the renderer contains no resolver,
      randomizer, pipeline, curl parser or report renderer. — **`{}` since WI 6**, and the file's last test now
      verifies all eight authorities, which is the first time no row was `planned:`.
- [x] `packages/gui/src/lib/resolveVars.ts`, `packages/gui/src/lib/randomizer.ts` and
      `packages/gui/src/lib/curlParser.ts` are **deleted**, not emptied. — **Two of three, and the third is a
      correction rather than a shortfall.** `resolveVars.ts` and `curlParser.ts` are deleted.
      `lib/randomizer.ts` **survives with its token catalogue**, which is D28's explicit decision: the UI keeps
      a *pinned copy of the list* for `RandomizerHint`'s autocomplete while the generator is the engine's. The
      criterion was written before D28 existed; the construct the ratchet tracked is gone, which is what the
      criterion was for.
- [x] One implementation of each: resolution (including the global/active merge), randomizers, the request
      pipeline, the report renderer. — The merge reached `utils/resolve.ts` in WI 3 and is now the only one:
      `App.tsx`'s copy is gone and `env.getMerged` reads the engine's.
- [x] A saved tab's Send resolves `{{host}}` and runs its pre-script **with the renderer holding no resolver
      and no script runner** — verified by the boundary ratchet plus a real end-to-end case. — WI 3;
      `tests/integration/protocolExecution.integration.test.ts`.
- [x] `savedEntitySend.ts` reduces to `!isDraft && !hasLocalChanges`, with its docblock no longer explaining
      resolvers. — WI 3's completion note.
- [x] The collection runner runs with the UI closed (or the client disconnected) and the engine reports its
      progress; the UI renders it from events. — WI 4.
- [x] A run survives a client disconnect, and the run's state is reachable afterwards (`runner.status`). —
      WI 4, and `tests/runner/runRegistry.test.ts` drives it.
- [x] WI 7's contract exists, and a new protocol module can be added without touching `packages/gui/src`
      beyond its tab's rendering. — **WI 7, and WI 6 is the first module to arrive under it.** The contract's
      §4 checklist is what made the socket's four decisions explicit rather than discovered, and
      `tests/gui/protocolModule.test.ts` is now `{}` — the renderer contains none of the seven transport
      constructions.

---

## 10. Verification

- **Per item:** the boundary ratchet's allowlist shrinking is the primary signal — it is a *structural*
  assertion, not a count. Judge each deletion by the allowlist entry being removed in the same commit.
- **Deltas:** re-measure the suite rather than carrying a figure; unit + integration = the full total.
- **The end-to-end case that matters** is the one the owner described: a **saved** request whose url contains
  `{{host}}` and which has a pre-script, sent from a tab with no local changes, must resolve and run the
  script — with `resolveVars` absent from the renderer. Today that case takes the *payload* path precisely
  because the gate refuses the id path; after WI 3 it takes the id path and behaves identically.
- **Traps 1 and 2** get a test each, because both are invisible in a suite that uses no shadowed variable.
- `npm run dev` / `test:e2e` remain the owner's to run (an agent-shell failure is evidence about the shell).

---

## 11. Status

> **⚠ Read this section last-to-first.** It is a running log, so the *newest* entry is at the bottom and the
> paragraph below is the state as of 2026-09-29. **Current state: P19 is COMPLETE — all seven work items
> landed, WI 6 on 2026-09-30, and both ratchets are `{}`.** Protocol 107 commands / 13 events; client surface
> 154. The one item deliberately left open is `packages/gui/src/lib/blocks.ts` (see the WI 6 entry).

**WI 1 landed. WI 2's engine half landed. WI 3's engine half landed.** Written 2026-09-29, immediately
after the two unplanned changes in §0 landed.

**WI 1 — the boundary ratchet — is `tests/gui/boundary.test.ts`, and its allowlist is seeded from a
measurement, not from §6's predictions.** Nine entries in nine files, against §6's predicted
`−2 / −1 / −1 / −2` (six). §6 counts *files deleted*; the suite counts *constructs present* —
`collectionRunner.ts` carries the pipeline, the run loop and the report renderer's only caller, and
`scriptRunner.ts` / `testRunner.ts` are not in §1's table at all. **Measured split: WI 2 → 2, WI 3 → 4,
WI 4 → 1, WI 5 → 2.** Teeth verified by three mutations — a planted renderer resolver, a renamed engine
authority, and a renamed renderer construct each fail the assertion that owns them. Written up in
`TESTING.md` §3.7.

**WI 2's engine half is landed, and so is D28's catalogue half.** `packages/engine/src/utils/resolve.ts`
holds `resolveVars` and `resolveHeaders`; `mocks/mockHandler.ts` imports them instead of carrying the
engine's private copy; and the cases that pinned them moved with them to `tests/proxy/resolve.test.ts`. The
engine's `utils/randomizer.ts` now holds the **authority** `RANDOMIZER_TOKENS`, with
`tests/gui/randomizerCatalogue.test.ts` pinning the renderer's copy to it. The engine half is landed now
precisely because it is the part that is **not** gated — see finding 1.

**WI 3 is landed for REST and GraphQL — two of four protocols, both halves each.** `packages/engine/src/pipeline/runPipeline.ts` is D24's one
pipeline, with `send` injected so the file knows no protocol; `workspace/envState.ts` supplies the merged
environment the engine is now the authority for; `utils/resolve.ts` gained `mergeEnvVars`;
`tests/pipeline/runPipeline.test.ts` (15 cases) asserts §10's two named traps. Then on 2026-09-30 the wire
and the tab followed: `request.send` replaced `request.replaySaved`, and `useRestActions` stopped resolving
and orchestrating. **The allowlist went 9 → 8, and that is the structural evidence** — the entry that
disappeared is `useRestActions.ts#pipeline`, and the staleness assertion is what made it impossible to keep.
§6's WI 3 section carries the corrected deletion schedule: §6's own "Allowlist −1" was wrong in both
directions, because the two step wrappers *outlive* this item — their last caller is `collectionRunner`.

**GraphQL followed, and it is the first protocol whose migration is a FIX rather than a move.**
`graphql.send` replaced `graphql.executeSaved`; the tab stopped resolving and **its `activeEnv` prop was
deleted**, which is that migration's visible evidence. ⚠ **The tab resolved fewer fields than the gate's token
scan treated as resolvable** — `endpointUrl`, `headers` and `variables`, but not `query` or `operationName` —
so a `{{token}}` in a GraphQL query never resolved in **either** path. The engine resolves all five, and a new
integration case asserts each one arrived at the upstream resolved. **SOAP followed the same day, with the same shape and the same class of fix.** `soap.send` replaced
`soap.executeSaved`, and the tab's `activeEnv` prop went with it. Its gap was narrower — the tab resolved
`endpointUrl`, `headers` and `body` but not **`soapAction`** — and SOAP is the one protocol where the stored
shape and the command's params are *identical*, so there is no rename for the engine to perform.

**gRPC closed the set, and with it the family.** `grpc.send` replaced `grpc.executeSaved`; its gap was
`serviceName` / `methodName`; and `protoFileId` is deliberately **not** resolved, because it names a stored
file rather than template text and the gate never scanned it. **`workspace/entities/savedRequestCommands.ts` is
deleted** — the module that held four load-and-dispatch bodies, whose *"one module, not four"* argument had
narrowed three times and then became empty. **`savedEntitySend.ts` reached §7's end state**: `!isDraft &&
!hasLocalChanges`, with `hasExecutableScripts` gone since REST and `usesVariables` gone with gRPC, and
`containsVariableToken` deleted alongside it.

**⚠ gRPC's resolution is the one thing in this item that is NOT observable, and the gap is recorded rather
than papered over.** `grpc.execute` is a `wontfix` stub that ignores every parameter it is handed, so there is
no upstream at which a resolved field could be seen; replacing it with an echo needs a monkey-patched registry,
because `CommandRegistry.register()` throws on a duplicate. The `NOT_FOUND` case is what proves the load half,
and an integration case for the resolution should be added with the stub. Recorded in `grpc/commands.ts`'s
header and in the `grpc.send` entry.

**⚠ The allowlist did not reach `{}`, and the two entries left are not WI 3's** — see §6's completion note:
`resolveVars` and the randomizer outlive the four tabs, because `useWebSocket.ts` (WI 6) and
`HealthBarPanel.tsx` still resolve. They belong to WI 6.

**WI 4's engine half is landed** — `packages/engine/src/runner/runCollection.ts` and `runRegistry.ts`, with
**D29** and **D30** decided by the owner rather than discovered, which is exactly what §8 traps 5 and 6 ask for.
The loop is thin *because* WI 3 exists: one `registry.invoke("request.send", {workspaceId, id})` per request,
with the runner owning only ordering, the delay, cancellation and the report.
`tests/runner/runRegistry.test.ts` (12 cases) drives both directly, including the three that make the decisions
checkable rather than asserted — `startRun` returns before the loop finishes, `cancelRun` is the **only** lever
that stops a run, and D30 holds in both directions (same folder refused, different folders concurrent, a
finished run not blocking a new one).

**The wire followed the same day** (`plan/protocol-changes.md`'s 2026-09-30 entry): `runner.start` /
`runner.cancel` / `runner.status`, and `event.runner.progress` / `event.runner.finished`. Protocol **93 → 96**
commands and **9 → 11** events; the client surface **136 → 141**. Two details are worth keeping: **`status` is
scoped `read` while `start` and `cancel` are `execute`**, which is what lets a reconnecting client ask what
happened without being granted the right to start or stop anything; and **`total` rides on every progress
event**, because a client attaching mid-run never saw the start call. The conformance suite's every-command and
every-event matrices widened **automatically** — both are derived from `COMMANDS` / `EVENT_NAMES`, so a new
name cannot be left out of them, which is the property that makes this wire cheap.

**The renderer half landed too, and WI 4 is complete.** `useCollectionRunner` is a subscription —
`runnerStatus` on mount so a tab opened mid-run attaches (D29 in the UI), `runnerStart` / `runnerCancel` for
the two buttons, and the two events for everything else — and the four deletions it owed are gone:
`collectionRunner.ts`'s loop, `scriptRunner.ts`, `testRunner.ts`, and `runnerReport.ts`'s HTML renderer. **The
allowlist went 8 → 4.** `runnerReport.ts` survives with its two *storage* wrappers, which is the
`file#construct` keying working as designed.

**⚠ A flaw the tests caught, and it was in the hook rather than in the test:** the first version only
*updated* a snapshot it already had, so a `runner.finished` event arriving with none was dropped — exactly the
disconnected-client case D29 exists for. Both handlers now establish state when there is none.

**The engine half was safe to land first, unlike WI 3's**, because two run loops do not interfere the way two
resolvers do — nothing routes to the renderer's loop but the UI's own button, so there was no window in which a
request is processed twice.

**`hasExecutableScripts` left the Send gate, and that is §7's accounting coming true early.** It was `true`
for the REST tab alone and `false` for the other three, so it was never a per-request question; REST's
migration made it `false` for all four, at which point it was a constant inside a gate whose whole job is to
prevent a silently wrong send. `usesVariables` stays until the last of the remaining three tabs migrates, and
`tests/renderer/savedEntitySend.test.ts` now carries a case guarding the deleted term's return.

**Two silent exits from the engine-layout rule were closed across this work, and they matter more than the
module that found them.** (1) `contractViolations()` returns `false` for a module with no `CONTRACT` row, so
**creating a directory was the one way to escape the layering rule without failing anything**. (2)
`ORPHANS_ALLOWED` was never checked for staleness, so the `pipeline` entry could have outlived its own
*"delete this when the `*.send` command arrives"* instruction indefinitely. Both are closed, and both are
recorded in `TESTING.md` §3.6 (seven assertions, was five).

**The three remaining protocols are next, and their order is free** — each is its own commit (§8 trap 1).
The work per protocol is now mechanical: rename its `*Saved` to `*.send` with the union payload, give it a
`send` closure in `pipeline/commands.ts`, rewire its tab, then delete that tab's resolver call. The resolver
and randomizer allowlist entries go with the **last** of the three.

### WI 3, second protocol: GraphQL — LANDED 2026-09-30

`graphql.executeSaved` → **`graphql.send`**, a union of `{workspaceId, id}` and `{workspaceId, request}` where
`request` is the **stored entity's shape** (`endpointUrl`, not `url`), so the two forms stay interchangeable
and the rename stays the engine's business. **No `scripts` member** — the sandbox's context is a
`{method, url, headers, body}` request and a GraphQL request is not one, so a `scripts` object nothing could
run would be a field that lies. Command count stays **93**.

**It landed in `graphql/commands.ts`, not in `pipeline/commands.ts`**, and that is §5.1 rather than taste: it
resolves and dispatches, so it reads `workspace/` + `utils/` and `kernel/` — `graphql/`'s §6 row exactly. It
does **not** call `runRequestPipeline`, because that function's `PipelineRequest` view exists for the three
script steps and this protocol has none; fabricating one would be inventing a shape to satisfy a signature.
The pattern for the remaining two is therefore: **their own feature module, direct resolve + dispatch.**

**⚠ The migration is a FIX, not a move, and the gate is what documented it.** The tab resolved `endpointUrl`,
`headers` and `variables` but **not** `query` or `operationName`, while the gate's token scan treated all
five as resolvable — so a `{{token}}` in a GraphQL query never resolved in *either* path, since the gate only
ever chose between two ways of sending the same unresolved field. **The engine resolves all five**, which is
the only form under which `usesVariables` can leave the gate at all. `tests/integration/protocolExecution.integration.test.ts`
gained a case asserting exactly that, field by field.

**The tab's `activeEnv` prop was deleted**, which is this migration's visible evidence: it was threaded from
the panel down to `handleSend`, and `handleSend` no longer resolves. An unused prop reads as a dependency the
component still has, so removing it was part of the change rather than a follow-up.

**The allowlist did not move** — `graphql`'s `resolveVars` calls were *uses*, not definitions, so the ratchet
never had an entry for them. `usesVariables` stays in the gate for SOAP and gRPC.

### Three findings that change this document

**1. WI 2's renderer half cannot land before WI 3, so WI 2 + WI 3 are one atomic change.** WI 2 says
"delete `gui/lib/resolveVars.ts` and `gui/lib/randomizer.ts`". It cannot. The four tabs, the runner and
`useWebSocket` all resolve before sending, and **the renderer has no `@bifurc/engine` dependency by
design** — `packages/gui/src/types/config.ts` and `types/ipc.ts` mirror engine types *by hand*, and
`hooks/useAppConfigSync.ts` says why in a comment ("this is the browser bundle, and `@bifurc/engine` is not
its dependency"). So the renderer cannot import the engine's resolver, and it cannot stop resolving until
the engine resolves for it — which is WI 3's pipeline. Sending unresolved fields to `request.replay` is
forbidden by §5's constraint that the raw primitive stays unresolved. **Consequence: the two WI-2
allowlist entries cannot be deleted until WI 3 lands.**

**2. The merge moves with the CALL, not with the function — a refinement of §8 trap 2.** Trap 2 is right
that the global+active merge must travel with resolution. But the merge answers *which* environment we
resolve against, and until WI 3 the engine is not the resolving party. Landing `mergeEnvVars` beside
`resolveVars` now would create a **second** merge implementation — `App.tsx:288`'s is still authoritative
and still runs on every render — while the engine's copy had no caller. That is trap 1's duplication
introduced rather than removed. So it lands in the same change that makes the engine resolve; the
ratchet's `resolver` construct keeps `mergeEnvVars` in its `detect` regex, so the entry is already covered
when it arrives.

**3. The catalogue split — RESOLVED by D28, and both halves landed.** The hazard was real: the renderer
cannot import the engine, so "expose the catalogue separately" had to be either a wire command (which this
document did not have) or a hand-mirror. The owner chose **the pinned copy** — the engine holds the
authority, the UI keeps its copy for autocomplete, and `tests/gui/randomizerCatalogue.test.ts` pins them
equal, including that every catalogued key has a matching `case` in the generator. The boundary ratchet's
`randomizer` construct was narrowed to the **generator**, since flagging the catalogue would contradict
D28. **The test's docblock records the one direction it cannot catch** — a generator `case` with no
catalogue entry, because nothing can enumerate a `switch` — and that gap was verified by mutation rather
than assumed.

### WI 5, first slice: the cURL parser — LANDED 2026-09-30

`request.parseCurl` is on the wire (protocol **96 → 97** commands), the parser is
`packages/engine/src/utils/curl.ts`, and `packages/gui/src/lib/curlParser.ts` is **deleted**. **The allowlist
went 4 → 3** — the sixth retirement overall, and the first since WI 4's four.

**Two decisions in it are worth carrying forward.** (1) The command returns the parse **unfiltered**, because
`SKIP_CURL_HEADERS` is the *caller's* policy — the tab drops hop-by-hop headers on import, the importer drops
them inside its own pipeline — so a parser that filtered would be choosing for both. (2) That leaves the
renderer needing the list for one read-side job the engine cannot do for it (filtering a **stored** mock's
`capturedHeaders` on load), so the renderer keeps a **pinned copy** — D28's list-vs-logic split applied a
second time — and the ratchet's `curlParser` `detect` was **narrowed to the parser**. A narrowing is the one
edit to that suite that loosens it, so it ships *with* the pin (`tests/gui/curlHeaderSkip.test.ts`), not
instead of it; `TESTING.md` §3.7 records the pairing rule.

**⚠ The duplication was larger than §1's table said.** That table named *two* cURL parsers — the renderer's
and `importexport/importers/requests-curl.ts`'s. The importer's was a near-verbatim copy of the renderer's,
and its own comment read *"duplicated from packages/gui/src/lib/curlParser.ts for main process use"* — so one
file existed three times across two packages. Both copies are gone, and the importer reaches the shared
parser through `parseCurlCommand()`. **The importer gained two behaviours it never had** (`-b` / `--cookie`
and `--data-ascii`), because the renderer's parser was the more complete of the two and became the authority;
that is a superset, recorded rather than smuggled.

**⚠ The handover note's "WI 5 → Allowlist −3" is a prediction error, and §6's own "−2" was right.** The four
entries are `resolveVars.ts#resolver`, `randomizer.ts#randomizer`, `curlParser.ts#curlParser` and
`utils.ts#pathHelpers`; WI 5 can retire the last two only. The resolver and the randomizer belong to **WI 6**,
and moving `HealthBarPanel`'s resolution (WI 5 item 4) does not change that — `useWebSocket.ts` remains the
renderer's last resolver *user*, and the ratchet keys on the **definition**. Measured WI 5: **−1 so far**,
with a further −1 when the path helpers go.

**WI 5's remaining sub-items, and body-mode is not one of them.** Capture→entity construction, the
`utils.ts` path helpers, and `HealthBarPanel`'s resolution are outstanding. **Body-mode sniffing needs no
change at all** — §6 asked for the content type to come from the engine's response, and WI 3's migration
already did it (`useRestActions` reads `res.headers["content-type"]`; `APPLY_CURL` reads the headers the
engine's parser returned). Recorded as a measurement rather than left as an open box.

### WI 7: the protocol-module contract — LANDED 2026-09-30

`Protocol_Module_Contract.md` (repo root) plus `tests/gui/protocolModule.test.ts`. **No production code
changed** — this item adds a rule and the guard that keeps it, which is why it could land before WI 6 rather
than after it.

**⚠ The one design decision worth recording: it is a second ratchet file, not a seventh construct.** The
obvious cheap move was to add a `transport` construct to `tests/gui/boundary.test.ts`. It is the wrong one.
That file's allowlist holds engine **logic** and empties when the last renderer resolver goes; this one holds
engine **I/O** and empties when the last tab stops opening its own connection. One list with two retirement
schedules makes the staleness assertion unreadable, because an entry's owner stops being a single work item —
and §9's whole point is that each entry names *the* work item that deletes it.

**The allowlist opens at one entry — `hooks/useWebSocket.ts#websocket`, owned by WI 6 — and the engine-side
landing point already exists**: `packages/engine/src/websocket/` is reserved in §9 and `tests/engine/layout
.test.ts` already carries its `CONTRACT` row (`kernel`, `utils`, `workspace`), so WI 6 starts under the
layering rule rather than outside it. **The goal is `{}`.**

**⚠ The suite's self-check is worth copying to any name-based scan.** Its staleness assertion passes only if
the scan found a live violation, so a `detect` pattern that silently matched nothing — the failure mode a
regex-based ratchet is most prone to — fails the run instead of passing vacuously. That is a stronger
guarantee than the boundary ratchet's separate "the tree is non-empty" guard, and it costs nothing.

**What it does not catch:** a transport reached through an alias, or a socket dependency added to
`packages/gui/package.json`. The contract's §6 records the dependency assertion as the cheap strengthening to
add with the first module that needs one.

### WI 5, item 4: `HealthBarPanel`'s resolution — LANDED 2026-09-30

`env.resolveText { workspaceId, text }` → `{ text }` (protocol **97 → 98**; client surface **142 → 143**), and
the panel no longer imports `@/lib/resolveVars`. The engine resolves against the **merged** environment
(`loadMergedEnvironment`, D23) and there is deliberately no way to ask for the active-only behaviour — that
option is what made the UI the authority for a workspace fact the engine already owns.

**⚠ The allowlist is unchanged at three entries, and that is the correct outcome rather than a shortfall.** The
`lib/resolveVars.ts#resolver` entry is keyed on the resolver's **definition**, and `useWebSocket.ts` still
imports it. WI 5's −2 is `curlParser` + `pathHelpers`; the resolver and the randomizer are WI 6's, and item 4
does not move that number. Recorded because a reader who expects a shrink here would otherwise read the flat
count as a missed deletion.

**⚠ The design fork, because the obvious answer is wrong.** The cheap version is to let `healthbar.checkUrl`
return the url it resolved and have the card display that. It fails on its own terms: the card shows the
resolved url **before** the first check, and a service with auto-refresh off may never be checked at all — so
the card would show `{{host}}/health` indefinitely. A read-only command that resolves on demand is the shape
that matches what the UI is actually asking.

**⚠ And it is not simply "a resolver, for the UI".** `{{random.*}}` mints a fresh value per call, so
`env.resolveText` is **idempotent but not deterministic**. That is safe here only because the caller *displays*
the result; a caller that sent a resolved value would need the pipeline's engine-side resolution, where
resolution and the send happen once, together. `packages/client/src/retry.ts` records both halves — otherwise
"idempotent" reads as "same answer twice", which it is not.

**⚠ A stale-closure trap, and the ref that exists for it.** `checkService` is reached from
`loadServices().then(...)`, whose effect deps are `[wsId]` — so it never re-reads `resolvedUrls` **state** and
would have probed the raw url on every auto-refresh check. The panel keeps the map in a **ref** for the checks
and in state for the render. Any future engine-resolved display value has the same hazard: **the async value
and the closure that consumes it do not arrive together.**

### WI 5, item 2: capture → entity construction — LANDED 2026-09-30

`capture.buildRequest` / `capture.buildMock` (protocol **98 → 100**; surface **143 → 145**), the construction
in `packages/engine/src/utils/captureEntities.ts`, and `captureUtils.ts` reduced to its formatting half.
**Allowlist unchanged at three** — neither function was one of §1's constructs, so this item's contribution to
the phase is *one implementation* rather than a ratchet move. Recorded so the flat count is not misread.

**⚠ Three findings, none of which the plan's one-line scope predicted.**

1. **The hop-by-hop header list was a *third* copy.** `captureUtils.ts` carried its own `SKIP_HEADERS`,
   character-for-character identical to `SKIP_CURL_HEADERS`. It is deleted and imported — so this item removed a
   duplicate it was not looking for — and that exposed a **naming** problem: the list is not curl-specific, and
   `utils/curl.ts` is now the wrong home for it. Consolidating it into `utils/hopByHop.ts` is the follow-up,
   recorded rather than done because the rename also touches the renderer's pinned copy and its pin test.
2. **A content-type predicate had to move with it, and that is a third pinned renderer copy.**
   `buildMockInitial` decided `responseBodyEncoding` through `isBinaryContentType`, so the engine needed the
   predicate; the renderer still needs it for presentation, and `@bifurc/gui` cannot import the engine. So
   `tests/gui/binaryContentType.test.ts` pins the copy — **the D28 list-vs-logic split a third time**, after
   `RANDOMIZER_TOKENS` and `SKIP_CURL_HEADERS`. It compares *behaviour on a sample* rather than two arrays
   (the engine's list is substring prefixes), and the sample covers both directions because the failure is
   asymmetric: "engine says binary" decodes a PNG as UTF-8 and corrupts it on save.
3. **The base64 decode is a behaviour *port*, not a copy-paste.** The renderer used `atob`, which throws, and
   returned the *raw input* on failure. `Buffer.from(x, "base64")` never throws — it silently skips offending
   characters — so a straight port changes what an odd capture becomes. The explicit validity check lives in
   `captureEntities.ts` and is pinned by `tests/engine/captureEntities.test.ts`, including a whitespace-wrapped
   case, because `atob` tolerates ASCII whitespace and a naive strict check would not. **This is the class of
   thing "a move, not a rewrite" hides: the two libraries disagree about *failure*.**

**⚠ And one behaviour that deliberately did *not* move.** `mockEntries` stores only the response side plus
method/url; it has never persisted `capturedHeaders` / `capturedBody` / the `*Mocked` flags. Spreading
`capture.buildMock`'s whole body would have started persisting the captured request silently. The caller's
field picking is unchanged, and its docblock says why.

**⚠ Found while doing this and *not* fixed: `packages/gui/src/lib/blocks.ts`.** It holds `buildBlockMock`,
`blockKey`, `foundBlocksFolder` and `ensureBlocksFolderId` — the same class of thing this item moved (construct
an entity from something the user did), still in the renderer, and absent from §1's table so no ratchet sees it.
Recorded as the next candidate for this item's treatment.

### WI 5, item 3: the path helpers — ENGINE HALF landed 2026-09-30, renderer half NOT

`workspace/entityPaths.ts` builds `entity id → relPath` and `folder id → directory prefix` by walking the
**kind registry** (`forEachCrudKind()`), and `sync.getEntityStatus` returns them as `EntityStatusPayload.paths`.
The gui has the type, the context (`lib/entityPaths.tsx`) and `entityPaths` on `useAppConfigSync`'s return.
**No protocol change** — the result type is hand-mirrored, so nothing was added to `COMMANDS`.

**⚠ The carrier was chosen by a safety check, not by convenience.** `config.save` passes its payload straight
to `saveConfig()`, so a `relPath` field on a `config.get` entity would be **written to disk and read back
stale**. A read-only *result* is the only place a computed field can live — the inverse of `entityStatus.ts`'s
"the payloads need no translation" note.

**⚠ The registry *is* the table.** `forEachCrudKind()` reads `configKey` / `folderConfigKey` / `isFlat` off the
same registration the CRUD commands dispatch through, so a new kind is in the maps for free. A hand-kept list
would have been this item's own defect one layer down — and `tests/engine/entityPaths.test.ts` pins that
property directly: it adds `proxyRules` to the fixture and expects `rules/rule1.json`.

**⚠ The renderer half stopped on a design fact: the provider cannot wrap `App`.** `useEntityGitActions` is a
hook called *inside* `App`, so a `useEntityPaths()` there would read the **default** context — `App` is not
inside its own provider. The honest fixes are **(a)** split `App` into an outer component owning
`useAppConfigSync` and an inner one consuming the context, or **(b)** pass the lookup into
`useEntityGitActions` as an argument. (a) is a real refactor of a ~500-line component; (b) is smaller but
leaves two mechanisms for one fact. **That is why nothing was rushed: the allowlist entry and the three
helpers are still present, and the engine half is additive and inert.** The next pass starts from here.

**⚠ And a wrong model a test caught, worth keeping.** The maps' first test ran against a real temp data root
and real `saveConfig`, and it **failed**: **`loadConfig()` does not read entities from the config JSON** — it
rebuilds them from the entity **files** (`readAllEntities` / `readEntityStubs` + the `names.json` index). So
`saveConfig(cfg)` does **not** round-trip entities, which no signature says. The test moved to a unit test with
`loadConfig` mocked and the **real registry**, pinning the composition instead of re-driving storage.

**The renderer half landed the next turn, and it did *not* need the App split after all.** The previous
pass stopped because `useEntityGitActions` is a hook that runs in `App`'s own body — above any provider `App`
renders — so a `useEntityPaths()` inside it would read the **default** context. The fix that avoided the
refactor: **`App` builds the lookup with `entityPathLookup(entityPaths)` and passes it in as an argument**,
while the provider serves everything below it. Two mechanisms for one fact, deliberately — the alternative
was splitting a ~500-line component for one lookup.

**⚠ Two details from that pass worth keeping.**

1. **The provider wrap costs a one-line diff.** The tree is assigned to `const tree = (…)` and returned
   wrapped, rather than inlining `<EntityPathsProvider>` around it — which would have re-indented ~120 lines
   for one line of plumbing.
2. **Two consumers are *separate components*, so each needs its own hook call.** `EnvItem` (inside
   `EnvironmentsPanel.tsx`) renders the History button while the panel renders the cards; a hook placed only
   in the panel leaves the button reading the default lookup. **Check component boundaries, not files** —
   and note the failure is silent: `""` is a legal answer meaning "unknown".

**⚠ And the hook's path function lost two things and kept a third.** The **kind recovery** is gone because
the map is keyed by **id** — ids are unique — so the multiplexed panel's "which kind is this?" stops
mattering to the path at all. The **fresh `getConfig()` read** became a **one-shot `getEntitySyncStatus`
re-ask**, because the bug it fixed was real: publishing a just-created mock from the tree silently no-op'd,
and the map is a render-time snapshot too.

**⚠ Three test files moved with it, and two of them for the *provider*.** `entityGitActions.test.tsx` gained
a `mount()` that supplies the map — with a documented kind→array table that is the **test's fixture**, not a
third implementation, because the engine's own copy is pinned by `tests/engine/entityPaths.test.ts`. Both
`mappingsPublish*.test.tsx` now render `MappingsPanel` inside `EntityPathsProvider`: the panel requires one,
and a test that renders it must supply one. **19 failures → 0**, and the allowlist went **3 → 2** — WI 5 is
complete, leaving only WI 6's two entries.

### WI 6: WebSocket to the engine — LANDED 2026-09-30, and **P19 is complete**

`packages/engine/src/websocket/` is real: `wsRegistry.ts` (the sockets and the four
`Protocol_Module_Contract.md` §4 decisions) and `wsCommands.ts` (six `ws.*` commands and the bus binding),
replacing the reserved `index.ts` placeholder that `Engine_Module_Plan.md` §9 had said not to fill with
invented content. It was not filled with invented content — it was filled with the content that existed in the
wrong process. Protocol **100 → 107** commands and **11 → 13** events; the client surface **145 → 154**, its
largest single step, because `ws.*` has **no legacy channel** to rename: the renderer's socket spoke to the
browser's `WebSocket` constructor rather than to the shell.

**The engine half is `wsRegistry.ts`, and the shape of it is `runRegistry.ts`'s one namespace over.** A
module-level map, no persistence, no teardown hook, and `disconnectSocket()` as the only lever — D29 stated as
code, with the paragraph in the file explaining why a reader looking for where a disconnect is handled will not
find one. The state **outlives the socket** (`ws.status` answers for a closed one, which is what makes
re-attaching work), and a reconnect **replaces** the entry rather than adding one.

**⚠ The concurrency policy is deliberately NOT D30, and the difference is the kind of thing each command
makes.** D30 refused a second run of the same folder because a run is a *unit of work* and a duplicate is
effort nobody asked for twice. A socket is a *channel*, so a second connect on a live key is a **reconnect** —
the renderer did exactly that ("close any existing connection for this tab") — and refusing would make a broken
socket unrecoverable without a separate disconnect first. The replacement is atomic in the sense that matters:
the old socket is marked superseded *before* it is closed, so its close event cannot emit a `disconnected` that
arrives after the new `connecting`. That is the registry's `generation` counter, and the bug it prevents is an
intermittently wrong-coloured dot rather than an error message.

**⚠ D29 in the UI, and it is the one user-visible behaviour change.** `useWebSocket` has **no
disconnect-on-unmount**: an unmount cannot tell "the user closed this tab" from "the window is going away", and
inferring a close from either is exactly what D29 forbids. `WebSocketsPanel` issues the explicit `wsDisconnect`
from the tab-close handler, which **is** a gesture. The consequence, recorded rather than hidden: a tab that
disappears without being closed leaves its socket running — which is the point — and reopening that tab
re-attaches and shows the retained transcript. The limit banner reports the engine's live count, so those
sockets are visible rather than mysterious.

**⚠ Two improvements rather than ports, both unreachable before.** The browser `WebSocket` constructor
**cannot send headers at all** — the renderer's own comment said the resolved headers were "stored, not
injected" — so a `{{TOKEN}}` in an Authorization header went from *resolved and discarded* to *resolved and
sent*. And binary frames are now read from the client's `isBinary` rather than inferred from
`typeof ev.data === "string"`, which was a proxy for the frame's opcode rather than a reading of it.

**⚠ One bug the tests caught, and it was in the registry rather than in a test.** A `ws.disconnect` issued
while the socket is still CONNECTING aborts the handshake, and `ws` reports that as `close(1006)` — abnormal.
Read by close code alone, a user pressing Disconnect mid-connect would have been shown *"Connection closed
(code 1006)"*. So `closeRequested`, which the `error` handler already consulted, is consulted by the `close`
handler too: **a close the caller asked for is clean whatever code the client reports**, because the code
describes how the *handshake* ended and the flag describes who ended it. The case that found it is
`tests/websocket/wsRegistry.test.ts`'s, and it needed a *fake* socket to exist at all — a real client emits
`error` and `close` glued together, so the flag cannot be shown to matter against one.

**The two ratchets both reached `{}`, and emptying them cost something that had to be replaced.**

- `tests/gui/protocolModule.test.ts` **1 → 0**. The entry was `hooks/useWebSocket.ts#websocket`; the renderer
  now contains none of the seven transport patterns.
  **⚠ Emptying the allowlist removed this file's vacuity guard.** While it held one entry, its staleness
  assertion *also* proved the scanner worked — it passed only if the scan found that live violation — which is
  a property the boundary ratchet does not have and which the file's own docblock advertised. With `{}` there
  is no entry whose liveness depends on a hit, so a `detect` regex that silently stopped matching would leave
  every assertion green while measuring nothing. The guard is now explicit and stronger: every pattern is run
  against a sample that must match **and one that must not**, and the control table is checked in both
  directions against `CONSTRUCTS`, so adding a construct without a control fails too.
- `tests/gui/boundary.test.ts` **2 → 0**, and the two were §6's mis-attribution to WI 3 rather than WI 6's
  oversight — see §6's WI 3 completion note. `lib/resolveVars.ts` is **deleted**; `lib/randomizer.ts` survives
  with its catalogue only, which is the `file#construct` keying working as designed (the WI 4
  `runnerReport.ts` outcome). The file's last test now verifies all eight authorities with no `planned:` row
  left, which is the first time that has been true.

**⚠ A factual error in `protocolModule.test.ts`'s docblock, found by this change and corrected.** It claimed
*"a comment mentioning `new WebSocket` in a design note is not a violation"*. The scan reads the file as
**text**, so a comment that quotes a construction **is** one — and it bit immediately: the new
`useWebSocket.ts` docblock described what it used to do by writing the constructor out, and the suite failed on
its own explanation. The docblock was reworded rather than the scanner loosened, because **a false positive is
safe and a false negative is not**: a scan that errs toward flagging prose is better than one that strips
comments and can therefore hide a real construction on the same line as a `//` inside a string literal. The
`strings.sockets.createSocket` half of the claim *was* accurate — the pattern needs the `dgram.` qualifier —
which is why the claim read as verified when half of it was not.

**`env.getMerged` came with this item, and it is the last renderer implementation of a workspace fact.**
`App.tsx` merged `__global__` with the active environment on every render and threaded the result down as
`activeEnv` for `EnvVarHint`'s autocomplete. The merge is D23's — the engine owns the `environments` kind, the
`activeEnvironmentId` pointer and the synthetic `__global__` — so it is now a read. **This is the D28
list-vs-logic split rather than a new rule**: the UI may hold the *list* it offers the user; the *resolver* is
never the UI's. Which is exactly why `lib/resolveVars.ts` could be deleted outright rather than trimmed, and
why the catalogue in `lib/randomizer.ts` stays.

**⚠ And it introduced the first *render-time* `window.api` call in `App`, which broke `App.test.tsx` — a
regression this change caused and then fixed, recorded because the class of it will recur.** Every other
`window.api` call in `App` is inside an event handler, where the bridge is present by construction. A mount
effect is reached by a *unit render* with a partial bridge stub, and `App.test.tsx` mounts `App` with exactly
that — so the unguarded call threw `envGetMerged is not a function` out of a passive effect and took the tree
with it. The read is now guarded (`window.api?.envGetMerged?.(…)`), which is honest rather than defensive: the
value is a **hint's** data, and without it `EnvVarHint` renders nothing — the same thing it renders for a
workspace with no environment. **Any future render-time read added to `App` inherits this hazard**, and the
symptom is a `TypeError` in a test that has nothing to do with the feature.

**⚠ `blocks.ts` is still open, and it is the one item this phase leaves on the table.**
`packages/gui/src/lib/blocks.ts` holds `buildBlockMock` / `blockKey` / `foundBlocksFolder` /
`ensureBlocksFolderId` — the same class of construction as WI 5 item 2 (build an entity from something the user
did), still in the renderer, and **absent from §1's table so no ratchet sees it**. It was found while doing
WI 5's capture work and is unchanged by WI 6; the reason it is not simply added to the boundary ratchet is that
its construct would be a *fourth* entry in a file whose whole point is that entries name the work item that
deletes them — and there is no such work item yet. It belongs to a follow-up item of its own, which is where it
should be recorded rather than smuggled into WI 6's allowlist.

**Verified.** `tests/websocket/wsRegistry.test.ts` (32 cases) drives the registry directly, including the
three that make §4's decisions checkable rather than asserted — the snapshot outliving the socket, a superseded
socket's late close not overwriting the new state, and a reconnect not being refused by the limit it is already
part of. `tests/renderer/websocket.test.ts` (12 cases) covers the hook as a mirror, including **the hydration
race** (a frame arriving before `wsStatus` answers) and **that an unmount does not disconnect**.
`tests/engine/randomizer.test.ts` is the renderer's old generator suite, moved with the function; the cases
`tests/renderer/resolveVars.test.ts` had that the engine's suite did not — partial-key matching, case
sensitivity, an empty value, no input mutation — moved into `tests/proxy/resolve.test.ts`, which also gained
`mergeEnvVars`'s shadowed-key case.
