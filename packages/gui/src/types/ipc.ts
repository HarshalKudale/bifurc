import type { AppConfig, SyncState, SyncStatus, Workspace } from "./config";
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
  /**
   * Mirrors `RequestLogEntry["via"]` in `@bifurc/engine/proxy/logEmitter.ts`, by hand — that file is
   * the source of truth and carries the note on what each value means and why the union is
   * engine-local. Widen it there first; this copy is what makes a forgotten `VIA_LABEL` /
   * `fulfilledBy` case a compile error instead of a badge that renders blank.
   */
  via: "rfc6761" | "mapping" | "proxy" | "rule" | "mock" | "error";
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

/**
 * `graphql.send`'s two input forms, and the result it returns — mirrors of `GraphqlSendParams` in
 * `@bifurc/protocol` (`commands/saved.ts`) and `HttpLikeResult` (`commands/graphql.ts`).
 *
 * `request` is the **stored entity's shape** — `endpointUrl`, not `url` — because the inline form carries
 * what the *editor* holds. The rename is the engine's business, and using the command's name here would stop
 * the id form and the inline form being interchangeable. Hand-mirrored like the rest of this file: see
 * `RequestSendRequest`'s note for why `packages/gui` declares these rather than importing them.
 */
export interface GraphqlSendRequest {
  endpointUrl: string;
  headers: Record<string, string>;
  query: string;
  variables: string;
  operationName: string;
}

export type GraphqlSendParams = { id: string } | { request: GraphqlSendRequest };

/** `graphql.execute`'s result, which `graphql.send` returns unchanged. */
export interface HttpLikeResult {
  status: number;
  headers: Record<string, string>;
  body: string;
  durationMs: number;
}

/**
 * `soap.send`'s two input forms — mirrors of `SoapSendParams` in `@bifurc/protocol` (`commands/saved.ts`).
 *
 * `request` is the stored entity's shape, which for SOAP is **also** `soap.execute`'s: all four fields are
 * named identically on both sides, so unlike GraphQL there is no rename for the engine to perform.
 */
export interface SoapSendRequest {
  endpointUrl: string;
  soapAction: string;
  headers: Record<string, string>;
  body: string;
}

export type SoapSendParams = { id: string } | { request: SoapSendRequest };

/**
 * `grpc.send`'s two input forms and its result — mirrors of `GrpcSendParams` in `@bifurc/protocol`
 * (`commands/saved.ts`) and `GrpcExecuteResult` (`commands/grpc.ts`).
 *
 * `protoFileId` and `useReflection` are **required**, matching the protocol: the editor always has both (a
 * proto file may be `null`, reflection may be off), so making them optional here would let a caller omit a
 * decision the protocol requires it to make.
 */
export interface GrpcSendRequest {
  serverAddress: string;
  serviceName: string;
  methodName: string;
  requestBody: string;
  metadata: Record<string, string>;
  protoFileId: string | null;
  useReflection: boolean;
}

export type GrpcSendParams = { id: string } | { request: GrpcSendRequest };

/**
 * `grpc.execute`'s result, which `grpc.send` returns unchanged.
 *
 * It declares `ok`/`error`, unlike its three siblings' results — and that is why `grpc.send` throws
 * `NOT_FOUND` for a missing entity rather than resolving a failure-shaped value: the stub *also* answers
 * `{ok:false, error}`, so a missing entity reported the same way would be indistinguishable from it.
 */
export interface GrpcExecuteResult {
  ok: boolean;
  responses?: string[];
  metadata?: Record<string, string>;
  status?: number;
  statusMessage?: string;
  durationMs?: number;
  error?: string;
}

/**
 * The collection runner's result shapes — mirrors of `RunnerRequestResult` / `CollectionRunReport` in
 * `packages/engine/src/runner/runCollection.ts` and `RunnerStatusResult` in `@bifurc/protocol`.
 *
 * **They move here rather than living beside the runner's UI**, which is where they were until 2026-09-30
 * (`packages/gui/src/lib/collectionRunner.ts`). `plan/19` WI 4 moves the run itself into the engine, so the
 * renderer keeps only the *types* — and they belong with the other wire mirrors, not with a module that no
 * longer implements anything.
 */
export interface RunnerRequestResult {
  requestId: string;
  requestName: string;
  /** The method and url **as actually sent** — resolved, and rewritten by the pre-script if there was one. */
  method: string;
  url: string;
  status: number | null;
  responseTime: number;
  error?: string;
  tests: { name: string; passed: boolean; error?: string; durationMs: number }[];
  testLogs: string[];
  preScriptError?: string;
  postScriptError?: string;
}

export interface CollectionRunReport {
  folderId: string;
  folderName?: string;
  startedAt: number;
  completedAt: number;
  totalRequests: number;
  totalTests: number;
  passedTests: number;
  failedTests: number;
  results: RunnerRequestResult[];
}

/** `runner.status`'s result. `null` (the method's return) means "never started", which is not "ran and produced nothing". */
export interface RunnerStatusResult {
  workspaceId: string;
  folderId: string;
  startedAt: number;
  total: number;
  completed: number;
  running: boolean;
  cancelled: boolean;
  completedAt?: number;
  error?: string;
  results: RunnerRequestResult[];
  report?: CollectionRunReport;
}

/** `event.runner.progress`'s payload — mirrors the protocol's schema (`events.ts`). */
export interface RunnerProgressEvent {
  workspaceId: string;
  folderId: string;
  /** 0-based, matching the position in the finished report's `results`. */
  index: number;
  /** On every event rather than only on the start call, because a client can attach mid-run (D29). */
  total: number;
  result: RunnerRequestResult;
}

/** `event.runner.finished`'s payload — carries the whole report, which is what a disconnected client renders from. */
export interface RunnerFinishedEvent {
  workspaceId: string;
  folderId: string;
  report: CollectionRunReport;
}

/**
 * `request.send`'s two input forms — mirrors of `RequestSendParams` in `@bifurc/protocol`
 * (`commands/saved.ts`).
 *
 * **Hand-mirrored, like the rest of this file, and for the same reason:** `packages/gui` has no
 * `@bifurc/engine` or `@bifurc/protocol` dependency by design (it is the browser bundle), so the renderer
 * declares the shapes it consumes. The authority is the protocol package; this is the copy the UI compiles
 * against, and `tests/gui/boundary.test.ts` is what keeps a *behavioural* copy from appearing beside it.
 */
export interface RequestSendRequest {
  method: string;
  url: string;
  headers: Record<string, string>;
  /** **Text**, not base64 — resolution and the base64 boundary are the engine's (`plan/19` §5). */
  body: string;
}

export interface RequestSendScripts {
  pre?: string;
  post?: string;
  test?: string;
}

export type RequestSendParams =
  | { id: string }
  | { request: RequestSendRequest; scripts: RequestSendScripts };

/**
 * `request.send`'s result — the response **plus** what the pipeline did on the way through.
 *
 * The response fields are `ReplayResult`'s. The rest is here because the pipeline is no longer the
 * renderer's job: `request` is the request **as actually sent** (resolved, and rewritten by the
 * pre-script if there was one), which after engine-side resolution the renderer genuinely cannot compute
 * for itself.
 */
export interface RequestSendResult extends ReplayResult {
  /** The send's own duration. This is also what the test-script sees as `responseTime`. */
  durationMs: number;
  request: RequestSendRequest;
  tests: { name: string; passed: boolean; error?: string; durationMs: number }[];
  testLogs: string[];
  preScriptError?: string;
  postScriptError?: string;
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

// -- Import/Export types (mirrored from packages/engine/src/importexport/types.ts) --------

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

export type { BifurcApi } from "./window";
