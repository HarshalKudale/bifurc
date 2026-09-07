// Shared entity interfaces — used by src/store/config.ts, workspaceFs.ts, and gitStore.ts

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
  capturedBody: string;
  responseStatus: number;
  responseStatusMocked?: boolean;
  responseHeaders: Record<string, string>;
  mockedResponseHeaders?: string[];
  responseBody: string;
  responseBodyMocked?: boolean;
  responseBodyEncoding?: "utf8" | "base64";  // default "utf8"; "base64" for binary bodies
  responseDelay?: number;     // ms to wait before sending response (0 = no delay)
  responseDelayMocked?: boolean;
  streamingMode?: "none" | "sse" | "chunked";  // default "none"
  streamingChunkDelay?: number;  // ms between chunks (default 100)
  streamingChunkSeparator?: string;  // delimiter to split body into chunks (default "\n\n")
  protocol?: "rest" | "graphql" | "grpc" | "soap";
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
  body: string;
  protocol?: "rest" | "graphql" | "grpc" | "soap";
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
  /** The user-defined suffix appended after /localpanel/webhooks/ */
  urlSuffix: string;
  createdAt: number;
  folderId?: string | null;
  workspaceId: string;
}

export interface WorkspaceSyncConfig {
  remote: string;
  branch: string;
  autoSync: boolean;
}

export interface WorkspaceSyncMeta {
  lastPushedAt: number | null;
  lastPulledAt: number | null;
  lastSyncedCommit: string | null;
}

export interface Workspace {
  id: string;
  name: string;
  createdAt: number;
  activeEnvironmentId: string | null;
  syncConfig?: WorkspaceSyncConfig | null;
  syncMeta?: WorkspaceSyncMeta | null;
}

// Audit types
export type AuditAction = "create" | "update" | "delete";
export type AuditEntity =
  | "mock"
  | "mapping"
  | "rule"
  | "environment"
  | "request"
  | "wsConnection"
  | "webhook"
  | "folder"
  | "workspace";

export interface AuditEntry {
  commitHash: string;
  ts: number;           // commit author timestamp in ms
  action: AuditAction;
  entity: AuditEntity;
  entityId: string;
  entityName: string;
  workspaceId: string;
  actor: string;        // email or "local"
  changedFields?: string[];  // field names that changed (update only)
}
