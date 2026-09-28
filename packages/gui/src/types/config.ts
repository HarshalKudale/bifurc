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

/**
 * One entity file's position relative to HEAD — the first axis of "is this entity synced?".
 *
 * Mirrors `SyncGetEntityStatusResult["files"]` in `@bifurc/protocol`. This copy exists because the
 * GUI bundle does not depend on the protocol package; `packages/gui/src/lib/entityGit.ts` is the one
 * place that interprets these values into an affordance, so a new member here is a compile error
 * there rather than a surface that silently offers nothing.
 */
export type EntitySyncStatus = "clean" | "modified" | "new" | "deleted";

/**
 * The workspace's position relative to its **remote** — the second axis.
 *
 * Mirrors `WorkspaceGitState` in `@bifurc/protocol`. Needed because `EntitySyncStatus` cannot tell
 * *committed and pushed* from *committed and not pushed*: both read `clean`, so an unpushed entity
 * used to be offered no button at all.
 */
export interface WorkspaceGitState {
  hasRemote: boolean;
  branch: string | null;
  ahead: boolean;
  behind: boolean;
}

/** What a status refresh returns: both axes, read at the same instant. */
export interface EntityStatusPayload {
  files: Record<string, EntitySyncStatus>;
  git: WorkspaceGitState;
}

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
