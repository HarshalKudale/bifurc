# 06 — Phase 5: Typed RPC client

**Goal:** a typed client that reconstructs the `window.api` surface over any transport, so the renderer
needs **zero changes** when the shell switches over.

**Effort:** 1.5–2 weeks. **Depends on:** P4. **Blocks:** P6, P7, P8, P11.

> This phase exists to protect one property: **`window.api` stays byte-identical.** That property is the
> regression signal for the entire critical path. If it holds, P6 is a wiring change. If it breaks, P6
> becomes a renderer refactor and the schedule is gone.

---

## Status — 2026-09-18

**The client is built and all seven acceptance criteria are met.** `packages/client` (`@bifurc/client`),
**5 suites / 54 tests**, all green; `packages/client/src/` is `index.ts` (the 144 methods),
`surface.ts` (the inventory — the spec), `retry.ts`, `subscriptions.ts`, `local.ts`. The repo-wide
suite is unaffected by the addition (see `TESTING.md` §4.13).

The headline property is asserted the way this document asked for it — *"a script asserting
`Object.keys(oldApi)` vs `Object.keys(newApi)` reports zero differences"* — except written as a test
rather than a script, so it runs inside the gate instead of being remembered.
`tests/client.test.ts` imports `src/preload.ts` **for real** and diffs the key sets in both
directions. That test found a real defect on its first run: `close()` was enumerable on the client,
so `Object.keys(client)` returned **145** against the preload's 144. It is now non-enumerable — the
plan's own instrument (`Object.keys`) enumerates *enumerable own* properties, so the plan's choice of
instrument is what defines the surface, and the shell can still call `close()` because
`contextBridge` copies only enumerable own properties. See `index.ts`'s comment.

**Four corrections to this document, each verified against the code rather than assumed:**

| this document says | the code says | why it matters |
|---|---|---|
| `WindowApi`, "136 methods" | **`BifurcApi`**, **144** methods | `renderer/types/window.ts` has never exported `WindowApi`. The counts are stale in both directions — see the next row. |
| `renderer/types/window.ts` is "the authoritative list of the surface to reproduce" | **`src/preload.ts` is**; the type declares **140** | The type disagrees with the runtime on **seven** keys, in **two** directions. A client built to satisfy the type alone would have silently dropped four runtime keys *and* left three methods it must provide looking optional. |
| `createClient(transport): WindowApi` | `createClient(transport, { local })` | The client-local half has no sensible default — a native file picker and an OS trust store cannot be faked, and a default that did nothing would look like a working install. |
| "Timeouts: per-command, **from the protocol**" | the protocol has **no timeout field** | `COMMANDS[action]` is `{ params, legacyChannel }`. Nothing in `packages/protocol` carries a duration. See "What is still open" below. |

### The seven-key gap, which is the finding this phase was worth running for

`src/preload.ts` is the authority and `renderer/types/window.ts` is a secondary check. They disagree
on exactly seven keys:

- **Four the type omits** — `isFirstLaunch`, `completeFirstLaunch`, `getZoomLevel`, `setZoomLevel`.
  Exposed by the preload and implemented in `clientHandlers.ts`, never added to `BifurcApi`. Returning
  plain `BifurcApi` would make them excess properties; dropping them would shrink the runtime surface.
- **Three the type marks optional** — `setTitleBarOverlay`, `getTheme`, `setTheme` are declared `?`
  although the preload always provides them. This direction is the more insidious of the two: a client
  that omitted `getTheme` would satisfy `BifurcApi`, pass the shape test, and fail the first time the
  renderer called it — and "possibly undefined" reads as defensive typing rather than as a hole.

`BifurcApiFull` corrects both (the four added, the three tightened), and `tests/shape.test-d.ts`
asserts the disagreement is **exactly those seven keys** — as a `Record<>` over the key union, so an
eighth difference fails the build rather than quietly widening the gap. The three tightened keys were
found by the *compiler*, not by reading: the first typecheck of `client.test.ts` reported
`TS2722: Cannot invoke an object which is possibly 'undefined'` on three lines.

### Work item 5 — the retry policy needed a third term

`plan/06`'s own rule for retry is `retryable` **and** the command's idempotency, and both halves are
implemented and tested (`retry.ts`). Two things had to be added to make that policy correct rather
than merely plausible:

1. **It has to be applied.** The first cut had `retry.ts` complete, tested and **never imported** —
   a policy that nothing calls passes every test in isolation and retries nothing in production. Every
   request now goes through one `withRetry` call, including the two internal ones (`config.get` for the
   three `list*` methods, `blob.read` for artifact egress) that are not surface methods.
2. **`isSafeToRetry` alone would hammer a dead transport.** All four transports reject a request on a
   **closed** transport with `ENGINE_ERROR` (`inProcess.ts`'s `assertOpen`, `stdio.ts`'s `teardown`,
   `ws.ts`'s, `socket.ts`'s), and the protocol's `RETRYABLE_CODES` **includes** `ENGINE_ERROR`. So the
   two-term policy retries a permanently-dead transport three times over 150ms. This contradicts the
   same document's "connection loss mid-request: reject with a typed error; **the caller decides
   whether to retry**". A closure flag is the guard; the **fix** is a distinct non-retryable code for a
   closed transport, which is a protocol change plus seven call sites across four transports and
   belongs with `plan/05`'s item 5 hardening. Not done here — recorded so it is a decision, not an
   oversight.

### What is still open

- **Per-command timeouts** (work item 5's last bullet) are **not** implemented. `plan/05`'s own
  `socket.ts` and `stdio.ts` headers both say "no per-command timeout — work item 5's remaining row",
  and `ws.ts` has only a *connect* timeout (`DEFAULT_CONNECT_TIMEOUT_MS`), so no transport applies one.
  This document says the durations come "from the protocol"; they do not, and there is nowhere to put
  them. Adding `timeoutMs` to `CommandSpec` is the right home — `plan/05`'s hardening needs the same
  table, so a client-only copy would be the second copy — but it is a protocol change, so it is left
  as an explicit decision rather than taken in passing. Today a hung `soap.execute` still hangs.
- **`renderer/types/window.ts` is imported by relative path** across the package boundary
  (`packages/client/src/index.ts` → `../../../renderer/types/window`), which the Risks table below
  flags. It compiles and resolves in both `tsc` and the built `dist/`, so it is not yet a problem.
  Moving the type to `@bifurc/protocol` or a `packages/contract` would require **editing
  `renderer/types/window.ts`**, which the non-negotiables forbid for P1–P6 — so the relative path is
  the compliant choice until P7 at the earliest.

---

## Preconditions

- Conformance suite green on `in-process` and `stdio` (P4).
- Auth implemented for `ws`.
- `renderer/types/window.ts` is the authoritative list of the surface to reproduce. — **corrected
  2026-09-18: `src/preload.ts` is the authority.** The type declares 140 of the 144 runtime keys and
  marks three more optional, so it is a *secondary check*, not the spec. See the status section.

---

## Work item 1 — Take the existing surface as the spec

`renderer/types/window.ts` already declares the whole API — **144 methods, not the 136 this document
said** (see the status section; the type declares 140 and the preload exposes 144). **Do not redesign
it.** Treat it as the contract — but take the **runtime** surface, not the declared one, as the thing
to reproduce.

Two shapes appear in the current preload:

```ts
// 1. Request/response — 129 methods
getConfig: () => ipcRenderer.invoke("config:get")
addMock: (mock: unknown) => ipcRenderer.invoke("mock:add", mock)

// 2. Subscription — 7 methods, returning an unsubscribe function
onLogEntry: (cb) => {
  const handler = (_, entry) => cb(entry);
  ipcRenderer.on("log:entry", handler);
  return () => ipcRenderer.off("log:entry", handler);
}
```

The client must reproduce both, including the **unsubscribe return value** — several renderer effects
depend on it for cleanup, and silently returning `undefined` leaks listeners.

---

## Work item 2 — Build the client

```
packages/client/
  src/
    index.ts            # createClient(transport, { local }) → BifurcApiFull (+ close)
    surface.ts          # the 144-key inventory, classified — THIS is the spec
    retry.ts            # idempotency partition + the retry loop
    subscriptions.ts    # subscription multiplexing (ref-counted hub)
    local.ts            # ClientLocal — the client-local half, injectable
```

The transports are **not** in this package. They are `@bifurc/engine/transport/*` and the client is
built against the `Transport` interface only, so `in-process`, `stdio`, `ws` and `socket` are already
covered — a `transports/` directory here would be four copies of what P4 built.

```ts
export function createClient(transport: Transport, opts: CreateClientOptions): BifurcClient {
  return {
    getConfig: () => call("getConfig", {}),
    addMock: (mock) => call("addMock", { kind: "mocks", entity: mock }),
    onLogEntry: (cb) => hub.subscribe("event.log.entry", cb),
    // ...141 more, with the command resolved through `SURFACE` where it can be
  };
}
```

### Generate, don't hand-write — where it was possible, and where it was not

The command resolution **is** generated: every one of the 144 methods looks its command up in
`surface.ts`, so the mapping cannot drift from the classification, and `assertSurfaceIsTotal()` keeps
the classification honest. What could not be generated is **the payload**: `window.api` is
positional while commands take objects, and no data table can express
`(wsId, kind, id) → { workspaceId, kind, id }` with type safety. So the one-liners are hand-written
and the *return type annotation* (`const api: BifurcClient = …`) makes the compiler the shape test.

The compatibility shims live in **one file** (`surface.ts`), which is what this section asked for —
`SURFACE`'s `shim` and `entity` kinds are the auditable list.

---

## Work item 3 — Compatibility shims

The old surface has several methods whose names and shapes differ from the new commands. Every shim is a
place where behaviour can drift, so keep them explicit and tested.

| Old `window.api` | New command | Shim |
|---|---|---|
| `addMock` / `updateMock` / `deleteMock` | `entity.create` / `update` / `delete` | inject `kind: "mocks"` |
| `addRule`, `addWebhook`, `addWsConnection`, … (×12 kinds) | same | inject the right `kind` |
| `exportData` | `export.create` + blob fetch | client-side two-step |
| `preflightImport` / `importData` | `blob.put` + `import.preflight` / `import.commit` | client-side three-step, carries `blobId` |
| `openFileDialog` | **client-local** | not a transport call at all |
| `pickFilePath`, `pickFolderPath` | **client-local** | |
| `setTitleBarOverlay`, `getZoomLevel`, `setZoomLevel`, `getTheme`, `setTheme` | **client-local** | |
| `platform` | **client-local** | was `process.platform` |
| `tlsInstallCA` | **client-local** | OS trust store operation |
| `isFirstLaunch`, `completeFirstLaunch` | **client-local** | shell launch state |

**`window.api.platform` is dead code.** `renderer/types/window.ts:193` declares it and nothing reads it
(verified). Keep it for shape-compatibility if you like, but do not wire it to anything.

---

## Work item 4 — Type-level conformance check

The property that makes P6 safe. Add a compile-time assertion:

```ts
// packages/client/tests/shape.test-d.ts
import type { WindowApi } from "../../../renderer/types/window";

const client = createClient(fakeTransport);
// Fails to compile if the client is missing a method or has the wrong signature
const _check: WindowApi = client;
```

This turns "the renderer doesn't need changing" from an intention into a build failure. **It is the
single most valuable test in the phase.**

Note: `renderer/types/window.ts` must be importable from the client package. If that requires moving the
type into `@bifurc/protocol` or a shared `packages/contract`, do that — a shared type is better than a
relative path across package boundaries.

---

## Work item 5 — Error handling and retry

> **Done as of 2026-09-18, with two deviations — see the status section.** The `EngineError` shape
> below already existed in `@bifurc/protocol` (`code`, `retryable`, `details`), so nothing had to be
> built for it; the retry *policy* is `retry.ts` and it is applied to every request. The two
> deviations: the policy needs a **third** term (a closed transport rejects with the retryable
> `ENGINE_ERROR`, so a closure guard is required — the real fix is a protocol change), and
> **per-command timeouts are not implemented** because the protocol has no field to carry them.

```ts
// protocol error → typed Error
class EngineError extends Error {
  code: ErrorCode;
  retryable: boolean;
  details?: unknown;
}
```

- `retryable: true` → the client retries with backoff (respecting the command's idempotency).
- **Not every command is safe to retry.** `entity.create` will duplicate on retry. Mark mutating commands
  as non-idempotent and do not auto-retry them.
- Timeouts: per-command, from the protocol. A hung `soap.execute` must surface as `TIMEOUT`, not a
  spinner forever.
- Connection loss mid-request: reject with a typed error; the caller decides whether to retry.

---

## Work item 6 — Event subscription multiplexing

The renderer subscribes to the same event from multiple components (e.g. `onLogEntry` in several
panels). Do not open one transport subscription per call.

- Reference-count subscriptions per event name.
- Subscribe on first listener, unsubscribe on last.
- Fan out to all listeners in one place.
- Preserve the unsubscribe-function contract exactly — renderer effects rely on it.

---

## How to start — the first three things

1. **Write `shape.test-d.ts` first**, before the client exists. Let it fail to compile. It defines the
   target precisely and prevents the surface drifting while you build.
2. **Implement the client against the `in-process` transport.** No I/O, instant feedback, and it proves
   the method mapping without debugging sockets.
3. **Port the preload bridge in a scratch file** and diff the two surfaces programmatically — a script
   that enumerates `Object.keys(window.api)` on both sides catches omissions that reading cannot.

---

## Acceptance criteria

All seven met as of 2026-09-18. The evidence is named so a later reader can re-run it rather than
trust the tick.

- [x] `shape.test-d.ts` compiles: the client satisfies `WindowApi`. — **`tests/shape.test-d.ts`**, and
      the type is `BifurcApi`, not `WindowApi`. Checked by `packages/client/tsconfig.json`
      (`include: ["tests/**/*.ts"]`), **not** by the root `tsc` leg, whose `include` is `src/**/*`.
      Four deliberate violations were compiled against it and produced exactly the four expected
      errors, so the assertions are load-bearing rather than vacuous.
- [x] All **144** methods present, including the 7 subscription methods returning working unsubscribe
      functions. — the "136" is stale. `tests/surface.test.ts` (kind counts 73/48/7/3/13),
      `tests/client.test.ts` (key diff against the real preload), and `shape.test-d.ts` (the
      unsubscribe contract, per method).
- [x] A script asserting `Object.keys(oldApi)` vs `Object.keys(newApi)` reports zero differences. —
      `tests/client.test.ts`, written as a test rather than a script. Set equality in both
      directions, so a swap (one added, one dropped) fails. This is what found the enumerable
      `close()`.
- [x] Client-local methods (dialogs, zoom, theme, titlebar, platform, installCA, first-launch) are
      **not** routed over the transport. — `tests/client.test.ts` calls all twelve and asserts
      **zero** transport calls. `surface.ts` classifies them `local` and `commandFor()` **throws**
      for a non-command kind, so routing one is a bug rather than a missing case.
- [x] `EngineError` carries `code` and `retryable`; mutating commands are not auto-retried. — the
      protocol's `EngineError` already had both (`get retryable()` → `isRetryable(this.code)`), and
      all four transports reject with a real one. The retry policy is `tests/retry.test.ts` (17) plus
      `tests/client.test.ts` (5, proving it is **applied**). See "the retry policy needed a third
      term" above for the closed-transport gap.
- [x] Subscription multiplexing verified: two listeners, one transport subscription. — 
      `tests/subscriptions.test.ts` (9), covering ref-counted teardown, idempotent unsubscribe,
      fan-out isolation, and mid-dispatch removal.
- [x] Client works over `in-process`, `stdio` and `ws`. — `tests/transports.test.ts`, one case per
      transport, all three running the **same** assertion body so they cannot drift apart. `ws` is
      the only one built with `auth`, and the handshake is a plain `request("hello", …)` on the raw
      transport before `createClient()` — so the client never learns it is remote, which is what P6
      needs. Removing the handshake fails the `ws` case with `UNAUTHORIZED` and leaves the other two
      passing (verified), so the gate is genuinely exercised.

---

## Rollback

The client is additive and unused until P6. If the shim approach proves wrong, the client can be
regenerated — nothing depends on it yet.

---

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| Hand-writing 129 methods misses some | **High** | Generate from protocol + the programmatic key diff |
| The CRUD collapse changes a signature subtly (`addMock` returns the entity, `entity.create` returns `{id, entity}`) | High | Shim in one auditable file; test each shim |
| Subscription cleanup contract broken → listener leaks | Medium | Explicit test: subscribe, unsubscribe, assert no further deliveries |
| `WindowApi` type not importable across package boundaries | Medium | Move it to a shared package rather than using a relative path |
| Auto-retry duplicates entities | Medium | Idempotency flags per command; no auto-retry on mutations |
