# 05 — Phase 4: Transport layer

**Goal:** a JSON-RPC transport that carries the frozen protocol over multiple channels, with
authentication, session management, event fan-out and reconnect recovery.

**Effort:** 3–4 weeks. **Depends on:** P1 (protocol), P2 (engine + EventBus). **Blocks:** P5, P9.

> **Security note before anything else:** this phase creates the network attack surface. The engine
> generates TLS CAs, executes scripts, spawns child processes and runs mock servers. **An
> unauthenticated remote RPC surface is a remote code execution hole.** Auth is item 4 of this phase, not
> a hardening task in P13.

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

- [ ] Conformance suite green on `in-process`, `stdio` and `ws`.
- [ ] Wrong protocol version is refused with a clear error.
- [ ] Missing/invalid token is refused; no unauthenticated fallback path exists.
- [ ] Scope enforcement verified per command class; `read`-scope cannot mutate.
- [ ] `seq` monotonic across a session; replay works inside the buffer.
- [ ] `resyncRequired` returned when replay is impossible.
- [ ] `log.entry` coalescing verified under flood; `server.error` never dropped.
- [ ] Engine binds loopback by default; `0.0.0.0` requires an explicit flag.
- [ ] The **released** companion extension still works against the new server (not a local copy — it is a separate repo now).
- [ ] RPC calls appear in the audit log.

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
| Conformance suite written per-transport instead of shared | Medium | One suite, three runners — review this explicitly |
