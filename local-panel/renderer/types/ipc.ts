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

export type { LocalPanelApi } from "./window";
