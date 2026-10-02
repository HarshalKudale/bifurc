import type { AppConfig, SettingsPatch, SyncState, SyncStatus, Workspace, EntitySyncStatus, EntityStatusPayload, WorkspaceGitState } from "./config";
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
  ParsedCurlRequest,
  CapturedRequestPayload,
  CapturedMockPayload,
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
  WsStatusResult,
  WsListResult,
  WsMessage,
  WsStatusEvent,
  WsMessageEvent,
  LogChunk,
  HealthBarService
} from "./ipc";

export interface BifurcApi {
  checkUpdate(): Promise<UpdateCheckResult>;
  getConfig(): Promise<AppConfig>;
  loadEntity(wsId: string, kind: string, id: string): Promise<{ ok: boolean; entity?: unknown }>;
  setEntityEnabled(wsId: string, kind: string, id: string, enabled: boolean): Promise<{ ok: boolean; error?: string }>;
  updateSettings(patch: SettingsPatch): Promise<{ ok: boolean }>;
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
  addFolder(kind: "mock" | "request" | "ws" | "rule" | "graphqlRequest" | "grpcRequest" | "soapRequest", folder: Omit<Folder, "id" | "createdAt" | "workspaceId">): Promise<Folder>;
  renameFolder(kind: "mock" | "request" | "ws" | "rule" | "graphqlRequest" | "grpcRequest" | "soapRequest", id: string, name: string): Promise<{ ok: boolean }>;
  moveFolder(kind: "mock" | "request" | "ws" | "rule" | "graphqlRequest" | "grpcRequest" | "soapRequest", id: string, parentId: string | null): Promise<{ ok: boolean }>;
  deleteFolder(kind: "mock" | "request" | "ws" | "rule" | "graphqlRequest" | "grpcRequest" | "soapRequest", id: string): Promise<{ ok: boolean }>;
  addEnvironment(env: Omit<Environment, "id" | "createdAt" | "workspaceId">): Promise<Environment>;
  updateEnvironment(env: Environment): Promise<{ ok: boolean }>;
  deleteEnvironment(id: string): Promise<{ ok: boolean }>;
  setActiveEnvironment(id: string | null): Promise<{ ok: boolean }>;
  /** Resolve `{{tokens}}` in `text` against the workspace's **merged** environment (`plan/19` WI 5, item 4).
   *
   * The engine owns resolution since WI 3, so this is how the renderer *displays* the value a probe will
   * use — the health bar shows the resolved url it is about to check. `workspaceId` is optional and the
   * client resolves the active one when omitted.
   *
   * **⚠ `{{random.*}}` mints a fresh value per call**, so the result is for display, never for sending: a
   * request that needs a value stable across a retry must let the engine resolve it inside the pipeline. */
  envResolveText(text: string, workspaceId?: string): Promise<{ text: string }>;
  /** The workspace's **merged** environment — `__global__` overridden by the active one (`plan/19` WI 6).
   *
   * It replaced `App.tsx`'s `mergeEnvVars`, which was the last renderer implementation of a workspace fact the
   * engine owns (D23). The renderer keeps needing it for exactly one thing, and it is a **list**, not a
   * resolver: `EnvVarHint` offers the variable names available at the cursor. That is the D28 list-vs-logic
   * split — the UI may hold the data, never compute the answer.
   *
   * `null` means the workspace has neither a global nor an active environment, which is different from an
   * environment with no variables: `EnvVarHint` renders nothing for `null` and an empty picker for `[]`. */
  envGetMerged(workspaceId?: string): Promise<{ id: string; name: string; variables: { id: string; key: string; value: string }[]; createdAt: number; workspaceId: string } | null>;
  /** Build a `requests` entity body from a captured log entry (`plan/19` WI 5, item 2).
   *
   * The construction — which headers to drop, how to decode the body — is the engine's. The renderer used to
   * compute it (`captureUtils.reqToHeadersBody`) and cannot any longer, because `@bifurc/gui` has no
   * `@bifurc/engine` dependency by design. The result's `name` is blank; the caller labels it. */
  captureBuildRequest(entry: RequestLogEntry): Promise<{ request: CapturedRequestPayload }>;
  /** Build a `mocks` entity body from a captured exchange. **The captured headers are kept raw** — a mock
   * replays the capture byte-for-byte — which is the one respect in which this differs from
   * `captureBuildRequest`, whose headers are stripped. */
  captureBuildMock(entry: RequestLogEntry): Promise<{ mock: CapturedMockPayload }>;
  /** The engine's captured-traffic history for a workspace, newest first. */
  captureList(wsId: string): Promise<RequestLogEntry[]>;
  captureClear(wsId: string): Promise<{ ok: boolean }>;
  captureRemove(wsId: string, ids: string[]): Promise<{ ok: boolean; removed: number }>;
  addWorkspace(name: string): Promise<Workspace>;
  renameWorkspace(id: string, name: string): Promise<{ ok: boolean }>;
  deleteWorkspace(id: string): Promise<{ ok: boolean }>;
  setActiveWorkspace(id: string): Promise<{ ok: boolean; config: AppConfig }>;
  replayRequest(method: string, url: string, headers: Record<string, string>, body: string): Promise<ReplayResult>;
  /** Parse a cURL command (`plan/19` WI 5).
   *
   * The parser is the engine's (`packages/engine/src/utils/curl.ts`); this replaced
   * `packages/gui/src/lib/curlParser.ts`, which the boundary ratchet's `curlParser` construct tracked.
   * The returned header map is **unfiltered** — the caller drops hop-by-hop headers itself, from the
   * pinned copy in `@/lib/utils`. */
  requestParseCurl(curl: string): Promise<ParsedCurlRequest>;
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
  /**
   * The socket registry's two events (`plan/19` WI 6). **Both payloads carry `socketId`**, and that is not
   * decoration: one bus serves every socket in the process and the renderer has one subscriber per open tab,
   * so an event that did not say which socket it belonged to could only be routed by guessing.
   */
  onWsStatus(cb: (payload: WsStatusEvent) => void): () => void;
  /** One frame, in either direction. The whole `WsMessage` is carried, `id` and `ts` included. */
  onWsMessage(cb: (payload: WsMessageEvent) => void): () => void;
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

  // -- WebSocket connection registry (`plan/19` WI 6) -------------------------
  /**
   * Open (or re-open) a socket. **`socketId` is the caller's name for it** — for the UI, the tab id it already
   * had — because the engine has no idea what a tab is and does not pretend to. The engine enforces one live
   * socket per `{workspaceId, socketId}`; a second `wsConnect` on a live key **replaces** it, which is what the
   * renderer's own "Reconnecting" path did.
   *
   * **Resolves when the socket has been constructed, not opened.** The transition to `connected` arrives as
   * `onWsStatus`, and `wsStatus` answers for the state in between — D29's shape (`runnerStart`'s), for the same
   * reason: a caller that had to await the open would hold the connection's lifetime, which is what moved to
   * the engine.
   *
   * `url` and every header **value** are resolved against the workspace's merged environment (D23). Unlike the
   * browser API the renderer used, the engine's client **can** send headers — so a resolved `Authorization`
   * header now reaches the wire instead of being resolved and discarded.
   */
  wsConnect(wsId: string, socketId: string, url: string, headers?: Record<string, string>): Promise<{ ok: boolean; error?: string }>;
  /** Close a socket — **the only thing that does**. A client disconnect and a workspace switch do not (D29).
   *
   * `{ok:true}` for a socket that is already closed, and `{ok:false}` only when the engine has never been asked
   * to open that id — a typo'd `socketId` must not read as success. */
  wsDisconnect(wsId: string, socketId: string): Promise<{ ok: boolean; error?: string }>;
  /** Write a frame. The payload is resolved **engine-side**, so `{{random.*}}` re-mints per send — correct for
   * a stream, where each frame is its own event rather than a retry of an earlier one.
   *
   * `{ok:false, error:"Not connected"}` when nothing is open, rather than a throw. */
  wsSend(wsId: string, socketId: string, data: string): Promise<{ ok: boolean; error?: string }>;
  /** The socket's state, live or finished, or `null` if it was never opened. **This is what a reconnecting
   * client asks**, and it returns the engine's retained message log — so a tab re-opened mid-stream renders the
   * backlog rather than only what happens next. */
  wsStatus(wsId: string, socketId: string): Promise<WsStatusResult | null>;
  /** The **live** sockets, plus the registry's limit. `max` comes from the engine rather than a pinned copy in
   * the UI, because the registry is what refuses the sixth connection. */
  wsList(wsId: string): Promise<WsListResult>;
  /** Drop the engine's retained log for one socket. The socket is untouched.
   *
   * A command rather than local state, because `wsStatus` returns that log: a renderer-only clear would be
   * undone by the next re-attach, which is the class of bug this work item removes. */
  wsClearMessages(wsId: string, socketId: string): Promise<{ ok: boolean; error?: string }>;
}

declare global {
  interface Window {
    api: BifurcApi;
  }
}
