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
