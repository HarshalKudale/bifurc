import * as path from "path";
import { loadSettings, saveSettings, AppSettings, WorkspaceMeta } from "@/store/appSettings";
import {
  readAllEntities, readEntityStubs, readEntity,
  readNamesIndex, bootstrapNamesIndex,
  readIndex, writeEntity, deleteEntityFile,
  writeFlatEntity, deleteFlatEntityFile, initWorkspaceDir,
  wsDir, dataRoot, autoSyncFsDirectories, readEnabledSet, bootstrapEnabledSet,
  getPendingDeletions,
} from "@/store/workspaceFs";
import type {
  MockRule, LocalMapping, ProxyRule, SavedRequest,
  SavedWsConnection, SavedWebhook, Environment, Folder, Workspace,
} from "@/store/types";

export type { LocalMapping, ProxyRule, MockRule, SavedRequest, SavedWsConnection, SavedWebhook, Folder, Environment, Workspace };

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
  graphqlRequests: any[];
  graphqlMocks: any[];
  graphqlSchemas: any[];
  graphqlRequestFolders: Folder[];
  graphqlMockFolders: Folder[];
  grpcRequests: any[];
  grpcMocks: any[];
  protoFiles: any[];
  grpcRequestFolders: Folder[];
  grpcMockFolders: Folder[];
  grpcMockServerPort: number;
  soapRequests?: any[];
  soapMocks?: any[];
  savedWsdls?: any[];
  soapRequestFolders?: Folder[];
  soapMockFolders?: Folder[];
  environments: Environment[];
  activeEnvironmentId: string | null;
}


export function generateId(): string {
  return Date.now().toString(36) + Math.random().toString(36).slice(2, 7);
}

// ── Adapter: assemble flat AppConfig from folder-based store ──────────────────

export function loadConfig(): AppConfig {
  const settings = loadSettings();
  const wsId = settings.activeWorkspaceId;

  // Ensure enabled.json exists for proxy server fast-path (bootstrap once if missing)
  const mockEnabledSet = readEnabledSet(wsId, "mocks") ?? bootstrapEnabledSet(wsId, "mocks");
  const mappingEnabledSet = readEnabledSet(wsId, "mappings") ?? bootstrapEnabledSet(wsId, "mappings");
  const ruleEnabledSet = readEnabledSet(wsId, "rules") ?? bootstrapEnabledSet(wsId, "rules");

  // Load all mocks/mappings (full data needed by proxy server and UI panels for editing)
  // Inject enabled status from enabled.json since it's not stored in entity files
  // Rules use stub-based loading like requests (scripts can be large)
  const mocks = readAllEntities<MockRule>(wsId, "mocks").map((m) => ({ ...m, enabled: mockEnabledSet.has(m.id) }));
  const mappings = readAllEntities<LocalMapping>(wsId, "mappings").map((m) => ({ ...m, enabled: mappingEnabledSet.has(m.id) }));

  // Environments are small and always needed for variable resolution
  let environments = readAllEntities<Environment>(wsId, "environments");
  // Auto-create the global environment for this workspace if it doesn't exist yet
  if (!environments.find((e) => e.id === "__global__")) {
    const globalEnv: Environment = { id: "__global__", name: "Global", variables: [], createdAt: Date.now(), workspaceId: wsId };
    writeFlatEntity(wsId, "environments", "__global__", globalEnv);
    environments = [globalEnv, ...environments];
  }

  // For requests/sockets/rules: load stubs (id + folderId) + names index for list display.
  // Full entity data is loaded on-demand when a tab is opened via entity:load IPC.
  const requestStubs = readEntityStubs(wsId, "requests");
  const wsConnectionStubs = readEntityStubs(wsId, "sockets");
  const webhookStubs = readEntityStubs(wsId, "webhooks");
  const ruleStubs = readEntityStubs(wsId, "rules");
  // Bootstrap names.json if it doesn't exist yet (first-time migration)
  const requestNames = (() => { const n = readNamesIndex(wsId, "requests"); return Object.keys(n).length || requestStubs.length === 0 ? n : bootstrapNamesIndex<SavedRequest>(wsId, "requests"); })();
  const wsNames = (() => { const n = readNamesIndex(wsId, "sockets"); return Object.keys(n).length || wsConnectionStubs.length === 0 ? n : bootstrapNamesIndex<SavedWsConnection>(wsId, "sockets"); })();
  const webhookNames = (() => { const n = readNamesIndex(wsId, "webhooks"); return Object.keys(n).length || webhookStubs.length === 0 ? n : bootstrapNamesIndex<SavedWebhook>(wsId, "webhooks"); })();
  const ruleNames = (() => { const n = readNamesIndex(wsId, "rules"); return Object.keys(n).length || ruleStubs.length === 0 ? n : bootstrapNamesIndex<ProxyRule>(wsId, "rules"); })();

  const mockIdx = autoSyncFsDirectories(wsId, "mocks", generateId);
  const reqIdx = autoSyncFsDirectories(wsId, "requests", generateId);
  const wsIdx = autoSyncFsDirectories(wsId, "sockets", generateId);
  const webhookIdx = autoSyncFsDirectories(wsId, "webhooks", generateId);
  const ruleIdx = autoSyncFsDirectories(wsId, "rules", generateId);

  const activeEnv = settings.workspaces.find((w) => w.id === wsId)?.activeEnvironmentId ?? null;

  return {
    port: settings.port,
    webhookPort: settings.webhookPort ?? 9101,
    companionPort: settings.companionPort ?? 9271,
    minimizeToTray: settings.minimizeToTray,
    tlsEnabled: settings.tlsEnabled ?? false,
    tlsCaCertPath: settings.tlsCaCertPath ?? null,
    tlsCaKeyPath: settings.tlsCaKeyPath ?? null,
    workspaces: settings.workspaces.map((w) => ({
      id: w.id,
      name: w.name,
      createdAt: 0,
      activeEnvironmentId: w.activeEnvironmentId,
      syncConfig: w.syncConfig ?? null,
      syncMeta: w.syncMeta ?? null,
    })),
    activeWorkspaceId: wsId,
    activeEnvironmentId: activeEnv,
    mappings: mappings.map((m) => ({ ...m, workspaceId: m.workspaceId ?? wsId, enabled: mappingEnabledSet.has(m.id) })),
    // Rule stubs: id + folderId + display name/pattern from names.json — scripts loaded on tab open
    proxyRules: [
      ...ruleStubs.map((s) => ({
        id: s.id, folderId: s.folderId, workspaceId: wsId,
        name: ruleNames[s.id]?.name ?? "",
        pattern: (ruleNames[s.id] as any)?.url ?? "",
        useRegex: true, targetType: "mapping" as const, targetMappingId: "",
        targetExternal: "", requestScript: "", responseScript: "",
        enabled: ruleEnabledSet.has(s.id), createdAt: 0,
      } as ProxyRule)),
      // Pending-deletion rules: minimal stubs so FolderTree can show them as deleted
      ...getPendingDeletions(wsId, "rules")
        .filter((p) => !ruleStubs.some((s) => s.id === p.id))
        .map((p) => ({
          id: p.id, workspaceId: wsId, folderId: p.folderId ?? undefined,
          name: p.name, pattern: p.url ?? "",
          useRegex: true, targetType: "mapping" as const, targetMappingId: "",
          targetExternal: "", requestScript: "", responseScript: "",
          enabled: false, createdAt: 0,
        } as ProxyRule)),
    ],
    ruleFolders: ruleIdx.folders as Folder[],
    mocks: [
      ...mocks.map((m) => ({ ...m, workspaceId: m.workspaceId ?? wsId, enabled: mockEnabledSet.has(m.id) })),
      // Pending-deletion mocks: minimal stubs so FolderTree can show them as deleted
      ...getPendingDeletions(wsId, "mocks")
        .filter((p) => !mocks.some((m) => m.id === p.id))
        .map((p) => ({
          id: p.id, workspaceId: wsId, folderId: p.folderId ?? undefined,
          name: p.name, method: p.method ?? "GET",
          urlPattern: p.url ?? "", useRegex: false,
          enabled: false, createdAt: 0,
          capturedHeaders: {}, capturedBody: "",
          responseStatus: 200, responseHeaders: {}, responseBody: "",
        } as MockRule)),
    ],
    // Stubs carry id + folderId + display name/method/url from names.json — no full file reads
    requests: requestStubs.map((s) => ({
      id: s.id, folderId: s.folderId, workspaceId: wsId,
      name: requestNames[s.id]?.name ?? "",
      method: requestNames[s.id]?.method ?? "GET",
      url: requestNames[s.id]?.url ?? "",
      headers: {}, body: "", createdAt: 0,
    } as SavedRequest)),
    wsConnections: wsConnectionStubs.map((s) => ({
      id: s.id, folderId: s.folderId, workspaceId: wsId,
      name: wsNames[s.id]?.name ?? "",
      url: wsNames[s.id]?.url ?? "",
      headers: {}, createdAt: 0,
    } as SavedWsConnection)),
    mockFolders: mockIdx.folders as Folder[],
    requestFolders: reqIdx.folders as Folder[],
    wsFolders: wsIdx.folders as Folder[],
    webhooks: webhookStubs.map((s) => ({
      id: s.id, folderId: s.folderId, workspaceId: wsId,
      name: webhookNames[s.id]?.name ?? "",
      urlSuffix: (webhookNames[s.id] as any)?.urlSuffix ?? "",
      createdAt: 0,
    } as SavedWebhook)),
    webhookFolders: webhookIdx.folders as Folder[],
    environments: environments.map((e) => ({ ...e, workspaceId: e.workspaceId ?? wsId })),
    // GraphQL entities
    graphqlRequests: (() => {
      const stubs = readEntityStubs(wsId, "graphqlRequests");
      const names = readNamesIndex(wsId, "graphqlRequests");
      return stubs.map((s) => ({
        id: s.id, folderId: s.folderId, workspaceId: wsId,
        name: names[s.id]?.name ?? "",
        endpointUrl: (names[s.id] as any)?.endpointUrl ?? "",
        headers: {}, query: "", variables: "", operationName: "", createdAt: 0,
      }));
    })(),
    graphqlMocks: (() => {
      const enabledSet = readEnabledSet(wsId, "graphqlMocks") ?? bootstrapEnabledSet(wsId, "graphqlMocks");
      return readAllEntities(wsId, "graphqlMocks").map((m: any) => ({ ...m, enabled: enabledSet.has(m.id) }));
    })(),
    graphqlSchemas: [],
    graphqlRequestFolders: (() => { try { return autoSyncFsDirectories(wsId, "graphqlRequests", generateId).folders as Folder[]; } catch { return []; } })(),
    graphqlMockFolders: (() => { try { return autoSyncFsDirectories(wsId, "graphqlMocks", generateId).folders as Folder[]; } catch { return []; } })(),
    // gRPC entities
    grpcRequests: (() => {
      const stubs = readEntityStubs(wsId, "grpcRequests");
      const names = readNamesIndex(wsId, "grpcRequests");
      return stubs.map((s) => ({
        id: s.id, folderId: s.folderId, workspaceId: wsId,
        name: names[s.id]?.name ?? "",
        serverAddress: (names[s.id] as any)?.serverAddress ?? "",
        serviceName: (names[s.id] as any)?.serviceName ?? "",
        methodName: (names[s.id] as any)?.methodName ?? "",
        requestBody: "",
        metadata: {},
        protoFileId: null,
        useReflection: false,
        streamingType: "unary",
        createdAt: 0,
      }));
    })(),
    grpcMocks: (() => {
      const enabledSet = readEnabledSet(wsId, "grpcMocks") ?? bootstrapEnabledSet(wsId, "grpcMocks");
      return readAllEntities(wsId, "grpcMocks").map((m: any) => ({ ...m, enabled: enabledSet.has(m.id) }));
    })(),
    protoFiles: (() => {
      try { return readAllEntities(wsId, "protoFiles"); } catch { return []; }
    })(),
    grpcRequestFolders: (() => { try { return autoSyncFsDirectories(wsId, "grpcRequests", generateId).folders as Folder[]; } catch { return []; } })(),
    grpcMockFolders: (() => { try { return autoSyncFsDirectories(wsId, "grpcMocks", generateId).folders as Folder[]; } catch { return []; } })(),
    grpcMockServerPort: 9102,
    // SOAP entities
    soapRequests: (() => {
      const stubs = readEntityStubs(wsId, "soapRequests");
      const names = readNamesIndex(wsId, "soapRequests");
      return stubs.map((s) => ({
        id: s.id, folderId: s.folderId, workspaceId: wsId,
        name: names[s.id]?.name ?? "",
        endpointUrl: (names[s.id] as any)?.endpointUrl ?? "",
        soapAction: (names[s.id] as any)?.soapAction ?? "",
        headers: {}, body: "", createdAt: 0,
      }));
    })(),
    soapMocks: (() => {
      const enabledSet = readEnabledSet(wsId, "soapMocks") ?? bootstrapEnabledSet(wsId, "soapMocks");
      return readAllEntities(wsId, "soapMocks").map((m: any) => ({ ...m, enabled: enabledSet.has(m.id) }));
    })(),
    savedWsdls: [],
    soapRequestFolders: (() => { try { return autoSyncFsDirectories(wsId, "soapRequests", generateId).folders as Folder[]; } catch { return []; } })(),
    soapMockFolders: (() => { try { return autoSyncFsDirectories(wsId, "soapMocks", generateId).folders as Folder[]; } catch { return []; } })(),
  };
}

/** Load a single entity by kind and ID. Used by panels when opening a tab. */
export function loadEntity<T>(wsId: string, kind: string, id: string): T | null {
  return readEntity<T>(wsId, kind, id);
}

// ── Adapter: route writes back to individual entity files ─────────────────────

export function saveConfig(cfg: AppConfig): void {
  // Update global settings
  const settings = loadSettings();
  settings.port = cfg.port;
  settings.webhookPort = cfg.webhookPort ?? 9101;
  settings.companionPort = cfg.companionPort ?? 9271;
  settings.minimizeToTray = cfg.minimizeToTray;
  settings.tlsEnabled = cfg.tlsEnabled ?? false;
  settings.tlsCaCertPath = cfg.tlsCaCertPath ?? null;
  settings.tlsCaKeyPath = cfg.tlsCaKeyPath ?? null;
  settings.activeWorkspaceId = cfg.activeWorkspaceId;
  // Sync workspace metadata
  for (const ws of (cfg.workspaces ?? [])) {
    const existing = settings.workspaces.find((w) => w.id === ws.id);
    if (existing) {
      existing.name = ws.name;
      existing.activeEnvironmentId = ws.activeEnvironmentId;
    } else {
      settings.workspaces.push({
        id: ws.id, name: ws.name, activeEnvironmentId: ws.activeEnvironmentId,
        syncConfig: (ws as any).syncConfig ?? undefined,
        syncMeta: (ws as any).syncMeta ?? undefined,
      });
    }
  }
  // Remove workspaces that were deleted
  const cfgIds = new Set((cfg.workspaces ?? []).map((w) => w.id));
  settings.workspaces = settings.workspaces.filter((w) => cfgIds.has(w.id));
  saveSettings(settings);

  // Write entity files for the active workspace only.
  // Guard: only write entities whose workspaceId matches — prevents cross-workspace contamination
  // when saveConfig is called with a mutated activeWorkspaceId (e.g. during workspace:setActive).
  // NOTE: requests and wsConnections are managed directly by their IPC handlers (stubs in config).
  const wsId = cfg.activeWorkspaceId;
  const mockFolderMap = new Map((cfg.mockFolders ?? []).map((f) => [f.id, f.name]));
  const ruleFolderMap = new Map((cfg.ruleFolders ?? []).map((f) => [f.id, f.name]));
  const pendingDeletedMockIds = new Set(getPendingDeletions(wsId, "mocks").map((p) => p.id));
  const pendingDeletedRuleIds = new Set(getPendingDeletions(wsId, "rules").map((p) => p.id));
  for (const m of (cfg.mocks ?? []).filter((m) => m.workspaceId === wsId && !pendingDeletedMockIds.has(m.id)))
    writeEntity(wsId, "mocks", m.id, m, m.folderId ? (mockFolderMap.get(m.folderId) ?? null) : null);
  for (const m of (cfg.mappings ?? []).filter((m) => m.workspaceId === wsId))
    writeFlatEntity(wsId, "mappings", m.id, m);
  // Rules are now folder-capable; stubs from config don't carry full data so skip config-level writes
  // (individual writes happen in rule:add/update IPC handlers which have the full entity)
  void ruleFolderMap; void pendingDeletedRuleIds;
  for (const e of (cfg.environments ?? []).filter((e) => e.workspaceId === wsId))
    writeFlatEntity(wsId, "environments", e.id, e);
}
