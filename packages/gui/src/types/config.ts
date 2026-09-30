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

/** What a status refresh returns: both axes, read at the same instant, plus the engine's path maps. */
export interface EntityStatusPayload {
  files: Record<string, EntitySyncStatus>;
  git: WorkspaceGitState;
  /**
   * `entity id → relPath` and `folder id → directory prefix`, computed by the engine (`plan/19` WI 5,
   * item 3). The renderer used to derive these itself from `(kind, entity, folders)` — three helpers in
   * `lib/utils.ts` that were a hand-mirror of `workspaceFs.ts` — and they are engine work: a path is a
   * property of the storage *layout*, not of the screen.
   *
   * **They ride this read-only result rather than the config**, because `config.save` writes its payload
   * straight through: a computed field on an entity would be persisted to disk. See
   * `packages/engine/src/workspace/entityPaths.ts`, which also records the two limits (active-workspace
   * only, and refreshed on config changes rather than on the status event).
   *
   * Optional so a client newer than the engine still typechecks: an engine that predates the field sends
   * nothing, and `undefined` means "unknown" rather than "no paths".
   */
  paths?: { entities: Record<string, string>; folders: Record<string, string> };
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
