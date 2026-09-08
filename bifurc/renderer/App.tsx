import React, { useEffect, useState, useCallback, useMemo } from "react";
import { MockRule, SavedRequest, Folder, SyncStatus } from "@/types";
import { mergeEnvVars } from "@/lib/resolveVars";
import { CaptureStats } from "@/panels/CapturePanel";
import HistorySidebar from "@/components/sidebar/HistorySidebar";
import AppSidebar from "@/components/sidebar/AppSidebar";
import TitleBar from "@/components/layout/TitleBar";
import GlobalFooter from "@/components/layout/GlobalFooter";
import { useColorMode } from "@/hooks/useTheme";
import { Panel, enabledPanels, PANEL_HELP } from "@/lib/panelRegistry";
import { renderPanel, PanelRenderContext } from "@/lib/panelFactory";
import { useSidebarVisibility } from "@/hooks/useSidebarVisibility";
import { usePersistedState } from "@/hooks/usePersistedState";
import SearchModal from "@/components/search/SearchModal";
import TermsAcceptanceScreen from "@/components/onboarding/TermsAcceptanceScreen";
import { readStorage, writeStorage } from "@/lib/storage";

import { useAppConfigSync } from "@/hooks/useAppConfigSync";
import { useWorkspaceView } from "@/hooks/useWorkspaceView";
import { usePublishHandlers } from "@/hooks/usePublishHandlers";

export default function App() {
  const [tosAccepted, setTosAccepted] = usePersistedState<boolean>("app:tos-accepted", false);

  if (!tosAccepted) {
    return <TermsAcceptanceScreen onAccept={() => setTosAccepted(true)} />;
  }

  return <AppShell />;
}

function AppShell() {
  const [colorMode, setColorMode] = useColorMode();
  const [panel, setPanel] = usePersistedState<Panel>("app:active-panel", "services");
  const { visibility, setPanelVisible, isPanelVisible } = useSidebarVisibility();
  
  const {
    config, setConfig,
    wsLoading, setWsLoading,
    services, refreshServices,
    serverRunning, setServerRunning,
    serverError, setServerError,
    entitySyncStatus, refreshEntitySyncStatus,
    syncStatus,
    handleConfigChange,
    handleWsConfigChange: rawHandleWsConfigChange,
    refreshConfig,
    clearWorkspaceContext: baseClearContext,
  } = useAppConfigSync();

  const wsId = config.activeWorkspaceId;
  const wsConfig = useWorkspaceView(config, wsId);

  const [mappingPrefill, setMappingPrefill] = useState<string | undefined>();
  const [pendingOpenRequest, setPendingOpenRequest] = useState<Omit<SavedRequest, "id" | "createdAt" | "workspaceId"> | null>(null);
  const [pendingMockInitial, setPendingMockInitial] = useState<Partial<MockRule> | null>(null);
  const [pendingRuleId, setPendingRuleId] = useState<string | null>(null);
  const [captureStats, setCaptureStats] = useState<CaptureStats | null>(null);
  const [historyOpen, setHistoryOpen] = useState(false);
  const [openedEntityPath, setOpenedEntityPath] = useState<string>("");
  const [historyReloadKey, setHistoryReloadKey] = useState(0);

  const [searchModalOpen, setSearchModalOpen] = useState(false);
  const [searchModalMode, setSearchModalMode] = useState<"current" | "global">("current");

  const handleWsConfigChange = useCallback((next: any) => rawHandleWsConfigChange(next, wsId), [rawHandleWsConfigChange, wsId]);

  const clearWorkspaceContext = useCallback((msg?: string) => {
    setPendingOpenRequest(null);
    setPendingMockInitial(null);
    setPendingRuleId(null);
    setHistoryOpen(false);
    setMappingPrefill(undefined);
    baseClearContext(msg);
  }, [baseClearContext]);

  const openHistory = useCallback((filePath: string) => {
    if (historyOpen && openedEntityPath === filePath) { setHistoryOpen(false); return; }
    setOpenedEntityPath(filePath);
    if (!historyOpen) setHistoryOpen(true);
  }, [historyOpen, openedEntityPath]);

  const closeHistory = useCallback(() => setHistoryOpen(false), []);
  const bumpHistoryReload = useCallback(() => setHistoryReloadKey((k) => k + 1), []);
  const handleEntityPathChange = useCallback((filePath: string) => setOpenedEntityPath(filePath), []);

  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      const isCtrlOrCmd = e.ctrlKey || e.metaKey;
      if (isCtrlOrCmd && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setSearchModalMode(e.shiftKey ? "global" : "current");
        setSearchModalOpen(true);
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);

  const getStorageKeyForPanel = useCallback((p: Panel): string | null => {
    switch (p) {
      case "requests": case "req-rest": case "req-graphql": case "req-soap": case "req-grpc": return "requests";
      case "mocks": case "mock-rest": case "mock-graphql": case "mock-soap": case "mock-grpc": return "mocks";
      case "rules": return "rules";
      case "sockets": return "ws";
      case "webhooks": return "webhooks";
      default: return null;
    }
  }, []);

  const handleSearchSelectTab = useCallback((targetPanel: Panel, tabId: string) => {
    if (targetPanel === "rules") {
      setPendingRuleId(tabId); setPanel("rules");
      window.dispatchEvent(new CustomEvent("bifurc:select-rule", { detail: { ruleId: tabId } }));
      return;
    }
    const storageKey = getStorageKeyForPanel(targetPanel);
    if (storageKey) writeStorage(`${storageKey}:activeTab`, tabId);
    setPanel(targetPanel);
    window.dispatchEvent(new CustomEvent("bifurc:open-tab", { detail: { panel: targetPanel, tabId } }));
  }, [getStorageKeyForPanel, setPanel]);

  const handleSearchOpenEntity = useCallback((targetPanel: Panel, entityId: string) => {
    if (targetPanel === "rules") {
      setPendingRuleId(entityId); setPanel("rules");
      window.dispatchEvent(new CustomEvent("bifurc:select-rule", { detail: { ruleId: entityId } }));
      return;
    }
    const storageKey = getStorageKeyForPanel(targetPanel);
    if (storageKey) {
      const openKey = `${storageKey}:openTabs`;
      const currentOpen = readStorage<string[]>(openKey, []);
      if (!currentOpen.includes(entityId)) writeStorage(openKey, [...currentOpen, entityId]);
      writeStorage(`${storageKey}:activeTab`, entityId);
    }
    setPanel(targetPanel);
    window.dispatchEvent(new CustomEvent("bifurc:open-tab", { detail: { panel: targetPanel, tabId: entityId } }));
  }, [getStorageKeyForPanel, setPanel]);

  const handleSearchNavigatePanel = useCallback((targetPanel: Panel, target?: any) => {
    if (targetPanel === "mappings" && typeof target === "string") setMappingPrefill(target);
    else if (targetPanel === "rules") {
      const ruleId = typeof target === "string" ? target : target?.id;
      if (ruleId) {
        setPendingRuleId(ruleId);
        window.dispatchEvent(new CustomEvent("bifurc:select-rule", { detail: { ruleId } }));
      }
    }
    setPanel(targetPanel);
  }, [setPanel]);

  useEffect(() => { if (panel === "applications") setPanel("services"); }, [panel, setPanel]);

  const handleOpenInRequests = useCallback((req: any) => { setPendingOpenRequest(req); setPanel("requests"); }, []);
  const handleOpenMockEditor = useCallback((initial: any) => { setPendingMockInitial(initial); setPanel("mocks"); }, []);

  const publishHandlers = usePublishHandlers({ wsConfig, wsId, refreshConfig, refreshEntitySyncStatus, panel });

  const globalEnv = (wsConfig.environments ?? []).find((e) => e.id === "__global__") ?? null;
  const selectedActiveEnv = (wsConfig.environments ?? []).find((e) => e.id === wsConfig.activeEnvironmentId) ?? null;
  const activeEnv = mergeEnvVars(globalEnv, selectedActiveEnv);

  const cnt = (n: number) => n > 0 ? n : undefined;
  const totalRequests = (wsConfig.requests ?? []).length + (wsConfig.graphqlRequests ?? []).length + (wsConfig.grpcRequests ?? []).length + (wsConfig.soapRequests ?? []).length;
  const totalMocks = (wsConfig.mocks ?? []).length + (wsConfig.graphqlMocks ?? []).length + (wsConfig.grpcMocks ?? []).length + (wsConfig.soapMocks ?? []).length;
  const navBadges: Partial<Record<Panel, number | undefined>> = {
    mappings: cnt((wsConfig.mappings ?? []).length),
    rules: cnt((wsConfig.proxyRules ?? []).length),
    requests: cnt(totalRequests), mocks: cnt(totalMocks), "mock-rest": cnt(totalMocks), "req-rest": cnt(totalRequests),
    sockets: cnt((wsConfig.wsConnections ?? []).length), webhooks: cnt((wsConfig.webhooks ?? []).length),
    environments: cnt((wsConfig.environments ?? []).filter((e) => e.id !== "__global__").length),
  };

  const visiblePanels = useMemo(() => enabledPanels.filter((e) => isPanelVisible(e.id)), [visibility]);
  const currentWorkspace = (config.workspaces ?? []).find((w) => w.id === wsId) ?? null;

  const footerRightContent = useMemo(() => {
    const pl = (n: number, s: string) => `${n} ${s}${n !== 1 ? "s" : ""}`;
    switch (panel) {
      case "capture":
        if (!captureStats) return null;
        return (
          <>
            <span>{captureStats.total} captured</span>
            {captureStats.shown < captureStats.total && <span>· {captureStats.shown} shown</span>}
            {captureStats.paused && <span className="text-yellow">· paused</span>}
            <span className="opacity-50">newest first · last 200 kept</span>
          </>
        );
      case "mocks": case "mock-rest": return <span>{pl(totalMocks, "mock")}</span>;
      case "requests": case "req-rest": return <span>{pl(totalRequests, "request")}</span>;
      case "sockets": return <span>{pl(wsConfig.wsConnections?.length ?? 0, "socket")}</span>;
      case "webhooks": return <span>{pl(wsConfig.webhooks?.length ?? 0, "webhook")}</span>;
      case "mappings": return <span>{pl(wsConfig.mappings?.length ?? 0, "mapping")}</span>;
      case "rules": return <span>{pl(wsConfig.proxyRules?.length ?? 0, "rule")}</span>;
      case "environments": return <span>{pl(wsConfig.environments?.length ?? 0, "environment")}</span>;
      case "services": return <span>{pl(services.length, "service")}</span>;
      default:
        return (
          <span className="flex items-center gap-1.5">
            <span>{pl(totalRequests, "request")}</span><span className="opacity-40">·</span>
            <span>{pl(totalMocks, "mock")}</span><span className="opacity-40">·</span>
            <span>{pl(wsConfig.mappings?.length ?? 0, "mapping")}</span>
          </span>
        );
    }
  }, [panel, captureStats, totalMocks, totalRequests, wsConfig, services]);

  const panelRenderCtx: PanelRenderContext = useMemo(() => ({
    wsConfig, config, wsId, services, serverRunning, serverError, colorMode, setColorMode,
    activeEnv, openHistory, handleEntityPathChange, historyOpen, bumpHistoryReload,
    entitySyncStatus, refreshEntitySyncStatus, handleConfigChange, handleWsConfigChange, setConfig, refreshServices,
    mappingPrefill, onPrefillConsumed: () => setMappingPrefill(undefined), setPanel, setMappingPrefill,
    pendingOpenRequest, onPendingRequestConsumed: () => setPendingOpenRequest(null),
    pendingMockInitial, onPendingMockConsumed: () => setPendingMockInitial(null),
    pendingRuleId, onPendingRuleConsumed: () => setPendingRuleId(null),
    handleOpenMockEditor, handleOpenInRequests, onStatsChange: setCaptureStats,
    ...publishHandlers,
    onWorkspaceRename: async (id: string, name: string) => {
      await window.api.renameWorkspace(id, name);
      setConfig(await window.api.getConfig());
    },
    onWorkspaceDelete: async (id: string) => {
      localStorage.removeItem(`capture:entries:${id}`);
      await window.api.deleteWorkspace(id);
      setConfig(await window.api.getConfig());
      setPanel("services");
    },
    onServerRestart: async () => {
      await window.api.restartServer();
      const status = await window.api.serverStatus();
      setServerRunning(status.running); setServerError(status.error);
    },
    sidebarVisibility: visibility, setSidebarPanelVisible: setPanelVisible,
  }), [
    wsConfig, config, wsId, services, serverRunning, serverError, colorMode, setColorMode,
    activeEnv, openHistory, handleEntityPathChange, historyOpen, bumpHistoryReload,
    entitySyncStatus, refreshEntitySyncStatus, handleConfigChange, handleWsConfigChange,
    refreshServices, mappingPrefill, pendingOpenRequest, pendingMockInitial, pendingRuleId,
    handleOpenMockEditor, handleOpenInRequests, publishHandlers, visibility, setPanelVisible,
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
        config={config} serverRunning={serverRunning} serverError={serverError} helpText={PANEL_HELP[panel]}
        onServerStart={async () => {
          await window.api.startServer();
          const status = await window.api.serverStatus();
          setServerRunning(status.running); setServerError(status.error);
        }}
        onServerStop={async () => { await window.api.stopServer(); setServerRunning(false); setServerError(null); }}
        onEnvChange={async (id) => { await window.api.setActiveEnvironment(id); setConfig(await window.api.getConfig()); }}
        onManageEnvs={() => setPanel("environments")}
        workspaces={config.workspaces ?? []} activeWorkspaceId={config.activeWorkspaceId}
        onWorkspaceChange={async (id) => {
          clearWorkspaceContext("Switching workspace…");
          try {
            const result = await window.api.setActiveWorkspace(id);
            if (result.ok) { setConfig(result.config); refreshEntitySyncStatus(id); }
          } finally { setWsLoading(null); }
        }}
        onWorkspaceCreate={async () => {
          clearWorkspaceContext("Creating workspace…");
          try {
            const ws = await window.api.addWorkspace("");
            const result = await window.api.setActiveWorkspace(ws.id);
            if (result.ok) { setConfig(result.config); setPanel("workspace"); }
          } finally { setWsLoading(null); }
        }}
        onWorkspaceRename={async (id, name) => { await window.api.renameWorkspace(id, name); setConfig(await window.api.getConfig()); }}
        onWorkspaceDelete={async (id) => {
          clearWorkspaceContext("Switching workspace…");
          localStorage.removeItem(`capture:entries:${id}`);
          try { await window.api.deleteWorkspace(id); setConfig(await window.api.getConfig()); } finally { setWsLoading(null); }
        }}
        activePanel={panel} onOpenWorkspaceSettings={() => setPanel("workspace")}
        onOpenSearch={(m) => { setSearchModalMode(m ?? "current"); setSearchModalOpen(true); }}
      />
      <div className="flex flex-1 min-h-0 overflow-hidden">
        <nav className="w-[74px] bg-surface border-r border-border flex flex-col flex-shrink-0 overflow-hidden">
          <AppSidebar entries={visiblePanels} activePanel={panel} onPanelSelect={setPanel} badges={navBadges} />
        </nav>
        <main className="flex-1 overflow-hidden flex flex-col min-w-0">{renderPanel(panel, panelRenderCtx)}</main>
        <HistorySidebar filePath={openedEntityPath} workspaceId={wsId} onClose={closeHistory} open={historyOpen} reloadKey={historyReloadKey} />
      </div>
      <GlobalFooter panel={panel} workspace={currentWorkspace} entitySyncStatus={entitySyncStatus} syncStatus={syncStatus} onPublishPanel={publishHandlers.handlePublishPanel} rightContent={footerRightContent} />
      <SearchModal
        open={searchModalOpen} initialMode={searchModalMode} activePanel={panel} config={wsConfig} services={services}
        onClose={() => setSearchModalOpen(false)} onSelectTab={handleSearchSelectTab} onOpenEntity={handleSearchOpenEntity}
        onNavigatePanel={handleSearchNavigatePanel} onRefreshServices={refreshServices}
      />
    </div>
  );
}
