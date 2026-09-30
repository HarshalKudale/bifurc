/**
 * The command registry — every ENGINE and SPLIT (engine-half) command, keyed by its
 * `namespace.verb` wire name (work item 3), mapped to its params schema.
 *
 * This is what P2's `CommandRegistry` (checklist: "Replace `registerIpcHandlers()` with the
 * `CommandRegistry`") validates every inbound payload against, and what the P1 acceptance
 * criterion "every command has a Zod schema; a smoke test validates a valid and an invalid
 * payload per command" (see `packages/protocol/src/commands/index.test.ts`) exercises.
 *
 * `legacyChannel` records today's `namespace:verb` IPC channel for each command, purely as a
 * migration aid for P2 (mapping the new registry back onto the existing handler functions) and
 * for anyone diffing this file against `plan/handler-classification.md`. It is not part of the
 * wire protocol.
 */
import { z } from "zod";
import * as EntityCmd from "./entity";
import * as FolderCmd from "./folder";
import * as ConfigCmd from "./config";
import * as ServerCmd from "./server";
import * as GraphqlCmd from "./graphql";
import * as SoapCmd from "./soap";
import * as GrpcCmd from "./grpc";
import * as ApplicationCmd from "./application";
import * as WebhookCmd from "./webhook";
import * as RunnerCmd from "./runner";
import * as WsCmd from "./ws";
import * as SyncCmd from "./sync";
import * as AuditCmd from "./audit";
import * as TlsCmd from "./tls";
import * as ExportCmd from "./export";
import * as BlobCmd from "./blob";
import * as MiscCmd from "./misc";
import * as SavedCmd from "./saved";

export * from "./entity";
export * from "./folder";
export * from "./config";
export * from "./server";
export * from "./graphql";
export * from "./soap";
export * from "./grpc";
export * from "./application";
export * from "./webhook";
export * from "./runner";
export * from "./ws";
export * from "./sync";
export * from "./audit";
export * from "./tls";
export * from "./export";
export * from "./blob";
export * from "./misc";
export * from "./saved";
// `COMMAND_FIXTURES` — one valid and one invalid payload per command. Exported from the package
// rather than left inside `index.test.ts` because the transport conformance suite
// (`packages/engine/tests/conformance/`) drives its every-command matrix off the same table, and two
// copies of a 93-entry table is a guarantee that one will be wrong. `fixtures.ts` imports
// `CommandAction` from here with `import type`, which is erased — so this re-export creates no
// runtime cycle.
export * from "./fixtures";

interface CommandSpec {
  params: z.ZodType;
  legacyChannel: string;
}

function spec(params: z.ZodType, legacyChannel: string): CommandSpec {
  return { params, legacyChannel };
}

export const COMMANDS = {
  // entity.* (collapsed CRUD, work item 2)
  "entity.create": spec(EntityCmd.EntityCreateParams, "<kind>:add"),
  "entity.update": spec(EntityCmd.EntityUpdateParams, "<kind>:update"),
  "entity.delete": spec(EntityCmd.EntityDeleteParams, "<kind>:delete"),
  "entity.load": spec(EntityCmd.EntityLoadParams, "entity:load"),
  "entity.list": spec(EntityCmd.EntityListParams, "entity:list"), // new — no direct legacy channel, was implicit in config:get
  "entity.setEnabled": spec(EntityCmd.EntitySetEnabledParams, "entity:setEnabled"),

  // folder.*
  "folder.add": spec(FolderCmd.FolderAddParams, "folder:add"),
  "folder.rename": spec(FolderCmd.FolderRenameParams, "folder:rename"),
  "folder.move": spec(FolderCmd.FolderMoveParams, "folder:move"),
  "folder.delete": spec(FolderCmd.FolderDeleteParams, "folder:delete"),
  "folder.publish": spec(FolderCmd.FolderPublishParams, "folder:publish"),

  // config.*, workspace.*, env.*
  "config.get": spec(ConfigCmd.ConfigGetParams, "config:get"),
  "config.save": spec(ConfigCmd.ConfigSaveParams, "config:save"),
  "workspace.add": spec(ConfigCmd.WorkspaceAddParams, "workspace:add"),
  "workspace.rename": spec(ConfigCmd.WorkspaceRenameParams, "workspace:rename"),
  "workspace.delete": spec(ConfigCmd.WorkspaceDeleteParams, "workspace:delete"),
  "workspace.setActive": spec(ConfigCmd.WorkspaceSetActiveParams, "workspace:setActive"),
  "env.setActive": spec(ConfigCmd.EnvSetActiveParams, "env:setActive"),
  // `plan/19` WI 5, item 4 — the resolution *display* path. The engine resolves; the health bar shows what
  // a check will probe. See `config.ts` for why a check-only path would leave the card unresolved.
  "env.resolveText": spec(ConfigCmd.EnvResolveTextParams, "env.resolveText"),
  // `plan/19` WI 6 — the other half of the same idea, for the one renderer need that is a *list* rather than
  // a resolved string: the env-var autocomplete hint. It replaced `App.tsx`'s `mergeEnvVars`, which was the
  // last renderer implementation of a workspace fact the engine already owns.
  "env.getMerged": spec(ConfigCmd.EnvGetMergedParams, "env.getMerged"),

  // server.*, proxy.*, services.*
  "server.status": spec(ServerCmd.ServerStatusParams, "server:status"),
  "server.start": spec(ServerCmd.ServerStartParams, "server:start"),
  "server.stop": spec(ServerCmd.ServerStopParams, "server:stop"),
  "server.restart": spec(ServerCmd.ServerRestartParams, "server:restart"),
  "proxy.status": spec(ServerCmd.ProxyStatusParams, "proxy:status"),
  "services.discover": spec(ServerCmd.ServicesDiscoverParams, "services:discover"),

  // graphql.*
  "graphql.introspect": spec(GraphqlCmd.GraphqlIntrospectParams, "graphql:introspect"),
  "graphql.execute": spec(GraphqlCmd.GraphqlExecuteParams, "graphql:execute"),

  // soap.*
  "soap.fetchWsdl": spec(SoapCmd.SoapFetchWsdlParams, "soap:fetchWsdl"),
  "soap.execute": spec(SoapCmd.SoapExecuteParams, "soap:execute"),

  // grpc.*
  "grpc.execute": spec(GrpcCmd.GrpcExecuteParams, "grpc:execute"),
  "grpc.reflect": spec(GrpcCmd.GrpcReflectParams, "grpc:reflect"),

  // application.*
  "application.list": spec(ApplicationCmd.ApplicationListParams, "applications:list"),
  "application.save": spec(ApplicationCmd.ApplicationSaveParams, "applications:save"),
  "application.delete": spec(ApplicationCmd.ApplicationDeleteParams, "applications:delete"),
  "application.start": spec(ApplicationCmd.ApplicationStartParams, "applications:start"),
  "application.stop": spec(ApplicationCmd.ApplicationStopParams, "applications:stop"),
  "application.getState": spec(ApplicationCmd.ApplicationGetStateParams, "applications:getState"),
  "application.getAllStates": spec(ApplicationCmd.ApplicationGetAllStatesParams, "applications:getAllStates"),
  "application.getLogs": spec(ApplicationCmd.ApplicationGetLogsParams, "applications:getLogs"),
  "application.checkPort": spec(ApplicationCmd.ApplicationCheckPortParams, "applications:checkPort"),
  "application.killPort": spec(ApplicationCmd.ApplicationKillPortParams, "applications:killPort"),

  // webhook.*, webhookServer.*
  "webhook.registerActive": spec(WebhookCmd.WebhookRegisterActiveParams, "webhook:registerActive"),
  "webhook.unregisterActive": spec(WebhookCmd.WebhookUnregisterActiveParams, "webhook:unregisterActive"),
  "webhookServer.start": spec(WebhookCmd.WebhookServerStartParams, "webhookServer:start"),
  "webhookServer.stop": spec(WebhookCmd.WebhookServerStopParams, "webhookServer:stop"),
  "webhookServer.status": spec(WebhookCmd.WebhookServerStatusParams, "webhookServer:status"),

  // runner.*
  "runner.saveReport": spec(RunnerCmd.RunnerSaveReportParams, "runner:saveReport"),
  "runner.exportReport": spec(RunnerCmd.RunnerExportReportParams, "runner:exportReport"),
  "runner.getHistory": spec(RunnerCmd.RunnerGetHistoryParams, "runner:getHistory"),
  "runner.saveConfig": spec(RunnerCmd.RunnerSaveConfigParams, "runner:saveConfig"),
  "runner.loadConfig": spec(RunnerCmd.RunnerLoadConfigParams, "runner:loadConfig"),
  "runner.listFolderIds": spec(RunnerCmd.RunnerListFolderIdsParams, "runner:listFolderIds"),
  // ── the run's own commands — `plan/19` WI 4 (D29/D30) ──────────────────────────────────────────
  //
  // `runner.start` / `runner.cancel` / `runner.status` drive the run itself; the six above store its report
  // and its per-folder configuration. The split is deliberate and the storage commands' contract is
  // **unchanged**, so these three are purely additive — a client that does not call them behaves exactly as
  // it did before.
  "runner.start": spec(RunnerCmd.RunnerStartParams, "runner.start"),
  "runner.cancel": spec(RunnerCmd.RunnerCancelParams, "runner.cancel"),
  "runner.status": spec(RunnerCmd.RunnerStatusParams, "runner.status"),

  // ws.* — the WebSocket connection registry, engine-side (`plan/19` WI 6).
  //
  // The module `Protocol_Module_Contract.md` §4 is written for: a socket outlives a single call, so the four
  // decisions (scope, lifetime D29, concurrency, event completeness) are made in `ws.ts` rather than
  // discovered. **`ws.status` / `ws.list` are `read` while the other four are not** — see `scopes.ts`, and
  // `ws.ts` for why `ws.connect` *replaces* rather than refusing.
  //
  // These replace a renderer-only implementation, so this is the one namespace in the protocol with **no
  // legacy channel at all**: `useWebSocket.ts` spoke to the browser's `WebSocket` constructor, not to the
  // shell, and nothing was ever registered under these names.
  "ws.connect": spec(WsCmd.WsConnectParams, "ws.connect"),
  "ws.disconnect": spec(WsCmd.WsDisconnectParams, "ws.disconnect"),
  "ws.send": spec(WsCmd.WsSendParams, "ws.send"),
  "ws.status": spec(WsCmd.WsStatusParams, "ws.status"),
  "ws.list": spec(WsCmd.WsListParams, "ws.list"),
  "ws.clearMessages": spec(WsCmd.WsClearMessagesParams, "ws.clearMessages"),

  // sync.*, git.*, entity.publish/restore
  "sync.setRemote": spec(SyncCmd.SyncSetRemoteParams, "sync:setRemote"),
  "sync.disconnect": spec(SyncCmd.SyncDisconnectParams, "sync:disconnect"),
  "sync.push": spec(SyncCmd.SyncPushParams, "sync:push"),
  "sync.pull": spec(SyncCmd.SyncPullParams, "sync:pull"),
  "sync.getState": spec(SyncCmd.SyncGetStateParams, "sync:getState"),
  "sync.setAutoSync": spec(SyncCmd.SyncSetAutoSyncParams, "sync:setAutoSync"),
  "sync.getEntityStatus": spec(SyncCmd.SyncGetEntityStatusParams, "sync:getEntityStatus"),
  "git.diff": spec(SyncCmd.GitDiffParams, "git:diff"),
  "git.discard": spec(SyncCmd.GitDiscardParams, "git:discard"),
  "git.sync": spec(SyncCmd.GitSyncParams, "git:sync"),
  "git.history": spec(SyncCmd.GitHistoryParams, "git:history"),
  "entity.publish": spec(SyncCmd.EntityPublishParams, "entity:publish"),
  "entity.restore": spec(SyncCmd.EntityRestoreParams, "entity:restore"),

  // audit.*, history.*
  "audit.list": spec(AuditCmd.AuditListParams, "audit:list"),
  "audit.diff": spec(AuditCmd.AuditDiffParams, "audit:diff"),
  "audit.export": spec(AuditCmd.AuditExportParams, "audit:export"),
  "history.list": spec(AuditCmd.HistoryListParams, "history:list"),
  "history.diff": spec(AuditCmd.HistoryDiffParams, "history:diff"),

  // tls.*
  "tls.generate": spec(TlsCmd.TlsGenerateParams, "tls:generate"),
  "tls.certStatus": spec(TlsCmd.TlsCertStatusParams, "tls:certStatus"),
  "tls.removeCert": spec(TlsCmd.TlsRemoveCertParams, "tls:removeCert"),
  "tls.exportCert": spec(TlsCmd.TlsExportCertParams, "tls:exportCert"),
  "tls.importCert": spec(TlsCmd.TlsImportCertParams, "tls:importCert"),
  "tls.importKey": spec(TlsCmd.TlsImportKeyParams, "tls:importKey"),

  // export.*, import.*
  "export.formats": spec(ExportCmd.ExportFormatsParams, "importExport:formats"),
  "export.create": spec(ExportCmd.ExportCreateParams, "importExport:export"),
  "import.preflight": spec(ExportCmd.ImportPreflightParams, "importExport:preflight"),
  "import.commit": spec(ExportCmd.ImportCommitParams, "importExport:import"),

  // blob.* — the staging layer (P3). New in P3, so there is no legacy channel to record; the wire
  // name is repeated here purely to keep the map uniform. See `File_Ops_Protocol.md` §3.
  "blob.put": spec(BlobCmd.BlobPutParams, "blob:put"),
  "blob.stat": spec(BlobCmd.BlobStatParams, "blob:stat"),
  "blob.read": spec(BlobCmd.BlobReadParams, "blob:read"),
  "blob.release": spec(BlobCmd.BlobReleaseParams, "blob:release"),

  // misc
  "script.execute": spec(MiscCmd.ScriptExecuteParams, "script:execute"),
  "request.replay": spec(MiscCmd.RequestReplayParams, "request:replay"),
  // `plan/19` WI 5 — the cURL parser. A pure function of its input, which is why it is `read`-scoped and
  // idempotent; it exists so the renderer stops carrying `lib/curlParser.ts` (deleted by that item).
  "request.parseCurl": spec(MiscCmd.RequestParseCurlParams, "request.parseCurl"),
  "healthbar.getServices": spec(MiscCmd.HealthbarGetServicesParams, "healthbar:getServices"),
  "healthbar.saveServices": spec(MiscCmd.HealthbarSaveServicesParams, "healthbar:saveServices"),
  "healthbar.checkUrl": spec(MiscCmd.HealthbarCheckUrlParams, "healthbar:checkUrl"),
  "capture.shareJson": spec(MiscCmd.CaptureShareJsonParams, "capture:shareJson"),
  // `plan/19` WI 5, item 2 — capture → entity construction. The renderer's `captureUtils.ts` kept only its
  // formatting half; these two are what it used to compute for itself.
  "capture.buildRequest": spec(MiscCmd.CaptureBuildRequestParams, "capture.buildRequest"),
  "capture.buildMock": spec(MiscCmd.CaptureBuildMockParams, "capture.buildMock"),

  // ── the saved-entity counterparts, and `request.send` ─────────────────────────────────────────
  //
  // The headless consumer is the whole reason: a CLI has a request *id*, not a request payload, and
  // re-sending every field from the client would put the stored shape in the client's hands. Three of
  // the four take `{workspaceId, id}`.
  //
  // **`request.send` is no longer one of those.** It is `plan/19` D24/D27's REST pipeline — a union of
  // `{workspaceId, id}` and `{workspaceId, request, scripts}` — because D24 makes "load the entity" and
  // "send what the caller assembled" the *same five steps* with two sources of request. It **replaced**
  // `request.replaySaved`, a name that had never been released; see `plan/protocol-changes.md`'s
  // 2026-09-30 entry, which is also why this is not a `PROTOCOL_VERSION` bump. The other three are
  // renamed and widened when their own pipelines land — `plan/19` §8 trap 1, one commit per protocol.
  //
  // See `saved.ts` for the group, the naming, and the four near-misses (`graphql.introspect`,
  // `soap.fetchWsdl`, `grpc.reflect`, `script.execute`) that were deliberately left out.
  //
  // `legacyChannel` repeats the wire name, following `blob.*`: these commands never had a legacy
  // channel, so there is nothing to record there and the field exists only to keep the map uniform.
  "request.send": spec(SavedCmd.RequestSendParams, "request.send"),
  "graphql.send": spec(SavedCmd.GraphqlSendParams, "graphql.send"),
  "soap.send": spec(SavedCmd.SoapSendParams, "soap.send"),
  "grpc.send": spec(SavedCmd.GrpcSendParams, "grpc.send"),
} as const;

export type CommandAction = keyof typeof COMMANDS;

export function isKnownCommand(action: string): action is CommandAction {
  return Object.prototype.hasOwnProperty.call(COMMANDS, action);
}

export function getCommandParamsSchema(action: CommandAction): z.ZodType {
  return COMMANDS[action].params;
}

export function getLegacyChannel(action: CommandAction): string {
  return COMMANDS[action].legacyChannel;
}

/**
 * The companion extension's four commands are a FROZEN public API across at least one minor
 * protocol version (work item 7, "The extension exception"). Their action names are unchanged
 * by the P1 naming pass — do not rename or reshape `mock:add`, `request:add`, `folder:add`,
 * `config:get` here. `folder:add`/`config:get` already read as legacy channel names above (the
 * extension speaks the OLD wire shape directly); `mock:add`/`request:add` collapse into the
 * generic `entity.create{kind:"mocks"|"requests"}` internally, so the engine-side transport
 * must keep accepting the literal strings `"mock:add"` and `"request:add"` as aliases that
 * translate to `entity.create` with the right `kind` — this mapping belongs in the same
 * hand-written shim file as the P5 compatibility shims (`plan/README.md`, "Protocol
 * compatibility").
 */
export const EXTENSION_FROZEN_COMMANDS = ["mock:add", "request:add", "folder:add", "config:get"] as const;
