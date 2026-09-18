# 05 — Phase 4: Transport layer

**Goal:** a JSON-RPC transport that carries the frozen protocol over multiple channels, with
authentication, session management, event fan-out and reconnect recovery.

**Effort:** 3–4 weeks. **Depends on:** P1 (protocol), P2 (engine + EventBus). **Blocks:** P5, P9.

> **Security note before anything else:** this phase creates the network attack surface. The engine
> generates TLS CAs, executes scripts, spawns child processes and runs mock servers. **An
> unauthenticated remote RPC surface is a remote code execution hole.** Auth is item 4 of this phase, not
> a hardening task in P13.

---

## Status — 2026-09-18

**Item 2 is done: all four transports run the conformance suite green, which is this phase's first
acceptance criterion.** `in-process`, `stdio`, `ws`, and — as of this step — `socket` / named pipe,
D3's decided desktop transport. Item 3's **replay half is done** (`eventLog.ts` — one engine-scoped
`seq` authority and one bounded ring buffer) and item 4 is done as a mechanism.

**Item 1's file move is done, and its command migration now has a decision rather than a blocker.** The
companion server lives at `packages/engine/src/transport/legacyCompanion.ts` and `src/companion/` no longer
exists, so item 1's "the directory name `companion` will make no sense" half is complete. The four frozen
command names are **still served by that file, not by the generalised transport**, and the reason they are
is that aliasing them onto `config.get` / `entity.create` / `folder.add` is **four behaviour changes rather
than a migration** — including one that would let a lower-trust caller's "add" be destructive. The decision
taken on 2026-09-18 is that the conflict resolution behind that one is **scope-gated on `admin`**, so the
additive-only guarantee holds by construction and the extension is never named in the code. What remains is
an **extension release**, not engine work. The routes weighed, the two gated sites and the persistence bug
this surfaced are in "Item 1" below. So there are still **two WebSocket servers**, and that remains true
until the extension moves to v2.

**Item 3's backpressure half is done, so item 3 is done except for the flow-control window.** As of this
step `VERIFIABLE.backpressure` is `true` and the three transports with a wire claim it, which is the
phase's **8th** acceptance criterion. The half that had been open was open on a *design decision* rather
than on work — coalescing is invisible to a client by construction, so the capability could not be
verified without a testing hook until the payload shape was settled. `eventPump.ts`'s section below has
the argument, and the two defects it found.

**Item 4's audit criterion is now half done, and the half that is done closed a real defect.** Session
attribution exists: `CommandContext.session` (`SessionIdentity`) is filled by the auth decorator's
`onSession` observer, so a mutation arriving over a transport carries *who* made it instead of being
logged as the engine's own device name — which is what it did before. The *record* half is not done and
the criterion therefore stays unchecked: the log is **derived from git commits** rather than written to,
so an RPC entry cannot be recorded there without fabricated `commitHash`/`entity`/`entityId`/
`entityName` fields, and `commitMutation` — the only function that takes an `actor` — has no production
caller. See "Item 4's audit bullet" below, which also corrects the plan's own premise that this would be
"an extension, not a new system".

**Item 5's hardening beyond the two bind guards is still deliberately unclaimed.**

The 2026-09-17 block further down said "Nothing here opens a socket" and listed replay under
"deliberately not done"; the earlier-today block said `socket` was "the only transport left". Both were
accurate when written. They are kept rather than rewritten, because each one's findings are what made
the next step as small as it was — and the 2026-09-17 block's `deliver()` finding is precisely what let
the `socket` runner be a runner.

```
npx vitest run packages/engine/tests/conformance/ packages/engine/tests/transport/
  run-in-process.test.ts          57 tests — 41 passed, 16 skipped
  run-stdio.test.ts               57 tests — 43 passed, 14 skipped   (item 3: backpressure)
  run-authenticated.test.ts       57 tests — 54 passed,  3 skipped   (item 4)
  run-ws.test.ts                  57 tests — 56 passed,  1 skipped   (item 2 step 3, item 3)
  run-socket.test.ts              57 tests — 56 passed,  1 skipped   (item 2 step 4, item 3)
  conformance total              285 tests — 250 passed, 35 skipped, 0 failed

  transport unit suites          197 tests — framing 17, stdio 17, auth 60, eventLog 30,
                                            eventPump 24, ws 14, session 17,
                                            socket 18 (13 + 5 POSIX-only)
  ────────────────────────────────────────────────────────────────────────────────────────────
  total                          482 tests — 442 passed, 40 skipped, 0 failed

npx vitest run          (whole repo, 92 files)
  total                         2364 tests — 2323 passed, 1 failed, 40 skipped
```

The single failure is `tests/spike/protocolPoc.test.ts` → `soap.execute`, the documented pre-existing
flaky family (a dead endpoint, which the sandbox intercepts) — not a regression. Of the 40 skips, **35 are
the conformance suite's** (2 per runner for the capabilities each does not claim: `handshake`/`auth`/
`replay` for `in-process` and `stdio`, and `backpressure` for `in-process` and the authenticated runner,
which has no wire underneath) and **5 are `socket.test.ts`'s POSIX-only cases**, which have no Windows
counterpart — a unix domain socket is a filesystem entry and a named pipe is not. All of them are emitted
rather than filtered, so the remaining gaps stay visible rather than silently green.

The counts moved from 462/427/35 to 481/441/40 with the session work, and **the shift is worth reading
rather than skimming**: 10 of the 19 new tests are conformance cases, and because the pair is
*"a handler sees a session"* / *"a handler sees no session"* — the second gated by the new `noHandshakeIt`
(`capabilities.handshake ? it.skip : it`) — **every runner moved in both columns at once**: the five
runners each gained one case they *run* and one they *skip*, so the conformance total rose by 10 tests,
5 passes and 5 skips. The other 9 are unit cases (8 in `auth.test.ts`, 1 in `ws.test.ts`), which are all
passes.

The counts then moved from 481/441/40 to **482/442/40** with item 1's scope gate — a single
`auth.test.ts` case — and the whole-repo total from 2344 to **2364**. That second shift is +20 rather than
+1, because the gate's two suites live outside the command above, next to the behaviour they test:
`packages/engine/tests/commands/registry.test.ts` (10, the predicate) and
`tests/integration/entityConflictScope.integration.test.ts` (9, a real workspace read through a real
`loadConfig()`). The item-1 gate is therefore:

```
npx vitest run packages/engine/tests/commands/registry.test.ts \
                tests/integration/entityConflictScope.integration.test.ts \
                packages/engine/tests/transport/auth.test.ts
  total                           79 tests — 79 passed, 0 failed
```

### The delivery-barrier negative control has been retired

`run-deferred-delivery.test.ts` was **deleted on 2026-09-18**, the day `run-ws.test.ts` landed — exactly
what its own header and the section below said should happen. `run-ws` is now the control, and a better
one: a real TCP socket is asynchronous for real reasons rather than by construction. The evidence the
double produced (neutering `deliver()` failed **exactly six** cases, all of them on that runner, with
`in-process` and `stdio` green throughout) is preserved below, so retiring the file did not retire the
finding.

### What this step found that the plan did not predict

Two defects in the **suite**, not in `ws`, and both were load-bearing:

1. **`deliver()` awaited *delivery* but not *subscription establishment*.** `subscribe()` is
   synchronous, so `in-process` and `PassThrough` make a subscription live before the next line — and
   the suite silently depended on that. Over a socket a `subscribe` is only *queued* as a control
   frame. The first `run-ws` run produced nine `expected [] to have a length of 1` failures for this
   reason. The sting is that the events were not *delayed*, they were **dropped**: retention is lazy
   (`eventLog.ts`), so an event emitted before anything had subscribed was never numbered and never
   buffered. Fixed by giving `deliver()` an **emit callback** so the emit happens *inside* the barrier,
   after a command round trip. All ten affected call sites were rewritten; the four pre-existing
   runners came out unchanged, which is the regression signal.
2. **`make()` had to become async-capable.** Binding a port is asynchronous and `ws` cannot know its
   own URL until the OS assigns one, so `make()` is now `Transport | Promise<Transport>` and `harness()`
   awaits it.

### Item 2 step 4 — `socket`, and the shared client core it forced out

`socket.ts` is ~600 lines including its header, and the **client half is one object of six members**.
That is not brevity for its own sake: it is the first transport written after the client state machine
was extracted into `session.ts`, and the extraction is the real product of this step.

**Why the extraction was not optional.** `stdio.ts` and `ws.ts` had each grown their own copy of the
same ~150 lines — the pending-request map, the control-frame id set, the listener-token map, the single
`teardown()` that snapshots and rejects everything in flight, and the envelope rule on the way back —
and they were **~90% identical with every difference at the channel boundary**. A third copy is where
"similar" becomes "drifted", and the two existing copies *already* disagreed about `seq`: `stdio`'s
client numbered arriving events with a local counter, which is sound only because `stdio` has no replay,
i.e. it depended on a property of the transport rather than of the code. That disagreement was invisible
until it was looked for, which is the general shape of this bug class. `session.ts` now owns the rule and
its doc says why (`seq` is engine-scoped, so a subset subscription must see gaps).

**A defect caught in the new code before it ever ran, recorded because it is a class.** The first version
of `opened()` replayed the session's outbox by calling `sendFrame(frame)` on entries that were *already
encoded*. `encodeFrame` does `JSON.stringify`, so replaying a `Buffer` would have produced
`{"type":"Buffer","data":[…]}` — a frame the peer rejects as `not_an_object`, silently losing every
subscription queued before the channel opened. Fixed by splitting `writeEncoded(encoded)` (queue-or-write)
from `sendFrame(frame)` (encode-then-`writeEncoded`), so `opened()` replays **verbatim**. The general rule:
an outbox of encoded bytes must never be re-encoded, and the reason it went unnoticed is that the *queue*
path and the *flush* path are exercised by different tests.

**The stale-socket rule, and the guard that is about data loss rather than convenience.** A unix socket is
a filesystem entry that outlives its process, so a crashed engine leaves one behind and `bind()` then fails
with `EADDRINUSE` forever. The rule is therefore: if the path exists, `isSocket()` it, **probe** it, and
only unlink it if it is a socket nobody is listening on. The `isSocket()` check is the load-bearing one —
without it an engine starting up would `unlink()` whatever happened to be at the path, and a user who
mistyped a path in a config file would lose a file, silently, on next launch. Both halves are asserted
(`tests/transport/socket.test.ts`), including that a live engine is refused loudly rather than unlinked:
unlinking a live entry would leave the running engine unreachable while a second one took its name.

**`0600` is asserted, not assumed — and the residual window is stated rather than hidden.** `plan/05`'s
auth table makes filesystem permissions the whole of this transport's access control when no token is
configured, so `createSocketServer()` does not resolve until the mode reads back with no group or other
bits; a socket that cannot be made private is refused rather than served. The window between `bind()`
creating the entry and the `chmod` is **not** closed, because closing it needs the process umask, which is
global mutable state a library must not touch. What the caller gets is the guarantee that matters: nothing
built from this server can connect while the socket is not private. A caller that needs the window gone
too puts the socket in a `0700` directory — which is why `defaultSocketPath()` prefers `$XDG_RUNTIME_DIR`.

**One deliberate divergence from `ws`.** A frame the decoder rejects is reported through `onFatal` *and*
closes that one connection. `ws` closes without reporting, because its framing lives inside the library and
a close code is all it has; here the framing is ours, so there is a real error — and `oversized_frame` in
particular is a peer asking the engine to reserve 4 GiB, which is worth recording rather than silently
disconnecting over. Everything else follows `ws`: one bad peer closes one connection, never the process.

**Two shared constants that were duplicates waiting to happen.** `IDENTITY_FAILURE_CODES`
(`UNAUTHORIZED` + `UNSUPPORTED`) moved out of `ws.ts` into `auth/authenticated.ts`, next to the decision
table it encodes, because `socket.ts` must apply the same set with a different mechanism — a WebSocket
close code there, a flushed reply followed by a destroyed socket here. And `Capability.SOCKET` was added
to the protocol: `Capability.WS` already existed for exactly this purpose, and the `hello` response is the
only place capabilities are reported. Both are additive; `HelloResponseSchema` derives its enum from
`Object.values(Capability)`, so the protocol needed one line and no schema change.

**What the runner cost: nothing.** `run-socket.test.ts` is 112 lines of wiring with no assertions of its
own, and the shared suite needed **no changes at all** — the delivery barrier and the async-capable
`make()` that `run-ws` forced out are now properties of the suite. The plan predicted this
("the suite is ready for `socket` in the way it was not ready for `ws`") and the prediction held, which is
the strongest available evidence that the `ws` step's findings were fixed at the right layer.


| Where | What |
|---|---|
| `packages/engine/src/transport/types.ts` | `Transport` (`request` / `subscribe` / `close`), `EngineEvent`, the bus-name ↔ wire-name bridge, and `toRpcError` |
| `packages/engine/src/transport/inProcess.ts` | `createInProcessTransport(registry, opts?)` — dispatches straight into `CommandRegistry.invoke()`, no `ipcMain` |
| `packages/engine/src/transport/framing.ts` | the length-prefixed JSON codec (`uint32` big-endian + UTF-8) shared by `stdio` and `socket`, plus `frameFailure` — the one projection of a codec error onto `EngineError` |
| `packages/engine/src/transport/stdio.ts` | `createStdioTransport` (client), `createStdioServer` (engine side), `spawnStdioEngine` |
| `packages/engine/src/transport/session.ts` | the **shared client core** of every serialising transport — correlation, listener tokens, the single `teardown()`, the envelope rule; parameterised by a six-member `ClientChannel` |
| `packages/engine/src/transport/eventLog.ts` | `EventLog` — the **engine-scoped** `seq` authority and the bounded ring buffer (item 3's replay half) |
| `packages/engine/src/transport/ws.ts` | `createWsTransport` (client) / `createWsServer` (engine side) — item 2 step 3, and the generalised `companionServer.ts` |
| `packages/engine/src/transport/socket.ts` | `createSocketTransport` / `createSocketServer` / `defaultSocketPath` — item 2 step 4, D3's decided desktop transport; `0600` asserted on POSIX, stale-socket and live-socket guards |
| `packages/engine/src/transport/legacyCompanion.ts` | **v1, deprecated, frozen** — the browser extension's loopback server on 9271, moved here from `src/companion/companionServer.ts`; its header carries the four differences that stop its commands being aliased onto the registry |
| `packages/engine/src/transport/legacyCompanionActions.ts` | `V1_COMPANION_ACTIONS` — the extension's frozen four-name allowlist (was `companion/allowedActions.ts`'s `ALLOWED_ACTIONS`) |
| `packages/engine/src/transport/auth/scopes.ts` | the 93-entry command → scope table, asserted total at import, fails closed |
| `packages/engine/src/transport/auth/token.ts` | per-start token: rotation, `0600`, constant-time verification, env-var path |
| `packages/engine/src/transport/auth/authenticated.ts` | `createAuthenticatedTransport` — the `hello` handshake, the scope gate, and `IDENTITY_FAILURE_CODES` (shared with both socket transports) |
| `packages/engine/src/transport/index.ts` | barrel, and a table of which of the four transports exist |
| `packages/protocol/src/envelope.ts` | `Capability.SOCKET` — the counterpart of `Capability.WS`, so a client can tell which transport it is on from the `hello` response |
| `packages/engine/tests/conformance/protocol.conformance.ts` | the transport-agnostic suite |
| `packages/engine/tests/conformance/run-in-process.test.ts` | runner — **no assertions of its own**, by design |
| `packages/engine/tests/conformance/run-stdio.test.ts` | runner — same, over a `PassThrough` pair |
| `packages/engine/tests/conformance/run-authenticated.test.ts` | runner — the auth decorator over `in-process`, claiming `handshake` + `auth` |
| `packages/engine/tests/conformance/run-ws.test.ts` | runner — a real `ws` server on an ephemeral loopback port, claiming `handshake` + `auth` + `replay` |
| `packages/engine/tests/conformance/run-socket.test.ts` | runner — a real unix socket / named pipe, claiming the same three; the shared suite needed no changes for it |
| `packages/engine/tests/transport/auth.test.ts` | the mechanism's own 60 cases: scopes, token, decorator, session identity |
| `packages/engine/tests/transport/eventLog.test.ts` | the ring buffer's own 30 cases: both bounds, lazy retention, the replay windows |
| `packages/engine/tests/transport/framing.test.ts` | the codec, which has no `Transport` to run against |
| `packages/engine/tests/transport/stdio.test.ts` | the pipe-lifecycle cases the shared suite cannot express |
| `packages/engine/tests/transport/ws.test.ts` | the 14 **socket-specific** cases the shared suite cannot express — bind guards, TLS refusal, close codes, payload ceiling, the `peer` address |
| `packages/engine/tests/transport/socket.test.ts` | the 18 cases the shared suite cannot express — path naming, the stale/live/not-a-socket guards, `0600`, unlink-on-close, the frame ceiling, and the two that are POSIX-only |
| `packages/engine/tests/transport/session.test.ts` | the shared client core's own cases, over a fake channel — verbatim outbox replay, refcounting, the envelope rule, and the single teardown path |
| `packages/engine/src/index.ts` | exports the transport **above** the provisional divider (it is permanent, not a P6 shim) |
| `packages/protocol/src/events.ts` | `ProcessOutputEventSchema` + `SettingsChangedEventSchema`; `EVENT_NAMES` is now 9 |
| `packages/protocol/src/commands/fixtures.ts` | `COMMAND_FIXTURES` — the 93-entry valid/invalid table, shared by the protocol schema test and the transport matrix |

### The wire contract is the protocol's, not `stdio`'s

Frames carry the **frozen** `@bifurc/protocol` envelopes — `RpcRequest {id, action, payload}` /
`RpcResponse {id, ok, data|error}` / `EventEnvelope {event, seq, payload}` — discriminated by which key
is present. A `stdio`-specific envelope would have meant a translation layer for `ws` and a second
place for the envelope rule to be got wrong, and the rule is the thing that keeps `window.api`
byte-identical.

This is also where the envelope rule gets its second chance to be broken, because a serialising
transport has **two `ok` flags that are not the same flag**:

```ts
{ id, ok: true,  data: { ok: false, error: "..." } }   // the handler ran; inner ok:false rides through as DATA
{ id, ok: false, error: { code, message, ... } }        // the handler THREW; this rejects
```

`tests/transport/session.test.ts` asserts both directions directly, over a fake channel — that is the
one place the rule can be tested as a rule rather than as a handler's behaviour — and every runner's
suite exercises it end to end. Conflating the two would break every legacy handler that reports failure
as a resolved value, which is most of them.

### Item 4 — auth is done as a mechanism, and the gate is now satisfiable

| Module | Responsibility |
|---|---|
| `packages/engine/src/transport/auth/scopes.ts` | which scope each of the 93 commands requires |
| `packages/engine/src/transport/auth/token.ts` | the per-start secret: generation, `0600` persistence, constant-time verification |
| `packages/engine/src/transport/auth/authenticated.ts` | `createAuthenticatedTransport` — the handshake and the scope gate |

**It is a decorator, and that is the decision that makes the gate workable.** `ws` is gated on item 4
("an unauthenticated remote RPC surface is a remote code execution hole"), and a gate that can only be
satisfied by building the thing it gates is not a gate. A decorator over any `Transport` is testable
over `in-process` today, so the handshake, the token check and the scope gate are all built and proven
*before* the socket exists — and when `ws` lands, auth is a wrapper rather than a redesign.
`in-process` and `stdio` are simply never wrapped: item 4's own table says `stdio` needs none ("the
pipe is inherited and process-scoped"), and `in-process` has no boundary at all.

#### The scope table is explicit, total, and fails closed

`plan/05`'s table is the intent; the file is the **resolution**, because the plan's rules overlap and
two of them do not match the real protocol:

- The plan says `applications.*`; the namespace is **`application.*`** (singular).
- The plan says `runner.*` → `execute`, but **no runner-execution command exists**. The six `runner.*`
  commands are `getHistory`/`listFolderIds` (read) and `saveReport`/`exportReport`/`saveConfig`/
  `loadConfig` (write); collection execution must happen outside the registry.
- `*.list` → read and `applications.*` → admin **overlap on `application.list`**. Resolved by the
  narrower rule: pure reads inside an admin namespace are reads, so a client can still see what it is
  allowed to touch while `start`/`stop`/`delete` stay admin-only.

93 explicit lines rather than pattern rules, for the same reason `COMMAND_FIXTURES` is a table: this
gates `tls.*`, `server.*` and every `*.execute`, and a derived-by-regex table with overlapping rules is
exactly what produces a silent mis-gate. `assertScopesAreTotal()` throws **at import time** if the
table and `COMMANDS` disagree in either direction — so adding a command to the protocol cannot produce
an accidentally ungated command; the engine refuses to load until someone assigns it a scope. And
`requiredScopeFor()` returns `undefined` rather than defaulting, with `commandIsAllowed()` then
returning **false**: unknown means denied, never allowed.

**No implicit hierarchy** — `admin` does not imply `read`. Scopes are an explicit set, so "which
commands can this session reach?" is a lookup rather than a derivation, which is the question a
security review actually asks. `SHELL_SCOPES` / `COMPANION_SCOPES` / `READ_ONLY_SCOPES` exist so
callers do not hand-roll the sets. `COMPANION_SCOPES` is `ALLOWED_ACTIONS`'
`{mock:add, request:add, folder:add, config:get}` generalised to `{read, write}` — the plan calls it
"write-only", but it needs `read` too, because `config:get` is a read and the extension must be able to
check state before adding to it.

#### The token, and one limitation stated rather than papered over

`ensureEngineToken()` **generates a new token on every call** instead of reusing a file from a previous
run. A token that survives a restart is a credential that survives a restart, so anything that ever
read it — a stale shell, a log, a backup of the data dir — stays authorised forever. Rotation is what
makes it a *session* secret. `BIFURC_ENGINE_TOKEN` supplies it out of band for Docker or a mounted
secret, and when it is set **no file is written**: a container that mounts its secret should not also
have it written into a volume that may be more widely readable. An empty env var falls through to
generation, because an empty token is a trivially guessable credential.

`verifyToken()` uses `crypto.timingSafeEqual`, not `===`. A string comparison short-circuits at the
first differing byte, so the time taken to reject reveals how many leading bytes were right — turning a
2^256 search into a byte-at-a-time one. Over loopback that is theoretical; the whole point of this
module is the remote case, where it is not. `timingSafeEqual` throws on a length mismatch, so lengths
are compared first: that leaks the length, which for a fixed 32-byte token is public anyway.

**`0600` is honoured at creation and re-applied by `chmod` for the overwrite case — but on Windows
neither has any effect.** There are no POSIX permission bits, and Node maps `mode` to the read-only
attribute only. So on Windows the token is protected by the data directory's ACL and nothing else.
That is a real gap, and it is the *same* gap the `socket` transport has on Windows — where the
permission assertions in `socket.ts` are guarded by `!isWindows` and a named pipe therefore relies on
the default DACL of the creating process rather than on an explicit mode. It belongs in P9/P12 rather
than being papered over with a `chmod` that silently does nothing.

#### Three failure modes, deliberately not one code

| Situation | Code | Session |
|---|---|---|
| first request is not `hello`, or the token is wrong | `UNAUTHORIZED` | **closed** |
| the client's protocol **major** differs | `UNSUPPORTED` | **closed** |
| the command needs a scope the session lacks | `FORBIDDEN` | stays open |
| the `hello` payload is malformed | `BAD_REQUEST` | stays open |

The first two are *identity* failures — the peer has not established who it is — so item 5's rule
applies: "Auth failure returns `UNAUTHORIZED` and closes. Never fall back to unauthenticated." A
`FORBIDDEN` is not: the peer proved who it is and asked for something out of scope, and closing would
turn a scope error into a disconnect so a mis-scoped client looks like a network fault. A malformed
`hello` claimed nothing to revoke, so it is answered and the client can correct it.

**The version check is `checkVersionCompatibility()`, not `!==`.** A naive equality check would refuse
the companion extension, which the protocol's own comment calls the *expected* state for weeks after
every release: it is an external client on its own release cycle. Only a **major** mismatch is refused.
There is a conformance case for the minor-mismatch path specifically, because that is the one a
"simplify this to `!==`" refactor would break.

The gate also runs **before** payload validation, so a forbidden command is refused identically whether
or not its payload is well-formed. If the order were reversed, the *shape* of the error would tell an
unauthorised caller whether a command exists and what its schema wants — a free oracle for mapping the
protocol from a session that should see nothing.

#### Wiring it into the suite took an async harness, and the counts prove it was safe

`handshake` and `auth` are now in `VERIFIABLE`, so a runner can claim them — and the five cases that
were skips are real. `ConformanceOptions` gained an `auth: { hello() }` option (required at definition
time when either capability is claimed, or the cases would skip *despite* the claim), and `make()` gained
an optional `scopes` argument so the FORBIDDEN case can build a narrow session — the harness's own
session must hold every scope or the command matrix could not run, and a session that can do everything
cannot demonstrate a refusal.

Because the harness now performs the handshake, it had to become **async** — 30 call sites awaited and
5 `it` callbacks marked `async`, done by `scripts/p4-async-harness.py` with the counts asserted. The
regression signal is that the three pre-existing runners came out **identical**: 117 passed / 27
skipped / 144, unchanged. A missed call site would have left a test asserting against a Promise, and
every one of those still passes as a truthy object — which is why the rewrite was scripted with
asserted counts rather than done by hand.

`run-authenticated.test.ts` is the fourth runner: the decorator over `in-process`, claiming both
capabilities, with **no assertions of its own** (54 passed / 3 skipped — the three are item 3's `replay`
and `backpressure` plus the no-handshake case the session pair adds). It runs the whole command matrix
*through* the decorator, which is what proves the decorator is transparent to everything else a transport
does. `tests/transport/auth.test.ts` holds the mechanism's own 60 cases.

This runner is also where the session wiring is spelled out, because it is the only runner that has to
**construct** the decorator rather than receive one. Its `make()` builds a single `CommandContext`, hands
it to `createInProcessTransport`, and wires `onSession` to mutate *that same object* — the identity
arrives by mutation, after the inner transport has already captured its context, so a second context
object would receive the session and never be invoked. See "Item 4's audit bullet" below.

#### Item 4's audit bullet: the premise is false, and the real gap was attribution

Work item 4 ends with *"Extend the existing audit machinery (`audit:list`, `audit:diff`, `audit:export`)
to record RPC calls: who, what command, from what address, when, and the outcome. **The engine already
writes an audit log; this is an extension, not a new system.**"* Measured against the code, that last
sentence is **false for RPC calls**, and the reason is structural rather than a matter of wiring.

- **The audit log is derived from git; nothing writes audit rows.** `queryLog()`
  (`packages/engine/src/store/gitStore.ts:132`) does not read a table — it walks the commit history and
  **parses commit bodies** for `entity-id:`, `workspace-id:` and `actor:` (line 152). Every `AuditEntry`
  it can produce carries a `commitHash`, an `entity`, an `entityId` and an `entityName`. An RPC call is
  not a commit and has no entity, so recording one there would mean **fabricating all four fields** — and
  `audit.diff` would then be offered on rows where `getEntityAtCommit()` cannot possibly work.
- **`commitMutation` has no production caller.** It is the one function that takes an `actor` and writes
  an audit-shaped commit (`gitStore.ts:73`), and **nothing in `src/` or `packages/engine/src/` calls
  it** — only `tests/store/gitStore.test.ts`, `tests/integration/{auditLog,fileOps}.integration.test.ts`
  and `vi.fn()`s in `tests/ipc/handlers.test.ts` and `tests/spike/protocolPoc.test.ts`. Real entries come
  from the sync path: `entityCrudFactory` → `syncChanges()` → `publishService`'s `g.commit(fullMessage)`,
  with **`actor = opts.actor ?? deviceName()`** (line 83).
- **That is the actual gap, and it is worse than a missing feature.** Because nothing carried a session
  into the command context, **a mutation made over a transport was attributed to the engine's own machine
  name** — indistinguishable in the log from a local edit. `HelloRequestSchema` has declared
  `clientName`/`clientVersion` since P1 with the comment *"Recorded per session so the engine can log and
  diagnose which build is connected"*, and **nothing recorded them**. A second declared-but-unused
  protocol field, and this time the field was the specification for the fix.

So the criterion splits in two, and **only the first half is done**:

**Done — attribution.** `CommandContext` gained an optional `session: SessionIdentity`
(`{ sessionId, clientName?, clientVersion?, peer? }`), and `createAuthenticatedTransport` gained an
`onSession?: (identity) => void` observer — the counterpart of the existing `onAuthFailure`. It fires
**exactly once** and **never on a rejected handshake**, and it is an *observer* rather than something the
decorator does itself, which is what keeps the decorator usable over `in-process` where there is no
context to write to. It is called **after** `scopesFor` deliberately: `scopesFor` is caller-supplied and
may throw, and a recorded session that never opened is worse than a missing one. `ws` and `socket` now
build a **per-session** context rather than closing over one template at server construction, and only
`ws` sets `peer` — because only `ws` has a peer *address*: a unix socket's path names the engine's own
endpoint, so putting it in a field called `peer` would be a fiction a reader would take for a client
identifier. `in-process` and `stdio` have no handshake, so their handlers see **no session at all**;
inventing one would put a fiction in the log, which is the opposite of the point.

**Not done — the record.** Nothing *commits* a record per mutating command. That needs per-mutation
commits in the `src/` layer, inside a log that is derived from git, plus two decisions this phase should
not take unilaterally: whether **every** mutation should become a git commit (a durability and history
question, not a transport one), and whether the result extends `audit.*` or is a separately queryable
stream — given that `audit.list` is deliberately **not** registry-wired, because `AuditListParams` is
`.strict()` and lacks the `fromTs`/`toTs` that `AuditLogPanel.tsx` sends.

### Item 3 — the event log, and why `seq` is engine-scoped rather than per-session

`packages/engine/src/transport/eventLog.ts` is the single `seq` authority and the single bounded ring
buffer, created by `createEngine()` and closed by `stop()`. One per **engine**, not per transport and
not per session.

**`plan/05` says "per session", and its own `hello` sketch is what contradicts it:**

```ts
// client reconnects
hello { protocolVersion, lastSeq: 48210 }
```

A reconnect is a **new session** — the old transport is gone. If `seq` were counted per session, the
`lastSeq` a client presents would be a number from a session that no longer exists, and the same number
would mean different events in the new one. There is no session id in the sketch to disambiguate it.
Replay across a reconnect is therefore only *definable* if `seq` belongs to the engine. That is the
whole argument, and it is why this file exists rather than a counter inside each transport.

**The cost, stated rather than hidden:** "gap-free" is lost. A session subscribed to only *some* events
now sees `seq` values with gaps, because events it did not subscribe to still consume a number. A gap can
no longer be read as "events were lost". It buys the only thing that makes replay work, and `plan/05`
calls replay the most likely source of "works locally, broken remotely" bugs. `types.ts`'s older
"per-session, gap-free" note is corrected accordingly, and the `ws` client carries **the engine's `seq`**
through rather than recounting locally — a transport that renumbered would hide exactly the gaps a client
needs to see.

**Retention is lazy, and it outlives its subscribers — both deliberately.** A bus listener for an event
name is attached the *first* time anything subscribes to that name and is held until `close()`; it is
**not** released when the last subscriber leaves. That is the point: during a network blip nobody is
subscribed, and a buffer that only filled while someone was listening would replay an incomplete history
— silently, which is the failure mode this whole section is about. An engine that never uses events pays
nothing, because nothing is retained until a name is first asked for.

The consequence to know about: **an `EventLog` holds bus listeners, so `bus.listenerCount(...)` is no
longer a proxy for "does this transport leak?"** The conformance suite asserts on `subscriberCount()`
instead — the same property, measured at the layer that now owns it.

Bounds are `DEFAULT_MAX_EVENTS = 1000` / `DEFAULT_MAX_AGE_MS = 5 min`, enforced on **write and on read**
(the age bound moves with the clock, so a read after a quiet period must prune too), with an injectable
clock so the age bound is testable without sleeping.

`replayFrom(lastSeq, names?)` returns a discriminated outcome, and the three resync cases are three
*different* mistakes rather than one:

| Situation | Outcome |
|---|---|
| nothing retained, and `lastSeq` is not the current `seq` | **resync** — the client claims to have seen events this engine has no record of |
| `lastSeq` older than the oldest retained event | **resync** — the events in between were dropped and cannot be replayed |
| `lastSeq` **ahead** of the newest | **resync** — not an empty replay: replaying nothing would leave the client believing it is up to date, which is a silent staleness bug |
| otherwise | **replay**, filtered to `names` when given |

`names` is optional on purpose: the transport passes the names the client is subscribing to, so it does
not replay events the client never asked for, while `hello` omits it to ask only *whether* replay is
possible — which is all `replayedFrom` / `resyncRequired` need.

### Item 3 — the coalescing half, and the design decision that unblocked it

`packages/engine/src/transport/eventPump.ts` batches `event.log.entry` into the protocol's
`LogEntryBatchEvent` — 100 entries or 250 ms, whichever is first — and sends every other event
immediately. One pump **per session**, created by each of the three transports with a wire. The plan's
policy table is implemented exactly, with `log.chunk` and `process.output` deliberately **not** batched:
their row says "stream, with a flow-control window", and a chunk's whole value is arriving as it is
produced, so holding it for 250 ms would trade the feature for the frame count. **The flow-control window
is still missing** — see "Deliberately not done".

**The unblocking decision was not the mechanism, it was the payload shape.** Coalescing is *invisible to
a client by construction*: it receives the same events whether the engine sent one frame or a hundred.
That is the point of coalescing and also the reason `VERIFIABLE.backpressure` stayed `false` for as long
as it did — no case written against `Transport` can distinguish a coalescing engine from one that
forwards every entry, so the only way to verify it would have been a frame counter handed to the suite,
i.e. a testing hook in a production contract.

What made it verifiable instead is `LogEntryBatchEventSchema`, which `@bifurc/protocol` has carried
**since P1 and never used**. It is the protocol saying that what a client receives for
`event.log.entry` is `{ entries: [...] }` — so a subscriber *sees* the batching, and "250 entries arrive
as 3 deliveries, not 250" is an assertion the interface can make. A single entry is a batch of one, which
is what `in-process` delivers: it has no wire and therefore no coalescing, but the *shape* is the
protocol's rather than the batching's, so it produces the batch too. Two shapes for one event name would
be worse than either.

**The cost, and who pays it.** `window.api`'s `onLogEntry` is per-entry and is a frozen P1 surface, so it
stays per-entry. That means the expansion a "client unwraps the batch" design would have done inside the
transport happens one layer higher: **`src/ipc/eventBridge.ts` will have to iterate `payload.entries`
and broadcast each one** when P5/P6 migrate the shell onto a transport. Today the shell is still
in-process and reads the bus directly, so nothing is broken yet — this is a note for the shell migration,
not a defect, and it is the price of the decision above rather than an oversight in it.

**Why the batch's `seq` is its *last* entry's.** A client's `lastSeq` is the last envelope `seq` it saw
and the engine replays everything with `seq > lastSeq`. A batch carrying its *first* entry's `seq` would
put the resume point **inside** the batch, and every entry after the first would be replayed again on
every reconnect. Carrying the last one puts the whole batch behind the resume point, which is true
because a frame is atomic.

**FIFO is not negotiable, so an immediate event flushes the batch in hand.** Coalescing introduces a
delay, and a delay in front of a queue is how reordering bugs get in: if a pending batch were left to
its timer while `server.error` overtook it, wire order would stop being emission order — and the
conformance suite's `deliver()` barrier depends on FIFO *being* that order. Every immediate event
therefore flushes the pending batch first, then sends itself.

**The drop policy needs no drop mechanism, and that is the finding.** "Under sustained overload, drop
`log.entry` before `server.error`" reads as an unbounded-queue problem. It is not, for two structural
reasons: `server.error` is **never queued** (it takes the immediate path, so a value that is never held
cannot be displaced), and the pending queue **cannot grow past `maxBatch`** because the cap-triggered
flush is synchronous. `tests/transport/eventPump.test.ts` asserts the boundedness — 10,000 entries, no
batch over 100 — so if a future change makes `send` asynchronous the test fails and the policy becomes a
real question again. One honest limitation: because nothing is dropped, a dropped entry would have no
reporter. Telling a client its log has holes needs a protocol addition that does not exist
(`ResyncRequiredEvent` is about replay, not loss).

#### Two defects this found, both worth not re-introducing

**1. The internal `RequestLogEntry` is not the wire's `LogEntryEvent`, and batching made that
load-bearing rather than cosmetic.** The bus payload carries the captured bodies — `reqBody`, `resBody`
(base64, the response capped at 512 KB) — plus `host`, `via` and `target`, and names its timestamp `ts`.
`LogEntryEventSchema` is six fields with the timestamp named `timestamp`, and like the batch schema it
had been declared-but-unused since P1. **One entry with a 512 KB body is a large frame; a batch of 100 of
them is up to ~50 MB in a single frame.** So the projection is what makes coalescing affordable rather
than harmful, and it is applied on every transport. The conformance suite asserts the captured body does
not cross.

**2. Applying the shape in the "obvious" two places produced a batch of 100 empty log rows.** The first
version shaped in `inProcess.ts` and in the pump. But `ws` and `socket` compose a client-facing transport
(`in-process`, wrapped in the auth decorator) **inside their own server**, so their pump is *downstream*
of something that has already shaped: it projected an already-shaped `{entries}` payload as if it were a
raw entry, every field fell back to its default, and the client got empty rows. `stdio` was correct
throughout, because its server subscribes to the bus directly — which is exactly why the defect survived
a green `stdio` run and showed up only on `ws`/`socket`. Fixed by making `toLogEntryBatch` **idempotent**
(a batch is a batch, a raw entry is a batch of one) and asserting that, rather than by adding a flag
saying which kind of input a pump is being fed.

The same defect existed in a quieter form on the **replay** path, and fixing it closed a pre-existing
hole: the auth decorator delivers its replay backlog by **synthesising** the event itself rather than
receiving it from the inner transport, so it bypassed whatever shape the inner transport applies on its
live path. A reconnecting client replaying `log.entry` over `in-process + auth` would have got the raw
payload while its live entries arrived shaped. The decorator now applies `toClientEvent` to the backlog
it synthesises. The three wire transports were never affected, because their pump normalises.

### Item 2 step 3 — `ws`

`createWsTransport(url, opts)` (client) and `createWsServer(opts)` (engine side). The server factory is
**async** because it binds, and it takes `port: 0` by default so the OS assigns one — which is also why
the suite's `make()` had to become async-capable.

**`ws.ts` contains no auth logic at all, and that is the payoff of item 4's decorator decision.** Each
connection builds its own session: `createInProcessTransport(registry)` wrapped in
`createAuthenticatedTransport(...)`. The handshake, the constant-time token check and the scope gate were
built and proven over `in-process` *before* this socket existed, so this file only forwards frames into a
`Transport` and writes the answers back out.

The one thing it does own is the **consequence** of an auth failure. Item 5: *"Auth failure returns
`UNAUTHORIZED` and closes. Never fall back to unauthenticated."* So an `UNAUTHORIZED` or `UNSUPPORTED`
reply is **flushed and then** the socket is closed — the reply goes first because a peer that is
disconnected without being told sees a network fault instead of a rejected credential, which is the one
diagnostic a remote client has. `FORBIDDEN` and `BAD_REQUEST` stay open, as item 4's table requires.

Four smaller decisions, each of which is a bug that would otherwise have shipped:

- **One `session.subscribe([wire], …)` per name on the server, refcounted by name.** Not one call with
  all names: the decorator drains the replay backlog **per call**, filtered to that call's names, so a
  batched call would hand the backlog to whichever name happened to be first.
- **A refused control frame is surfaced through `onFatal`.** `subscribe()` is synchronous and returns an
  unsubscribe function, so it has no other channel through which to report a refusal that arrives later.
  Silently dropping it would leave a caller with a subscription that never fires and never errors.
- **`close()` uses `terminate()` plus `closeAllConnections()`.** `WebSocketServer.close()` waits on every
  client to disconnect and `server.close()` waits on idle keep-alive sockets, so either alone can hang a
  test run rather than end it.
- **`maxPayload` is 16 MiB and a non-upgrade HTTP request gets a 426.** The frame ceiling belongs at the
  transport (work item 5's "request size" row); the 426 exists so a browser hitting the port gets an
  explanation rather than a socket that opens and dies.

**Bind address and TLS — item 5's two rules, encoded rather than documented:**

| `host` | `allowNonLoopback` | `tls` | result |
|---|---|---|---|
| loopback (default) | any | any | binds |
| non-loopback | absent / false | any | **refused** — "remote mode is opt-in and explicit" |
| non-loopback | `true` | absent | **refused** — "TLS on the channel, not optional" |
| non-loopback | `true` | present | binds, `wss://` |

Two guards rather than one because they are two different mistakes: binding wide by accident, and binding
wide in the clear on purpose. Encoding them means the engine cannot be talked into an unauthenticated,
unencrypted, remotely-reachable RPC surface by a config typo — the process refuses to start rather than
exposing one.

**The practical consequence, stated so nobody is surprised: remote mode is not reachable yet.** Nothing
in the programme sets `allowNonLoopback`, and the TLS path is a pass-through to `node:https` with no test
behind it. The guard is what makes it safe to leave that work to P9/P12.

`RESERVED_ACTIONS` and `remoteErrorToEngineError` moved from `stdio.ts` into `types.ts` when this landed,
so the two serialising transports share one copy of each. `remoteErrorToEngineError` in particular
preserves a handler's **own** error code rather than flattening it, and a second copy is precisely the
per-transport drift the risks table warns about.

### Item 1 — the file moved, and the four commands may be aliased only with the conflict resolution gated

Item 1 says *"Do not write a new server"* and lists five changes to the companion server. Four of the
five are now satisfied by `transport/ws.ts`: the enriched `RpcError`, the mandatory `hello`, per-session
scopes in place of the static allowlist, and — as of this step — the move out of `src/companion/`, which
no longer exists. The file is `transport/legacyCompanion.ts`, and its allowlist moved with it to
`transport/legacyCompanionActions.ts`.

The fifth is the four colon-named commands, and the honest status is that **they must not be migrated
the obvious way**. The plan's own wording was "a compatibility migration with a deprecation window, not
a rename", which anticipated a mechanical mapping. Measured on 2026-09-18 against
`src/ipc/handlers/crudHandlers.ts`, `entityCrudFactory.ts` and `folderHandlers.ts`, the mechanical
mapping is a **behaviour change in four places**:

| | v1 (`legacyCompanion.ts`) | registry equivalent |
|---|---|---|
| mock validation | `"urlPattern is required"` | `"urlPattern is required for mocks"` |
| request validation | `"url is required"` | `"url is required for requests"` |
| duplicate mock | adds verbatim | `onAddConflict` **disables an existing enabled mock** |
| blank folder name | refused, `"name is required"` | no check — creates a folder named `"   "` |
| unknown folder kind | `"Unknown folder kind: bogus"` | `TypeError` from `FOLDER_CONFIG[undefined]` |

The first four are asserted **verbatim** by `tests/integration/companionServer.integration.test.ts`
(exact `error` strings), so a straight alias fails the suite outright. The third is the one that matters
beyond tests: `onAddConflict` mutates **existing** state, while `V1_COMPANION_ACTIONS`' own header and
its unit test define the companion as a lower-trust caller whose surface must stay **additive-only**. A
page in the user's browser capturing a URL that already has an enabled mock would silently switch that
mock off — the extension would gain a destructive capability nobody decided to grant it.

So this is a **decision, not a refactor**: either the extension inherits the registry's de-duplication
(and the additive-only guarantee is formally relaxed, which is a security call), or the v2 commands keep
explicit additive semantics. **That decision was taken on 2026-09-18 and is recorded in the next
subsection.** Until the extension ships, the four commands stay in `legacyCompanion.ts`, byte-compatible,
on `settings.companionPort ?? 9271`.

That means **two WebSocket servers in the process** remain — the legacy one on 9271 and the new one on
an ephemeral port. It is a transitional state, not a design, and it is called out here so it is not
mistaken for one. The released extension keeps working throughout, which is the constraint item 1 sets.

#### The decision, taken 2026-09-18: the de-duplication is scope-gated, so the companion stays additive

Three routes were on the table, and the third is the one taken:

| Route | Cost |
|---|---|
| alias and inherit `onAddConflict` | formally drops the additive-only guarantee — a page in the user's browser could switch off the user's mock |
| register the four names as their own commands | every v1 guarantee preserved by construction, at the cost of the engine carrying a second spelling of each operation forever |
| **alias, and gate the conflict resolution on `admin`** | one implementation, and the guarantee enforced by the existing total scope table rather than by a rule written for the extension |

The third works because of what the two callers already *are*, not because anything new was invented for
the extension: `COMPANION_SCOPES` is `{read, write}` and `SHELL_SCOPES` is all four, so "may this caller
affect entities it did not name?" has a different answer for each **without the extension being named
anywhere in the code**.

**Why `admin` and not `write`.** Conflict resolution disables an entity the caller never mentioned,
found by scanning config — it is the one thing here that reaches *outside* the caller's own request.
`entity.update` is `write` because it mutates an entity the caller **identified**; `onAddConflict`
mutates ones it did not. So the rule is stated as: **a command may only affect entities the caller
names.** It is expressed once, as `mayAffectUnnamedEntities()` in `commands/registry.ts`, rather than as
a bare `callerHasScope(ctx, "admin")` at each site, because the two sites must agree about it.

**A caller with no session is the engine itself, and keeps full behaviour.** `in-process`, `stdio` and
every legacy `ipcMain.handle` adapter pass a session-less context, because none of them crosses a
boundary — so the shell's own behaviour is unchanged, which is what the regression cases assert. A caller
that *has* a session but reported no scopes is **refused**: unknown authority fails closed, or forgetting
to report scopes would silently grant them.

**Two sites, not one.** `entity.create`'s `onAddConflict` and `entity.setEnabled`'s inline loop both
disable same-signature siblings, so both take the gate. The second is the one the shell exercises when a
user flips a mock on, and it is the more dangerous of the two: it acts on an entity that is already
enabled.

**Where the scope vocabulary lives.** `Scope` moved to `commands/registry.ts` and is re-exported from
`transport/auth/scopes.ts`. It had to move: `transport/` imports `commands/` and never the reverse (all
four transports import `CommandContext` from the registry), and a handler needs the vocabulary in order
to read `CommandContext.session.scopes` — so it belongs on the lower side. Re-exporting keeps `./scopes`
the single import for both the vocabulary and the table, and **no protocol change was needed**.

#### The bug this surfaced: `onAddConflict` never persisted anything

Writing the integration test against **`loadConfig()`** rather than a mocked config object is what found
this, and it is the reason the test is written that way.

`onAddConflict` disables siblings by mutating `cfg`, and `createEntityCore` then calls `saveConfig(cfg)`
— which reads like it persists. It does not. `writeEntity()` **strips `enabled` on the way out**, because
enabled state lives exclusively in `enabled.json` (`workspaceFs.ts`), and `saveConfig` never writes the
enabled set. So the disable was applied to an in-memory object that was then thrown away: the sibling
stayed enabled on disk, and — because `createEntityCore` had already `unshift`ed the new entity, so it
still won routing — the only visible consequence was that deleting the new entity silently switched the
old one back on.

`entity.setEnabled`'s equivalent loop has **always** written through `syncEnabledSet`, so the two sites
disagreed about the same rule. `tests/ipc/handlers.test.ts`'s "disables existing mocks with the same
signature when new mock is enabled" could not see it, because it asserts on a mocked in-memory config —
precisely the shape of the bug.

Fixed by diffing the flag across the `onAddConflict` call and writing the differences through
`syncEnabledSet`, which is what the other site already did. Recorded for two reasons: it is a
**behaviour change to the shell** (creating a duplicate mock now really does disable the old one, as
`legacyCompanion.ts`'s comparison table has always claimed it did), and it is what makes the gate on this
path mean anything — a gate guarding a no-op is a false assurance.

`legacyCompanion.ts`'s own `handleMockAdd` is untouched: it has its own body, its own `syncEnabledSet`
for its own id, and never reaches `createEntityCore`. The v1 endpoint's behaviour is unchanged.

### Two smaller decisions worth not re-litigating


1. **`request()` resolves with a handler's value *uninspected*.** `plan/01`'s spike established that a
   handler which *resolves* — with any shape, including `{ok:false, error}` — is an envelope-level
   success. `request()` has exactly one failure path: the registry **threw**. The suite asserts this
   directly, because it is the assertion that keeps P5 honest.
2. **Bus names and wire names are bridged in one validated place.** The bus says `"log.entry"`, the
   protocol says `"event.log.entry"`. The bridge is derived from the protocol's own `EVENT_NAMES` and
   `assertBridgeIsTotal()` runs **at import time**, because the failure mode of getting it wrong is a
   subscription that succeeds and never fires. One engine event, **`process.statusChange`**, still has
   no wire name; it is listed explicitly in `BUS_EVENTS_NOT_ON_THE_WIRE` and subscribing to it is
   refused with `UNSUPPORTED` rather than accepted and silently ignored.

   `process.output` and `settings.changed` were on that list and are not any more — item 3's protocol
   half added `ProcessOutputEventSchema` and `SettingsChangedEventSchema` to `packages/protocol/src/
   events.ts`, taking `EVENT_NAMES` from 7 to 9. `statusChange` was deliberately left behind: its bus
   payload is `{appId, status, [key: string]: unknown}`, an open-ended index signature, and putting it
   on the wire would freeze that shape as a contract for every future status field.

### The matrix is now a sweep, not a sample

The every-command matrix used to sample **one** command (`env.setActive`). It now derives from
`COMMAND_FIXTURES` — the 93-entry valid/invalid payload table — and covers **92** commands (all but the
reserved `subscribe`/`unsubscribe`), for **two assertions each, per transport**: the valid payload must
resolve, and the near-miss invalid payload must reject `BAD_REQUEST`.

The invalid half is the load-bearing one. `COMMAND_FIXTURES`' invalid payloads are otherwise
well-formed — a bad enum member, a missing required field, a negative offset — so a `BAD_REQUEST` there
is evidence that **the frozen Zod schema actually ran on the far side of the transport**. A `null`
payload would have failed every schema for reasons having nothing to do with the transport, and a
transport that let everything through would still have passed it.

Two guards keep the sweep from quietly shrinking, because both failure modes look like a green run:
`MINIMUM_MATRIX_COMMANDS = 50` is a floor (an empty list is how "100% of commands covered" becomes a
claim about nothing), and a namespace-diversity check catches a matrix that narrowed to one namespace
and still cleared the floor. `COMMAND_FIXTURES` lives in the **protocol** package and its own test
asserts it keys `COMMANDS` exactly, so a runner cannot narrow the matrix and the matrix cannot fall
behind the protocol.

### `stdio` specifics

- **Transport control uses reserved *bare* action names** — `subscribe` / `unsubscribe` — because every
  real command is namespaced (`config.get`, `blob.put`). `stdio.ts` **throws at import time** if the
  protocol ever gains a command with one of those names, so the convention is load-bearing rather than
  merely conventional.
- **No `hello`, no auth, no scope.** Work item 4's table says `stdio` needs none: "the pipe is
  inherited and process-scoped". There is no boundary to authenticate across, and a token here would be
  security theatre.
- **`close()` and a fatal error differ, and the difference is the contract.** A graceful `close()`
  leaves the output stream alone (the output is usually `process.stdout`; closing it is not this
  object's decision). A **fatal frame error ends it**, because a peer that is not told waits forever —
  every request in flight hangs with no error and no timeout. `plan/05` work item 5 requires a failed
  session to *close*; the same applies to every unrecoverable frame error, not just auth.
- **`spawnStdioEngine` spawns `process.execPath`, not an `npm` shim.** On Windows, spawning a `.cmd`
  needs `shell: true` (the CVE-2024-27980 fix) and that would put the arguments through a shell.

### Three engine-side enablers this needed

- **`CommandRegistry.invoke()` now throws `EngineError`** — `UNKNOWN_COMMAND` / `BAD_REQUEST` — instead
  of a bare `Error`. Messages are unchanged (so existing `toThrow(/No handler registered/)` assertions
  still hold); what is new is that the code travels with the message.
- **`EngineEventBus` is exported, and so is `ENGINE_EVENT_NAMES`.** `EngineEvents` is a type, so nothing
  could enumerate it at runtime — which meant the bridge check had nothing to validate against. The
  class export exists so a **test** can build an isolated bus; the `bus` singleton is process-wide and
  Vitest shares a module registry, so asserting listener accounting against it makes one test's leak
  look like the next test's bug.
- **`packages/engine/tsconfig.json` now includes `tests/**/*.ts`.** It previously included only
  `src/**/*.ts`, and the root tsconfig includes only `src/**/*`, so `packages/engine/tests/**` was
  checked by **neither** — the same gap that let a missed `filePath` call site through in item 2 of P3.
  Confirmed to have teeth with a negative control. The suite imports its siblings **relatively**
  (`../../src/…`) rather than via `@bifurc/engine/*`, because the package's `exports` map wildcards to
  `./dist/*.d.ts` and would have typechecked the suite against **build output**.

### What running it actually found

The suite had never executed before this change, so this is the first time any of it was tested. Four
things came out of it, and the first is the one that matters for step 3.

1. **The suite's `subscribe()` cases assumed synchronous delivery — fixed, with a negative control.**
   They asserted immediately after the emit:
   ```ts
   bus.emitTyped("server.error", "one");
   expect(seen).toHaveLength(1);      // no await
   ```
   That held for `in-process` (a direct call) and — measured, not assumed — for `PassThrough`, which in
   flowing mode emits `data` synchronously from `write()`:
   ```
   MEASUREMENT  delivery immediately after emitTyped: 1; after settle: 1
   ```
   **A TCP socket is never synchronous**, so the `ws` runner would have failed those cases for a reason
   that is not a defect in `ws`. The interface promises *delivery*, not *synchronous* delivery; the suite
   was asserting an implementation detail of the only two transports that existed.

   The fix is `deliver()` in the shared suite: it emits a **sentinel** on `event.entity.changed` — a
   channel no case uses as its subject — and waits for that to arrive. Delivery is FIFO within a session,
   so a sentinel arriving proves every event emitted before it has already been delivered; in the
   negative cases it proves the event was *skipped* rather than merely slow. No sleep, and no polling for
   the event under test: a `setTimeout(0)` would not do, because a socket's bytes can take several ticks
   and a timeout that is *usually* long enough is a flaky test. Every case that emits now awaits it.

   **The fix was verified by a negative control, not by the two transports that never needed it.**
   `run-deferred-delivery.test.ts` ran the same suite against `createInProcessTransport` wrapped so that
   every event callback is deferred by one macrotask — the one property that actually distinguishes the
   two situations, supplied without a socket, a port or an auth handshake. Neutering `deliver()` and
   re-running fails **exactly six** cases, **all of them on the deferred runner**:
   ```
   Test Files  1 failed | 2 passed (3)     ← the deferred runner, and only it
        Tests  6 failed | 111 passed | 27 skipped (144)
   ```
   `in-process` and `stdio` stayed green throughout, which is the other half of the control: the barrier
   is not papering over anything. Worth noting that the one *negative* case
   ("does not deliver an event the caller did not subscribe to") passes either way — a removed barrier
   cannot fail a `toHaveLength(0)` assertion, which is precisely why the barrier matters there: without it
   that case is not wrong, it is vacuous.

   `run-deferred-delivery.test.ts` was **deleted on 2026-09-18**, the day `run-ws.test.ts` landed — at
   which point the real transport is the control, and a real socket is asynchronous for real reasons
   rather than by construction. It was a test double, exported from nowhere and reachable only by this
   suite. The finding above is what survived it, and the finding is the part that mattered.
2. **The frame decoder retained its buffer after a fatal error.** For an oversized header that is 4
   bytes, but a rejected `invalid_json` frame can hold up to `MAX_FRAME_BYTES`, and the decoder is owned
   by the session object and outlives the stream. Fixed by routing all three failure sites through one
   `fail()` that releases the buffer — the same "one failure path" rule `request()` follows, for the
   opposite reason.
3. **`close()` left the *server's* bus listeners attached.** In spawn mode the pipe ends and the engine
   cleans up, but an **attached** transport does not own the pipe and must not end it, so the engine
   accumulated a listener per closed session until Node's 11-listener warning fired. Fixed: a graceful
   close sends `unsubscribe` for every live name before tearing down.
4. **A server-side fatal error never told the client**, so the client waited forever — the classic
   stdio-transport hang, and invisible until something crashes. Fixed by having `fail()` end the output
   (see the `close()`-vs-fatal bullet above).

`toRpcError` was extracted into `types.ts` at the same time. `in-process` does not need it (nothing
crosses a boundary), but `stdio`, `socket` and `ws` all do, and three copies is the same
"written per-transport instead of shared" drift the risks table lists for the suite.

### ✅ Environment note — the Vitest runner works again

`TESTING.md` §8 carried a banner saying the runner could not bootstrap a worker in this sandbox at all
("Vitest failed to find the runner", on every file including untouched ones). **That is resolved** — the
full suite runs: **2041 tests, 2013 passed, 1 failed, 27 skipped**. The one failure is
`tests/spike/protocolPoc.test.ts` → `soap.execute — a dead/unreachable endpoint never throws through the
envelope`, which is the known pre-existing flaky family (the sandbox intercepts connections to dead
endpoints), not a regression. §8 has been updated.

The two temporary bootstrap scripts (`scripts/verify-transport.cjs`, `scripts/verify-stdio.cjs`) have
been **deleted**: their assertions now live in `tests/transport/framing.test.ts` and
`tests/transport/stdio.test.ts`, and two places holding the same assertions is the drift this plan
warns about.

### Deliberately not done

- **D3's TCP fallback — `socket`'s option (a).** D3 chose the socket *"with (a) as a fallback for
  environments where sockets are unavailable"*, and that fallback is **not implemented**. No such
  environment is known to the programme (Windows has named pipes, POSIX has UDS), and a TCP listener
  would re-open work item 5's bind guards — the ones `ws.ts` carries — for a transport whose entire
  value is *"not network-reachable"*. If it is ever needed it is an additive `{host, port}` target
  behind an explicit opt-in, not a default. `createSocketTransport` takes a path and nothing else.
- **The shared *server* core was not extracted.** The three client halves now share `session.ts`, but
  the two server halves (`ws.ts`, `socket.ts`) remain separate ~250-line blocks that differ mainly in
  the channel. That is deliberate: `stdio`'s server is **session-less by design** — its own `seq`
  counter is a required part of `createStdioServer`, because a pipe has exactly one peer and no
  `EventLog` — so unifying the three would be a *behavioural* change to a tested transport rather than
  a refactor. `ws` and `socket` are near-identical and could be unified; the two-transport cost has not
  yet justified the change, and the honest statement is that this is a known duplication rather than an
  unnoticed one.
- **The `0600` window between `bind()` and `chmod` is not closed.** Closing it needs the process umask,
  which is global mutable state a library must not touch. What is guaranteed is what the caller can
  rely on: `createSocketServer()` does not resolve until the mode reads back private. A caller that
  needs the window gone as well puts the socket in a `0700` directory, which is why
  `defaultSocketPath()` prefers `$XDG_RUNTIME_DIR`.
- **`event.server.error` has no production emitter, and its two declared shapes disagree.** Found while
  making payload shapes load-bearing (item 3). Two separate things, and the second is only decidable once
  the first is fixed:
  - **Nothing in the engine emits it.** Every `bus.emitTyped("server.error", …)` in the repository is a
    **test**. The proxy reports a bind failure on `logEmitter` (`"server-error"`), which
    `src/ipc/eventBridge.ts` forwards to `webContents` — not to the bus. So the event exists in
    `EngineEvents`, in `EVENT_NAMES` and in `EventLog`'s retention set, and no client can ever receive it
    from the engine. `startup.ts`'s comment already describes the intended wiring ("the server reports its
    own bind failure via `bus.emit("server.error", …)` when it actually tries to listen"), which is what
    makes this an unfinished wire rather than a dead name.
  - **The payload shapes disagree.** `EngineEvents` types it `string`; `@bifurc/protocol`'s
    `ServerErrorEventSchema` says `{ error: string }`. Which is right depends on what ends up emitting it,
    so the conformance case pins the **bus** shape (a string) and says why, rather than guessing.

  This is a wiring change in the proxy/startup path, not a transport change, so it is out of item 3's
  scope. It is recorded because the conformance suite's "`server.error` is never dropped" case is
  otherwise a guarantee about an event nothing sends.
- **On Windows a named pipe has no enforced access control, and that is the same gap the token file
  has.** Every permission assertion in `socket.ts` is guarded by `!isWindows`, because a named pipe has
  no mode and no filesystem entry — so it relies on the default DACL of the creating process, which is
  not the same guarantee as an explicit `0600` and **has not been tested**. The 5 POSIX-only cases in
  `tests/transport/socket.test.ts` are skipped on Windows for this reason, not for convenience. Closing
  it means setting an explicit ACL on the pipe, which is P9/P12 work; until then, `auth` is the only
  access control a Windows named pipe actually has.
- **The companion extension's four command names — the decision is taken; the migration waits on the
  extension.** The *file* move is done (`transport/legacyCompanion.ts`), and the security question is
  resolved: conflict resolution is now gated on `admin`, so the four names can be aliased onto
  `config.get` / `entity.create` / `folder.add` without handing a lower-trust caller a destructive
  capability. See "Item 1" above. The other three differences are validation-message wording and two
  missing guards on the folder path — compatibility work, not decisions. What actually holds this open
  is that the **released** v1.0.0 extension cannot speak to an auth-wrapped server at all (no `hello`,
  and a string `error` that `showToast` interpolates), so the migration lands with the extension's v2
  release rather than before it. Until then the process runs two WebSocket servers.
- **No `hello` handling on `in-process` or `stdio`.** A handshake protects a *boundary*; neither has
  one. `createAuthenticatedTransport` owns the handshake, and a client that sends `hello` over an
  unwrapped `stdio` gets `UNKNOWN_COMMAND` — which is the honest answer for a transport with no
  handshake, and is what the `handshake: false` capability encodes. **A universal handshake on every
  transport is an open decision**, not an oversight: a protocol-conforming client would naturally send
  `hello` first everywhere, and if that is wanted it is a change to `in-process` and `stdio`, not to
  the decorator. It is left open because it affects every client and nothing needs it yet.
- **No audit record of RPC calls — and the plan's premise for it is wrong.** Item 4's third bullet is not
  done, but the reason recorded here previously ("only the `onAuthFailure` hook exists") understated it.
  The log is **derived from git** — `queryLog()` parses commit bodies — so an RPC entry would need a
  fabricated `commitHash`/`entity`/`entityId`/`entityName` to fit `AuditEntry`, and `commitMutation`, the
  only function that accepts an `actor`, **has no production caller**. The gap that *was* real — a remote
  write being logged as the engine's own device name, because nothing carried a session into
  `CommandContext` — is now **closed**: `CommandContext.session` carries the identity and the auth
  decorator's `onSession` reports it. What remains is the **record**, which needs per-mutation commits
  plus a granularity decision. See "Item 4's audit bullet" above for the evidence.
- **No flow control, and no per-command timeout.** Coalescing — item 3's other half — is **done**
  (`eventPump.ts`). What remains of item 3 is the **flow-control window** for `log.chunk` /
  `process.output`, which `plan/02` describes as "keep streaming, but add a flow-control window so the
  engine can pause a chatty child process". That is a client→engine **credit**, so it is a protocol
  addition rather than a change to the pump, and the two names are deliberately left streaming
  immediately in the meantime rather than batched: a chunk's value is arriving as it is produced, and
  holding it for 250 ms would trade the feature for the frame count. A request that never returns stays
  pending until the stream dies, at which point it rejects — that is item 5.
- **No rate limiting, no concurrency cap, and no tested TLS path.** Item 5's remaining rows. The two
  *bind* guards are done and encoded, which is what makes the rest safe to defer; `remote mode is not
  reachable` is the current, deliberate state (see item 2 step 3 above).
- **No fan-out semantics beyond one-subscriber-one-token.** Two identical `subscribe()` calls both fire
  and each `unsubscribe` removes only its own; there is no dedupe. `lastSeq` replay **is** done, and so is
  `log.entry` coalescing — see item 3 above.


---

## Preconditions

- `@bifurc/protocol` frozen.
- Engine starts headlessly and exposes a `CommandRegistry` (P2 item 7).
- D3 answered (socket vs ephemeral port).

---

## Work item 1 — Generalise `companionServer.ts` into a real transport

Do not write a new server. `src/companion/companionServer.ts` already implements:

```ts
// client → server
{ id: string, action: string, payload: any }
// server → client
{ id: string, ok: boolean, data?: any, error?: string }
```

bound to `127.0.0.1`, gated by `ALLOWED_ACTIONS` (`src/companion/allowedActions.ts`), with a shipping
consumer: the **companion extension**, now in its own repository (`../bifurc-extension`). Widen it.

Changes needed:
1. **Enrich the envelope.** `error: string` → `error: { code, message, details?, retryable }` (P1 item 5).
2. **Add `hello`** as the mandatory first message (P1 item 7).
3. **Replace the static allowlist** with per-session authorisation (item 4).
4. **Add event delivery** with `seq` (item 3).
5. **Move it out of `src/companion/`** into `packages/engine/src/transport/`. The directory name
   `companion` will make no sense once the browser extension is just another client.

Keep the released companion extension working throughout — it is an **external consumer on its own release cycle** and a free integration
test. Version the protocol so the extension can keep speaking v1 until updated.

---

## Work item 2 — Transport abstraction

The shell needs **embedded**, **spawned** and **remote** to be the same interface. Build all four
transports anyway, even though the Tauri path is parked (D4/D5) — the abstraction is what keeps the shell
replaceable, and it costs one interface plus four thin adapters.

> **Status note (2026-09-14):** the Tauri shell is parked under D4/D5, so `stdio` is needed **now** for
> the CLI (P8), and `ws` for the web UI (P7) and Docker (P9). `in-process` stays as a test transport and
> as the fallback if the shell migration resumes. None of this changes — only the *justification* for
> `stdio` shifts from "Tauri sidecar" to "CLI".

```ts
export interface Transport {
  readonly kind: "in-process" | "stdio" | "ws" | "socket";
  request(cmd: string, payload: unknown): Promise<unknown>;
  subscribe(events: string[], cb: (e: EngineEvent) => void): () => void;
  close(): Promise<void>;
}
```

| Transport | Used by | Notes |
|---|---|---|
| `in-process` | Unit tests, conformance suite; MSIX fallback if P11 resumes | Direct call into the registry. No serialisation. |
| `stdio` | **CLI (P8)** — spawn-on-demand and attached modes | Length-prefixed JSON frames. No ports, no auth needed (inherited pipe). |
| `unix socket` / `named pipe` | Desktop shell spawn (D3 = b) | Preferred: no collisions, not network-reachable. |
| `ws` | **Web UI (P7)**, remote engine, Docker (P9) | Needs auth + TLS. |

**`in-process` is not optional.** It is how the conformance suite runs fast, and it is the fallback if the
shell migration ever resumes and MSIX turns out to be unable to spawn a child.

---

## Work item 3 — Event fan-out, sequencing and replay

The seven existing events plus the new `entity.changed` / `process.output` / `settings.changed`.

```
// engine → client
{ event: "event.log.entry", seq: 48213, payload: { ... } }
```

### Replay on reconnect

This is the most likely source of "works locally, broken remotely" bugs. A brief network blip must not
silently lose log entries or leave the UI showing stale state forever.

```ts
// client reconnects
hello { protocolVersion, lastSeq: 48210 }
  ←   { ..., replayedFrom: 48210 }        // engine buffers and replays
  ←   { ..., resyncRequired: true }       // buffer expired — client must re-fetch state
```

- Engine keeps a bounded ring buffer of recent events per session (e.g. 1,000 events or 5 minutes).
- If `lastSeq` is outside the buffer, the engine sets `resyncRequired` and the client re-fetches
  authoritative state via normal commands rather than replaying events.
- `resyncRequired` **must** be handled by every client. Silently ignoring it produces a UI that is
  permanently wrong in a way nobody can reproduce.

### Backpressure

| Event | Frequency | Policy |
|---|---|---|
| `log.entry` | Per proxied request | **Coalesce** — batch up to 100 entries or 250 ms |
| `log.chunk` | Per stdout/stderr write | Stream, with a flow-control window so the engine can pause a chatty child |
| `process.output` | Same | Same window |
| `sync.status`, `entity.changed` | Low | Send immediately |
| `server.error` | Low | Send immediately, never dropped |

Document the drop policy: under sustained overload, drop `log.entry` before `server.error`.

> **Implemented as of 2026-09-18** by `packages/engine/src/transport/eventPump.ts`, with two deviations
> from this table's literal reading, both argued in "Item 3 — the coalescing half" above:
>
> - **`log.chunk` / `process.output` are sent immediately, not windowed.** The window is a client→engine
>   credit and therefore a protocol addition; until it exists the alternative to streaming is batching,
>   which would trade the feature for the frame count. So they stream, and the window is still open.
> - **The drop policy is satisfied structurally, not by a dropping mechanism.** `server.error` is never
>   queued (so it cannot be displaced), and the pending batch is bounded by `maxBatch` because the
>   cap-triggered flush is synchronous. `tests/transport/eventPump.test.ts` asserts the boundedness —
>   10,000 entries, no batch over 100 — rather than trusting the argument.

---

## Work item 4 — Authentication and authorisation

Nothing like this exists today — the current model is "bind to localhost + 4-action allowlist".

### Authentication

| Transport | Mechanism |
|---|---|
| `stdio` | None needed — the pipe is inherited and process-scoped. |
| `socket` / `named pipe` | Filesystem permissions (0600). Token additionally recommended. |
| `ws` (local) | Random token written to the data dir, read by the shell, sent in `hello`. |
| `ws` (remote) | **Required.** Token or mTLS. TLS on the channel, not optional. |

```ts
hello { protocolVersion, clientName, token }
  ←   { ok: false, error: { code: "UNAUTHORIZED", ... } }   // or
  ←   { ok: true, sessionId, capabilities, ... }
```

Token generation: `crypto.randomBytes(32).toString("base64url")`, rotated per engine start, written
`0600` to `<dataDir>/engine.token`. Docker reads it from an env var or a mounted secret instead.

### Authorisation

A coarse allowlist is not enough for ~85 commands. Add a per-session scope:

```ts
type Scope = "read" | "write" | "execute" | "admin";
```

| Command class | Required scope |
|---|---|
| `*.get`, `*.list`, `*.status` | `read` |
| `entity.*`, `config.save`, `import.*`, `export.*` | `write` |
| `script.execute`, `graphql.execute`, `grpc.execute`, `soap.execute`, `request.replay`, `runner.*` | `execute` |
| `tls.*`, `server.*`, `applications.*` | `admin` |

The companion extension keeps a `write`-only, non-destructive scope — which is what
`ALLOWED_ACTIONS` approximates today.

### Audit

Extend the existing audit machinery (`audit:list`, `audit:diff`, `audit:export`) to record RPC calls:
who, what command, from what address, when, and the outcome. The engine already writes an audit log;
this is an extension, not a new system.

---

## Work item 5 — Remote hardening

| Concern | Action |
|---|---|
| TLS | Required for `ws` beyond loopback. The engine already has a CA and cert machinery. |
| Rate limiting | Per session, per command class. `execute`-scope commands especially. |
| Request size | Enforced at the blob layer (P3) and at the frame level. |
| Timeouts | Every command has one. `soap.execute` against a hung endpoint must not pin a connection forever. |
| Concurrency | Cap in-flight `execute` commands per session. |
| Bind address | **Never default to `0.0.0.0`.** Remote mode is opt-in and explicit. |
| Failure mode | Auth failure returns `UNAUTHORIZED` and closes. Never fall back to unauthenticated. |

---

## Work item 6 — The conformance suite

One suite, run against **every** transport. This is the highest-value test asset in the programme — it
is what lets you add the CLI, the web UI and the Tauri shell without re-testing the protocol each time.

```
packages/engine/tests/conformance/
  protocol.conformance.ts        # transport-agnostic
  run-in-process.ts              # fast, no I/O
  run-stdio.ts
  run-ws.ts
```

Cover:
- `hello` handshake: valid, wrong version (must refuse), missing token (must refuse).
- Every command: valid payload → expected result; invalid payload → `BAD_REQUEST`.
- Every command: unauthorised scope → `FORBIDDEN`.
- Event delivery with monotonic `seq`.
- Reconnect with `lastSeq` inside the buffer → replay.
- Reconnect with `lastSeq` outside the buffer → `resyncRequired`.
- Backpressure: flood `log.entry`, assert coalescing and that `server.error` still arrives.
- Blob round-trip over the transport.
- Timeout behaviour on a deliberately slow command.

---

## How to start — the first three things

1. **Define `Transport` and implement `in-process` + the conformance suite skeleton.** Fastest possible
   feedback loop, and it forces the interface to be right before I/O complicates it.
2. **Add `stdio` next.** It is simpler than WebSocket (no ports, no auth), it is what the CLI and the
   Tauri sidecar will use, and it validates the framing.
3. **Then WebSocket + auth.** Do not ship a `ws` transport before item 4 is done, even to test.

---

## Acceptance criteria

**8 of 10 met as of 2026-09-18**, and **neither open criterion is blocked on a decision any more**:

- **Item 1 — the frozen-API migration.** The decision was taken on 2026-09-18: the conflict resolution is
  scope-gated on `admin`, so the companion keeps its additive-only semantics by construction. What
  remains is an **extension release**, not engine work — the published v1.0.0 sends no `hello` and
  cannot talk to an auth-wrapped server at all.
- **Item 4 — the audit record.** The *attribution* half landed and closed a real defect (a remote write
  used to be logged as the engine's own device name). The *record* half needs per-mutation commits plus a
  granularity decision, which is a durability question rather than a transport one.

- [x] Conformance suite green on `in-process`, `stdio`, `ws` and `socket`. — measured: 5 runners, 285
      tests, **250 passed / 35 skipped / 0 failed**. The 35 skips are 2 per runner for the capabilities
      that runner does not claim, plus the **inverse** pair the session work added — a case a
      handshake-less transport runs where the others skip it. Emitted rather than filtered, so the gaps
      stay visible.
- [x] Wrong protocol version is refused with a clear error. — `UNSUPPORTED` **and closes**; only a
      *major* mismatch is refused (`checkVersionCompatibility()`, never `!==`), and there is a conformance
      case for the minor-mismatch path specifically.
- [x] Missing/invalid token is refused; no unauthenticated fallback path exists. — `UNAUTHORIZED` and
      closes, with the reply flushed **before** the close. Asserted over a real WebSocket in
      `tests/transport/ws.test.ts` and over a real unix socket in `tests/transport/socket.test.ts`. The
      ordering is asserted in both, and it matters *more* on the plain socket: a WebSocket can carry a
      close code, whereas a socket has no way to say why it went away, so the reply is the only signal.
- [x] Scope enforcement verified per command class; `read`-scope cannot mutate. — the whole command
      matrix runs *through* the decorator (`run-authenticated.test.ts`), plus `FORBIDDEN`-stays-open and
      a `read`-only session that reaches reads and nothing else.
- [x] `seq` monotonic across a session; replay works inside the buffer. — done, with one correction to
      the criterion's wording: `seq` is **engine**-scoped, not session-scoped, so a session sees gaps
      for events it did not subscribe to. That is what makes replay survive a reconnect; see item 3.
- [x] `resyncRequired` returned when replay is impossible. — three distinct resync cases (nothing
      retained, backlog expired, `lastSeq` ahead of the engine), each with its own reason.
- [x] `log.entry` coalescing verified under flood; `server.error` never dropped. — **done.** `eventPump.ts`
      batches at 100 entries or 250 ms and the three wire transports claim `backpressure`; the suite's two
      cases assert strictly fewer deliveries than entries (which a *forwarding* transport would fail, so
      the claim has teeth) and that every `server.error` survives a flood. `VERIFIABLE.backpressure` is
      `true`. Two caveats recorded rather than hidden: the **flow-control window** for
      `log.chunk`/`process.output` is still missing, and `event.server.error` still has **no production
      emitter** — see "Deliberately not done".
- [x] Engine binds loopback by default; `0.0.0.0` requires an explicit flag. — and goes further: a
      non-loopback bind requires **both** `allowNonLoopback` and TLS, so it cannot be configured into a
      clear-text remote surface by accident. The `socket` transport's counterpart is its `0600` mode,
      asserted by reading the file back before `createSocketServer()` resolves — and a socket whose
      mode cannot be made private is **refused** rather than served.
- [ ] The **released** companion extension still works against the new server (not a local copy — it is
      a separate repo now). — **the decision that was blocking this is now taken**: the de-duplication is
      scope-gated, so the companion keeps additive-only semantics without a rule written for it (see
      "Item 1" above). What remains is **not a server-side change at all**. The published extension is
      v1.0.0, sends no `hello` and interpolates `resp.error` as a string, so against the auth-wrapped
      server it gets `UNAUTHORIZED` and the connection closes. The only ways to satisfy this criterion
      are an extension release that speaks v2 — which makes the *released* extension the v2 one — or a
      v1-compat mode on the new server, which would break item 5's "never fall back to unauthenticated".
      So the criterion is gated on the **extension's release cycle**, and `legacyCompanion.ts` stays
      byte-compatible on 9271 until then.
- [ ] RPC calls appear in the audit log. — **half done, and the criterion's premise is false.** The
      *attribution* half is done: `CommandContext.session` (`SessionIdentity`) is filled by the auth
      decorator's `onSession` observer, so a mutation over a transport now carries who made it instead of
      logging as the engine's own device name — which was the real defect. The *record* half is not: the
      log is derived from git commits, so an RPC entry would need fabricated `commitHash`/`entity`/
      `entityId`/`entityName` fields, and `commitMutation` has no production caller. It needs
      per-mutation commits and a granularity decision. See "Item 4's audit bullet" above.

---

## Rollback

The transport is additive — the engine works headlessly without it (P2 gate). If a transport proves
wrong, drop it and keep the others; `in-process` and `stdio` are the low-risk pair and `ws` is the one
carrying the security burden.

---

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| Shipping `ws` before auth is complete | Medium | Hard gate: no `ws` transport merged until item 4 passes review |
| Reconnect semantics get half-implemented | **High** | `resyncRequired` is an acceptance criterion, not a nice-to-have |
| `log.entry` volume saturates a remote link | Medium | Coalescing + documented drop policy, tested under flood |
| Event ordering races between `entity.changed` and `sync.entityStatus` | Medium | Single ordered event stream per session |
| Auth token leaks via the data dir on a shared machine | Medium | `0600` perms; document; prefer sockets which use FS permissions natively |
| Conformance suite written per-transport instead of shared | Medium | One suite, five runners — review this explicitly. The runners hold no assertions of their own; `run-socket.test.ts` needed **no** changes to the suite |
