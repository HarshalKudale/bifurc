/**
 * The `window.api` surface, enumerated and classified. **This is the spec.**
 *
 * P5 exists to protect exactly one property (plan/README "Non-negotiables" #3):
 * `window.api` stays **byte-identical** through P5–P6. That property is the regression
 * signal for the whole critical path, so the surface is written down here as data rather
 * than left implicit in 136 hand-written methods.
 *
 * ## Why this file exists at all
 *
 * The plan's precondition says `packages/gui/src/types/window.ts` is "the authoritative list of the
 * surface to reproduce", and its acceptance criterion says "All 136 methods present". Both
 * numbers are stale, and the difference is load-bearing:
 *
 * | source | keys |
 * |---|---|
 * | `src/preload.ts` — what `window.api` actually **is** at runtime | **136** |
 * | `packages/gui/src/types/window.ts` — what it is **declared** to be | 132 |
 *
 * **2026-09-29: both rows +4** — the four `*Saved` execute methods. The gap between the rows is
 * **still exactly four**, which is the property the table exists to show: a change that moved one row
 * without the other would be a declared/runtime divergence, and this one moved both.
 *
 * **2026-09-30: `replayRequestSaved` → `requestSend`, and the rows do not move.** That is the other
 * property worth pinning. The REST member of the four was **renamed and widened** into `plan/19` D24's
 * pipeline (`request.send` — a union of `{id}` and `{request, scripts}`), so a method that used to execute
 * a saved entity now also resolves the request and runs its scripts. Both rows stay at **136 / 132** — one
 * key replaced by one key — and the gap stays exactly four, which is what tells a reader this was a
 * *rename* rather than a drop hiding inside an addition.
 *
 * **`plan/17` work item 8 reduced both by twelve** — 144 → 132 and 140 → 128. Mocks became REST-only,
 * so the nine mock CRUD methods (`graphqlMocks` / `grpcMocks` / `soapMocks` × add/update/delete) and
 * the three `grpc*MockServer` transport methods left the surface. The gap between the two rows is
 * still exactly four, which is the property the table exists to show: the change was a *drop*, not a
 * reclassification, and a drop that the runtime and the declared type took together is the one kind
 * that cannot hide a fourth discrepancy.
 *
 * The type under-declares the runtime by four keys, and every one of them is a method the
 * renderer can call today:
 *
 * ```
 * isFirstLaunch, completeFirstLaunch, getZoomLevel, setZoomLevel
 * ```
 *
 * They are exposed by `src/preload.ts` and implemented in `src/ipc/handlers/clientHandlers.ts`
 * (zoom + first-launch), but they were never added to `BifurcApi`. So a client built to
 * satisfy *the type* would pass `shape.test-d.ts`, pass the "zero differences" script if that
 * script also used the type, and still have silently dropped four keys off the runtime
 * surface. **The runtime is the authority; the type is a secondary check.**
 *
 * (Nothing reads those four today — verified repo-wide; they are shell-side concerns. But
 * "nothing reads it today" is not the same as "safe to drop", which is why they are
 * classified as `local` here rather than omitted.)
 *
 * There is a **second** disagreement, pointing the other way, and the same reasoning catches it:
 * `setTitleBarOverlay`, `getTheme` and `setTheme` are declared **optional** (`?`) in
 * `packages/gui/src/types/window.ts` although `src/preload.ts` always provides them. So the declared type
 * is *looser* than the runtime there rather than narrower — and a client that simply omitted
 * `getTheme` would satisfy `BifurcApi`, pass the type-level shape test, and then fail the first
 * time the renderer called it. Optionality is the more insidious of the two directions, because
 * "possibly undefined" reads as defensive typing rather than as a hole in the contract.
 *
 * `BifurcApiFull` in `index.ts` corrects both: the four added, the three tightened.
 * `tests/shape.test-d.ts` asserts the total disagreement is **exactly those seven keys**, so it
 * cannot grow unnoticed.
 *
 * ## The four kinds of entry
 *
 * - **`transport`** — a 1:1 mapping onto a protocol command. The easy majority.
 * - **`entity`** — the CRUD collapse. Thirteen kinds × `add`/`update`/`delete`(/`list`) become
 *   `entity.create`/`update`/`delete`/`list` with a `kind` injected. This is where the
 *   signature drift risk lives, so it is data rather than code.
 * - **`shim`** — a genuine multi-step or transformed mapping (`exportData` is
 *   `export.create` + a blob fetch; `preflightImport` is `blob.put` + `import.preflight`).
 * - **`local`** — **not a transport call at all.** Dialogs, zoom, theme, titlebar, `platform`,
 *   OS trust-store, first-launch, and — since 2026-09-20 — the update check. These must never be
 *   routed over the transport: they are operations on the *client's own machine*, and on a remote
 *   engine they would be either meaningless or a security hole. `checkUpdate` is the one member
 *   here that is not a protocol command at all, rather than a command served locally.
 * - **`subscribe`** — one of the 7 subscription methods.
 *
 * `assertSurfaceIsTotal()` is the guard. It is deliberately the same shape as the engine's
 * `assertBridgeIsTotal()`: a classification that is merely *written down* drifts, and the
 * failure mode here is a key that silently vanishes from the client.
 */

import type { EntityKindValue } from "@bifurc/protocol";
import { EVENT_NAMES } from "@bifurc/protocol";

/** A wire event name — the `event.<namespace>.<verb>` form, not the bus form. */
export type WireEventName = (typeof EVENT_NAMES)[number];

/** Which CRUD verb an `entity.*` method collapses onto. */
export type EntityOp = "create" | "update" | "delete" | "list";

export type SurfaceEntry =
  | { readonly kind: "transport"; readonly command: string; readonly note?: string }
  | {
      readonly kind: "entity";
      readonly entityKind: EntityKindValue;
      readonly op: EntityOp;
      /**
       * `entity.list`'s `workspaceId` is **required**, unlike `entity.create`'s and `folder.add`'s
       * which are optional (the engine defaults those to the active workspace). The three `window.api`
       * listers carry no `wsId` — the legacy handlers read `loadConfig().activeWorkspaceId` — so the
       * client has to resolve it. See `resolveWorkspaceId()` in `index.ts`.
       */
      readonly needsActiveWorkspace?: boolean;
    }
  | { readonly kind: "shim"; readonly command: string; readonly note: string }
  | { readonly kind: "subscribe"; readonly event: WireEventName; readonly note?: string }
  | { readonly kind: "local"; readonly note: string };

const t = (command: string, note?: string): SurfaceEntry =>
  note === undefined ? { kind: "transport", command } : { kind: "transport", command, note };
const e = (entityKind: EntityKindValue, op: EntityOp, needsActiveWorkspace = false): SurfaceEntry =>
  needsActiveWorkspace
    ? { kind: "entity", entityKind, op, needsActiveWorkspace: true }
    : { kind: "entity", entityKind, op };
const s = (command: string, note: string): SurfaceEntry => ({ kind: "shim", command, note });
const sub = (event: WireEventName, note?: string): SurfaceEntry =>
  note === undefined ? { kind: "subscribe", event } : { kind: "subscribe", event, note };
const local = (note: string): SurfaceEntry => ({ kind: "local", note });

/**
 * Every key `src/preload.ts` exposes, classified. Order follows `src/preload.ts` so a reader
 * can diff the two by eye.
 */
export const SURFACE: Readonly<Record<string, SurfaceEntry>> = {
  // ── app / config ──────────────────────────────────────────────────────────
  // `checkUpdate` was `t("app.checkUpdate")` — the surface's only `SPLIT` — until 2026-09-20. It is
  // `local` because no engine can answer it correctly: `currentVersion` is the version of the thing
  // the *user* runs, and the download URL is the asset for the *user's* platform. An engine matching
  // its own `process.platform` hands a Windows user the AppImage. `packages/protocol` no longer
  // declares `app.checkUpdate`; this is the one key whose whole implementation is client-side.
  checkUpdate: local("GitHub release check; needs this client's own version and platform"),
  getConfig: t("config.get"),
  // ── the two caller-supplied-kind entries, reclassified `transport` → `shim` (2026-09-21) ──────
  // Both forward a `kind` the *caller* chose, which makes them the only two surface entries whose
  // payload the client cannot derive from the key. Each one's shell channel owned a step the engine
  // does not perform, and both steps are now here. `plan/07` §5b-septies (sixth group) has the
  // accounting; the short version is that a `transport` filing *is* the claim that nothing happens
  // beyond the call, and for these two that claim was false — which is exactly the mis-filing that
  // cost the TLS paths (§5b-undecies).
  loadEntity: s(
    "entity.load",
    "the caller may name an engine-internal kind (`rules`, `sockets`); the wire wants the protocol's `proxyRules` / `wsConnections`",
  ),
  setEntityEnabled: s(
    "entity.setEnabled",
    "an out-of-enum kind resolves `{ok:false, error:'invalid_kind'}` rather than throwing",
  ),
  saveConfig: t("config.save"),

  // ── import / export (all three are multi-step over the transport) ─────────
  getImportExportFormats: t("export.formats"),
  exportData: s("export.create", "two-step: export.create returns an artifact, then fetch its blob"),
  preflightImport: s("import.preflight", "three-step: blob.put the content, then import.preflight with the blobId"),
  importData: s("import.commit", "three-step: blob.put the content, then import.commit with the blobId"),
  discoverServices: t("services.discover"),

  // ── the CRUD collapse: the remaining kinds × add/update/delete(/list) ────
  // `plan/17` work item 8 removed the three protocol MOCK kinds (`graphqlMocks`, `grpcMocks`,
  // `soapMocks`), so nine methods left with them. The four request kinds and their
  // schema/proto/wsdl companions are untouched.
  addMapping: e("mappings", "create"),
  updateMapping: e("mappings", "update"),
  deleteMapping: e("mappings", "delete"),
  addRule: e("proxyRules", "create"),
  updateRule: e("proxyRules", "update"),
  deleteRule: e("proxyRules", "delete"),
  addMock: e("mocks", "create"),
  updateMock: e("mocks", "update"),
  deleteMock: e("mocks", "delete"),
  addRequest: e("requests", "create"),
  updateRequest: e("requests", "update"),
  deleteRequest: e("requests", "delete"),
  addWsConnection: e("wsConnections", "create"),
  updateWsConnection: e("wsConnections", "update"),
  deleteWsConnection: e("wsConnections", "delete"),
  addWebhook: e("webhooks", "create"),
  updateWebhook: e("webhooks", "update"),
  deleteWebhook: e("webhooks", "delete"),
  addGraphQLRequest: e("graphqlRequests", "create"),
  updateGraphQLRequest: e("graphqlRequests", "update"),
  deleteGraphQLRequest: e("graphqlRequests", "delete"),
  addGraphQLSchema: e("graphqlSchemas", "create"),
  deleteGraphQLSchema: e("graphqlSchemas", "delete"),
  listGraphQLSchemas: e("graphqlSchemas", "list", true),
  addGrpcRequest: e("grpcRequests", "create"),
  updateGrpcRequest: e("grpcRequests", "update"),
  deleteGrpcRequest: e("grpcRequests", "delete"),
  addProtoFile: e("protoFiles", "create"),
  deleteProtoFile: e("protoFiles", "delete"),
  listProtoFiles: e("protoFiles", "list", true),
  addSoapRequest: e("soapRequests", "create"),
  updateSoapRequest: e("soapRequests", "update"),
  deleteSoapRequest: e("soapRequests", "delete"),
  addWsdl: e("wsdls", "create"),
  deleteWsdl: e("wsdls", "delete"),
  listWsdls: e("wsdls", "list", true),
  addEnvironment: e("environments", "create"),
  updateEnvironment: e("environments", "update"),
  deleteEnvironment: e("environments", "delete"),

  // ── folders ──────────────────────────────────────────────────────────────
  // NOTE: `folder.*` takes a *singular* kind ("mock", "request", "ws", …) — a different
  // vocabulary from `EntityKindValue`'s plurals. That asymmetry is in the frozen protocol,
  // not introduced here; see `packages/engine/src/kernel/kinds.ts`'s sibling.
  addFolder: t("folder.add"),
  renameFolder: t("folder.rename"),
  moveFolder: t("folder.move"),
  deleteFolder: t("folder.delete"),

  // ── environments / workspaces ────────────────────────────────────────────
  setActiveEnvironment: t("env.setActive"),
  /**
   * `plan/19` WI 5, item 4 — resolve `{{tokens}}` against the workspace's **merged** environment.
   *
   * Filed `transport`: one method, one command, no client half. It exists because WI 3 made the engine the
   * resolving party, which left the renderer unable to show the value a probe will actually use — the
   * health bar displays a resolved url. See `packages/protocol/src/commands/config.ts`.
   */
  envResolveText: t("env.resolveText", "resolves {{tokens}} for display; the engine owns the merge"),
  /**
   * `plan/19` WI 6 — the **merged environment itself**, for the one renderer need that is a *list* rather
   * than a resolved string: `EnvVarHint`'s autocomplete.
   *
   * Filed `transport`: one method, one command, no client half. It replaced `App.tsx`'s `mergeEnvVars`, which
   * was the last renderer implementation of a workspace fact the engine already owned (D23) — and the merge
   * is the part that cannot stay: resolving against the *active* environment alone resolves a shadowed key
   * differently, which is `plan/19` §8 trap 2.
   */
  envGetMerged: t("env.getMerged", "the merged environment; the list to choose from, not a resolved value"),
  addWorkspace: t("workspace.add"),
  renameWorkspace: t("workspace.rename"),
  deleteWorkspace: t("workspace.delete"),
  setActiveWorkspace: t("workspace.setActive"),

  // ── server / proxy ───────────────────────────────────────────────────────
  replayRequest: t("request.replay"),
  /**
   * `plan/19` WI 5 — the cURL parser.
   *
   * Filed as `transport` because it is a 1:1 command mapping: one method, one command, no shim and no
   * client half. It exists so the renderer stops carrying `packages/gui/src/lib/curlParser.ts` — the
   * boundary ratchet's `curlParser` construct — while its import field keeps parsing on every keystroke.
   */
  requestParseCurl: t("request.parseCurl", "parses a cURL command into the request it describes"),
  /**
   * The four **saved-entity** counterparts, added 2026-09-29. Each takes an entity id where its base
   * takes the whole request, and the engine loads the entity and executes it.
   *
   * ## This is the one place a new key was added to a frozen surface
   *
   * The surface was frozen byte-identical through P5–P6, and that freeze had a job: it was the
   * regression signal for the client extraction, and the pinned counts below are what made any change
   * to it visible. P6 is complete, so the freeze has done its work — and a capability the renderer is
   * being asked to *use* cannot be added without a key, because `window.api` is the only door the
   * renderer has.
   *
   * So the freeze is **lifted deliberately rather than eroded**, which is why the four are filed as
   * `transport` (they are 1:1 command mappings) and why every pinned count below moves by exactly
   * four: a change that showed up as one count rising while another fell would be a reclassification
   * hiding inside an addition.
   *
   * ## Why the CLI is the reason they exist at all
   *
   * A CLI has a request *id*, not a request payload, so the payload-passing commands cannot serve it —
   * see `plan/protocol-changes.md`. The renderer is the first consumer because it is here; the id
   * shape is what the CLI will use.
   */
  requestSend: t("request.send", "runs the REST pipeline, from a saved `requests` entity or an inline request"),
  proxyStatus: t("proxy.status"),
  serverStatus: t("server.status"),
  restartServer: t("server.restart"),
  stopServer: t("server.stop"),
  startServer: t("server.start"),

  // ── client-local: OS / window operations, never a transport call ─────────
  openExternal: local("opens a URL on the client's own machine"),
  setTitleBarOverlay: local("Electron BrowserWindow; read by packages/gui/src/hooks/useTheme.ts"),
  tlsInstallCA: local("mutates the client machine's OS trust store"),

  // ── the trust-store pair, reclassified `transport` → `local` on 2026-09-20 ─
  // Both were routed to the engine, and both thereby lost the client's half of a **two-sided**
  // operation — the `checkUpdate` failure class again (`plan/07` §5b-bis). `tls.certStatus` returns
  // the engine's fingerprint and paths; the *drift answer* — `trustState`, `firefoxDetected`,
  // `firefoxProfiles` — comes from `certLifecycle.readTrustSummary()`, which reads the OS store and
  // the `AppSettings.tlsTrustedCa` record this client wrote at install time. `tls.removeCert` is the
  // worse half: it is two-sided by design (`File_Ops_Protocol.md` §6.1), and routing it deleted the
  // engine's keypair while leaving the OS store trusting the CA — reachable from the Settings panel,
  // so the UI reported success and the browser went on trusting a certificate that was gone.
  //
  // Local rather than routed-with-a-client-half because no engine can answer either one correctly:
  // on a remote engine (P9) it cannot read the user's OS trust store at all. This follows the
  // `tlsInstallCA` precedent — the whole trust-store surface uses one mechanism — and the engine
  // halves (`tls.certStatus` / `tls.removeCert`) stay registered, invoked *by* the local hook.
  tlsRemoveCert: local("composes the engine's removal with un-trusting the client's OS store"),
  tlsCertStatus: local("composes the engine's fingerprint with the client's trust-store summary"),

  // ── audit / history ──────────────────────────────────────────────────────
  listAudit: t("audit.list"),
  auditDiff: t("audit.diff"),
  exportAudit: t("audit.export", "returns an artifact; the CLIENT writes the file, not the engine"),
  listHistory: t("history.list"),
  historyDiff: t("history.diff"),

  // ── sync / git ───────────────────────────────────────────────────────────
  syncSetRemote: t("sync.setRemote"),
  syncDisconnect: t("sync.disconnect"),
  syncPush: t("sync.push"),
  syncPull: t("sync.pull"),
  syncGetState: t("sync.getState"),
  syncSetAutoSync: t("sync.setAutoSync"),
  publishEntity: t("entity.publish"),
  publishFolder: t("folder.publish"),
  restoreEntity: t("entity.restore"),
  gitGetDiff: t("git.diff"),
  gitDiscard: t("git.discard"),
  gitSync: t("git.sync"),
  gitGetHistory: t("git.history"),
  getEntitySyncStatus: t("sync.getEntityStatus"),
  executeScript: t("script.execute"),

  // ── healthbar ────────────────────────────────────────────────────────────
  healthbarGetServices: t("healthbar.getServices"),
  healthbarSaveServices: t("healthbar.saveServices"),
  healthbarCheckUrl: t("healthbar.checkUrl"),

  // ── webhook server ───────────────────────────────────────────────────────
  registerActiveWebhook: t("webhook.registerActive"),
  unregisterActiveWebhook: t("webhook.unregisterActive"),
  // Two names, one channel. Kept as two entries because the renderer may call either, and
  // "byte-identical" means both must exist.
  getWebhookServerStatus: t("webhookServer.status"),
  webhookServerStatus: t("webhookServer.status"),
  startWebhookServer: t("webhookServer.start"),
  stopWebhookServer: t("webhookServer.stop"),

  // ── SOAP / GraphQL / gRPC ────────────────────────────────────────────────
  soapFetchWsdl: t("soap.fetchWsdl"),
  soapExecute: t("soap.execute"),
  graphqlIntrospect: t("graphql.introspect"),
  graphqlExecute: t("graphql.execute"),
  grpcExecute: t("grpc.execute"),
  grpcReflect: t("grpc.reflect"),
  // The saved-entity counterparts of the three above — see `requestSend`'s note for why the surface
  // gained keys at all, and `plan/protocol-changes.md` for the protocol half. These three are still thin
  // counterparts; the REST member stopped being one on 2026-09-30 and is `requestSend` above.
  soapSend: t("soap.send", "resolves and POSTs a SOAP envelope, from a saved entity or inline"),
  graphqlSend: t("graphql.send", "resolves and executes a GraphQL request, from a saved entity or inline"),
  grpcSend: t("grpc.send", "resolves and dispatches a gRPC call, from a saved entity or inline"),

  // ── TLS / certificates ───────────────────────────────────────────────────
  //
  // The first three are **shims, not transport**, and the distinction is the whole point. The engine
  // deliberately reports no filesystem path (`File_Ops_Protocol.md` §8 keeps its directory layout out
  // of the protocol) — `tls.generate` answers `{ok, fingerprint}` and the two imports answer `{ok}`.
  // The renderer needs one anyway: `TlsSettingsSection.tsx` records `tlsCaCertPath` / `tlsCaKeyPath`
  // from what these return, and reads a missing field as "not configured". So each of the three
  // appends the path the **shell** computes, which the `tlsCertStatus` local hook already reports.
  //
  // Classified `transport` until 2026-09-20, and that misclassification *was* the bug: a 1:1 mapping
  // cannot express a step the engine does not perform, so routing them dropped the field on a live
  // path — the CA was generated on disk and the settings never learned it. See `plan/07` §5b-undecies.
  tlsImportCert: s("tls.importCert", "needs a blobId the signature never supplies: dialog + blob.put precede it, then the engine's `{ok}` gains the shell's cert path"),
  tlsImportKey: s("tls.importKey", "needs a blobId the signature never supplies: dialog + blob.put precede it, then the engine's `{ok}` gains the shell's key path"),
  tlsGenerate: s("tls.generate", "the engine answers `{ok, fingerprint}` with no path by design; the shell's certPath / keyPath are appended"),
  tlsExportCert: t("tls.exportCert", "returns an artifact; the CLIENT writes the file, not the engine"),
  // `tlsRemoveCert` and `tlsCertStatus` are deliberately **not** here: both are client-local, because
  // both compose the engine's half with the client's trust store. See the trust-store note in the
  // client-local block below.

  // ── dialogs + client-local shell state ───────────────────────────────────
  openFileDialog: local("native file picker on the client's own machine"),
  pickFilePath: local("native file picker on the client's own machine"),
  pickFolderPath: local("native folder picker on the client's own machine"),
  platform: local("was process.platform; the plan calls it dead code, kept for shape"),
  isFirstLaunch: local("shell launch state; exposed by preload but undeclared in BifurcApi"),
  completeFirstLaunch: local("shell launch state; exposed by preload but undeclared in BifurcApi"),
  getZoomLevel: local("Electron webContents zoom; exposed by preload but undeclared in BifurcApi"),
  setZoomLevel: local("Electron webContents zoom; exposed by preload but undeclared in BifurcApi"),
  getTheme: local("Electron nativeTheme; read by packages/gui/src/hooks/useTheme.ts"),
  setTheme: local("Electron nativeTheme; read by packages/gui/src/hooks/useTheme.ts"),

  // ── collection runner ────────────────────────────────────────────────────
  saveRunnerReport: t("runner.saveReport"),
  getRunHistory: t("runner.getHistory"),
  exportRunnerReport: t("runner.exportReport", "requires a format the signature omits; returns an artifact the CLIENT writes"),
  saveRunnerConfig: t("runner.saveConfig"),
  loadRunnerConfig: t("runner.loadConfig"),
  listRunnerFolderIds: t("runner.listFolderIds"),
  // The run's own commands (`plan/19` WI 4, D29/D30). **Additive** — the six above store a run's report and
  // its per-folder configuration and are unchanged, so a client that never calls these three behaves exactly
  // as it did before.
  runnerStart: t("runner.start", "starts a run and returns; progress arrives as events"),
  runnerCancel: t("runner.cancel"),
  runnerStatus: t("runner.status", "the run's state, running or finished, or null if never started"),

  // ── websocket (`plan/19` WI 6) ───────────────────────────────────────────
  //
  // The one namespace with **no legacy channel**: `useWebSocket.ts` spoke to the browser's `WebSocket`
  // constructor rather than to the shell, so nothing was ever registered under these names and there is
  // nothing here that maps onto a pre-existing `window.api` method. Six commands, two events — the four
  // decisions `Protocol_Module_Contract.md` §4 asks for are made in `packages/protocol/src/commands/ws.ts`.
  //
  // `wsConnect` resolves when the socket has been *constructed*, not opened: the transition to `connected`
  // arrives as `event.ws.status`, and `wsStatus` answers for the state in between. That is D29's shape
  // (`runnerStart`'s), and for the same reason — a caller that had to await the open would hold the
  // connection's lifetime, which is exactly what moved to the engine.
  wsConnect: t("ws.connect", "opens or re-opens a socket; the state arrives as events"),
  wsDisconnect: t("ws.disconnect", "the ONLY thing that closes a socket — a client disconnect does not"),
  wsSend: t("ws.send", "resolves the payload engine-side, then writes it"),
  wsStatus: t("ws.status", "the socket's state, live or finished, or null if never opened"),
  wsList: t("ws.list", "the live sockets, plus the registry's limit"),
  wsClearMessages: t("ws.clearMessages", "drops the engine's retained log for one socket"),
  shareCaptureJson: t("capture.shareJson", "returns an artifact; the CLIENT writes the file, not the engine"),
  /**
   * `plan/19` WI 5, item 2 — capture → entity construction, the two halves the renderer used to compute.
   *
   * Filed `transport`: 1:1 mappings, no shim and no client half. They exist because the renderer cannot
   * import the engine, so `captureUtils.ts` had to stop holding a second implementation of "which headers
   * to drop, how to decode a body, how a response is stored".
   */
  captureBuildRequest: t("capture.buildRequest", "a captured entry → a `requests` entity body"),
  captureBuildMock: t("capture.buildMock", "a captured exchange → a `mocks` entity body"),

  // ── the 7 subscriptions ──────────────────────────────────────────────────
  onSyncStatus: sub("event.sync.status"),
  onEntitySyncStatus: sub("event.sync.entityStatus"),
  onLogEntry: sub("event.log.entry"),
  onLogChunk: sub("event.log.chunk"),
  onServerError: sub("event.server.error"),
  onWebhookPayload: sub("event.webhook.payload"),
  /**
   * A shim in disguise, and **the name is a fossil — it does not mean the companion**.
   *
   * The renderer only wants *a signal* that config changed; the legacy
   * `companion:refresh` channel carried exactly that, and the shell's `eventBridge.ts` used to
   * degrade the richer bus event back down to it. The wire event is `event.entity.changed` — the
   * protocol's own comment records the rename ("`companion:refresh` becomes `entity.changed`") — so
   * the client subscribes to the payload-carrying event and **discards the payload**, which is what
   * the legacy channel's consumers already did.
   *
   * **Do not remove this with the companion server (2026-09-23).** It was checked when the v1 bridge
   * was deleted and it is **live**: `event.entity.changed` is emitted by `entityCrudCommands` and
   * `folderCommands` on the ordinary CRUD path, `useAppConfigSync` re-reads the config on it, and it
   * is one of the 136 keys `shape.test-d.ts` pins. Only the *name* is historical. A rename would be a
   * change to the frozen client surface, which is a separate decision from this one.
   */
  onCompanionRefresh: sub("event.entity.changed", "payload discarded; the renderer only wants the signal"),
  // The run's own events (`plan/19` WI 4). `progress` carries the result object, so a UI renders results as
  // they arrive instead of waiting for the run to end; `finished` carries the whole report, which is what a
  // client that was disconnected for the run's entire life has to render from.
  onRunnerProgress: sub("event.runner.progress"),
  onRunnerFinished: sub("event.runner.finished"),
  // The socket's two events (`plan/19` WI 6). **Every payload carries `socketId`**, because one bus serves
  // every socket in the process and the renderer has one subscriber per open tab — a subscriber that could
  // not tell which socket an event belonged to could only guess.
  onWsStatus: sub("event.ws.status"),
  onWsMessage: sub("event.ws.message"),
};

/** The keys the client must expose. Derived, so it cannot disagree with `SURFACE`. */
export const SURFACE_KEYS: readonly string[] = Object.keys(SURFACE);

/**
 * Throw unless `keys` is exactly `SURFACE_KEYS` — in **both** directions.
 *
 * Both directions matter and for different reasons. A key present in the surface but not in
 * the classification means a method was written with no recorded decision about what it maps
 * to. A key classified but absent at runtime means the classification describes a method the
 * client would then have to invent, which is how a surface silently grows.
 *
 * Mirrors `assertBridgeIsTotal()` in `packages/engine/src/transport/types.ts`: the same
 * "a map that is merely written down will drift" reasoning, and the same insistence on
 * failing loudly at a known point rather than silently at an unknown one.
 */
export function assertSurfaceIsTotal(keys: readonly string[]): void {
  const declared = new Set(SURFACE_KEYS);
  const actual = new Set(keys);

  const unclassified = keys.filter((k) => !declared.has(k));
  const phantom = SURFACE_KEYS.filter((k) => !actual.has(k));

  if (unclassified.length > 0 || phantom.length > 0) {
    const parts: string[] = [];
    if (unclassified.length > 0) {
      parts.push(`exposed but unclassified: ${unclassified.sort().join(", ")}`);
    }
    if (phantom.length > 0) {
      parts.push(`classified but not exposed: ${phantom.sort().join(", ")}`);
    }
    throw new Error(
      `packages/client/src/surface.ts is out of step with the real window.api surface — ` +
        `${parts.join("; ")}. Every key needs a decision (transport / entity / shim / local / ` +
        `subscribe); see the file header.`,
    );
  }
}
