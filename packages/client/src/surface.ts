/**
 * The `window.api` surface, enumerated and classified. **This is the spec.**
 *
 * P5 exists to protect exactly one property (plan/README "Non-negotiables" #3):
 * `window.api` stays **byte-identical** through P5–P6. That property is the regression
 * signal for the whole critical path, so the surface is written down here as data rather
 * than left implicit in 144 hand-written methods.
 *
 * ## Why this file exists at all
 *
 * The plan's precondition says `renderer/types/window.ts` is "the authoritative list of the
 * surface to reproduce", and its acceptance criterion says "All 136 methods present". Both
 * numbers are stale, and the difference is load-bearing:
 *
 * | source | keys |
 * |---|---|
 * | `src/preload.ts` — what `window.api` actually **is** at runtime | **144** |
 * | `renderer/types/window.ts` — what it is **declared** to be | 140 |
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
 * `renderer/types/window.ts` although `src/preload.ts` always provides them. So the declared type
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
 * - **`entity`** — the CRUD collapse. Sixteen kinds × `add`/`update`/`delete`(/`list`) become
 *   `entity.create`/`update`/`delete`/`list` with a `kind` injected. This is where the
 *   signature drift risk lives, so it is data rather than code.
 * - **`shim`** — a genuine multi-step or transformed mapping (`exportData` is
 *   `export.create` + a blob fetch; `preflightImport` is `blob.put` + `import.preflight`).
 * - **`local`** — **not a transport call at all.** Dialogs, zoom, theme, titlebar, `platform`,
 *   OS trust-store, first-launch. These must never be routed over the transport: they are
 *   operations on the *client's own machine*, and on a remote engine they would be either
 *   meaningless or a security hole.
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
  checkUpdate: t("app.checkUpdate"),
  getConfig: t("config.get"),
  loadEntity: t("entity.load"),
  setEntityEnabled: t("entity.setEnabled"),
  saveConfig: t("config.save"),

  // ── import / export (all three are multi-step over the transport) ─────────
  getImportExportFormats: t("export.formats"),
  exportData: s("export.create", "two-step: export.create returns an artifact, then fetch its blob"),
  preflightImport: s("import.preflight", "three-step: blob.put the content, then import.preflight with the blobId"),
  importData: s("import.commit", "three-step: blob.put the content, then import.commit with the blobId"),
  discoverServices: t("services.discover"),

  // ── the CRUD collapse: 16 kinds × add/update/delete(/list) = 48 methods ───
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
  addGraphQLMock: e("graphqlMocks", "create"),
  updateGraphQLMock: e("graphqlMocks", "update"),
  deleteGraphQLMock: e("graphqlMocks", "delete"),
  addGraphQLSchema: e("graphqlSchemas", "create"),
  deleteGraphQLSchema: e("graphqlSchemas", "delete"),
  listGraphQLSchemas: e("graphqlSchemas", "list", true),
  addGrpcRequest: e("grpcRequests", "create"),
  updateGrpcRequest: e("grpcRequests", "update"),
  deleteGrpcRequest: e("grpcRequests", "delete"),
  addGrpcMock: e("grpcMocks", "create"),
  updateGrpcMock: e("grpcMocks", "update"),
  deleteGrpcMock: e("grpcMocks", "delete"),
  addProtoFile: e("protoFiles", "create"),
  deleteProtoFile: e("protoFiles", "delete"),
  listProtoFiles: e("protoFiles", "list", true),
  addSoapRequest: e("soapRequests", "create"),
  updateSoapRequest: e("soapRequests", "update"),
  deleteSoapRequest: e("soapRequests", "delete"),
  addSoapMock: e("soapMocks", "create"),
  updateSoapMock: e("soapMocks", "update"),
  deleteSoapMock: e("soapMocks", "delete"),
  addWsdl: e("wsdls", "create"),
  deleteWsdl: e("wsdls", "delete"),
  listWsdls: e("wsdls", "list", true),
  addEnvironment: e("environments", "create"),
  updateEnvironment: e("environments", "update"),
  deleteEnvironment: e("environments", "delete"),

  // ── folders ──────────────────────────────────────────────────────────────
  // NOTE: `folder.*` takes a *singular* kind ("mock", "request", "ws", …) — a different
  // vocabulary from `EntityKindValue`'s plurals. That asymmetry is in the frozen protocol,
  // not introduced here; see `packages/engine/src/commands/entityKindMap.ts`'s sibling.
  addFolder: t("folder.add"),
  renameFolder: t("folder.rename"),
  moveFolder: t("folder.move"),
  deleteFolder: t("folder.delete"),

  // ── environments / workspaces ────────────────────────────────────────────
  setActiveEnvironment: t("env.setActive"),
  addWorkspace: t("workspace.add"),
  renameWorkspace: t("workspace.rename"),
  deleteWorkspace: t("workspace.delete"),
  setActiveWorkspace: t("workspace.setActive"),

  // ── server / proxy ───────────────────────────────────────────────────────
  replayRequest: t("request.replay"),
  proxyStatus: t("proxy.status"),
  serverStatus: t("server.status"),
  restartServer: t("server.restart"),
  stopServer: t("server.stop"),
  startServer: t("server.start"),

  // ── client-local: OS / window operations, never a transport call ─────────
  openExternal: local("opens a URL on the client's own machine"),
  setTitleBarOverlay: local("Electron BrowserWindow; read by renderer/hooks/useTheme.ts"),
  tlsInstallCA: local("mutates the client machine's OS trust store"),

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
  grpcMockServerStatus: t("grpc.mockServerStatus"),
  grpcStartMockServer: t("grpc.startMockServer"),
  grpcStopMockServer: t("grpc.stopMockServer"),

  // ── TLS / certificates ───────────────────────────────────────────────────
  tlsImportCert: t("tls.importCert", "needs a blobId the signature never supplies: dialog + blob.put precede it"),
  tlsImportKey: t("tls.importKey", "needs a blobId the signature never supplies: dialog + blob.put precede it"),
  tlsRemoveCert: t("tls.removeCert"),
  tlsGenerate: t("tls.generate"),
  tlsExportCert: t("tls.exportCert", "returns an artifact; the CLIENT writes the file, not the engine"),
  tlsCertStatus: t("tls.certStatus"),

  // ── dialogs + client-local shell state ───────────────────────────────────
  openFileDialog: local("native file picker on the client's own machine"),
  pickFilePath: local("native file picker on the client's own machine"),
  pickFolderPath: local("native folder picker on the client's own machine"),
  platform: local("was process.platform; the plan calls it dead code, kept for shape"),
  isFirstLaunch: local("shell launch state; exposed by preload but undeclared in BifurcApi"),
  completeFirstLaunch: local("shell launch state; exposed by preload but undeclared in BifurcApi"),
  getZoomLevel: local("Electron webContents zoom; exposed by preload but undeclared in BifurcApi"),
  setZoomLevel: local("Electron webContents zoom; exposed by preload but undeclared in BifurcApi"),
  getTheme: local("Electron nativeTheme; read by renderer/hooks/useTheme.ts"),
  setTheme: local("Electron nativeTheme; read by renderer/hooks/useTheme.ts"),

  // ── collection runner ────────────────────────────────────────────────────
  saveRunnerReport: t("runner.saveReport"),
  getRunHistory: t("runner.getHistory"),
  exportRunnerReport: t("runner.exportReport", "requires a format the signature omits; returns an artifact the CLIENT writes"),
  saveRunnerConfig: t("runner.saveConfig"),
  loadRunnerConfig: t("runner.loadConfig"),
  listRunnerFolderIds: t("runner.listFolderIds"),
  shareCaptureJson: t("capture.shareJson", "returns an artifact; the CLIENT writes the file, not the engine"),

  // ── the 7 subscriptions ──────────────────────────────────────────────────
  onSyncStatus: sub("event.sync.status"),
  onEntitySyncStatus: sub("event.sync.entityStatus"),
  onLogEntry: sub("event.log.entry"),
  onLogChunk: sub("event.log.chunk"),
  onServerError: sub("event.server.error"),
  onWebhookPayload: sub("event.webhook.payload"),
  /**
   * A shim in disguise. The renderer only wants *a signal* that config changed; the legacy
   * `companion:refresh` channel carried exactly that, and the shell's `eventBridge.ts` used to
   * degrade the richer bus event back down to it. The wire event is `event.entity.changed` — the
   * protocol's own comment records the rename ("`companion:refresh` becomes `entity.changed`") — so
   * the client subscribes to the payload-carrying event and **discards the payload**, which is what
   * the legacy channel's consumers already did.
   */
  onCompanionRefresh: sub("event.entity.changed", "payload discarded; the renderer only wants the signal"),
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
