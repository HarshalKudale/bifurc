# The Protocol Module Contract

**What this is.** The checklist a **new protocol module** must satisfy to land the right way round, plus the
one automated guard that keeps it that way (`tests/gui/protocolModule.test.ts`).

**Who it binds.** Every protocol the app speaks: REST, GraphQL, SOAP, gRPC, WebSocket — and **MQTT, which is
the module this document exists for**. It is written now, *after* `plan/19`'s four migrations and *before*
MQTT, because those migrations are the evidence for every rule below: each one is something the codebase
actually got wrong once, not a style preference.

**Read this before adding a protocol.** The failure mode it prevents is not "MQTT is slow to write" — it is
"MQTT is written in the renderer, and the next five phases inherit it".

---

## 1. The rule, in one sentence

> **The tab renders what the engine did and sends what the user typed. Every byte that reaches the wire is
> the engine's.**

The test for any line of a module's tab, from `plan/19` D22: *would this line differ if the UI were a
terminal, a web page, or a test harness?* If yes, it is engine work.

The renderer reaches the engine through **`window.api` only**. `@bifurc/gui` has **no** dependency on
`@bifurc/engine` — nor on `@bifurc/protocol` — by design (`plan/19` finding 1), so its types are hand-mirrored
and its calls are typed commands. A module that imports a protocol library, constructs a socket, or resolves
a `{{token}}` has broken the contract, and the ratchet in §6 will say so.

## 2. What a module must provide — engine-first

Work down this list; each row names the file that will *throw* or *fail* if it is skipped.

| # | Deliverable | Where it goes | What catches a miss |
|---|---|---|---|
| 1 | **A command set** | `packages/protocol/src/commands/<ns>.ts` + the `COMMANDS` map | `COMMAND_FIXTURES` equality, `assertScopesAreTotal()`, `assertRetryPolicyIsTotal()` — **all three throw at import** |
| 2 | **One `*.send` entry point** if the protocol has a request/response lifecycle | its own feature module, following D24/D27 | `tests/ipc/handlers.test.ts` (every command must be served by the shell) |
| 3 | **An entity kind**, if its requests are saved | `packages/engine/src/workspace/entities/kinds/` | `entity.*` throws `kind "<k>" is not routed through the CommandRegistry yet` |
| 4 | **Events**, if the engine owns state the UI watches | `packages/protocol/src/events.ts`'s `EVENT_NAMES` | the conformance matrices, which are derived from `COMMANDS`/`EVENT_NAMES` and therefore widen **automatically** |
| 5 | **A module directory** | `packages/engine/src/<module>/` + a `CONTRACT` row | `tests/engine/layout.test.ts` — **"every real directory is named in `CONTRACT`"**; an unregistered directory is the one silent way out of the layering rule |
| 6 | **A registration call** | `packages/engine/src/registerAll.ts` **and** `src/ipc/handlers.ts` | `tests/integration/workspaceHost.integration.test.ts` compares the two lists by test, not by trust |
| 7 | **A tab that holds no transport code** | `packages/gui/src/components/<module>/` | `tests/gui/protocolModule.test.ts` (§6) |

**Seven places, and four of them throw.** The count is not decoration: `plan/19` §11 records that a command
added to five of the seven passes typecheck, passes the build, and fails only when a *particular* suite runs.
Add the protocol entry to `plan/protocol-changes.md` **first** — before the code — so the wire change has a
written reason that predates it.

## 3. The `*.send` shape (D24/D27), for any module with a request lifecycle

One command per protocol, whose params are a **union of two complete input forms**:

| form | payload | request comes from |
|---|---|---|
| id | `{ workspaceId, id }` | the stored entity — what a **CLI** sends |
| inline | `{ workspaceId, request, … }` | the caller — what a **draft tab** sends |

The two forms differ **only** in where the request comes from. That is the whole claim, and it is why they
share a name rather than getting one each.

**The engine resolves and the renderer does not.** Environment variables *and* `{{random.*}}` are resolved
engine-side, and a field the caller leaves as `{{host}}` reaches the engine verbatim. **Never resolve in both
places, even briefly** (`plan/19` §8 trap 1): a resolved value can itself contain `{{…}}`, so a shadowed case
double-resolves and looks correct in every test that does not use one.

**Two consequences that are easy to get wrong:**

- **A `scripts` member exists only if the protocol's request is a `{method, url, headers, body}` one.** The
  sandbox's context is that shape, so GraphQL, SOAP and gRPC have **no** `scripts` member rather than one
  nothing could run. A field nothing can execute is a lie about what the command does.
- **Resolve *every* field the tab's gate scans.** `plan/19`'s central finding was that every non-REST tab
  resolved **fewer** fields than its own Send gate checked, so a `{{token}}` in a GraphQL query had never
  resolved in *either* path. Resolving the narrower set keeps the gate permanently non-empty.

## 4. Long-lived state: events, scope, lifetime, concurrency

A module whose work outlives a single call — a subscription, a socket, a collection run — owns four decisions
and must make each one explicitly. `plan/19` WI 4 is the worked example, and **WI 6 (the WebSocket registry) is
the second and the more useful one to copy**: it is a *client* rather than a server, and it is the case where
one of the four answers deliberately differs from the precedent — see the note under *Concurrency* below.

1. **Scope.** `start`/`cancel` are `execute`; `status` is `read`. The split is what lets a reconnecting client
   ask *what happened* without being granted the right to start or stop anything.
2. **Lifetime (D29).** A disconnect and a workspace switch **do not** cancel the work. A host restart loses
   it. "Cancel on disconnect" is the wrong default and is the one most implementations reach for.
3. **Concurrency (D30).** Pick a policy — one per `{workspaceId, folderId}` is the precedent — and state it.
   Undefined concurrency is what the single-browser-tab era hid.
   **⚠ Copy the *shape* of the precedent, not its answer.** D30 *refuses* a second run of the same folder,
   because a run is a unit of work and a duplicate is effort nobody asked for twice. `ws.connect` on a live key
   **replaces** instead, because a socket is a channel and a second connect is a *reconnect* — the renderer did
   exactly that, and refusing would make a broken socket unrecoverable without a separate disconnect first.
   The decision is required; the answer is not. What *is* required is that a replacement be atomic in the sense
   that matters: mark the old one superseded **before** closing it, so its close event cannot report a state
   the new one has already moved past.
4. **Every event carries the state a late attacher needs.** `total` rides **every** progress event, because a
   client that attached mid-run never saw the start call. A handler that only *updates* a snapshot it already
   has will silently drop the event for exactly the disconnected client the engine-owned lifetime exists for —
   both handlers must be able to **establish** state, not just amend it.

## 5. Anti-patterns — each one measured in this codebase

| Anti-pattern | What it cost |
|---|---|
| A second resolver / randomizer / parser in the renderer | The five re-implementations `plan/19` §1 opened with. They agreed by discipline, not by construction |
| A copy of engine **data** with no pin | Fixable, and legitimate — but only with a test asserting equality. `RANDOMIZER_TOKENS` (D28) and `SKIP_CURL_HEADERS` are the two, and **the pin is what makes the copy legal** |
| A shared list vs a shared decision | Splitting the catalogue from the generator is right; the *resolver* is never the UI's |
| Resolving in both places "for now" | Trap 1. A shadowed case double-resolves and every other test stays green |
| A tab that knows the wire | This document. The tab sends intent; the engine owns the socket |

## 6. The ratchet — `tests/gui/protocolModule.test.ts`

The boundary ratchet (`tests/gui/boundary.test.ts`, §3.7 of `TESTING.md`) asks *"is a piece of engine logic
implemented in the renderer?"*. It answers that by **name**, for the constructs `plan/19` §1 enumerated.

This one asks the complementary question: ***"does a tab speak the wire itself?"*** It scans
`packages/gui/src` for transport construction — `new WebSocket`, `new EventSource`, `fetch(`, `http`/`https`
`.request`, `net.connect`/`createConnection`, `dgram.createSocket`, `XMLHttpRequest` — and it is an
**allowlist that can only shrink**, with every entry naming the work item that deletes it.

It is deliberately a *separate* file from the boundary ratchet, because the two scan for different categories
and are deleted by different work: the boundary allowlist emptied when the last renderer resolver went; this
one emptied when the last tab stopped opening its own connection — **`hooks/useWebSocket.ts`, which was
`plan/19` WI 6, and both allowlists are now `{}`.**

**⚠ Emptying this one cost a guard, and the replacement is part of the design rather than a detail.** While
the allowlist held one entry, its staleness assertion did double duty: it passed only if the scan found that
live violation, so a `detect` regex that silently stopped matching failed the run. **With `{}` there is no
entry whose liveness depends on a hit**, so a pattern that matched nothing would leave every assertion green
while measuring nothing — the failure mode a name-based scan is most prone to. The guard is now explicit:
every pattern is run against a sample that must match **and one that must not**, with the control table checked
in both directions against the construct table. **A ratchet whose allowlist is empty needs a positive control,
or it is not a ratchet** — and the same reasoning applies to the boundary ratchet, whose allowlist is now
empty too.

**Detection reads the file as text, and that has a consequence worth knowing before writing a design note.** A
comment that quotes a construction — writing the constructor out to explain what used to be there — **is
flagged**. That is deliberate rather than a bug to be papered over: a false positive is safe (the suite fails,
the note is reworded) and a false negative is not, so a scan that errs toward flagging prose is better than one
that strips comments and can therefore hide a real construction on the same line as a `//` inside a string
literal.

**What it does not catch, recorded rather than papered over:** detection is by **name**, so a transport
constructed through an alias escapes. The stronger form is a dependency check (`packages/gui/package.json`
gaining a socket library), which is cheap and could be added the day a module needs one. The gap is benign in
the direction that matters: the mistake this guards against is a contributor reaching for a socket because the
engine's copy was not where they looked, and that is exactly what a name-based scan catches.

---

## Checklist, for copying into the first commit of a new module

- [ ] `plan/protocol-changes.md` entry written **first**, with the reason and the checklist answers
- [ ] Commands in `packages/protocol/src/commands/`, in `COMMANDS`, and in `COMMAND_FIXTURES`
- [ ] A scope in `SCOPE_BY_COMMAND` and an idempotency decision in `client/src/retry.ts`
- [ ] The `*.send` union, if there is a request lifecycle — resolving **every** scanned field
- [ ] An entity kind, if requests are saved
- [ ] Events in `EVENT_NAMES`, if the engine owns state — with `total`-style completeness on every payload
- [ ] `packages/engine/src/<module>/`, a `CONTRACT` row, and a registration call in `registerAll.ts`
- [ ] The same registration call in `src/ipc/handlers.ts`
- [ ] A tab under `packages/gui/src/components/<module>/` that calls `window.api` and nothing lower
