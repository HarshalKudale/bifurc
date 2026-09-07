import { useState, useCallback, useEffect } from "react";
import { AppConfig, ServiceInfo, SyncStatus } from "@/types";

export const EMPTY_CONFIG: AppConfig = {
  port: 80,
  companionPort: 9271,
  minimizeToTray: true,
  tlsEnabled: false,
  tlsCaCertPath: null,
  tlsCaKeyPath: null,
  workspaces: [],
  activeWorkspaceId: "default",
  mappings: [],
  proxyRules: [],
  ruleFolders: [],
  mocks: [],
  requests: [],
  mockFolders: [],
  requestFolders: [],
  wsConnections: [],
  wsFolders: [],
  webhooks: [],
  webhookFolders: [],
  graphqlRequests: [],
  graphqlMocks: [],
  graphqlSchemas: [],
  graphqlRequestFolders: [],
  graphqlMockFolders: [],
  grpcRequests: [],
  grpcMocks: [],
  protoFiles: [],
  grpcRequestFolders: [],
  grpcMockFolders: [],
  grpcMockServerPort: 9102,
  soapRequests: [],
  soapMocks: [],
  savedWsdls: [],
  soapRequestFolders: [],
  soapMockFolders: [],
  webhookPort: 9101,
  environments: [],
  activeEnvironmentId: null,
};

export function useAppConfigSync() {
  const [config, setConfig] = useState<AppConfig>(EMPTY_CONFIG);
  const [wsLoading, setWsLoading] = useState<string | null>("Loading workspace…");
  const [services, setServices] = useState<ServiceInfo[]>([]);
  const [serverRunning, setServerRunning] = useState(false);
  const [serverError, setServerError] = useState<string | null>(null);
  const [entitySyncStatus, setEntitySyncStatus] = useState<Record<string, "clean" | "modified" | "new" | "deleted">>({});
  const [syncStatus, setSyncStatus] = useState<SyncStatus>("idle");

  const loadConfig = useCallback(async (loadingMsg = "Loading workspace…") => {
    setWsLoading(loadingMsg);
    try {
      const cfg = await window.api.getConfig();
      setConfig(cfg);
      const status = await window.api.serverStatus();
      setServerRunning(status.running);
      setServerError(status.error);
    } finally {
      setWsLoading(null);
    }
  }, []);

  const refreshServices = useCallback(async () => {
    const svcs = await window.api.discoverServices();
    setServices(svcs);
  }, []);

  const refreshEntitySyncStatus = useCallback((wsId: string) => {
    window.api.getEntitySyncStatus(wsId).then((status) => {
      setEntitySyncStatus(status);
    }).catch(() => { });
  }, []);

  useEffect(() => {
    loadConfig().then(() => {
      window.api.getConfig().then((cfg) => refreshEntitySyncStatus(cfg.activeWorkspaceId)).catch(() => { });
    });
    refreshServices();
    
    const unsubError = window.api.onServerError((err) => {
      setServerError(err);
      setServerRunning(false);
    });
    
    const unsubSync = window.api.onSyncStatus((evt) => {
      setSyncStatus(evt.status as SyncStatus);
      if (evt.status === "idle") {
        window.api.getConfig().then((fresh) => {
          setConfig(fresh);
          refreshEntitySyncStatus(fresh.activeWorkspaceId);
        }).catch(() => { });
      }
    });
    
    const unsubEntityStatus = window.api.onEntitySyncStatus((data) => {
      setEntitySyncStatus(data.status as Record<string, "clean" | "modified" | "new" | "deleted">);
    });
    
    const unsubCompanion = window.api.onCompanionRefresh(() => {
      window.api.getConfig().then((fresh) => {
        setConfig(fresh);
        refreshEntitySyncStatus(fresh.activeWorkspaceId);
      }).catch(() => { });
    });
    
    return () => { unsubError(); unsubSync(); unsubEntityStatus(); unsubCompanion(); };
  }, [loadConfig, refreshServices, refreshEntitySyncStatus]);

  const handleConfigChange = useCallback(async (next: AppConfig) => {
    setConfig(next);
    await window.api.saveConfig(next);
    setServerRunning(true);
    setServerError(null);
    refreshEntitySyncStatus(next.activeWorkspaceId);
  }, [refreshEntitySyncStatus]);

  const refreshConfig = useCallback(async () => {
    const cfg = await window.api.getConfig();
    setConfig(cfg);
  }, []);

  const handleWsConfigChange = useCallback(async (next: AppConfig, wsId: string) => {
    const hasCrossWs = (
      (next.mappings ?? []).some((m) => m.workspaceId !== wsId) ||
      (next.mocks ?? []).some((m) => m.workspaceId !== wsId) ||
      (next.requests ?? []).some((r) => r.workspaceId !== wsId)
    );
    if (hasCrossWs) {
      await handleConfigChange(next);
      return;
    }
    const merged: AppConfig = {
      ...next,
      mappings: [...(config.mappings ?? []).filter((m) => m.workspaceId !== wsId), ...(next.mappings ?? [])],
      proxyRules: [...(config.proxyRules ?? []).filter((r) => r.workspaceId !== wsId), ...(next.proxyRules ?? [])],
      ruleFolders: [...(config.ruleFolders ?? []).filter((f) => f.workspaceId !== wsId), ...(next.ruleFolders ?? [])],
      mocks: [...(config.mocks ?? []).filter((m) => m.workspaceId !== wsId), ...(next.mocks ?? [])],
      requests: [...(config.requests ?? []).filter((r) => r.workspaceId !== wsId), ...(next.requests ?? [])],
      mockFolders: [...(config.mockFolders ?? []).filter((f) => f.workspaceId !== wsId), ...(next.mockFolders ?? [])],
      requestFolders: [...(config.requestFolders ?? []).filter((f) => f.workspaceId !== wsId), ...(next.requestFolders ?? [])],
      wsConnections: [...(config.wsConnections ?? []).filter((c) => c.workspaceId !== wsId), ...(next.wsConnections ?? [])],
      wsFolders: [...(config.wsFolders ?? []).filter((f) => f.workspaceId !== wsId), ...(next.wsFolders ?? [])],
      webhooks: [...(config.webhooks ?? []).filter((h) => h.workspaceId !== wsId), ...(next.webhooks ?? [])],
      webhookFolders: [...(config.webhookFolders ?? []).filter((f) => f.workspaceId !== wsId), ...(next.webhookFolders ?? [])],
      environments: [...(config.environments ?? []).filter((e) => e.workspaceId !== wsId), ...(next.environments ?? [])],
    };
    await handleConfigChange(merged);
  }, [config, handleConfigChange]);

  const clearWorkspaceContext = useCallback((loadingMsg = "Switching workspace…") => {
    setConfig(EMPTY_CONFIG);
    setWsLoading(loadingMsg);
  }, []);

  return {
    config, setConfig,
    wsLoading, setWsLoading,
    services, refreshServices,
    serverRunning, setServerRunning,
    serverError, setServerError,
    entitySyncStatus, refreshEntitySyncStatus,
    syncStatus,
    handleConfigChange,
    handleWsConfigChange,
    refreshConfig,
    clearWorkspaceContext,
  };
}
