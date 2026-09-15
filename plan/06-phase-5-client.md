# 06 — Phase 5: Typed RPC client

**Goal:** a typed client that reconstructs the `window.api` surface over any transport, so the renderer
needs **zero changes** when the shell switches over.

**Effort:** 1.5–2 weeks. **Depends on:** P4. **Blocks:** P6, P7, P8, P11.

> This phase exists to protect one property: **`window.api` stays byte-identical.** That property is the
> regression signal for the entire critical path. If it holds, P6 is a wiring change. If it breaks, P6
> becomes a renderer refactor and the schedule is gone.

---

## Preconditions

- Conformance suite green on `in-process` and `stdio` (P4).
- Auth implemented for `ws`.
- `renderer/types/window.ts` is the authoritative list of the surface to reproduce.

---

## Work item 1 — Take the existing surface as the spec

`renderer/types/window.ts` already declares the whole API — 136 methods. **Do not redesign it.** Treat
it as the contract.

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
    index.ts            # createClient(transport) → WindowApi
    transports/
      in-process.ts
      stdio.ts
      ws.ts
    events.ts           # subscription multiplexing
    errors.ts           # protocol error → typed Error
```

```ts
export function createClient(transport: Transport): WindowApi {
  return {
    getConfig: () => transport.request("config.get", {}),
    addMock: (mock) => transport.request("entity.create", { kind: "mocks", entity: mock }),
    onLogEntry: (cb) => transport.subscribe(["event.log.entry"], (e) => cb(e.payload)),
    // ...129 more, generated from the protocol where possible
  };
}
```

### Generate, don't hand-write

129 methods is too many to hand-write reliably. Generate the client from the protocol schemas, with a
small hand-written map for the cases where the old signature differs from the new command (e.g. the
CRUD collapse means `addMock` → `entity.create { kind: "mocks" }`).

Keep the hand-written mapping in **one file** so the compatibility shims are auditable.

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

- [ ] `shape.test-d.ts` compiles: the client satisfies `WindowApi`.
- [ ] All 136 methods present, including the 7 subscription methods returning working unsubscribe functions.
- [ ] A script asserting `Object.keys(oldApi)` vs `Object.keys(newApi)` reports zero differences.
- [ ] Client-local methods (dialogs, zoom, theme, titlebar, platform, installCA, first-launch) are
      **not** routed over the transport.
- [ ] `EngineError` carries `code` and `retryable`; mutating commands are not auto-retried.
- [ ] Subscription multiplexing verified: two listeners, one transport subscription.
- [ ] Client works over `in-process`, `stdio` and `ws`.

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
