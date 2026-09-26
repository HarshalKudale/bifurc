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
  /** The forward proxy's port — **engine-level**, one for every workspace (2026-09-25). */
  forwardProxyPort: number;
  /**
   * The companion (RPC over WebSocket) endpoint's port. Changing it needs a restart; the web GUI's port is
   * derived as this `+ 1` and is never stored.
   */
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
  graphqlSchemas: SavedGraphQLSchema[];
  graphqlRequestFolders: Folder[];
  grpcRequests: SavedGrpcRequest[];
  protoFiles: SavedProtoFile[];
  grpcRequestFolders: Folder[];
  soapRequests: SavedSoapRequest[];
  savedWsdls: SavedWsdl[];
  soapRequestFolders: Folder[];
  environments: Environment[];
  activeEnvironmentId: string | null;
}
