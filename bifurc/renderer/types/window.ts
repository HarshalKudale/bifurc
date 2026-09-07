import type { AppConfig, SyncState, SyncStatus } from "./config";
import type {
  Environment,
  Folder,
  LocalMapping,
  MockRule,
  ProxyRule,
  SavedGraphQLMock,
  SavedGraphQLRequest,
  SavedGraphQLSchema,
  SavedGrpcMock,
  SavedGrpcRequest,
  SavedProtoFile,
  SavedRequest,
  SavedSoapMock,
  SavedSoapRequest,
  SavedWebhook,
  SavedWsConnection,
  SavedWsdl,
  Workspace
} from "./entities";
import type {
  ImportExportFormatsMap,
  ExportRequest,
  PreflightRequest,
  ImportRequest,
  UpdateCheckResult,
  AuditEntry,
  AuditListOptions,
  ServiceInfo,
  RequestLogEntry,
  ReplayResult,
  LogChunk,
  WebhookPayload,
  HealthBarService
} from "./ipc";

export interface BifurcApi {
  checkUpdate(): Promise<UpdateCheckResult>;
  getConfig(): Promise<AppConfig>;
  loadEntity(wsId: string, kind: string, id: string): Promise<{ ok: boolean; entity?: unknown }>;
  setEntityEnabled(wsId: string, kind: string, id: string, enabled: boolean): Promise<{ ok: boolean; error?: string }>;
  saveConfig(config: AppConfig): Promise<{ ok: boolean }>;
  getImportExportFormats(): Promise<ImportExportFormatsMap>;
  exportData(req: ExportRequest): Promise<{ ok: boolean; filePath?: string; error?: string; canceled?: boolean }>;
  preflightImport(req: PreflightRequest): Promise<{ ok: boolean; filePath?: string; itemCount?: number; collisionIds?: string[]; error?: string; canceled?: boolean }>;
  importData(req: ImportRequest): Promise<{ ok: boolean; imported?: number; skipped?: number; error?: string }>;
  discoverServices(): Promise<ServiceInfo[]>;
  addMapping(mapping: Omit<LocalMapping, "id" | "workspaceId">): Promise<LocalMapping>;
  updateMapping(mapping: LocalMapping): Promise<{ ok: boolean }>;
  deleteMapping(id: string): Promise<{ ok: boolean }>;
  addRule(rule: Omit<ProxyRule, "id" | "createdAt" | "workspaceId">): Promise<ProxyRule>;
  updateRule(rule: ProxyRule): Promise<{ ok: boolean }>;
  deleteRule(id: string): Promise<{ ok: boolean }>;
  addMock(mock: Omit<MockRule, "id" | "createdAt" | "workspaceId">): Promise<MockRule>;
  updateMock(mock: MockRule): Promise<{ ok: boolean }>;
  deleteMock(id: string): Promise<{ ok: boolean }>;
  addRequest(req: Omit<SavedRequest, "id" | "createdAt" | "workspaceId">): Promise<SavedRequest>;
  updateRequest(req: SavedRequest): Promise<{ ok: boolean }>;
  deleteRequest(id: string): Promise<{ ok: boolean }>;
  addWsConnection(conn: Omit<SavedWsConnection, "id" | "createdAt" | "workspaceId">): Promise<SavedWsConnection>;
  updateWsConnection(conn: SavedWsConnection): Promise<{ ok: boolean }>;
  deleteWsConnection(id: string): Promise<{ ok: boolean }>;
  addFolder(kind: "mock" | "request" | "ws" | "webhook" | "rule" | "graphqlRequest" | "graphqlMock" | "grpcRequest" | "grpcMock" | "soapRequest" | "soapMock", folder: Omit<Folder, "id" | "createdAt" | "workspaceId">): Promise<Folder>;
  renameFolder(kind: "mock" | "request" | "ws" | "webhook" | "rule" | "graphqlRequest" | "graphqlMock" | "grpcRequest" | "grpcMock" | "soapRequest" | "soapMock", id: string, name: string): Promise<{ ok: boolean }>;
  moveFolder(kind: "mock" | "request" | "ws" | "webhook" | "rule" | "graphqlRequest" | "graphqlMock" | "grpcRequest" | "grpcMock" | "soapRequest" | "soapMock", id: string, parentId: string | null): Promise<{ ok: boolean }>;
  deleteFolder(kind: "mock" | "request" | "ws" | "webhook" | "rule" | "graphqlRequest" | "graphqlMock" | "grpcRequest" | "grpcMock" | "soapRequest" | "soapMock", id: string): Promise<{ ok: boolean }>;
  addEnvironment(env: Omit<Environment, "id" | "createdAt" | "workspaceId">): Promise<Environment>;
  updateEnvironment(env: Environment): Promise<{ ok: boolean }>;
  deleteEnvironment(id: string): Promise<{ ok: boolean }>;
  setActiveEnvironment(id: string | null): Promise<{ ok: boolean }>;
  addWorkspace(name: string): Promise<Workspace>;
  renameWorkspace(id: string, name: string): Promise<{ ok: boolean }>;
  deleteWorkspace(id: string): Promise<{ ok: boolean }>;
  setActiveWorkspace(id: string): Promise<{ ok: boolean; config: AppConfig }>;
  replayRequest(method: string, url: string, headers: Record<string, string>, body: string): Promise<ReplayResult>;
  proxyStatus(): Promise<{ running: boolean }>;
  serverStatus(): Promise<{ running: boolean; port: number; error: string | null }>;
  restartServer(): Promise<{ ok: boolean }>;
  stopServer(): Promise<{ ok: boolean }>;
  startServer(): Promise<{ ok: boolean }>;
  openExternal(url: string): Promise<void>;
  setTitleBarOverlay?(color: string, symbolColor: string): Promise<{ ok: boolean }>;
  getTheme?(): Promise<string | null>;
  setTheme?(themeId: string): Promise<{ ok: boolean }>;
  listAudit(opts?: AuditListOptions): Promise<{ entries: AuditEntry[]; total: number }>;
  auditDiff(commitHash: string, entity: string, entityId: string, wsId: string): Promise<{ before: unknown | null; after: unknown | null }>;
  exportAudit(format: "json" | "csv"): Promise<{ ok: boolean }>;
  listHistory(opts: { workspaceId?: string; filePath: string; limit?: number; offset?: number }): Promise<{ entries: AuditEntry[]; total: number }>;
  historyDiff(commitHash: string, filePath: string, wsId: string): Promise<{ before: unknown | null; after: unknown | null }>;
  syncSetRemote(wsId: string, remote: string, branch: string): Promise<{ ok: boolean; cloned?: boolean; adoptedId?: string; error?: string }>;
  syncDisconnect(wsId: string): Promise<{ ok: boolean }>;
  syncPush(wsId: string): Promise<{ ok: boolean; error?: string }>;
  syncPull(wsId: string): Promise<{ ok: boolean; updated?: boolean; error?: string }>;
  syncGetState(wsId: string): Promise<SyncState>;
  syncSetAutoSync(wsId: string, enabled: boolean): Promise<{ ok: boolean }>;
  onSyncStatus(cb: (state: { wsId: string; status: SyncStatus; error?: string | null; updatedIds?: string[] }) => void): () => void;
  publishEntity(wsId: string, paths: string[], message?: string): Promise<{ ok: boolean; error?: string }>;
  publishFolder(wsId: string, kind: string, folderName: string | null): Promise<{ ok: boolean; error?: string }>;
  restoreEntity(wsId: string, relPath: string): Promise<{ ok: boolean; error?: string }>;
  gitGetDiff(wsId: string, relPath: string): Promise<{ hasDiff: boolean; status: "clean" | "modified" | "new" | "deleted"; diff?: string; original?: string | null; current?: string | null }>;
  gitDiscard(wsId: string, relPath: string): Promise<{ ok: boolean; error?: string }>;
  gitSync(wsId: string, paths: string[], message?: string): Promise<{ ok: boolean; error?: string }>;
  gitGetHistory(wsId: string, relPath: string, opts?: { limit?: number; offset?: number }): Promise<{ entries: AuditEntry[]; total: number }>;
  getEntitySyncStatus(wsId: string): Promise<Record<string, "clean" | "modified" | "new" | "deleted">>;
  onEntitySyncStatus(cb: (data: { wsId: string; status: Record<string, "clean" | "modified" | "new" | "deleted"> }) => void): () => void;
  executeScript(opts: {
    script: string;
    context: "pre" | "post" | "test";
    request?: { method: string; url: string; headers: Record<string, string>; body: string };
    response?: { status: number; headers: Record<string, string>; body: string; responseTime?: number };
    envVars: Record<string, string>;
  }): Promise<{
    request?: { method: string; url: string; headers: Record<string, string>; body: string };
    response?: { status: number; headers: Record<string, string>; body: string };
    envVars: Record<string, string>;
    error?: string;
    testResults?: { name: string; passed: boolean; error?: string; durationMs: number }[];
    testLogs?: string[];
  }>;
  onLogEntry(cb: (entry: RequestLogEntry) => void): () => void;
  onServerError(cb: (error: string) => void): () => void;
  healthbarGetServices(wsId: string): Promise<HealthBarService[]>;
  healthbarSaveServices(wsId: string, services: HealthBarService[]): Promise<{ ok: boolean }>;
  healthbarCheckUrl(url: string): Promise<{
    ok: boolean;
    statusCode: number | null;
    body: string | null;
    headers: Record<string, string> | null;
    error: string | null;
    durationMs: number;
  }>;
  // Webhooks
  addWebhook(hook: Omit<SavedWebhook, "id" | "createdAt" | "workspaceId">): Promise<SavedWebhook>;
  updateWebhook(hook: SavedWebhook): Promise<{ ok: boolean }>;
  deleteWebhook(id: string): Promise<{ ok: boolean }>;
  registerActiveWebhook(webhookId: string, urlSuffix: string): Promise<{ ok: boolean }>;
  unregisterActiveWebhook(webhookId: string): Promise<{ ok: boolean }>;
  getWebhookServerStatus(): Promise<{ running: boolean; port: number; error: string | null }>;
  webhookServerStatus(): Promise<{ running: boolean; port: number; error: string | null }>;
  startWebhookServer(port?: number): Promise<{ ok: boolean }>;
  stopWebhookServer(): Promise<{ ok: boolean }>;
  onWebhookPayload(cb: (payload: WebhookPayload) => void): () => void;
  // -- SOAP -------------------------------------------------------------
  addSoapRequest(req: Omit<SavedSoapRequest, "id" | "createdAt" | "workspaceId">): Promise<SavedSoapRequest>;
  updateSoapRequest(req: SavedSoapRequest): Promise<{ ok: boolean }>;
  deleteSoapRequest(id: string): Promise<{ ok: boolean }>;
  addSoapMock(mock: Omit<SavedSoapMock, "id" | "createdAt" | "workspaceId">): Promise<SavedSoapMock>;
  updateSoapMock(mock: SavedSoapMock): Promise<{ ok: boolean }>;
  deleteSoapMock(id: string): Promise<{ ok: boolean }>;
  addWsdl(wsdl: Omit<SavedWsdl, "id" | "createdAt" | "workspaceId">): Promise<SavedWsdl>;
  deleteWsdl(id: string): Promise<{ ok: boolean }>;
  listWsdls(): Promise<SavedWsdl[]>;
  soapFetchWsdl(url: string): Promise<{ ok: boolean; content?: string; error?: string }>;
  soapExecute(endpointUrl: string, soapAction: string, headers: Record<string, string>, body: string): Promise<{ status: number; headers: Record<string, string>; body: string; durationMs: number }>;
  // -- GraphQL ------------------------------------------------------------
  addGraphQLRequest(req: Omit<SavedGraphQLRequest, "id" | "createdAt" | "workspaceId">): Promise<SavedGraphQLRequest>;
  updateGraphQLRequest(req: SavedGraphQLRequest): Promise<{ ok: boolean }>;
  deleteGraphQLRequest(id: string): Promise<{ ok: boolean }>;
  addGraphQLMock(mock: Omit<SavedGraphQLMock, "id" | "createdAt" | "workspaceId">): Promise<SavedGraphQLMock>;
  updateGraphQLMock(mock: SavedGraphQLMock): Promise<{ ok: boolean }>;
  deleteGraphQLMock(id: string): Promise<{ ok: boolean }>;
  addGraphQLSchema(schema: Omit<SavedGraphQLSchema, "id" | "createdAt" | "workspaceId">): Promise<SavedGraphQLSchema>;
  deleteGraphQLSchema(id: string): Promise<{ ok: boolean }>;
  listGraphQLSchemas(): Promise<SavedGraphQLSchema[]>;
  graphqlIntrospect(endpointUrl: string, headers: Record<string, string>): Promise<{ ok: boolean; sdl?: string; error?: string }>;
  graphqlExecute(endpointUrl: string, headers: Record<string, string>, query: string, variables: string, operationName: string): Promise<{ status: number; headers: Record<string, string>; body: string; durationMs: number }>;
  // -- gRPC --------------------------------------------------------------
  addGrpcRequest(req: Omit<SavedGrpcRequest, "id" | "createdAt" | "workspaceId">): Promise<SavedGrpcRequest>;
  updateGrpcRequest(req: SavedGrpcRequest): Promise<{ ok: boolean }>;
  deleteGrpcRequest(id: string): Promise<{ ok: boolean }>;
  addGrpcMock(mock: Omit<SavedGrpcMock, "id" | "createdAt" | "workspaceId">): Promise<SavedGrpcMock>;
  updateGrpcMock(mock: SavedGrpcMock): Promise<{ ok: boolean }>;
  deleteGrpcMock(id: string): Promise<{ ok: boolean }>;
  addProtoFile(proto: Omit<SavedProtoFile, "id" | "createdAt" | "workspaceId">): Promise<SavedProtoFile>;
  deleteProtoFile(id: string): Promise<{ ok: boolean }>;
  listProtoFiles(): Promise<SavedProtoFile[]>;
  grpcExecute(serverAddress: string, serviceName: string, methodName: string, requestBody: string, metadata: Record<string, string>, protoFileId: string | null, useReflection: boolean): Promise<{ ok: boolean; responses?: string[]; metadata?: Record<string, string>; status?: number; statusMessage?: string; durationMs?: number; error?: string }>;
  grpcReflect(serverAddress: string): Promise<{ ok: boolean; services?: { name: string; methods: { name: string; inputType: string; outputType: string; clientStreaming: boolean; serverStreaming: boolean }[] }[]; error?: string }>;
  grpcMockServerStatus(): Promise<{ running: boolean; port: number }>;
  grpcStartMockServer(): Promise<{ ok: boolean; error?: string }>;
  grpcStopMockServer(): Promise<{ ok: boolean }>;
  tlsImportCert(): Promise<{ ok: boolean; path?: string }>;
  tlsImportKey(): Promise<{ ok: boolean; path?: string }>;
  tlsRemoveCert(): Promise<{ ok: boolean }>;
  tlsGenerate(): Promise<{ ok: boolean; certPath?: string; keyPath?: string; error?: string }>;
  tlsInstallCA(): Promise<{ ok: boolean; needsManualInstall?: boolean; instructions?: string; error?: string }>;
  tlsExportCert(): Promise<{ ok: boolean; filePath?: string; canceled?: boolean; error?: string }>;
  tlsCertStatus(): Promise<{ generated: boolean; certPath: string | null; keyPath: string | null }>;
  onCompanionRefresh(cb: () => void): () => void;
  openFileDialog(): Promise<{ name: string; size: number; base64: string; mimeType: string } | { error: string } | null>;
  pickFilePath(title: string, filters?: { name: string; extensions: string[] }[]): Promise<string | null>;
  pickFolderPath(title: string): Promise<string | null>;
  platform: string;
  onLogChunk(cb: (chunk: LogChunk) => void): () => void;
  shareCaptureJson(entries: unknown[], suggestedName?: string): Promise<{ ok: boolean; filePath?: string; canceled?: boolean; error?: string }>;

  // -- Collection Runner ----------------------------------------------------
  saveRunnerReport(wsId: string, report: unknown): Promise<{ ok: boolean; error?: string }>;
  getRunHistory(wsId: string, folderId: string): Promise<{ timestamp: number; summary: { total: number; passed: number; failed: number } }[]>;
  exportRunnerReport(report: unknown): Promise<{ ok: boolean; filePath?: string; error?: string }>;
  saveRunnerConfig(wsId: string, folderId: string, config: { requestOrder: string[]; delayMs: number }): Promise<{ ok: boolean }>;
  loadRunnerConfig(wsId: string, folderId: string): Promise<{ requestOrder: string[]; delayMs: number } | null>;
  listRunnerFolderIds(wsId: string): Promise<string[]>;
}

declare global {
  interface Window {
    api: BifurcApi;
  }
}
