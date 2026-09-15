# Bifurc Decoupling — Roadmap

This folder is the execution plan for splitting the Bifurc Electron app into a **headless engine** plus
interchangeable clients (Electron shell → web UI → CLI → Docker).

**Read in order. Each phase has a hard gate. Do not start a phase until the previous gate is green.**

---

## Repo scope

This repository contains **the Electron desktop app only**. It was flattened on 2026-09-14 — the app
previously lived in a `bifurc/` subdirectory and is now the repository root.

| Was | Now |
|---|---|
| `bifurc/` (app subdirectory) | repository root |
| `bifurc/package.json` | `package.json` |
| `bifurc/plan/` | `plan/` |
| `bifurc/release/` | `release/` |
| `bifurc-extension/` (in this repo) | **separate repository** — `../bifurc-extension` |
| root `package.json` (`bifurc-monorepo`, npm workspaces) | removed; single-package repo |

Two consequences that affect this plan:

1. **All paths in these documents are repository-root relative.** Commands no longer need `cd bifurc`.
2. **The companion extension is now an external client with its own release cycle.** You can no longer
   update it atomically with the engine. See "Protocol compatibility" below.

Also note `../bifurc-studio` is a **marketing/landing site**, not the engine's web UI. It is unrelated to
P7 and should not be confused with it.

---

## Source documents (repo root)

| Document | What it is |
|---|---|
| [`../Decoupling_Assessment.md`](../Decoupling_Assessment.md) | v1 — the measurement and effort estimate for the Electron-only path |
| [`../Decoupling_Plan_v2.md`](../Decoupling_Plan_v2.md) | v2 — headless engine + Tauri shell, revised estimates |
| [`../File_Ops_Protocol.md`](../File_Ops_Protocol.md) | File/system operation mediation design (blob layer, cert lifecycle) |
| [`../Cleanup_plan.md`](../Cleanup_plan.md) | Separate, pre-existing refactor. **Hard prerequisite to P2** — see D6. |

These roadmaps are the *how*. The root docs are the *why* and the *what changed*.

> **Note:** `Decoupling_Plan_v2.md` describes the Tauri path. Per D1/D4/D5 that path is **parked**, so its
> Tier 3 and packaging sections no longer reflect the current plan. This folder supersedes it.

---

## Decisions — resolved 2026-09-14

All nine decisions are closed. See `00-decisions.md` for the full rationale.

| # | Decision | Effect on this plan |
|---|---|---|
| **D1** | Seam first; Tauri deferred | P6 is the milestone; P11 parked |
| **D2** | Bun compile, pending the P0 spike; fall back to `node` + JS | P10 scope unchanged but **de-prioritised** |
| **D3** | Unix socket / named pipe, TCP port as fallback | P0 spike 1 simplified; P4 target set |
| **D4** | **Keep Electron.** Shell migration revisited later; decoupling is the focus | **P11 parked** |
| **D5** | **Tauri deferred.** Electron stays until the decoupled engine matures | **P11 parked; MSIX stays with electron-builder** |
| **D6** | `Cleanup_plan.md` first — hard prerequisite to P2 | P2 cannot start until cleanup lands |
| **D7** | CLI v1 = engine lifecycle + read-only inspection | **P8 narrowed to ~3 w**; full parity deferred |
| **D8** | Web UI v1 = inspection + light editing | P7 scope set; full parity deferred |
| **D9** | Lockstep versioning | P1 handshake design; P12 |

**The single biggest consequence:** with D4 and D5 both deferring the shell migration, **Track C is
parked**. The programme is now scoped to "decouple the engine and give it real clients" — not "replace
Electron". This removes roughly **6–8 engineer-weeks** and, more importantly, removes the MSIX and
WebKitGTK risks from the critical path entirely.

---

## Tracks

```
TRACK A — Decoupling (critical path)              TRACK B — Clients
──────────────────────────────────────            ──────────────────────────────
P1  Protocol            ─┐
P2  Engine extraction    │  P3 ∥ P4 may overlap
P3  File ops / blobs     │
P4  Transport            │
P5  Typed RPC client     │
P6  Shell seam  ✅ SHIP ─┴──────────────────────► P7  Web UI   (∥ P8, P9)
                                                  P8  CLI      (narrowed by D7)
                                                  P9  Docker

TRACK C — Shell swap        ⏸ PARKED  (D1, D4, D5)   P10 Standalone binary · P11 Tauri shell
TRACK D — Packaging         touches P6 and P9 only   (no Tauri change point)
TRACK E — Hardening         continuous + final pass at P13
```

**The only milestone that matters is P6.** Everything before it is on the critical path. Everything
after it is additive and independently schedulable.

---

## Phase index

| # | Phase | Depends on | Effort | Gate |
|---|---|---|---:|---|
| [00](00-decisions.md) | **Decisions** | — | ✅ done | All D1–D9 answered |
| [01](01-phase-0-derisk.md) | **P0 — De-risk spikes** | D1–D4 | 2–3 w | 3 spikes resolved; spike 4 (protocol PoC) still required |
| [02](02-phase-1-protocol.md) | **P1 — Protocol** | P0 | 2–3 w | `@bifurc/protocol` builds; schemas frozen |
| [03](03-phase-2-engine-extraction.md) | **P2 — Engine extraction** | **D6 cleanup**, P1 | 4–6 w | Engine runs headless; zero Electron imports |
| [04](04-phase-3-file-ops.md) | **P3 — File ops / blobs** | P1, P2 | 1.5–2.5 w | Round-trip tests green for all 17 formats |
| [05](05-phase-4-transport.md) | **P4 — Transport** | P1, P2 | 3–4 w | Conformance suite green on 3 transports |
| [06](06-phase-5-client.md) | **P5 — Typed RPC client** | P4 | 1.5–2 w | Client satisfies `window.api` type-check |
| [07](07-phase-6-shell-seam.md) | **P6 — Shell seam ✅ SHIP** | P5 | 2–3 w | 35 unit + 11 e2e green; ships with no visible change |
| [08](08-phase-7-web-ui.md) | **P7 — Web UI** | P6 | 3–4 w | Inspection + light editing in a browser (D8) |
| [09](09-phase-8-cli.md) | **P8 — CLI** | P5 | 3 w | Engine lifecycle + read-only inspection (D7) |
| [10](10-phase-9-docker.md) | **P9 — Docker** | P2, P3, P4 | 1–2 w | Image runs headless, survives restart, authenticated |
| [11](11-phase-10-binary-and-11-tauri.md) | **P10 — Standalone binary** | P2 | 1–2 w | ⏸ **de-prioritised** (D2/D4) |
| [11](11-phase-10-binary-and-11-tauri.md) | **P11 — Tauri shell** | P5, P10 | 3–4 w | ⏸ **PARKED** (D1/D4/D5) |
| [12](12-phase-12-packaging-and-13-hardening.md) | **P12 — Packaging** | P6, P9 | 2–3 w | NSIS + APPX + AppImage + Docker publish from CI |
| [12](12-phase-12-packaging-and-13-hardening.md) | **P13 — Hardening** | P6 | 2–3 w | Auth, audit, docs, perf budgets signed off |
| [13](13-checklist.md) | **Master checklist** | — | — | Literal ordered task list |

---

## Effort, revised for the decisions

| Tier | Content | Effort |
|---|---|---:|
| **1 — Seam** (P1–P6) | Protocol, engine extraction, file ops, transport, RPC client, Electron thin host. **Ships as a normal release with zero user-visible change.** | **14–20.5 w** |
| **2 — Headless clients** (P7–P9) | Web UI (inspection + light editing), CLI (lifecycle + read-only), Docker image. Electron still the desktop shell. | **7–9 w** |
| **Cross-cutting** (P13 + tests) | Tests, security, observability, docs — spread across Tiers 1 and 2 | **5–7 w** |
| | **Subtotal** | **26–36.5 w** |
| | **Contingency (25%)** | **6.5–9 w** |
| | **Total — decoupled engine with clients** | **~33–46 engineer-weeks** |
| | ⏸ *Parked: P10 + P11 if the shell migration resumes* | *+4–6 w* |

**Corrected from v2.** The earlier "Tier 1 = 10–14 w" figure was wrong — it omitted the transport and
RPC client, which the seam cannot ship without. Tier 1 is P1 through P6, and that is 14–20.5 weeks.

**The number that matters: ~14–20.5 weeks to the seam.** After P6 the project is reversible, the shell
is disposable, and every remaining phase is independently schedulable.

---

## Protocol compatibility — new obligation

The companion extension moved to its own repository. It is now a **released external client you cannot
update atomically with the engine.**

Practical consequences for P1 and P4:

- The extension's four commands (`mock:add`, `request:add`, `folder:add`, `config:get` —
  `src/companion/allowedActions.ts`) must remain **backward-compatible across at least one minor
  protocol version**. Treat them as a frozen public API, not internal commands.
- The `hello` handshake must let an older extension connect and degrade gracefully rather than being
  refused (P1 item 7).
- P4's verification step becomes "works against the **released** extension build", not "works against
  the copy that used to live in this repo".
- Any change to those four commands needs a deprecation window and a note in `protocol-changes.md`.

This makes D9 (lockstep) slightly harder to hold — the extension sits outside the lockstep. Record it as
a known exception: engine and shell are lockstep; the extension trails by one minor version.

---

## How to use a phase document

Every phase doc has the same shape:

```
Goal                — one sentence, what "done" means
Depends on / Blocks — hard prerequisites and downstream consumers
Preconditions       — the gate from the previous phase, restated
Work items          — numbered, with file paths, in execution order
How to start        — the literal first three things to do
Acceptance criteria — checkable, not vibes
Rollback            — how to back out if it goes wrong
Risks               — what is most likely to bite
```

**Gates are hard stops.** A phase is not done because the code compiles. It is done when its acceptance
criteria pass. If a gate fails, fix it inside the phase — do not carry debt into the next one.

---

## Status

Update this table as you go. It is the single source of truth for programme state.

| Phase | Status | Started | Gate green | Notes |
|---|---|---|---|---|
| 00 Decisions | ✅ **done** | 2026-09-14 | ✅ | D1–D9 resolved |
| P0 De-risk | ✅ **done** | 2026-09-15 | ✅ | Spike 4 (protocol PoC) confirmed the renderer-unchanged thesis — see `plan/spike-results.md`. Spikes 1/3 skipped (D4/D5); spike 2 still optional. |
| P1 Protocol | ✅ **done** | 2026-09-15 | ✅ | `packages/protocol` — 89 commands, errors, events, `hello` handshake. `plan/handler-classification.md` complete. Frozen at v1.0.0, see `plan/protocol-changes.md`. |
| P2 Engine extraction | 🟡 in progress | 2026-09-15 | ⬜ | Work items 1–3 (EventBus + all 8 broadcast sites + `processSpawner`) and 4 (data-dir wired as primary path in `appSettings.ts`/`workspaceFs.ts`) done. Item 6 (headless `preflight()` in `src/startup.ts`) half done. Item 5 (client-only handlers split into `src/ipc/handlers/clientHandlers.ts`) started — 10 of 15 CLIENT channels moved. Item 7 (`src/commands/registry.ts` — the CommandRegistry) now covers **~100 commands**: `coreHandlers.ts`, `syncHandlers.ts` (16/19), `folderHandlers.ts` (4/4), `tlsHandlers.ts` (3/6), `runnerHandlers.ts` (2/6), `graphqlHandlers.ts`/`soapHandlers.ts`/`grpcHandlers.ts` (network calls + all CRUD), `applicationHandlers.ts` (10/10), `importExport/index.ts` (1/4), and — this session — **the full CRUD collapse**: `entityCrudFactory.ts`'s 36 add/update/delete channels (all 12 kinds, across `crudHandlers.ts`/`graphqlHandlers.ts`/`soapHandlers.ts`/`grpcHandlers.ts`) plus `entity:load`/`entity:setEnabled` now route through 5 generic `entity.*` commands, bridged by a new `src/commands/entityKindMap.ts` translation table for the one real kind-naming mismatch (`rules`↔`proxyRules`, `sockets`↔`wsConnections`). `audit.list`, `runner.saveConfig`, and `import.commit` remain deliberately unconverted (real protocol-schema gaps — see the phase doc). Remaining: `environments`/`graphqlSchemas`/`protoFiles`/`wsdls`'s bespoke handlers (~10), plus the SPLIT import/export channels which are P3 territory. Item 8 (`packages/*` restructure) started: `packages/protocol` is now a real linked npm workspace dependency (needed to make item 7 possible at all) — `packages/engine` itself is not. See `03-phase-2-engine-extraction.md` for the itemised status. |
| P3 File ops | ⬜ not started | | | |
| P4 Transport | ⬜ not started | | | |
| P5 RPC client | ⬜ not started | | | |
| P6 Shell seam | ⬜ not started | | | **ship milestone** |
| P7 Web UI | ⬜ not started | | | |
| P8 CLI | ⬜ not started | | | |
| P9 Docker | ⬜ not started | | | |
| P10 Standalone binary | ⏸ de-prioritised | | | only needed for Tauri or a single-binary CLI |
| P11 Tauri shell | ⏸ **parked** | | | D1/D4/D5 — revisit when the engine matures |
| P12 Packaging | ⬜ not started | | | no Tauri change point now |
| P13 Hardening | ⬜ not started | | | |

---

## Cleanup_plan.md status — assessed 2026-09-15 (D6 gate for P2)

D6 requires Cleanup_plan.md **Phase 1–2** to land before P2 starts. Verified against the working tree
(`master` @ `476d871`):

| Item | Status | Evidence |
|---|---|---|
| 1.1 Delete dead panel files | ✅ | `GraphQLRequestsPanel`, `LogsPanel` etc. absent from `renderer/panels/` |
| 1.2 Delete `applicationUtils.ts` | ✅ | file absent |
| 1.3 Purge dead strings | ✅ | no stale keys found |
| 1.4 Dead imports / unused vars | ✅ | *the leftover unused `import { app } from "electron"` in `src/store/gitStore.ts` flagged here previously has since been removed (verified 2026-09-15 — `gitStore.ts` imports only `simple-git`, `path`, `fs`, `os`, `@/store/types`, `@/store/workspaceFs`)* |
| 1.5 Remove gRPC stubs | ⚠️ **not done — recommend re-scoping** | stubs remain in `grpcHandlers.ts`. They are deliberately pinned by `protocolExecution.integration.test.ts` as the product's "gRPC not implemented" contract. Removing them is a product decision, not dead-code cleanup. Recommend marking 1.5 **wontfix** and referencing the pinned contract. |
| 2.1 `entityCrudFactory.ts` | ✅ | exists, generates the ~36 CRUD channels |
| 2.2 `folderHandlers.ts` | ✅ | exists |
| 2.3 `responseUtils.ts` | ✅ | exists |
| 2.4 Extract proxy dispatch flow | ✅ | `server.ts` imports the dispatch helpers from `proxyHandler.ts` |
| 2.5 `useProtocolEditor` hook | ✅ | `renderer/hooks/` |
| 2.6 `createTabReducer` factory | ✅ | `renderer/lib/createTabReducer.ts` |
| 2.7 `useMultiplexedEntities` hook | ✅ | `renderer/hooks/` |
| 2.8 Consolidate persistence hooks | ✅ | `useDraftPersist` / `usePersistedState` now in `renderer/hooks/` |
| 2.9 JSON import/export factory | ❌ **not landed** | no `src/ipc/importExport/jsonFactory.ts`; the exporter/importer files remain individual |

Phase 3 (shared components) — **not landed** (no `EntityEditorLayout`, `MasterDetailLayout`,
`ProtocolEditorLayout`, `ProxyRuleForm`; `panelFactory` still in `renderer/lib/`).
Phase 4 (300-line limit) — **not landed**: 18 files still exceed 300 lines (worst:
`GraphQLTab.tsx` 420, `SoapTab.tsx` 405, `restTabReducer.ts` 350).
Phase 5 (React anti-patterns) — not assessed; not blocking.

**Verdict:** the D6 prerequisite is met **except for 2.9** (and the 1.5 disposition decision).
Recommended path:

1. Decide 1.5 = wontfix (one paragraph in `Cleanup_plan.md`, reason: pinned stub contract).
2. Either land 2.9 (`jsonFactory.ts`) as a small standalone PR, **or** explicitly fold it into P3 —
   P3 rewrites the import/export layer for the blob protocol anyway, so doing 2.9 twice is waste.
   If folding into P3, record the D6 exception here.
3. Phases 3–5 of the cleanup remain post-P2 work; they touch the renderer, which P1–P6 must not
   edit — so they are **naturally sequenced after P6**, not before P2.

---

## Non-negotiables

1. **P6 ships with zero user-visible change.** If the renderer needs editing during P6, something upstream
   is wrong — stop and fix it there.
2. **No remote transport before auth.** The engine spawns processes and executes scripts. Shipping an
   unauthenticated remote RPC surface is a remote code execution hole. Auth is P4, not P13.
3. **`window.api` stays byte-identical through P5–P6.** It is the regression signal for the entire
   critical path.
4. **Do not merge `Cleanup_plan.md` into these phases** (D6). It is a prerequisite, not a parallel track.
5. **The extension's four commands are a frozen public API.** It ships on its own release cycle now.
