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

/**
 * What a **host** supplies. Every field is optional, so `<App />` means what it always meant.
 *
 * ## Why this channel exists at all
 *
 * D18 makes the UI one artifact shared by two hosts, so anything that *differs* between them has to be
 * told to it rather than discovered by it. Two things differ today:
 *
 * - **which host mounted it.** The browser has no native dialogs, no tray and no titlebar controls, so
 *   the UI has to be able to leave out affordances that would do nothing.
 * - **the endpoint**, when the host chose it at runtime — the browser's connection form is where it is
 *   entered, so the UI cannot know it otherwise.
 *
 * Deliberately a **prop and not a second global.** `window.api` is the communication seam, and D18's
 * premise is that it is the *only* thing that varies between the hosts. A UI that also read an injected
 * `window.hostKind` would have two seams to keep honest, and nothing would notice one of them drifting.
 */
export interface AppProps {
  /** Which host mounted this UI. Defaults to `"electron"`. */
  host?: "electron" | "web";
  /** The endpoint this UI is connected to, when the host chose it at runtime. */
  connection?: { url: string };
}

export default function App({ host = "electron", connection }: AppProps = {}) {
  const [tosAccepted, setTosAccepted] = usePersistedState<boolean>("app:tos-accepted", false);

  if (!tosAccepted) {
    return <TermsAcceptanceScreen onAccept={() => setTosAccepted(true)} />;
  }

  return <AppShell host={host} connection={connection} />;
}

function AppShell({ host = "electron", connection }: AppProps = {}) {
  /**
   * `h-screen` when this UI **is** the window (Electron); `h-full` when a host has wrapped it in chrome
   * of its own (the browser container's connection bar).
   *
   * Not a style preference — it is a layout bug if it is wrong. `h-screen` inside a flex column makes the
   * UI 100vh tall regardless of the bar above it, which pushes the bar off-screen.
   */
  const rootHeight = host === "web" ? "h-full" : "h-screen";

  const [colorMode, setColorMode] = useColorMode();
  const [panel, setPanel] = usePersistedState<Panel>("app:active-panel", "services");
  const { visibility, setPanelVisible, isPanelVisible } = useSidebarVisibility();
  
  const {
    config, setConfig,
    wsLoading, setWsLoading,
    loadError,
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

  const publishHandlers = usePublishHandlers({
    wsConfig,
    wsId,
    refreshConfig,
    refreshEntitySyncStatus,
    panel,
    onAfterPublish: bumpHistoryReload,
  });

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

  /**
   * A failed load is shown **where the loading message was**, rather than as a toast or a console error.
   *
   * The distinction matters because the two states used to be identical: "Loading workspace…" that never
   * resolves is the same screen whether the engine is slow or gone. `plan/08`'s risk table names exactly
   * this — *"Silent UI freeze on disconnect"* — and the mitigation it gives is connection state in the
   * footer, which `AppProps.connection` now supplies for the browser host.
   */
  if (loadError !== null) {
    return (
      <div className={`flex flex-col ${rootHeight} bg-background text-foreground overflow-hidden items-center justify-center gap-3`}>
        <div className="text-destructive text-sm font-medium">Could not load this workspace</div>
        <div className="text-muted-foreground text-xs font-mono max-w-md text-center">{loadError}</div>
      </div>
    );
  }

  if (wsLoading) {
    return (
      <div className={`flex flex-col ${rootHeight} bg-background text-foreground overflow-hidden items-center justify-center gap-3`}>
        <div className="text-muted-foreground text-sm animate-pulse">{wsLoading}</div>
      </div>
    );
  }

  return (
    <div className={`flex flex-col ${rootHeight} bg-background text-foreground overflow-hidden`}>
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
      <GlobalFooter
        panel={panel}
        workspace={currentWorkspace}
        entitySyncStatus={entitySyncStatus}
        syncStatus={syncStatus}
        onPublishPanel={publishHandlers.handlePublishPanel}
        rightContent={
          /*
           * The endpoint, shown only when a host chose it at runtime.
           *
           * `plan/08`'s risk table puts connection state in the global footer, and the reason is that a
           * browser has no process boundary to notice the engine is gone — so "which host am I on" has
           * to be visible rather than inferred. Composed into the footer's existing `rightContent` slot
           * rather than by editing the footer, because the footer is about git sync and has no business
           * knowing what a host is.
           */
          <>
            {connection !== undefined && (
              <span className="flex items-center gap-1.5">
                <span className="h-1.5 w-1.5 rounded-full bg-signal" aria-hidden="true" />
                <span className="font-mono">{connection.url}</span>
              </span>
            )}
            {footerRightContent}
          </>
        }
      />
      <SearchModal
        open={searchModalOpen} initialMode={searchModalMode} activePanel={panel} config={wsConfig} services={services}
        onClose={() => setSearchModalOpen(false)} onSelectTab={handleSearchSelectTab} onOpenEntity={handleSearchOpenEntity}
        onNavigatePanel={handleSearchNavigatePanel} onRefreshServices={refreshServices}
      />
    </div>
  );
}
