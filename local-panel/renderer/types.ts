export interface LocalMapping {
  id: string;
  domain: string;
  target: string;
  enabled: boolean;
  label?: string;
  workspaceId: string;
}

export interface ProxyRule {
  id: string;
  name: string;
  pattern: string;
  useRegex: boolean;
  targetType: "mapping" | "external";
  targetMappingId: string;
  targetExternal: string;
  requestScript: string;
  responseScript: string;
  enabled: boolean;
  createdAt: number;
  folderId?: string | null;
  workspaceId: string;
}

export interface Folder {
  id: string;
  name: string;
  parentId: string | null;
  createdAt: number;
  workspaceId: string;
}

export interface EnvVariable {
  id: string;
  key: string;
  value: string;
}

export interface Environment {
  id: string;
  name: string;
  variables: EnvVariable[];
  createdAt: number;
  workspaceId: string;
}

export interface MockRule {
  id: string;
  name: string;
  method: string;
  urlPattern: string;
  useRegex: boolean;
  enabled: boolean;
  capturedHeaders: Record<string, string>;
  capturedBody: string;       // base64
  responseStatus: number;
  responseStatusMocked?: boolean;
  responseHeaders: Record<string, string>;
  mockedResponseHeaders?: string[];
  responseBody: string;       // plain text or base64 (see responseBodyEncoding)
  responseBodyMocked?: boolean;
  responseBodyEncoding?: "utf8" | "base64";  // default "utf8"; "base64" for binary bodies
  responseDelay?: number;     // ms to wait before sending response (0 = no delay)
  responseDelayMocked?: boolean;
  streamingMode?: "none" | "sse" | "chunked";  // default "none"
  streamingChunkDelay?: number;  // ms between chunks (default 100)
  streamingChunkSeparator?: string;  // delimiter to split body into chunks
  protocol?: "rest";
  createdAt: number;
  folderId?: string | null;
  workspaceId: string;
}

export type ApiProtocol = "rest" | "graphql" | "grpc" | "soap";

export interface SavedRequest {
  id: string;
  name: string;
  method: string;
  url: string;
  headers: Record<string, string>;
  body: string;    // plain text
  protocol?: "rest";
  preScript?: string;
  postScript?: string;
  testScript?: string;
  createdAt: number;
  folderId?: string | null;
  workspaceId: string;
}

export interface SavedWsConnection {
  id: string;
  name: string;
  url: string;
  headers: Record<string, string>;
  createdAt: number;
  folderId?: string | null;
  workspaceId: string;
}

export interface SavedWebhook {
  id: string;
  name: string;
  /** User-defined suffix appended after /localpanel/webhooks/ */
  urlSuffix: string;
  createdAt: number;
  folderId?: string | null;
  workspaceId: string;
}

export interface SavedGrpcRequest {
  id: string;
  name: string;
  serverAddress: string;
  serviceName: string;
  methodName: string;
  requestBody: string;
  metadata: Record<string, string>;
  protoFileId?: string | null;
  useReflection: boolean;
  streamingType: "unary" | "server" | "client" | "bidi";
  protocol?: "grpc";
  preScript?: string;
  postScript?: string;
  testScript?: string;
  createdAt: number;
  folderId?: string | null;
  workspaceId: string;
}

export interface SavedGrpcMock {
  id: string;
  name: string;
  enabled: boolean;
  serviceName: string;
  methodName: string;
  responseBody: string;
  responseMetadata: Record<string, string>;
  responseDelay?: number;
  streamingResponses?: string[];
  errorCode?: number;
  errorMessage?: string;
  protoFileId: string;
  protocol?: "grpc";
  createdAt: number;
  folderId?: string | null;
  workspaceId: string;
}

export interface SavedProtoFile {
  id: string;
  name: string;
  content: string;
  parsedServices?: { name: string; methods: { name: string; inputType: string; outputType: string; clientStreaming: boolean; serverStreaming: boolean }[] }[];
  createdAt: number;
  workspaceId: string;
}

export interface SavedSoapRequest {
  id: string;
  name: string;
  endpointUrl: string;
  soapAction: string;
  headers: Record<string, string>;
  body: string;
  wsdlId?: string | null;
  operationName?: string;
  protocol?: "soap";
  preScript?: string;
  postScript?: string;
  testScript?: string;
  createdAt: number;
  folderId?: string | null;
  workspaceId: string;
}

export interface SavedSoapMock {
  id: string;
  name: string;
  enabled: boolean;
  endpointPattern: string;
  useRegex: boolean;
  soapActionPattern: string;
  operationName?: string;
  responseStatus: number;
  responseHeaders: Record<string, string>;
  responseBody: string;
  responseDelay?: number;
  wsdlId?: string | null;
  protocol?: "soap";
  createdAt: number;
  folderId?: string | null;
  workspaceId: string;
}

export interface SavedWsdl {
  id: string;
  name: string;
  content: string;
  sourceUrl?: string;
  importedAt: number;
  createdAt: number;
  workspaceId: string;
}

export interface SavedGraphQLRequest {
  id: string;
  name: string;
  endpointUrl: string;
  headers: Record<string, string>;
  query: string;
  variables: string;
  operationName: string;
  protocol?: "graphql";
  preScript?: string;
  postScript?: string;
  testScript?: string;
  schemaId?: string | null;
  createdAt: number;
  folderId?: string | null;
  workspaceId: string;
}

export interface SavedGraphQLMock {
  id: string;
  name: string;
  enabled: boolean;
  endpointPattern: string;
  useRegex: boolean;
  operationType: "query" | "mutation" | "subscription" | "any";
  operationName: string;
  responseStatus: number;
  responseHeaders: Record<string, string>;
  responseBody: string;
  responseDelay?: number;
  schemaId?: string | null;
  protocol?: "graphql";
  createdAt: number;
  folderId?: string | null;
  workspaceId: string;
}

export type UnifiedRequest =
  | (SavedRequest & { protocol?: "rest" })
  | (SavedGraphQLRequest & { protocol: "graphql" })
  | (SavedGrpcRequest & { protocol: "grpc" })
  | (SavedSoapRequest & { protocol: "soap" });

export type UnifiedMock =
  | (MockRule & { protocol?: "rest" })
  | (SavedGraphQLMock & { protocol: "graphql" })
  | (SavedGrpcMock & { protocol: "grpc" })
  | (SavedSoapMock & { protocol: "soap" });

export interface SavedGraphQLSchema {
  id: string;
  name: string;
  content: string;
  endpointUrl?: string;
  introspectedAt?: number;
  createdAt: number;
  workspaceId: string;
}

export interface SyncConfig {
  remote: string;
  branch: string;
  autoSync: boolean;
}

export interface SyncMeta {
  lastPushedAt: number | null;
  lastPulledAt: number | null;
  lastSyncedCommit: string | null;
}

export type SyncStatus = "idle" | "pushing" | "pulling" | "cloning" | "error";

export interface SyncState {
  status: SyncStatus;
  error: string | null;
  lastPushedAt: number | null;
  lastPulledAt: number | null;
  progressMessage: string | null;
}

export interface Workspace {
  id: string;
  name: string;
  createdAt: number;
  activeEnvironmentId: string | null;
  syncConfig?: SyncConfig | null;
  syncMeta?: SyncMeta | null;
}

export interface AppConfig {
  port: number;
  webhookPort: number;
  companionPort: number;
  minimizeToTray: boolean;
  tlsEnabled: boolean;
  tlsCaCertPath: string | null;
  tlsCaKeyPath: string | null;
  workspaces: Workspace[];
  activeWorkspaceId: string;
  mappings: LocalMapping[];
  proxyRules: ProxyRule[];
  ruleFolders: Folder[];
  mocks: MockRule[];
  requests: SavedRequest[];
  mockFolders: Folder[];
  requestFolders: Folder[];
  wsConnections: SavedWsConnection[];
  wsFolders: Folder[];
  webhooks: SavedWebhook[];
  webhookFolders: Folder[];
  graphqlRequests: SavedGraphQLRequest[];
  graphqlMocks: SavedGraphQLMock[];
  graphqlSchemas: SavedGraphQLSchema[];
  graphqlRequestFolders: Folder[];
  graphqlMockFolders: Folder[];
  grpcRequests: SavedGrpcRequest[];
  grpcMocks: SavedGrpcMock[];
  protoFiles: SavedProtoFile[];
  grpcRequestFolders: Folder[];
  grpcMockFolders: Folder[];
  grpcMockServerPort: number;
  soapRequests: SavedSoapRequest[];
  soapMocks: SavedSoapMock[];
  savedWsdls: SavedWsdl[];
  soapRequestFolders: Folder[];
  soapMockFolders: Folder[];
  environments: Environment[];
  activeEnvironmentId: string | null;
}

export interface ServiceInfo {
  port: number;
  address: string;
  pid: number;
  processName: string;
}

export interface HealthBarService {
  id: string;
  name: string;
  url: string;
  autoRefreshEnabled: boolean;
  createdAt: number;
}

export interface RequestLogEntry {
  id: string;
  ts: number;
  method: string;
  url: string;
  host: string;
  status: number | null;
  via: "rfc6761" | "proxy" | "rule" | "mock" | "error";
  target: string | null;
  durationMs: number | null;
  reqHeaders: Record<string, string>;
  reqBody: string;      // base64
  resHeaders: Record<string, string>;
  resBody: string;      // base64
  resStatus: number | null;
}

export interface ReplayResult {
  status: number;
  headers: Record<string, string>;
  body: string; // base64
}

export interface LogChunk {
  logId: string;
  chunk: string;   // base64-encoded chunk data
  done: boolean;
}

export interface WebhookPayload {
  webhookId: string;
  urlSuffix: string;
  ts: number;
  method: string;
  headers: Record<string, string>;
  body: string;
}

// -- Import/Export types (mirrored from src/ipc/importExport/types.ts) --------

export type ImportExportEntityKind =
  | "workspace" | "requests" | "mocks" | "environments"
  | "mappings" | "proxyRules" | "websockets" | "webhooks";

export type CollisionStrategy = "keep" | "override" | "new";

export interface ImportExportFormatDef {
  id: string;
  label: string;
  extensions: string[];
  supportsExport: boolean;
  supportsImport: boolean;
}

export type ImportExportFormatsMap = Record<string, ImportExportFormatDef[]>;

export interface ExportRequest {
  kind: ImportExportEntityKind;
  format: string;
  wsId: string;
}

export interface PreflightRequest {
  kind: ImportExportEntityKind;
  format: string;
  wsId: string;
}

export interface ImportRequest {
  kind: ImportExportEntityKind;
  format: string;
  wsId: string;
  filePath: string;
  collisionStrategy: CollisionStrategy;
}

export interface AuthUser {
  id: string;
  email: string;
  displayName?: string | null;
}

export interface SubscriptionState {
  active: boolean;
  plan: string | null;
  expiresAt: number | null;
}

declare const __APP_VERSION__: string;

export interface UpdateCheckResult {
  ok: boolean;
  hasUpdate: boolean;
  currentVersion: string;
  latestVersion: string;
  releaseName?: string;
  releaseNotes?: string;
  publishedAt?: string;
  downloadUrl: string;
  releaseUrl: string;
  assetName?: string;
  error?: string;
}

declare global {
  interface Window {
    api: {
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
      webhookServerStatus(): Promise<{ running: boolean; port: number; error: string | null }>;
      startWebhookServer(): Promise<{ ok: boolean }>;
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
      // -- Auth -----------------------------------------------------------------
      authConfigured(): Promise<boolean>;
      authGetUser(): Promise<AuthUser | null>;
      authSignInWithEmail(email: string, password: string): Promise<{ ok: boolean; user?: AuthUser; error?: string }>;
      authSignUpWithEmail(email: string, password: string): Promise<{ ok: boolean; needsConfirmation?: boolean; user?: AuthUser; error?: string }>;
      authSignOut(): Promise<{ ok: boolean; error?: string }>;
      onAuthChanged(cb: (data: { user: AuthUser | null }) => void): () => void;
      // -- Subscription ---------------------------------------------------------
      getSubscription(): Promise<SubscriptionState>;
      refreshSubscription(): Promise<SubscriptionState>;
      resetSubscription(): Promise<SubscriptionState>;
      onSubscriptionChanged(cb: (state: SubscriptionState) => void): () => void;
      // -- First-launch ---------------------------------------------------------
      isFirstLaunch(): Promise<boolean>;
      completeFirstLaunch(): Promise<{ ok: boolean }>;
      // -- Collection Runner ----------------------------------------------------
      saveRunnerReport(wsId: string, report: unknown): Promise<{ ok: boolean; error?: string }>;
      getRunHistory(wsId: string, folderId: string): Promise<{ timestamp: number; summary: { total: number; passed: number; failed: number } }[]>;
      exportRunnerReport(report: unknown): Promise<{ ok: boolean; filePath?: string; error?: string }>;
      saveRunnerConfig(wsId: string, folderId: string, config: { requestOrder: string[]; delayMs: number }): Promise<{ ok: boolean }>;
      loadRunnerConfig(wsId: string, folderId: string): Promise<{ requestOrder: string[]; delayMs: number } | null>;
      listRunnerFolderIds(wsId: string): Promise<string[]>;
    };
  }
}

export type AuditAction = "create" | "update" | "delete";
export type AuditEntity =
  | "mock" | "mapping" | "rule" | "environment"
  | "request" | "wsConnection" | "webhook" | "folder" | "workspace";

export interface AuditEntry {
  commitHash: string;
  ts: number;
  action: AuditAction;
  entity: AuditEntity;
  entityId: string;
  entityName: string;
  workspaceId: string;
  actor: string;
  changedFields?: string[];  // field names that changed (update only)
}

export interface AuditListOptions {
  entity?: AuditEntity;
  action?: AuditAction;
  workspaceId?: string;
  entityId?: string;
  fromTs?: number;
  toTs?: number;
  search?: string;
  limit?: number;
  offset?: number;
}
