# Spike results

Evidence for the P0 de-risk spikes. **Fill this in as you go, not afterwards** — the value is in the raw
observations, not a retrospective summary.

Rule: record the raw output (stdout, screenshots, numbers). "It worked" is not evidence.

> **Scope (2026-09-14):** only **spike 4 is required**. Spikes 1 and 3 were parked with the Tauri
> decision (D4/D5); spike 2 is optional. Their sections are kept below so they can be run unchanged if
> the shell migration resumes — mark them `SKIPPED` rather than deleting them.

| Spike | Status | Reason |
|---|---|---|
| 1 — MSIX | ⏸ SKIPPED | D5: Electron stays; electron-builder's APPX already works |
| 2 — Bun compile | 🔸 OPTIONAL | Blocks nothing — run only for a single-binary CLI or P10 |
| 3 — WebKitGTK | ⏸ SKIPPED | D5: Electron ships Chromium |
| 4 — Protocol PoC | ✅ REQUIRED | Gates P1 |

---

## Spike 1 — MSIX: loopback and child-process spawning ⏸ SKIPPED

*Skipped 2026-09-14 under D5. Run only if the shell migration resumes.*

**Dates run:**
**Environment:** Windows build / version, Developer Mode on/off

| Test | Result | Evidence |
|---|---|---|
| Loopback WS to `127.0.0.1:9271` | ☐ pass ☐ fail | |
| Child-process spawn | ☐ pass ☐ fail | |
| Path virtualisation (write outside package) | ☐ pass ☐ fail | |
| Spawning a sibling binary from `extraResources` | ☐ pass ☐ fail | |

**Where did the file actually land when writing outside the package?**

```
<paste actual resolved path here>
```

**Findings:**

**Answers:** D3 = , D5 =

**Temporary spike code removed:** ☐

---

## Spike 2 — Bun compile 🔸 OPTIONAL

**Dates run:**
**Bun version:**

| Dependency | Result | Notes |
|---|---|---|
| `ws` (server + client) | ☐ pass ☐ fail | |
| `mkcert` `createCA` / `createCert` | ☐ pass ☐ fail | pure JS — no native binary |
| `node-forge` (via mkcert) | ☐ pass ☐ fail | |
| `archiver` → `WriteStream` | ☐ pass ☐ fail | |
| `unzipper.Open.file()` | ☐ pass ☐ fail | needs a real path |
| `simple-git` (external `git`) | ☐ pass ☐ fail | |
| `node:child_process` spawn | ☐ pass ☐ fail | |
| `node:crypto` | ☐ pass ☐ fail | |
| Dynamic `require` present? | ☐ yes ☐ no | **check before building** |

| Metric | Value |
|---|---|
| Binary size | |
| Cold start | |
| Peak RSS | |

**Runs with `node_modules` absent:** ☐ yes ☐ no

**Fallback triggered?** ☐ no ☐ Node SEA ☐ node + JS

**Answers:** D2 =

---

## Spike 3 — WebKitGTK CSS ⏸ SKIPPED

**Dates run:**

| Distro | `webkit2gtk` version | `oklch()` | `color-mix()` | TitleBar renders | Screenshot |
|---|---|---|---|---|---|
| Ubuntu 22.04 LTS | | ☐ ☐ | ☐ ☐ | ☐ | |
| Ubuntu 24.04 LTS | | ☐ ☐ | ☐ ☐ | ☐ | |
| Debian stable | | ☐ ☐ | ☐ ☐ | ☐ | |
| Fedora current | | ☐ ☐ | ☐ ☐ | ☐ | |

**Computed values observed:**

```
var(--card) background       →
color-mix(in oklab, …)       →
```

**Affected renderer files** (`oklch` in 7, `color-mix` in 2):
`tokens.css`, `styles.css`, `lib/codemirrorTheme.ts`, `lib/utils.ts`, `TitleBar.tsx`,
`FolderTreeNode.tsx`, `FolderTreeItemNode.tsx`, `ProtocolSelectorTab.tsx`

**Fallback work required:** ☐ none ☐ `@supports` for N files (estimate: ___ days)

**Answers:** D4 =

---

## Spike 4 — Protocol proof of concept ✅ REQUIRED

**Dates run:** 2026-09-15
**Schema library chosen:** Zod (v4.6.5) — validates untrusted wire payloads at runtime and infers TS
types with `z.infer`, matching the P1 recommendation.

**Code:** `spike/protocol/` (`commands.ts`, `envelope.ts`, `transport.ts`, `legacyTransport.ts`,
`client.ts`, `shape.ts`). Verified two ways:
1. **Compile-time** — `npm run spike:typecheck` (`tsc -p spike/tsconfig.json`), zero errors.
2. **Runtime** — `tests/spike/protocolPoc.test.ts`, 12/12 passing, driving the generic envelope through
   the REAL registered handlers (`registerCoreHandlers`, `registerCrudHandlers`,
   `registerFolderHandlers`, `registerGraphqlHandlers`, `registerSoapHandlers`, `registerTlsHandlers`) —
   the same handler modules production `registerIpcHandlers()` calls, with the identical mocking pattern
   already used by `tests/ipc/handlers.test.ts`.

| Command | Type | Works? | Notes |
|---|---|---|---|
| `config:get` | read | ✅ | returns live config through the envelope |
| `mock:add` | mutate | ✅ | generated id + entity round-trips |
| `mock:delete` | mutate | ✅ | |
| `folder:add` | mutate | ✅ | |
| `folder:move` | mutate | ✅ | |
| `graphql:introspect` | network | ✅ | dead-endpoint path never throws through the envelope (sandbox intercepts `127.0.0.1:1` with a synthetic 404 — see `plan/baseline.md` "Environment caveats"; on real CI/desktop this is connection-refused, exercised by the existing integration suite) |
| `soap:execute` | network, slow | ✅ | same caveat as above |
| `tls:generate` | privileged | ✅ | real `mkcert`/`node-forge` CA generation end-to-end; `fs` automocked so no real disk writes in the test |
| `server:status` | read | ✅ | |
| `entity:setEnabled` | mutate | ✅ | kind must be the plural fs kind (`"mocks"`, not `"mock"`) — the same discriminator `folder:add` uses is a different, singular enum; this asymmetry is worth flattening in P1's naming pass |

Plus two envelope-semantics checks: unknown action → `UNKNOWN_COMMAND` (not a throw), invalid payload →
`INVALID_PARAMS` (not a throw). Both pass.

### The critical question

**Did the renderer need zero changes?**

☒ Yes — the thesis holds, for these 10 methods.

Evidence: `spike/protocol/shape.ts` type-checks a `ProtocolClient` (built purely from the Zod command
defs + the legacy-channel adapter) as **mutually assignable** with `Pick<BifurcApi, ...>` for the 10
spike methods, using the renderer's own `renderer/types/window.ts` — with **zero edits to that file or
any renderer source**. The client's method signatures (`addMock(mock)`, `setEntityEnabled(wsId, kind,
id, enabled)`, etc.) are positional and renderer-shaped; the object-shaped wire payload is assembled
inside the client via each command's `toArgs` adapter, which is exactly the "compatibility shim" P5
calls out. If a real transport is dropped in behind `createProtocolClient`, `window.api` can be
replaced verbatim.

No place needed renderer edits. **The thesis is confirmed, not refuted.**

### Other findings

- **Action namespace scales but is not perfectly uniform yet.** `folder.*` uses a singular `FolderKind`
  enum (`"mock" | "request" | ...`); `entity.setEnabled` uses the plural fs-kind string (`"mocks"`,
  `"rules"`, ...). Both are real, both are needed today, but P1's naming pass should decide one
  discriminator and document the mapping, rather than letting the CRUD collapse (work item 2) inherit
  two conventions silently.
- **Error shapes work for real failures**, with a caveat worth freezing into the protocol design: the
  *existing* handlers report most failures as **resolved** `{ok:false, error}` values, not thrown
  errors or rejected promises. The generic envelope must treat "handler resolved, with any shape" as an
  envelope-level success and let the value ride through untouched in `data` — only a thrown/rejected
  handler becomes an envelope-level `INTERNAL` error. This is implemented in `legacyTransport.ts` and is
  the single load-bearing design decision for renderer compatibility (P1 work item 5's error taxonomy
  must layer *on top of* this, not replace it, until every handler is migrated to throw structured
  errors).
- **Long-running commands** (`soap:execute`, `graphql:introspect`) completed synchronously within the
  single request/response envelope in this spike — no progress events were needed for correctness, but
  neither command streams partial results today, so this spike does not exercise that need. P1's event
  model (work item 6) still applies for the truly async cases (`runner:*`, `grpc:execute` streaming).
- **`z.custom<T>()` is a viable escape hatch for `any`-typed renderer payloads** (`mock.add`'s
  `Omit<MockRule, ...>`, `folder.add`'s `Omit<Folder, ...>`) without giving up runtime validation
  entirely — the wire envelope shape (`{ mock: {...} }`) is still checked; the entity's internal shape
  is not. This is a reasonable interim answer for `runner:saveReport`-style `unknown`-typed payloads,
  though P1 should still type the common entity shapes properly rather than making `z.custom` a habit
  (per the R3 risk note in `02-phase-1-protocol.md`).
- A real (non-simulated) generic-indexed-lookup issue surfaced in `legacyTransport.ts`: indexing the
  `commands` const object by the widened `CommandAction` union does not let TypeScript correlate a
  specific `CommandDef`'s `P` with the same `def`'s `toArgs` — an inherent limit of keyed-union lookups,
  not a spike-specific bug. Fixed here with an explicit `as CommandDef<unknown, unknown>` cast at the
  single dispatch site. P1/P2's `CommandRegistry` should design around this from the start (e.g. a
  discriminated dispatch function per command, not a shared generic call site) rather than repeating
  the cast.

**Protocol approach confirmed:** ☒ yes ☐ needs revision

**Spike branch deleted:** ☐ — not deleted. Per this session's scope (implementing the full decoupling
plan on `standalone-engine`, not a disposable spike branch), `spike/protocol/` and
`tests/spike/protocolPoc.test.ts` are kept in version control as the record of this de-risking work and
superseded by the real `@bifurc/protocol` package built in P1. They are clearly marked "SPIKE CODE —
throwaway, do not productionise" in every file header and are not imported by any production code path.
