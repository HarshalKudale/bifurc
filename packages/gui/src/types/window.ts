import type { AppConfig, SyncState, SyncStatus, Workspace, EntitySyncStatus, EntityStatusPayload, WorkspaceGitState } from "./config";
import type {
  Environment,
  Folder,
  LocalMapping,
  MockRule,
  ProxyRule,
  SavedGraphQLRequest,
  SavedGraphQLSchema,
  SavedGrpcRequest,
  SavedProtoFile,
  SavedRequest,
  SavedSoapRequest,
  SavedWebhook,
  SavedWsConnection,
  SavedWsdl
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
  RequestSendParams,
  RequestSendResult,
  GraphqlSendParams,
  SoapSendParams,
  GrpcSendParams,
  HttpLikeResult,
  GrpcExecuteResult,
  RunnerStatusResult,
  RunnerProgressEvent,
  RunnerFinishedEvent,
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
  addFolder(kind: "mock" | "request" | "ws" | "webhook" | "rule" | "graphqlRequest" | "grpcRequest" | "soapRequest", folder: Omit<Folder, "id" | "createdAt" | "workspaceId">): Promise<Folder>;
  renameFolder(kind: "mock" | "request" | "ws" | "webhook" | "rule" | "graphqlRequest" | "grpcRequest" | "soapRequest", id: string, name: string): Promise<{ ok: boolean }>;
  moveFolder(kind: "mock" | "request" | "ws" | "webhook" | "rule" | "graphqlRequest" | "grpcRequest" | "soapRequest", id: string, parentId: string | null): Promise<{ ok: boolean }>;
  deleteFolder(kind: "mock" | "request" | "ws" | "webhook" | "rule" | "graphqlRequest" | "grpcRequest" | "soapRequest", id: string): Promise<{ ok: boolean }>;
  addEnvironment(env: Omit<Environment, "id" | "createdAt" | "workspaceId">): Promise<Environment>;
  updateEnvironment(env: Environment): Promise<{ ok: boolean }>;
  deleteEnvironment(id: string): Promise<{ ok: boolean }>;
  setActiveEnvironment(id: string | null): Promise<{ ok: boolean }>;
  addWorkspace(name: string): Promise<Workspace>;
  renameWorkspace(id: string, name: string): Promise<{ ok: boolean }>;
  deleteWorkspace(id: string): Promise<{ ok: boolean }>;
  setActiveWorkspace(id: string): Promise<{ ok: boolean; config: AppConfig }>;
  replayRequest(method: string, url: string, headers: Record<string, string>, body: string): Promise<ReplayResult>;
  /** Run the REST pipeline (`plan/19` D24): resolve, pre-script, send, post-script, test.
   *
   * **Two input forms, one command.** `{id}` names a saved `requests` entity and lets the engine read it;
   * `{request, scripts}` sends what the caller assembled, for a draft tab. The engine decides from the
   * shape, and the two differ only in where the request comes from — which is why this tab no longer
   * resolves `{{variables}}` or runs scripts itself.
   *
   * `workspaceId` is optional because the renderer has none to send — the client resolves the active one,
   * as it does for `listGraphQLSchemas`. A caller addressing a specific workspace passes it. */
  requestSend(params: RequestSendParams, workspaceId?: string): Promise<RequestSendResult>;
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
  /**
   * Both resolve to the **commit id** they created, or `commit: null` when there was nothing to
   * commit. `ok: true` alone cannot distinguish those two, and that distinction is the whole point:
   * Publish and Sync are `git add` + `git commit`, so a hash is the proof that git did something.
   */
  publishEntity(wsId: string, paths: string[], message?: string): Promise<{ ok: boolean; error?: string; commit?: string | null }>;
  publishFolder(wsId: string, kind: string, folderName: string | null): Promise<{ ok: boolean; error?: string; commit?: string | null }>;
  restoreEntity(wsId: string, relPath: string): Promise<{ ok: boolean; error?: string }>;
  gitGetDiff(wsId: string, relPath: string): Promise<{ hasDiff: boolean; status: "clean" | "modified" | "new" | "deleted"; diff?: string; original?: string | null; current?: string | null }>;
  gitDiscard(wsId: string, relPath: string): Promise<{ ok: boolean; error?: string }>;
  gitSync(wsId: string, paths: string[], message?: string): Promise<{ ok: boolean; error?: string }>;
  gitGetHistory(wsId: string, relPath: string, opts?: { limit?: number; offset?: number }): Promise<{ entries: AuditEntry[]; total: number }>;
  getEntitySyncStatus(wsId: string): Promise<EntityStatusPayload>;
  onEntitySyncStatus(cb: (data: { wsId: string; status: Record<string, EntitySyncStatus>; git?: WorkspaceGitState }) => void): () => void;
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
  addWsdl(wsdl: Omit<SavedWsdl, "id" | "createdAt" | "workspaceId">): Promise<SavedWsdl>;
  deleteWsdl(id: string): Promise<{ ok: boolean }>;
  listWsdls(): Promise<SavedWsdl[]>;
  soapFetchWsdl(url: string): Promise<{ ok: boolean; content?: string; error?: string }>;
  soapExecute(endpointUrl: string, soapAction: string, headers: Record<string, string>, body: string): Promise<{ status: number; headers: Record<string, string>; body: string; durationMs: number }>;
  /** Resolve every field against the workspace's environment, then POST the SOAP envelope.
   *
   * **Two input forms, one command** — `{id}` for a saved `soapRequests` entity, `{request}` for what the
   * editor holds. Resolution is the engine's (`plan/19` D23): a `{{token}}` in the endpoint, the SOAPAction, a
   * header **or** the body now resolves engine-side. See `requestSend` for the optional `workspaceId`. */
  soapSend(params: SoapSendParams, workspaceId?: string): Promise<HttpLikeResult>;
  // -- GraphQL ------------------------------------------------------------
  addGraphQLRequest(req: Omit<SavedGraphQLRequest, "id" | "createdAt" | "workspaceId">): Promise<SavedGraphQLRequest>;
  updateGraphQLRequest(req: SavedGraphQLRequest): Promise<{ ok: boolean }>;
  deleteGraphQLRequest(id: string): Promise<{ ok: boolean }>;
  addGraphQLSchema(schema: Omit<SavedGraphQLSchema, "id" | "createdAt" | "workspaceId">): Promise<SavedGraphQLSchema>;
  deleteGraphQLSchema(id: string): Promise<{ ok: boolean }>;
  listGraphQLSchemas(): Promise<SavedGraphQLSchema[]>;
  graphqlIntrospect(endpointUrl: string, headers: Record<string, string>): Promise<{ ok: boolean; sdl?: string; error?: string }>;
  graphqlExecute(endpointUrl: string, headers: Record<string, string>, query: string, variables: string, operationName: string): Promise<{ status: number; headers: Record<string, string>; body: string; durationMs: number }>;
  /** Resolve every field against the workspace's environment, then execute the GraphQL request.
   *
   * **Two input forms, one command** — `{id}` for a saved `graphqlRequests` entity, `{request}` for what the
   * editor holds. Resolution is the engine's (`plan/19` D23), which is why this tab no longer calls
   * `resolveVars`: a `{{token}}` in the endpoint, a header, the query, the variables **or** the operation name
   * now resolves engine-side. See `requestSend` for the optional `workspaceId`. */
  graphqlSend(params: GraphqlSendParams, workspaceId?: string): Promise<HttpLikeResult>;
  // -- gRPC --------------------------------------------------------------
  addGrpcRequest(req: Omit<SavedGrpcRequest, "id" | "createdAt" | "workspaceId">): Promise<SavedGrpcRequest>;
  updateGrpcRequest(req: SavedGrpcRequest): Promise<{ ok: boolean }>;
  deleteGrpcRequest(id: string): Promise<{ ok: boolean }>;
  addProtoFile(proto: Omit<SavedProtoFile, "id" | "createdAt" | "workspaceId">): Promise<SavedProtoFile>;
  deleteProtoFile(id: string): Promise<{ ok: boolean }>;
  listProtoFiles(): Promise<SavedProtoFile[]>;
  grpcExecute(serverAddress: string, serviceName: string, methodName: string, requestBody: string, metadata: Record<string, string>, protoFileId: string | null, useReflection: boolean): Promise<{ ok: boolean; responses?: string[]; metadata?: Record<string, string>; status?: number; statusMessage?: string; durationMs?: number; error?: string }>;
  grpcReflect(serverAddress: string): Promise<{ ok: boolean; services?: { name: string; methods: { name: string; inputType: string; outputType: string; clientStreaming: boolean; serverStreaming: boolean }[] }[]; error?: string }>;
  /** Resolve every field against the workspace's environment, then dispatch the gRPC call.
   *
   * **Two input forms, one command** — `{id}` for a saved `grpcRequests` entity, `{request}` for what the
   * editor holds. Resolution is the engine's (`plan/19` D23): a `{{token}}` in the address, the service name,
   * the method name, the body or a metadata value now resolves engine-side. `protoFileId` is deliberately
   * **not** resolved — it names a stored file rather than being template text.
   *
   * `grpc.execute` is a `wontfix` stub, so this resolves to `{ok:false, error}` today. See `requestSend` for
   * the optional `workspaceId`. */
  grpcSend(params: GrpcSendParams, workspaceId?: string): Promise<GrpcExecuteResult>;
  tlsImportCert(): Promise<{ ok: boolean; path?: string }>;
  tlsImportKey(): Promise<{ ok: boolean; path?: string }>;
  tlsRemoveCert(): Promise<{ ok: boolean }>;
  tlsGenerate(): Promise<{ ok: boolean; certPath?: string; keyPath?: string; error?: string }>;
  tlsInstallCA(): Promise<{ ok: boolean; needsManualInstall?: boolean; instructions?: string; error?: string }>;
  tlsExportCert(): Promise<{ ok: boolean; filePath?: string; canceled?: boolean; error?: string }>;
  tlsCertStatus(): Promise<{ generated: boolean; certPath: string | null; keyPath: string | null }>;
  onCompanionRefresh(cb: () => void): () => void;
  /** One request of a run finished. See `runnerStart` for why progress is an event and not a return value. */
  onRunnerProgress(cb: (payload: RunnerProgressEvent) => void): () => void;
  /** The run ended, cancelled or not — carries the whole report. */
  onRunnerFinished(cb: (payload: RunnerFinishedEvent) => void): () => void;
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
  /** The run's own commands (`plan/19` WI 4, D29/D30). **Additive** — the storage methods above are unchanged.
   *
   * `runnerStart` resolves when the run is *registered*, not when it finishes (D29): progress arrives as
   * `event.runner.progress` and the end as `event.runner.finished`. `runnerStatus` is what a client that was
   * disconnected asks on reconnect, and `null` means the engine has never been asked to run this folder. */
  runnerStart(wsId: string, folderId: string): Promise<{ ok: boolean; total?: number; error?: string }>;
  runnerCancel(wsId: string, folderId: string): Promise<{ ok: boolean; error?: string }>;
  runnerStatus(wsId: string, folderId: string): Promise<RunnerStatusResult | null>;
}

declare global {
  interface Window {
    api: BifurcApi;
  }
}
