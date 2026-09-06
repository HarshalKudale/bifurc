import React, { useEffect, useState, useCallback, useMemo } from "react";
import { AppConfig, MockRule, SavedRequest, ServiceInfo, Folder, SyncStatus } from "@/types";
import { entityRelPath, flatEntityRelPath } from "@/lib/utils";
import { mergeEnvVars } from "@/lib/resolveVars";

import { CaptureStats } from "@/panels/CapturePanel";

import HistorySidebar from "@/components/sidebar/HistorySidebar";
import AppSidebar from "@/components/sidebar/AppSidebar";
import TitleBar from "@/components/layout/TitleBar";
import GlobalFooter from "@/components/layout/GlobalFooter";
import { useColorMode } from "@/lib/useTheme";
import { Panel, enabledPanels, PANEL_HELP } from "@/lib/panelRegistry";
import { renderPanel, PanelRenderContext } from "@/lib/panelFactory";
import { useSidebarVisibility } from "@/lib/useSidebarVisibility";
import { usePersistedState } from "@/lib/usePersistedState";
import SearchModal from "@/components/search/SearchModal";
import { readStorage, writeStorage } from "@/lib/storage";

const EMPTY_CONFIG: AppConfig = {
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

export default function App() {
  const [colorMode, setColorMode] = useColorMode();
  const [panel, setPanel] = usePersistedState<Panel>("app:active-panel", "services");
  const [config, setConfig] = useState<AppConfig>(EMPTY_CONFIG);
  const { visibility, setPanelVisible, isPanelVisible } = useSidebarVisibility();
  const [wsLoading, setWsLoading] = useState<string | null>("Loading workspace…");
  const [services, setServices] = useState<ServiceInfo[]>([]);
  const [serverRunning, setServerRunning] = useState(false);
  const [serverError, setServerError] = useState<string | null>(null);
  const [mappingPrefill, setMappingPrefill] = useState<string | undefined>();
  const [pendingOpenRequest, setPendingOpenRequest] = useState<Omit<SavedRequest, "id" | "createdAt" | "workspaceId"> | null>(null);
  const [pendingMockInitial, setPendingMockInitial] = useState<Partial<MockRule> | null>(null);

  // Per-entity sync status (clean/modified/new/deleted)
  const [entitySyncStatus, setEntitySyncStatus] = useState<Record<string, "clean" | "modified" | "new" | "deleted">>({});

  // Global sync operation status (idle/pulling/pushing/cloning/error)
  const [syncStatus, setSyncStatus] = useState<SyncStatus>("idle");

  // Capture panel stats - lifted so the global footer can display them
  const [captureStats, setCaptureStats] = useState<CaptureStats | null>(null);

  // History sidebar state
  const [historyOpen, setHistoryOpen] = useState(false);
  const [openedEntityPath, setOpenedEntityPath] = useState<string>("");
  const [historyReloadKey, setHistoryReloadKey] = useState(0);

  const openHistory = useCallback((filePath: string) => {
    // Toggle: if already open for same path, close it
    if (historyOpen && openedEntityPath === filePath) {
      setHistoryOpen(false);
      return;
    }
    setOpenedEntityPath(filePath);
    if (!historyOpen) {
      setHistoryOpen(true);
    }
  }, [historyOpen, openedEntityPath]);

  const closeHistory = useCallback(() => {
    setHistoryOpen(false);
  }, []);

  // After any save, bump the reload key so the history sidebar refreshes
  const bumpHistoryReload = useCallback(() => {
    setHistoryReloadKey((k) => k + 1);
  }, []);

  // When the opened entity path changes while history is open, just update the path (sidebar reloads itself)
  const handleEntityPathChange = useCallback((filePath: string) => {
    setOpenedEntityPath(filePath);
  }, []);

  // Search modal state
  const [searchModalOpen, setSearchModalOpen] = useState(false);
  const [searchModalMode, setSearchModalMode] = useState<"current" | "global">("current");

  // Global search shortcut: Ctrl+K / Cmd+K (current mode), Ctrl+Shift+K / Cmd+Shift+K (global mode)
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      const isCtrlOrCmd = e.ctrlKey || e.metaKey;
      if (isCtrlOrCmd && e.key.toLowerCase() === "k") {
        e.preventDefault();
        if (e.shiftKey) {
          setSearchModalMode("global");
          setSearchModalOpen(true);
        } else {
          setSearchModalMode("current");
          setSearchModalOpen(true);
        }
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);

  const getStorageKeyForPanel = useCallback((p: Panel): string | null => {
    switch (p) {
      case "requests":
      case "req-rest":
      case "req-graphql":
      case "req-soap":
      case "req-grpc":
        return "requests";
      case "mocks":
      case "mock-rest":
      case "mock-graphql":
      case "mock-soap":
      case "mock-grpc":
        return "mocks";
      case "rules":
        return "rules";
      case "sockets":
        return "ws";
      case "webhooks":
        return "webhooks";
      default:
        return null;
    }
  }, []);

  const handleSearchSelectTab = useCallback(
    (targetPanel: Panel, tabId: string) => {
      const storageKey = getStorageKeyForPanel(targetPanel);
      if (storageKey) {
        writeStorage(`${storageKey}:activeTab`, tabId);
      }
      setPanel(targetPanel);
      window.dispatchEvent(
        new CustomEvent("localpanel:open-tab", {
          detail: { panel: targetPanel, tabId },
        })
      );
    },
    [getStorageKeyForPanel, setPanel]
  );

  const handleSearchOpenEntity = useCallback(
    (targetPanel: Panel, entityId: string, _entityType: string, _original?: any) => {
      const storageKey = getStorageKeyForPanel(targetPanel);
      if (storageKey) {
        const openKey = `${storageKey}:openTabs`;
        const activeKey = `${storageKey}:activeTab`;
        const currentOpen = readStorage<string[]>(openKey, []);
        if (!currentOpen.includes(entityId)) {
          writeStorage(openKey, [...currentOpen, entityId]);
        }
        writeStorage(activeKey, entityId);
      }
      setPanel(targetPanel);
      window.dispatchEvent(
        new CustomEvent("localpanel:open-tab", {
          detail: { panel: targetPanel, tabId: entityId },
        })
      );
    },
    [getStorageKeyForPanel, setPanel]
  );

  const handleSearchNavigatePanel = useCallback(
    (targetPanel: Panel, target?: any) => {
      if (targetPanel === "mappings" && typeof target === "string") {
        setMappingPrefill(target);
      }
      setPanel(targetPanel);
    },
    [setPanel]
  );

  // Derive workspace-scoped view from full config
  const wsId = config.activeWorkspaceId;
  const wsConfig = useMemo<AppConfig>(() => ({
    ...config,
    mappings: (config.mappings ?? []).filter((m) => m.workspaceId === wsId),
    proxyRules: (config.proxyRules ?? []).filter((r) => r.workspaceId === wsId),
    ruleFolders: (config.ruleFolders ?? []).filter((f) => f.workspaceId === wsId),
    mocks: (config.mocks ?? []).filter((m) => m.workspaceId === wsId),
    requests: (config.requests ?? []).filter((r) => r.workspaceId === wsId),
    mockFolders: (config.mockFolders ?? []).filter((f) => f.workspaceId === wsId),
    requestFolders: (config.requestFolders ?? []).filter((f) => f.workspaceId === wsId),
    wsConnections: (config.wsConnections ?? []).filter((c) => c.workspaceId === wsId),
    wsFolders: (config.wsFolders ?? []).filter((f) => f.workspaceId === wsId),
    webhooks: (config.webhooks ?? []).filter((h) => h.workspaceId === wsId),
    webhookFolders: (config.webhookFolders ?? []).filter((f) => f.workspaceId === wsId),
    graphqlRequests: (config.graphqlRequests ?? []).filter((r) => r.workspaceId === wsId),
    graphqlMocks: (config.graphqlMocks ?? []).filter((m) => m.workspaceId === wsId),
    graphqlSchemas: (config.graphqlSchemas ?? []),
    graphqlRequestFolders: (config.graphqlRequestFolders ?? []).filter((f) => f.workspaceId === wsId),
    graphqlMockFolders: (config.graphqlMockFolders ?? []).filter((f) => f.workspaceId === wsId),
    grpcRequests: (config.grpcRequests ?? []).filter((r) => r.workspaceId === wsId),
    grpcMocks: (config.grpcMocks ?? []).filter((m) => m.workspaceId === wsId),
    protoFiles: (config.protoFiles ?? []),
    grpcRequestFolders: (config.grpcRequestFolders ?? []).filter((f) => f.workspaceId === wsId),
    grpcMockFolders: (config.grpcMockFolders ?? []).filter((f) => f.workspaceId === wsId),
    soapRequests: (config.soapRequests ?? []).filter((r) => r.workspaceId === wsId),
    soapMocks: (config.soapMocks ?? []).filter((m) => m.workspaceId === wsId),
    savedWsdls: (config.savedWsdls ?? []),
    soapRequestFolders: (config.soapRequestFolders ?? []).filter((f) => f.workspaceId === wsId),
    soapMockFolders: (config.soapMockFolders ?? []).filter((f) => f.workspaceId === wsId),
    environments: (config.environments ?? []).filter((e) => e.workspaceId === wsId),
  }), [config, wsId]);

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
    if (panel === "applications") {
      setPanel("services");
    }
  }, [panel, setPanel]);

  useEffect(() => {
    loadConfig().then(() => {
      // Fetch sync status after initial load
      window.api.getConfig().then((cfg) => refreshEntitySyncStatus(cfg.activeWorkspaceId)).catch(() => { });
    });
    refreshServices();
    const unsubError = window.api.onServerError((err) => {
      setServerError(err);
      setServerRunning(false);
    });
    // Track overall sync operation state for the global footer
    // and refresh config once a pull/push completes
    const unsubSync = window.api.onSyncStatus((evt) => {
      setSyncStatus(evt.status as SyncStatus);
      if (evt.status === "idle") {
        window.api.getConfig().then((fresh) => {
          setConfig(fresh);
          refreshEntitySyncStatus(fresh.activeWorkspaceId);
        }).catch(() => { });
      }
    });
    // Subscribe to live entity status updates (after publish or pull)
    const unsubEntityStatus = window.api.onEntitySyncStatus((data) => {
      setEntitySyncStatus(data.status as Record<string, "clean" | "modified" | "new" | "deleted">);
    });
    // Refresh config when companion extension adds entities
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
    // Refresh entity sync status after any config change (debounce handled by git status call)
    refreshEntitySyncStatus(next.activeWorkspaceId);
  }, [refreshEntitySyncStatus]);

  // Panels pass back wsConfig mutations - we need to merge them back into full config
  const handleWsConfigChange = useCallback(async (next: AppConfig) => {
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
  }, [config, wsId, handleConfigChange]);

  const clearWorkspaceContext = useCallback((loadingMsg = "Switching workspace…") => {
    setPendingOpenRequest(null);
    setPendingMockInitial(null);
    setHistoryOpen(false);
    setSidebarOpen(true);
    setMappingPrefill(undefined);
    setConfig(EMPTY_CONFIG);
    setWsLoading(loadingMsg);
  }, []);

  const handleOpenInRequests = useCallback((req: Omit<SavedRequest, "id" | "createdAt" | "workspaceId">) => {
    setPendingOpenRequest(req);
    setPanel("requests");
  }, []);

  const handleOpenMockEditor = useCallback((initial: Partial<MockRule>) => {
    setPendingMockInitial(initial);
    setPanel("mocks");
  }, []);

  // -- Publish helpers --------------------------------------------------------

  const refreshConfig = useCallback(async () => {
    const cfg = await window.api.getConfig();
    setConfig(cfg);
  }, []);

  const getItemFromConfig = (cfg: any, kind: string, id: string) => {
    if (kind === "rules") return (cfg.proxyRules ?? []).find((r: any) => r.id === id);
    if (kind === "requests") return (cfg.requests ?? []).find((r: any) => r.id === id);
    if (kind === "mocks") return (cfg.mocks ?? []).find((m: any) => m.id === id);
    if (kind === "webhooks") return (cfg.webhooks ?? []).find((h: any) => h.id === id);
    if (kind === "sockets") return (cfg.wsConnections ?? []).find((c: any) => c.id === id);
    if (kind === "graphqlRequests") return (cfg.graphqlRequests ?? []).find((r: any) => r.id === id);
    if (kind === "graphqlMocks") return (cfg.graphqlMocks ?? []).find((m: any) => m.id === id);
    if (kind === "soapRequests") return (cfg.soapRequests ?? []).find((r: any) => r.id === id);
    if (kind === "soapMocks") return (cfg.soapMocks ?? []).find((m: any) => m.id === id);
    if (kind === "grpcRequests") return (cfg.grpcRequests ?? []).find((r: any) => r.id === id);
    if (kind === "grpcMocks") return (cfg.grpcMocks ?? []).find((m: any) => m.id === id);
    return null;
  };

  const makePublishItem = useCallback((kind: string, folders: Folder[]) =>
    async (id: string) => {
      let item = getItemFromConfig(wsConfig, kind, id);
      if (!item) {
        const fresh = await window.api.getConfig();
        const ws = (fresh.workspaces ?? []).find((w: any) => w.id === wsId) ?? fresh;
        item = getItemFromConfig(ws, kind, id);
      }
      if (!item) return;
      const relPath = entityRelPath(kind, item as any, folders);
      await window.api.publishEntity(wsId, [relPath]);
      await refreshConfig();
      refreshEntitySyncStatus(wsId);
    },
    [wsConfig, wsId, refreshEntitySyncStatus, refreshConfig]);

  const makePublishFolder = useCallback((kind: "requests" | "mocks" | "sockets" | "webhooks" | "rules", folders: Folder[]) =>
    async (folderId: string | null) => {
      await window.api.publishFolder(wsId, kind, folderId ? folders.find((f) => f.id === folderId)?.name ?? null : null);
      await refreshConfig();
      refreshEntitySyncStatus(wsId);
    },
    [wsId, refreshEntitySyncStatus, refreshConfig]);

  const makeFlatPublish = useCallback((kind: "mappings") =>
    async (id: string) => {
      const relPath = flatEntityRelPath(kind, id);
      await window.api.publishEntity(wsId, [relPath]);
      refreshEntitySyncStatus(wsId);
    },
    [wsId, refreshEntitySyncStatus]);

  const makeFlatRevert = useCallback((kind: "mappings") =>
    async (id: string) => {
      const relPath = flatEntityRelPath(kind, id);
      await window.api.gitDiscard(wsId, relPath);
      await refreshConfig();
      refreshEntitySyncStatus(wsId);
    },
    [wsId, refreshEntitySyncStatus, refreshConfig]);

  const handlePublishHealthBar = useCallback(async () => {
    await window.api.publishEntity(wsId, ["healthbar/services.json"]);
    refreshEntitySyncStatus(wsId);
  }, [wsId, refreshEntitySyncStatus]);

  // -- Global footer publish --------------------------------------------------

  // Map from panel name to the git folder kind used for a bulk publish
  const PANEL_PUBLISH_KIND: Partial<Record<Panel, string>> = {
    requests: "requests",
    mocks: "mocks",
    "mock-rest": "mocks",
    "req-rest": "requests",
    sockets: "sockets",
    webhooks: "webhooks",
    mappings: "mappings",
    rules: "rules",
    environments: "environments",
    healthbar: "healthbar",
  };

  const handlePublishPanel = useCallback(async () => {
    const kind = PANEL_PUBLISH_KIND[panel];
    if (!kind) return;
    if (kind === "healthbar") {
      await window.api.publishEntity(wsId, ["healthbar/services.json"]);
    } else {
      await window.api.publishFolder(wsId, kind, null);
    }
    await refreshConfig();
    refreshEntitySyncStatus(wsId);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [panel, wsId, refreshConfig, refreshEntitySyncStatus]);

  const makeRestoreItem = useCallback((kind: string, folders: Folder[]) =>
    async (id: string) => {
      let item = getItemFromConfig(wsConfig, kind, id);
      if (!item) {
        const fresh = await window.api.getConfig();
        const ws = (fresh.workspaces ?? []).find((w: any) => w.id === wsId) ?? fresh;
        item = getItemFromConfig(ws, kind, id);
      }
      if (!item) return;
      const relPath = entityRelPath(kind, item as any, folders);
      await window.api.gitDiscard(wsId, relPath);
      await refreshConfig();
      refreshEntitySyncStatus(wsId);
    },
    [wsConfig, wsId, refreshEntitySyncStatus, refreshConfig]);

  const globalEnv = (wsConfig.environments ?? []).find((e) => e.id === "__global__") ?? null;
  const selectedActiveEnv = (wsConfig.environments ?? []).find((e) => e.id === wsConfig.activeEnvironmentId) ?? null;
  const activeEnv = mergeEnvVars(globalEnv, selectedActiveEnv);

  const cnt = (n: number) => n > 0 ? n : undefined;
  const totalRequests = (wsConfig.requests ?? []).length + (wsConfig.graphqlRequests ?? []).length + (wsConfig.grpcRequests ?? []).length + (wsConfig.soapRequests ?? []).length;
  const totalMocks = (wsConfig.mocks ?? []).length + (wsConfig.graphqlMocks ?? []).length + (wsConfig.grpcMocks ?? []).length + (wsConfig.soapMocks ?? []).length;
  const navBadges: Partial<Record<Panel, number | undefined>> = {
    mappings: cnt((wsConfig.mappings ?? []).length),
    rules: cnt((wsConfig.proxyRules ?? []).length),
    requests: cnt(totalRequests),
    mocks: cnt(totalMocks),
    "mock-rest": cnt(totalMocks),
    "req-rest": cnt(totalRequests),
    sockets: cnt((wsConfig.wsConnections ?? []).length),
    webhooks: cnt((wsConfig.webhooks ?? []).length),
    environments: cnt((wsConfig.environments ?? []).filter((e) => e.id !== "__global__").length),
  };

  // Filter sidebar entries based on user visibility preferences
  const visiblePanels = useMemo(
    () => enabledPanels.filter((e) => isPanelVisible(e.id)),
    [visibility]
  );

  // Active workspace object (used for syncConfig / branch in GlobalFooter)
  const currentWorkspace = (config.workspaces ?? []).find((w) => w.id === wsId) ?? null;

  // Right-side stats for the global footer - panel-specific counts / info
  const footerRightContent = useMemo(() => {
    const pl = (n: number, s: string) => `${n} ${s}${n !== 1 ? "s" : ""}`;
    switch (panel) {
      case "capture":
        if (!captureStats) return null;
        return (
          <>
            <span>{captureStats.total} captured</span>
            {captureStats.shown < captureStats.total && (
              <span>· {captureStats.shown} shown</span>
            )}
            {captureStats.paused && <span className="text-yellow">· paused</span>}
            <span className="opacity-50">newest first · last 200 kept</span>
          </>
        );
      case "mocks":
      case "mock-rest": return <span>{pl(totalMocks, "mock")}</span>;
      case "requests":
      case "req-rest": return <span>{pl(totalRequests, "request")}</span>;
      case "sockets": return <span>{pl(wsConfig.wsConnections?.length ?? 0, "socket")}</span>;
      case "webhooks": return <span>{pl(wsConfig.webhooks?.length ?? 0, "webhook")}</span>;
      case "mappings": return <span>{pl(wsConfig.mappings?.length ?? 0, "mapping")}</span>;
      case "rules": return <span>{pl(wsConfig.proxyRules?.length ?? 0, "rule")}</span>;
      case "environments": return <span>{pl(wsConfig.environments?.length ?? 0, "environment")}</span>;
      case "services": return <span>{pl(services.length, "service")}</span>;
      default:
        return (
          <span className="flex items-center gap-1.5">
            <span>{pl(totalRequests, "request")}</span>
            <span className="opacity-40">·</span>
            <span>{pl(totalMocks, "mock")}</span>
            <span className="opacity-40">·</span>
            <span>{pl(wsConfig.mappings?.length ?? 0, "mapping")}</span>
          </span>
        );
    }
  }, [panel, captureStats, totalMocks, totalRequests, wsConfig, services]);

  // -- Panel render context - single bag for the panel factory ----------------
  const panelRenderCtx: PanelRenderContext = useMemo(() => ({
    wsConfig,
    config,
    wsId,
    services,
    serverRunning,
    serverError,
    colorMode,
    setColorMode,
    activeEnv,
    openHistory,
    handleEntityPathChange,
    historyOpen,
    bumpHistoryReload,
    entitySyncStatus,
    refreshEntitySyncStatus,
    handleConfigChange,
    handleWsConfigChange,
    setConfig,
    refreshServices,
    mappingPrefill,
    onPrefillConsumed: () => setMappingPrefill(undefined),
    setPanel,
    setMappingPrefill,
    pendingOpenRequest,
    onPendingRequestConsumed: () => setPendingOpenRequest(null),
    pendingMockInitial,
    onPendingMockConsumed: () => setPendingMockInitial(null),
    handleOpenMockEditor,
    handleOpenInRequests,
    onStatsChange: setCaptureStats,
    makePublishItem,
    makePublishFolder,
    makeFlatPublish,
    makeFlatRevert,
    makeRestoreItem,
    handlePublishHealthBar,
    onWorkspaceRename: async (id: string, name: string) => {
      await window.api.renameWorkspace(id, name);
      const fresh = await window.api.getConfig();
      setConfig(fresh);
    },
    onWorkspaceDelete: async (id: string) => {
      localStorage.removeItem(`capture:entries:${id}`);
      await window.api.deleteWorkspace(id);
      const fresh = await window.api.getConfig();
      setConfig(fresh);
      setPanel("services");
    },
    onServerRestart: async () => {
      await window.api.restartServer();
      const status = await window.api.serverStatus();
      setServerRunning(status.running);
      setServerError(status.error);
    },
    sidebarVisibility: visibility,
    setSidebarPanelVisible: setPanelVisible,
  }), [
    wsConfig, config, wsId, services, serverRunning, serverError, colorMode, setColorMode,
    activeEnv, openHistory, handleEntityPathChange, historyOpen, bumpHistoryReload,
    entitySyncStatus, refreshEntitySyncStatus, handleConfigChange, handleWsConfigChange,
    refreshServices, mappingPrefill, pendingOpenRequest, pendingMockInitial,
    handleOpenMockEditor, handleOpenInRequests, makePublishItem, makePublishFolder,
    makeFlatPublish, makeFlatRevert, makeRestoreItem, handlePublishHealthBar,
    visibility, setPanelVisible,
  ]);

  if (wsLoading) {
    return (
      <div className="flex flex-col h-screen bg-background text-foreground overflow-hidden items-center justify-center gap-3">
        <div className="text-muted-foreground text-sm animate-pulse">{wsLoading}</div>
      </div>
    );
  }

  return (
    <div className="flex flex-col h-screen bg-background text-foreground overflow-hidden">
      <TitleBar
        config={config}
        serverRunning={serverRunning}
        serverError={serverError}
        helpText={PANEL_HELP[panel]}
        onServerStart={async () => {
          await window.api.startServer();
          const status = await window.api.serverStatus();
          setServerRunning(status.running);
          setServerError(status.error);
        }}
        onServerStop={async () => {
          await window.api.stopServer();
          setServerRunning(false);
          setServerError(null);
        }}
        onEnvChange={async (id) => {
          await window.api.setActiveEnvironment(id);
          const fresh = await window.api.getConfig();
          setConfig(fresh);
        }}
        onManageEnvs={() => setPanel("environments")}
        workspaces={config.workspaces ?? []}
        activeWorkspaceId={config.activeWorkspaceId}
        onWorkspaceChange={async (id) => {
          clearWorkspaceContext("Switching workspace…");
          try {
            const result = await window.api.setActiveWorkspace(id);
            if (result.ok) {
              setConfig(result.config);
              refreshEntitySyncStatus(id);
            }
          } finally {
            setWsLoading(null);
          }
        }}
        onWorkspaceCreate={async () => {
          clearWorkspaceContext("Creating workspace…");
          try {
            const ws = await window.api.addWorkspace("");
            const result = await window.api.setActiveWorkspace(ws.id);
            if (result.ok) { setConfig(result.config); setPanel("workspace"); }
          } finally {
            setWsLoading(null);
          }
        }}
        onWorkspaceRename={async (id, name) => {
          await window.api.renameWorkspace(id, name);
          const fresh = await window.api.getConfig();
          setConfig(fresh);
        }}
        onWorkspaceDelete={async (id) => {
          clearWorkspaceContext("Switching workspace…");
          localStorage.removeItem(`capture:entries:${id}`);
          try {
            await window.api.deleteWorkspace(id);
            const fresh = await window.api.getConfig();
            setConfig(fresh);
          } finally {
            setWsLoading(null);
          }
        }}
        activePanel={panel}
        onOpenWorkspaceSettings={() => setPanel("workspace")}
        onOpenSearch={(m) => {
          setSearchModalMode(m ?? "current");
          setSearchModalOpen(true);
        }}
      />

      <div className="flex flex-1 min-h-0 overflow-hidden">
        {/* Left Sidebar - single state, always visible */}
        <nav className="w-[74px] bg-surface border-r border-border flex flex-col flex-shrink-0 overflow-hidden">
          <AppSidebar
            entries={visiblePanels}
            activePanel={panel}
            onPanelSelect={setPanel}
            badges={navBadges}
          />
        </nav>

        {/* Main content */}
        <main className="flex-1 overflow-hidden flex flex-col min-w-0">
          {renderPanel(panel, panelRenderCtx)}
        </main>

        {/* Right History Sidebar - always mounted so close animates */}
        <HistorySidebar
          filePath={openedEntityPath}
          workspaceId={wsId}
          onClose={closeHistory}
          open={historyOpen}
          reloadKey={historyReloadKey}
        />
      </div>

      {/* Global footer - always visible, shows git sync status for current panel */}
      <GlobalFooter
        panel={panel}
        workspace={currentWorkspace}
        entitySyncStatus={entitySyncStatus}
        syncStatus={syncStatus}
        onPublishPanel={handlePublishPanel}
        rightContent={footerRightContent}
      />

      {/* Unified Search Modal */}
      <SearchModal
        open={searchModalOpen}
        initialMode={searchModalMode}
        activePanel={panel}
        config={wsConfig}
        services={services}
        onClose={() => setSearchModalOpen(false)}
        onSelectTab={handleSearchSelectTab}
        onOpenEntity={handleSearchOpenEntity}
        onNavigatePanel={handleSearchNavigatePanel}
      />
    </div>
  );
}
