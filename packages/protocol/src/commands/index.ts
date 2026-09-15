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
import * as SyncCmd from "./sync";
import * as AuditCmd from "./audit";
import * as TlsCmd from "./tls";
import * as ExportCmd from "./export";
import * as MiscCmd from "./misc";

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
export * from "./sync";
export * from "./audit";
export * from "./tls";
export * from "./export";
export * from "./misc";

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
  "grpc.mockServerStatus": spec(GrpcCmd.GrpcMockServerStatusParams, "grpc:mockServerStatus"),
  "grpc.startMockServer": spec(GrpcCmd.GrpcStartMockServerParams, "grpc:startMockServer"),
  "grpc.stopMockServer": spec(GrpcCmd.GrpcStopMockServerParams, "grpc:stopMockServer"),

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

  // misc
  "script.execute": spec(MiscCmd.ScriptExecuteParams, "script:execute"),
  "request.replay": spec(MiscCmd.RequestReplayParams, "request:replay"),
  "healthbar.getServices": spec(MiscCmd.HealthbarGetServicesParams, "healthbar:getServices"),
  "healthbar.saveServices": spec(MiscCmd.HealthbarSaveServicesParams, "healthbar:saveServices"),
  "healthbar.checkUrl": spec(MiscCmd.HealthbarCheckUrlParams, "healthbar:checkUrl"),
  "app.checkUpdate": spec(MiscCmd.AppCheckUpdateParams, "app:checkUpdate"),
  "capture.shareJson": spec(MiscCmd.CaptureShareJsonParams, "capture:shareJson"),
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
