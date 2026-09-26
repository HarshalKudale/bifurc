import { useState, useCallback, useEffect, useRef } from "react";
import { AppConfig, ServiceInfo, SyncStatus } from "@/types";

export const EMPTY_CONFIG: AppConfig = {
  port: 80,
  // Literals rather than imports: this is the browser bundle, and `@bifurc/engine` is not its dependency. The
  // engine is the authority for these numbers — see `store/appSettings.ts`'s `DEFAULT_*` constants.
  forwardProxyPort: 8080,
  companionPort: 4747,
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
  graphqlSchemas: [],
  graphqlRequestFolders: [],
  grpcRequests: [],
  protoFiles: [],
  grpcRequestFolders: [],
  soapRequests: [],
  savedWsdls: [],
  soapRequestFolders: [],
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
  /**
   * Why the initial load failed, if it did.
   *
   * Added for a **console** defect rather than a visible one: `loadConfig` had `try/finally` and **no
   * `catch`**, so a rejected `config.get` propagated out of the callback and the effect's `.then()` had
   * nothing to catch it with. A browser saw `Uncaught (in promise) EngineError: Cannot request(): this
   * session ended`, and the UI sat on "Loading workspace…" forever — which is the same screen a slow
   * engine shows, so the two are indistinguishable to a user.
   *
   * Catching is not enough on its own, though: a caught error nobody renders is a silent failure. `App`
   * renders this where the loading message was.
   */
  const [loadError, setLoadError] = useState<string | null>(null);

  /**
   * Whether a load has ever succeeded — which makes `loadError` a **first-load** failure, not a refresh
   * one.
   *
   * The distinction is load-bearing once a host can reconnect. A refresh can fail for reasons that are not
   * the user's to fix, and a host being restarted is the obvious one: blanking the whole UI for that would
   * discard every open tab and draft for an outage the user may not even have noticed. Measured in the
   * reconnect check — the refresh triggered by a failed reconnect was enough to replace the shell with the
   * error screen, while the banner above it said "reconnecting".
   */
  const loadedOnce = useRef(false);

  const loadConfig = useCallback(async (loadingMsg = "Loading workspace…") => {
    setWsLoading(loadingMsg);
    setLoadError(null);
    try {
      const cfg = await window.api.getConfig();
      setConfig(cfg);
      const status = await window.api.serverStatus();
      setServerRunning(status.running);
      setServerError(status.error);
      loadedOnce.current = true;
    } catch (error) {
      // Recorded, not rethrown: the caller is a `useEffect` with no handler, so rethrowing turns a state
      // change into an unhandled rejection.
      //
      // Only a *first* failure is surfaced. After that the UI is already showing a workspace, and replacing
      // it with an error screen loses more than it explains — a reconnect's own banner is the right place
      // for "the host is away".
      if (!loadedOnce.current) {
        setLoadError(error instanceof Error ? error.message : String(error));
      }
    } finally {
      setWsLoading(null);
    }
  }, []);

  const refreshServices = useCallback(async () => {
    try {
      setServices(await window.api.discoverServices());
    } catch {
      // Secondary. A host that cannot answer `discoverServices` is already reported by `loadConfig`, and
      // letting this reject would be the same console error wearing a different name.
      setServices([]);
    }
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

  /**
   * Re-read the config after something changed.
   *
   * **It swallows its own failure, and that is the point.** Every caller is a *background* refresh — an
   * effect reacting to a reconnect, an event handler — so none of them can act on a rejection, and the
   * alternative is an unhandled one. Measured: `void refreshConfig()` after a reconnect produced
   * `Uncaught (in promise) EngineError: Cannot request(): this session ended` in the browser console —
   * exactly the class of noise this work exists to remove, and exactly the shape of the original bug
   * report. Note that `void promise` does **not** catch: it discards the promise, rejection and all.
   *
   * `loadConfig` deliberately does **not** swallow. It is the *first* load, and its failure has somewhere
   * to go — `loadError`, which `App` renders.
   */
  const refreshConfig = useCallback(async () => {
    try {
      const cfg = await window.api.getConfig();
      setConfig(cfg);
    } catch {
      // Silent by design — see the note above. The UI is already showing a workspace; a failed *refresh*
      // is not a reason to replace it with an error, and the reconnect banner already says the host is away.
    }
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
    loadError,
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
