# Tab & per-protocol action seam — analysis

**Scope:** how a single request tab renders REST / GraphQL / gRPC / SOAP today, how the GUI's
per-protocol components are separated, how the engine's per-protocol actions are separated, and what
a "one container per tab type + one handler per type" design would actually cost.

**Method:** read-only. No file was modified. Everything below cites a file and a line range.

**Verdict up front, because it changes the plan:** the engine half of the proposal is *already
built* — four separate registered handlers in three separate `protocols/` files, dispatched by a
`Map` lookup, no if/else ladder anywhere on the request path. The GUI half is where the conditionals
live, and there the cost is **code shape, not render cost** — the four `{activeProtocol === …}`
blocks are a *switch*, so exactly one protocol component is mounted per tab.

---

## 0. The one-screen map

```
RequestsPanel.tsx                        owns tab list, empty state, side rails, modals
└── RequestTabContent.tsx                ONE per open tab — picks the protocol  ← 4 conditionals
    └── {Rest,GraphQL,Grpc,Soap}Tab.tsx  ONE per tab; owns that tab's reducer state
        ├── useProtocolEditor()          the SHARED state machine (reducer+draft+dirty+autosave)
        └── BottomBar (editor/RequestTab.tsx)   the SHARED footer — 1 component, 4 call sites

useRequestsPanelState.ts                 tabLabel() / tabBadge()  ← 7 conditionals
useMultiplexedEntities.ts                CRUD ladder             ← 18 conditionals
```

Engine, for the request path:

```
window.api.replayRequest      → request.replay    → miscCommands.ts:201   → proxy/serverReplay.ts
window.api.graphqlExecute     → graphql.execute   → protocols/graphqlCommands.ts:97
window.api.soapExecute        → soap.execute      → protocols/soapCommands.ts:81
window.api.grpcExecute        → grpc.execute      → protocols/grpcCommands.ts:50  (stub)
```

Four commands, four handlers, four files (three of them under `protocols/`). One Electron channel
carries all of them: `engine:rpc`, with the command name inside the frame.

---

## Part A — How one tab renders four protocols

### A.1 A tab's identity does not include its protocol

`useEntityTabs` (`packages/gui/src/hooks/useEntityTabs.ts`) is protocol-blind. It manages three
things and nothing else:

| state | where | shape |
|---|---|---|
| `openTabs` | `usePersistedState` → `requests:openTabs` | `string[]` of ids |
| `activeTab` | `usePersistedState` → `requests:activeTab` | `string \| null` |
| `loadedEntities` | `useState` | `Record<tabId, entity>` |

A tab id is one of:

- an **entity id** — the saved request's id;
- a **draft id** — recognised purely by prefix. `useRequestsPanelState.ts:62-70` configures
  `draftPrefix: "req-draft-"` plus `extraDraftPrefixes: ["pending-", "gql-req-draft-",
  "grpc-req-draft-", "soap-req-draft-", "runner-"]`.

So the protocol is **not** part of the tab's identity; it is derived, and derived from two different
places depending on whether the tab is saved:

- **Saved tab** → `itemProtocolMap.get(tabId)`, built in `useMultiplexedEntities.ts:36-43` by
  asking *which `AppConfig` array the id appears in* (`config.requests` → `"rest"`,
  `config.graphqlRequests` → `"graphql"`, `config.grpcRequests` → `"grpc"`,
  `config.soapRequests` → `"soap"`).
- **Draft tab** → `draftProtocols[tabId]`, a separate persisted map
  (`useRequestsPanelState.ts:57-60`), written when the user picks a card.
- **Neither** → `"rest"` by default (`RequestsPanel.tsx:199`).

That is the first structural fact worth naming: **the protocol is a lookup result, not a field.** It
is exactly the shape a descriptor registry wants to consume.

### A.2 What is rendered, and how many things are mounted

`RequestsPanel.tsx:139-238` maps `openTabs` and renders one `<RequestTabContent>` per tab, each
wrapped in `style={{ display: activeTab === tabId ? "flex" : "none" }}`. **Every open tab stays
mounted**; only the active one is visible. (This is deliberate and load-bearing — it is why the
export rail gates its push on `exportOpen && activeTab === tabId`: an ungated push would lift every
keystroke in every editor into the parent's state.)

Inside a tab, `RequestTabContent.tsx:56-179` renders **exactly one** of four components:

```tsx
{activeProtocol === "rest"    && <RestTab    … />}   // :56
{activeProtocol === "graphql" && <GraphQLTab … />}   // :89
{activeProtocol === "grpc"    && <GrpcTab    … />}   // :120
{activeProtocol === "soap"    && <SoapTab    … />}   // :150
```

Three further branches sit in `RequestsPanel.tsx`:

- runner tab → `<CollectionRunner>` (`:146-175`) — a separate concept, not a protocol;
- draft tab with no protocol yet → `<ProtocolSelectorTab>` (`:182-197`);
- zero tabs open → the *same* chooser, doubling as the empty state (`:139-143`).

### A.3 The leaf components are already fully separated

| protocol | component | lines | own folder contents |
|---|---|---|---|
| REST | `components/rest/RestTab.tsx` | 351 | `restTabReducer.ts`, `RestEditorPane.tsx`, `RestCurlImport.tsx`, `useRestActions.ts`, `CollectionRunner.tsx`, `useCollectionRunner.ts`, `RunnerConfig/Progress/Results.tsx`, `restTabHelpers.ts` |
| GraphQL | `components/graphql/GraphQLTab.tsx` | 334 | `graphqlTabReducer.ts`, `SchemaExplorer.tsx` |
| gRPC | `components/grpc/GrpcTab.tsx` | 317 | `grpcTabReducer.ts`, `GrpcLeftPane.tsx`, `GrpcResponsePane.tsx`, `ProtoExplorer.tsx` |
| SOAP | `components/soap/SoapTab.tsx` | 352 | `soapTabReducer.ts`, `WsdlExplorer.tsx` |

Each one, in its own file:

1. calls the **shared** state machine — `useProtocolEditor({ reducer, initState, stateToDraft,
   isDraftEmpty, stateToSavePayload, … })` (`hooks/useProtocolEditor.ts`, 201 lines: draft
   persistence, dirty diffing, the debounced autosave, sync/revert, `saveNow`);
2. owns its **own** `handleSend` calling its **own** `window.api.*` method
   (`RestTab.tsx:152` → `useRestActions.ts:37` `replayRequest`; `GraphQLTab.tsx:123`
   `graphqlExecute`; `SoapTab.tsx:124` `soapExecute`; `GrpcTab.tsx:131` `grpcExecute`);
3. renders its **own** panes and sub-tabs;
4. renders the **shared** `BottomBar` / `UrlBar` / `TabStrip` from
   `components/editor/RequestTab.tsx` (322 lines — 1 component, 4 call sites).

**So "each type should have its own Tab component" is already true.** What is missing is not the
components — it is a *descriptor* that lets the parent select one without a conditional, plus a
uniform prop contract so the selection is type-checked.

### A.4 Where the conditionals actually are

33 protocol-string comparisons across 6 files:

| file | count | what |
|---|---|---|
| `hooks/useMultiplexedEntities.ts` | 18 | `resolveEntityKind` (`:116-118`), `handleSaveEntity` (`:124-140`), delete (`:148-150`, `:160-162`), duplicate (`:180-182`), update (`:198-200`) |
| `panels/requests/useRequestsPanelState.ts` | 7 | `tabLabel` (`:191-198`), `tabBadge` (`:211-214`) |
| `panels/requests/RequestTabContent.tsx` | 4 | the four render blocks |
| `panels/mocks/MockTabContent.tsx` | 1 | REST-only since `plan/17` WI 8 |
| `panels/mocks/useMocksPanelState.ts` | 1 | tab label |
| `components/editor/ProtocolSelectorTab.tsx` | 1 | the mock-mode REST-only filter (`:99`) |

The worst of these is **`RequestTabContent`**, and it is worse than the count suggests: four blocks
of ~30 lines each (~120 lines) that differ in only three places — the component, the protocol string
in `onSave`, and the `kind` in the `onRevert` → `loadEntity` call. Everything else
(`onSync`, `onRevert`'s body, `onHistory`, `syncStatus`, `ref`, `draftTabId`, `folders`, `activeEnv`,
`onClose`, `onDirtyChange`) is byte-identical copy-paste.

`useMultiplexedEntities` is the largest, and it is a genuine ladder: five operations × four
protocols, each with an `if / else if / else if` chain over `window.api` method names.

### A.5 The house pattern for "one family, N variants" already exists — three times

The codebase already knows how to do this. The descriptor/registry shape appears in:

1. **`ProtocolSelectorTab.tsx:10-19`** — `interface ProtocolCard` + a local `cards: ProtocolCard[]`
   array holding `{id, title, badge, badgeColor, badgeBg, icon, description, features}`. It is a
   protocol registry, but it is *local to the component* and not exported, so nothing else can
   consult it — the chooser and the dispatch cannot currently disagree, but only because they cannot
   see each other.
2. **`lib/utils`** — `methodColor(badge)` / `methodBg(badge)`, a string-keyed table (already
   protocol-agnostic).
3. **`useProtocolEditor`** — a generic factory parameterised by the reducer quartet. The four tabs
   differ only in the four functions they pass.

And in the engine, the same pattern is the *norm* (see Part B).

---

## Part B — How the engine's per-protocol actions are separated

### B.1 The dispatch mechanism is a Map, not a ladder

`packages/engine/src/commands/registry.ts:176-236`:

```ts
class CommandRegistry {
  private readonly handlers = new Map<CommandAction, CommandHandler>();
  register(action, handler) { … }          // throws on duplicate, on purpose
  invoke(action, payload, ctx) {
    const handler = this.handlers.get(action);          // ← Map lookup
    if (!handler) throw new EngineError("UNKNOWN_COMMAND", …);
    const parsed = getCommandParamsSchema(action).safeParse(payload);
    if (!parsed.success) throw new EngineError("BAD_REQUEST", …);
    return handler(parsed.data, ctx);
  }
}
```

Every command is validated against a frozen Zod schema **before** its handler runs. There is no
`switch (protocol)` anywhere on this path.

- **89 commands** declared in `packages/protocol/src/commands/index.ts:66-188` (`COMMANDS`:
  name → params schema + `legacyChannel`).
- Registered by **18 `register*Commands()` + 5 `register*Kinds()`** calls in
  `packages/engine/src/registerAll.ts:136-156`, mirrored explicitly in
  `src/ipc/handlers.ts#registerIpcHandlers()`.
- **89 entries** in `SCOPE_BY_COMMAND` (`packages/engine/src/transport/auth/scopes.ts:63-158`),
  guarded by `assertScopesAreTotal()`.

### B.2 Per-protocol command files already exist — for three of the four

| protocol | command | file | notes |
|---|---|---|---|
| GraphQL | `graphql.introspect`, `graphql.execute` | `packages/engine/src/protocols/graphqlCommands.ts` | builds its own `http`/`https` request inline; carries the introspection query as a string constant (the engine has no GraphQL dependency) |
| SOAP | `soap.fetchWsdl`, `soap.execute` | `packages/engine/src/protocols/soapCommands.ts` | builds its own `http`/`https` request inline; `SOAPAction` header; `text/xml` |
| gRPC | `grpc.execute`, `grpc.reflect` | `packages/engine/src/protocols/grpcCommands.ts` | deliberate stubs resolving `{ok:false, error:"gRPC runtime not yet configured…"}` |
| **REST** | **`request.replay`** | **`packages/engine/src/miscCommands.ts:201-203`** | delegates to `proxy/serverReplay.ts#replayRequest` |

**This is the real asymmetry, and it is the thing that reads like an inconsistency.** REST's send
handler lives in `miscCommands.ts` next to `healthbar.getServices`, `healthbar.checkUrl` and
`script.execute` — three unrelated commands. Its three siblings live under `protocols/`.

Note also that the four handlers are *not* interchangeable in behaviour, and that is preserved
deliberately:

- `replayRequest` strips hop-by-hop headers and decompresses (`serverReplay.ts:43-49`), and uses
  `agent: false` so `connection: close` is honoured — a documented fix for spurious
  `HPE_CLOSED_CONNECTION` 400s.
- `graphql.introspect` and `soap.fetchWsdl` **resolve** `{ok:false, error}`; `graphql.execute`,
  `soap.execute` and `replayRequest` **throw**. That asymmetry is pinned by
  `tests/integration/protocolExecution.integration.test.ts` — a resolved `{ok:false}` is an envelope
  *success* carrying a failed result, a throw becomes a transport-level error code. Turning one into
  the other is a contract change, not a cleanup.

### B.3 The wire layer: one channel, command name inside the frame

`src/ipc/rpcContract.ts:48` — `RPC_CHANNEL = "engine:rpc"`. `src/ipc/rpcBridge.ts:217` handles it and
calls `session.request(cmd, payload)` where `session` is `createInProcessTransport(commandRegistry)`.

So **"proper different IPC commands" is already satisfied at the command-name level**, and a
per-protocol *channel* would be a regression:

- the same frame shape is what `socket`, `ws`, `stdio` and `in-process` carry — a channel-per-protocol
  design exists only on Electron and would break that uniformity;
- P6 spent six reduction groups deleting exactly this kind of per-namespace channel sprawl
  (`graphql:execute`, `soap:execute`, `grpc:execute`, the 17 in `coreHandlers.ts`, …). 28
  `ipcMain.handle` sites remain in `src/ipc/` and they are the *residue*, not the target.

### B.4 The second dispatch axis: entity CRUD (one command, sixteen kinds)

`packages/engine/src/store/entityCrudCommands.ts` is the other place a "ladder" could have lived,
and it does not:

```ts
const entityCrudRegistry = new Map<string, CrudFactoryOpts<any>>();   // :130
const simpleEntityKinds   = new Set<string>();                        // :137
```

`entity.create` / `entity.update` / `entity.delete` / `entity.list` are **four commands serving all
kinds**, each doing a `Map.get(engineKind)` (`:442`, `:467`, `:474`, `:481`) and throwing
`entity.create: kind "x" is not routed through the CommandRegistry yet` when the lookup misses.

The kinds are registered **per protocol**, in five files:

```
packages/engine/src/store/kinds/graphqlKinds.ts
packages/engine/src/store/kinds/grpcKinds.ts
packages/engine/src/store/kinds/soapKinds.ts
packages/engine/src/store/kinds/crudKinds.ts     (rest/mocks/mappings/rules/ws/webhooks/environments…)
packages/engine/src/store/kinds/coreKinds.ts     (environments)
```

Kind-name translation is one tiny table: `commands/entityKindMap.ts` (`rules ↔ proxyRules`,
`sockets ↔ wsConnections`).

**Residual conditionals inside the cores** — `opts.kind === "rules" | "mocks"` (prepend, `:263`;
`entity.changed` ping, `:301`), `opts.kind === "mappings"` (proxy-rule cascade, `:362`),
`opts.kind === "environments"` (the `"__global__"` guard `:345`, active-env reset `:369`). These are
five, they are documented as preserved shell behaviour, and they are **not** a protocol ladder — they
are per-kind quirks that a registry entry could absorb (`CrudFactoryOpts` already has `isFlat`,
`hasEnabledState`, `validate`, `onAddConflict`, `getNameEntry`, `gateKind`).

### B.5 The third axis: the client surface — this is where the ladder the plan senses really lives

`packages/client/src/surface.ts` (391 lines) is **the spec**: 132 keys, each classified
`transport` / `entity` / `shim` / `local` / `subscribe`, with `assertSurfaceIsTotal()` failing in
both directions.

`packages/client/src/index.ts` (1053 lines) is the implementation, and it is hand-written per
protocol:

```ts
addRequest:         async (r) => unwrapCreated(await call("addRequest",         { kind: "requests",         entity: r })) …
addGraphQLRequest:  async (r) => unwrapCreated(await call("addGraphQLRequest",  { kind: "graphqlRequests",  entity: r })) …
addGrpcRequest:     …                                     { kind: "grpcRequests",  … }
addSoapRequest:     …                                     { kind: "soapRequests",  … }
```

Four method names, four `kind` strings, one command (`entity.create`). Same for `update*`/`delete*`.
The *network* methods are already one-per-protocol and 1:1 with a command
(`graphqlExecute:852`, `soapExecute:847`, `grpcExecute:853`, `replayRequest` via `t("request.replay")`).

`commandFor(key)` (`index.ts:125-140`) already reads `SURFACE` to resolve the command — so the
*spec* is a table; only the payload assembly is hand-written.

### B.6 What adding one command costs (the honest price of "proper different IPC commands")

| # | touch point | guard that fails if you forget |
|---|---|---|
| 1 | `packages/protocol/src/commands/<family>.ts` — params schema | — |
| 2 | `packages/protocol/src/commands/index.ts` — the `COMMANDS` entry (89) | `isKnownCommand` → `UNKNOWN_COMMAND` |
| 3 | `packages/protocol/src/commands/fixtures.ts` — one valid + one invalid payload (~88 entries) | the conformance matrix in `packages/engine/tests/conformance/` |
| 4 | `packages/engine/src/transport/auth/scopes.ts` — `SCOPE_BY_COMMAND` (89) | `assertScopesAreTotal()` |
| 5 | a `register*Commands(registry)` module + the handler | `UNKNOWN_COMMAND` |
| 6 | `packages/engine/src/registerAll.ts` **and** `src/ipc/handlers.ts` (kept honest by `tests/integration/workspaceHost.integration.test.ts`) | a headless host answers `UNKNOWN_COMMAND` |
| 7 | `packages/client/src/surface.ts` + `index.ts` + `packages/gui/src/types/window.ts` (if on the `window.api` surface) | `assertSurfaceIsTotal()`, `tests/shape.test-d.ts` |

Seven files for one command. That is the design working as intended, but it is the number to have in
mind before proposing "a command per protocol per operation".

---

## Part C — Assessment of the proposed design

### C.1 "Different tab type containers instead of same tab parent with conditional stuff — that's not performative"

**The performance premise does not hold, and it is worth being precise about why.** The four blocks
in `RequestTabContent` are guarded by `&&`, so exactly **one** protocol component is mounted per tab.
They are a switch, not a both-and. There is no double-render to reclaim.

What *is* mounted when inactive is the **other tabs** (`display: none`), and that is a deliberate,
documented design — it is why the export-rail push is gated on `activeTab === tabId`. Splitting the
tab's interior into four containers does not change that, and changing *it* would be a different
(and riskier) proposal.

So the honest framing for this refactor is:

- **real gain:** ~120 lines of near-identical prop plumbing collapse to one; adding a fifth protocol
  becomes one descriptor instead of six file edits; the chooser and the dispatch stop being able to
  disagree;
- **not a gain:** render performance. If a performance target is the actual goal, it needs a
  measurement first — and the more likely candidates are the ungated-push class of problem the skill
  file already documents, not the mount count.

### C.2 "Each type should have its own Tab component that gets rendered"

Already true at the leaf (§A.3). The missing piece is the **descriptor**, not the components.

### C.3 "Separate handler for each type, not an if/else ladder in the backend"

Already true (§B.2). There is no backend ladder on the request path. What exists is:

- a **filing inconsistency**: REST's handler is in `miscCommands.ts`, the other three are in
  `protocols/`;
- a **naming inconsistency**: `request.replay` (noun-verb on a generic namespace) vs
  `graphql.execute` / `soap.execute` / `grpc.execute` (namespace-verb).

Both are cosmetic-to-moderate. Neither is a ladder.

### C.4 "factory or something or proper different IPC commands"

- The codebase's established answer for "one family, N variants" is: **a `Map`/`Set` container
  populated by N small registration files, consulted by one command.** That is
  `entityCrudRegistry`/`simpleEntityKinds` + the five `kinds/` files, and it is the house idiom.
  A GUI descriptor table is the same pattern one layer up — it would be idiomatic, not novel.
- **"Different IPC channels per protocol" is the wrong direction.** It would re-create the
  per-namespace channel sprawl P6 deleted across six reduction groups, and it exists only on
  Electron — `socket`, `stdio`, `ws` and `in-process` have no channels, so a channel-per-protocol
  design breaks the transport-independence property the whole P4/P6 programme exists to hold.
  The command name inside `{cmd, payload}` is already the per-protocol discriminator.

---

## Part D — Options, with trade-offs (no code written)

### Option A — GUI protocol descriptor registry *(smallest change, largest payoff)*

New `packages/gui/src/lib/protocols.ts` (does not exist today; `ProtocolSelectorTab`'s local `cards`
is the prototype), shaped roughly:

```
{ id, label, badge, badgeColor, badgeBg, icon, entityKind, folderKind,
  Component, tabTitle(draft), emptyDraft() }
```

Consumers, and what each becomes:

| file | today | after |
|---|---|---|
| `RequestTabContent.tsx` | 4 × 30-line blocks | one lookup + `<desc.Component {...common} />` |
| `useRequestsPanelState.ts` | `tabLabel`/`tabBadge` ladders (7) | `desc.tabTitle(draft)` / `desc.badge` |
| `useMultiplexedEntities.ts` | 5 ladders (18) | `desc.entityKind` + a per-op method table |
| `ProtocolSelectorTab.tsx` | local `cards` | reads the descriptor's copy — chooser and dispatch cannot drift |
| `MockTabContent.tsx` | 1 conditional | unchanged, or shares the descriptor with a `mode` filter |

**Guard to add with it:** `assertProtocolsAreTotal()`, mirroring `assertSurfaceIsTotal()` /
`assertScopesAreTotal()` — every `ApiProtocol` value has a descriptor, and every descriptor has a
component. That is the house idiom for "a map that is merely written down will drift", and it is what
makes the refactor self-defending rather than merely tidier.

**Type-safety prerequisite:** converge the four `*TabProps` interfaces on a shared
`ProtocolTabProps` so `desc.Component` can be typed. Today they differ (RestTab has
`onCreateMock` / `showCurlImport` / `enabled` / `onToggleEnabled`; GrpcTab has none of those and
takes an extra `isNew`).

### Option B — engine: relocate `request.replay` to `protocols/httpCommands.ts` *(zero wire change)*

Pure move: the command name, schema, scope and behaviour all stay identical; only the file the
registration lives in changes, and `miscCommands.ts` loses three lines. This is exactly the shape of
the P6 §5c slices and the cheapest way to make the four protocols four files.

Risk: low. The one thing to preserve verbatim is the resolve-vs-throw asymmetry (it throws), and
`tests/integration/protocolExecution.integration.test.ts` already pins it.

### Option C — engine: rename `request.replay` → `rest.execute` *(wire change — defer)*

Makes the naming uniform. Costs all seven touch points in §B.6 plus a protocol-version decision. The
protocol package is described as frozen and `legacyChannel` is documented as a migration aid, so this
is a deliberate API change, not a refactor. **Recommendation: do B now, decide C separately.**

### Option D — client: derive the per-protocol CRUD bodies from `SURFACE`

`commandFor()` already reads `SURFACE`. The 12 hand-written `add*Request` / `update*Request` /
`delete*Request` bodies differ only in the `kind` string, which `SURFACE` already carries as
`entityKind`. A small table-driven loop would remove ~36 hand-written lines and make
`shape.test-d.ts` the only thing pinning the surface. Lower value than A; worth noting because it is
the same insight applied one layer down.

---

## Part E — Constraints that bound any refactor here

These are the project's own recorded traps; they are the reason this is a "plan it carefully" change
rather than a quick one.

- **`packages/gui` has no `dist`.** `npm run build:packages` **omits** it. Sole build check:
  `CODEBUDDY_SAFE_DELETE_ENABLED=0 npm run build:renderer`.
- **`packages/gui` is not in `npm run typecheck`.** Sole runner:
  `npm run typecheck --workspace @bifurc/gui`, with a **standing 33 errors across 21 files**. The
  instrument is *which files*, not the count — a touched file must not appear in that list.
- **Vitest resolves `@/…` through its own alias plugin**, not the bundler's. A green unit test is not
  proof the bundle resolves.
- **No tsconfig covers `tests/`** (but `npm run typecheck:e2e` does cover `e2e/`) — `typecheck` stays
  green while a test file is broken.
- **Renaming a button label breaks e2e silently.** `npm run typecheck:e2e` will not catch it: a stale
  locator is a runtime failure, not a type error. `requests-rest`, `mocks-rest`, `proxy-rules`,
  `mappings`, `environments`, `app` and `happy-path-workflows` all drove `Save`/`Update` before the
  autosave move.
- **`tr:has-text(...)` cannot find the mapping / health-bar editors** — they are real `<Input>`s now;
  match on `input[value="…"]`.
- **`surface.ts` is the spec and the surface is "byte-identical".** `tests/shape.test-d.ts` pins the
  disagreement between the runtime surface and `BifurcApi` to **exactly 7 keys**. A new or renamed
  key must not move that number.
- **Three totality guards** must all be satisfied by a new command: `assertSurfaceIsTotal()`,
  `assertScopesAreTotal()`, `assertBridgeIsTotal()`.
- **One `Edit` per file per message** — two same-file edits silently revert each other.
- **Do not run `npm run dev` / `test:e2e`** — they need a real desktop session; an agent-shell failure
  is evidence about the shell, not the product.
- **A new test file changes the recorded suite total** (2,658 tests / 117 files as of 2026-09-28) —
  `TESTING.md` and the workspace memory `Current:` line both carry it, unit and integration measured
  separately.

---

## Part F — Open questions

1. **Is the goal maintainability or a measured performance target?** If performance, what is the
   measurement that shows a problem? (My reading of the code says the four conditionals are a switch
   and cost nothing to render.)
2. **Scope of the protocol-consistency fix:** relocate `request.replay` only (Option B), or also
   rename it (Option C, a wire change)?
3. **Does the mock panel join the descriptor?** It is REST-only since `plan/17` WI 8 but shares
   `ProtocolSelectorTab` with the request panel.
4. **Do the four `*TabProps` converge** on a shared `ProtocolTabProps`? This is the prerequisite for a
   type-safe `desc.Component`, and it touches the same files the autosave move touched.
5. **Where should the descriptor live** — `packages/gui/src/lib/protocols.ts` (pure, testable, matches
   the `lib/codegen.ts` / `lib/curlParser.ts` precedent) or beside the components?

---

## Appendix — file index

| path | role |
|---|---|
| `packages/gui/src/hooks/useEntityTabs.ts` | protocol-blind tab list / active tab / loaded entities (207 L) |
| `packages/gui/src/panels/requests/useRequestsPanelState.ts` | draft protocols, tab label/badge, save/delete/duplicate plumbing (309 L) |
| `packages/gui/src/panels/RequestsPanel.tsx` | tab list → content, runner branch, chooser, empty state (289 L) |
| `packages/gui/src/panels/requests/RequestTabContent.tsx` | the 4-way protocol switch + prop plumbing (182 L) |
| `packages/gui/src/components/editor/ProtocolSelectorTab.tsx` | local protocol `cards` table + chooser (145 L) |
| `packages/gui/src/components/editor/RequestTab.tsx` | shared `UrlBar` / `TabStrip` / `BottomBar` (322 L) |
| `packages/gui/src/hooks/useProtocolEditor.ts` | the shared editor state machine (201 L) |
| `packages/gui/src/hooks/useMultiplexedEntities.ts` | the CRUD ladder over four protocols |
| `packages/gui/src/types/window.ts` | `BifurcApi`, 128 declared keys |
| `packages/client/src/surface.ts` | the spec: 132 keys, classified |
| `packages/client/src/index.ts` | the hand-written implementation (1053 L) |
| `packages/protocol/src/commands/index.ts` | `COMMANDS`: 89 commands, name → schema |
| `packages/engine/src/commands/registry.ts` | the `Map`-based dispatcher (239 L) |
| `packages/engine/src/registerAll.ts` | 18 command + 5 kind registrations (156 L) |
| `packages/engine/src/protocols/{graphql,soap,grpc}Commands.ts` | per-protocol network handlers |
| `packages/engine/src/miscCommands.ts` | `request.replay` lives here (`:201`) |
| `packages/engine/src/proxy/serverReplay.ts` | REST's actual sender |
| `packages/engine/src/store/entityCrudCommands.ts` | the kind registry + 4 collapsed commands (487 L) |
| `packages/engine/src/store/kinds/*.ts` | 5 protocol-grouped kind registrations |
| `packages/engine/src/transport/auth/scopes.ts` | `SCOPE_BY_COMMAND`, 89 entries |
| `src/ipc/rpcBridge.ts` / `rpcContract.ts` | the single `engine:rpc` channel |
