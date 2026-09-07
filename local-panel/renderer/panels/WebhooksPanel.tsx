import React, { forwardRef, useImperativeHandle, useEffect, useCallback } from "react";
import { AppConfig } from "@/types";
import { SidebarLayout } from "@/components/ui";
import { strings } from "@/lib/strings";
import { Plus, Webhook } from "@/lib/icons";
import { useTabKeyBindings } from "@/hooks/useTabKeyBindings";
import WebhookTabs from "./webhooks/WebhookTabs";
import WebhookTabContent from "./webhooks/WebhookTabContent";
import WebhooksSidebar from "./webhooks/WebhooksSidebar";
import { useWebhooksPanelState } from "./webhooks/useWebhooksPanelState";

interface Props {
  config: AppConfig;
  onConfigChange: (cfg: AppConfig) => Promise<void>;
  onHistoryOpen?: (filePath: string) => void;
  onEntityPathChange?: (filePath: string) => void;
  historyOpen?: boolean;
  onAfterSave?: () => void;
  entitySyncStatus?: Record<string, "clean" | "modified" | "new" | "deleted">;
  onPublishItem?: (id: string) => void;
  onPublishFolder?: (folderId: string | null) => void;
  onRestoreItem?: (id: string) => void;
}

export default function WebhooksPanel({
  config, onConfigChange,
  onHistoryOpen, onEntityPathChange, historyOpen = false,
  onAfterSave, entitySyncStatus, onPublishItem, onPublishFolder, onRestoreItem,
}: Props) {

  const deregisterWebhook = useCallback(async (tabId: string) => {
    await window.api.unregisterActiveWebhook(tabId);
  }, []);

  const {
    webhooks,
    folders,
    webhookPort,
    sidebarOpen,
    setSidebarOpen,
    dirtyTabs,
    setDirtyTabs,
    serverRunning,
    setServerRunning,
    serverError,
    setServerError,
    serverLoading,
    setServerLoading,
    payloadMap,
    setPayloadMap,
    activeTabs,
    setActiveTabs,
    tabs,
    wrappedCloseTab,
    handleNewSave,
    handleTabSave,
    handleDelete,
    handleDuplicate,
    handleMoveItems,
    handleFoldersChange,
    handleMoveFolder,
    getEntityFilePath,
    tabLabel,
    folderViewItems,
    folderStatusMap,
    isDraftId,
    MAX_ACTIVE_WEBHOOKS,
  } = useWebhooksPanelState({
    config,
    onConfigChange,
    onAfterSave,
    deregisterWebhook,
  });

  const {
    openTabs,
    activeTab,
    setActiveTab,
    loadedEntities,
    setLoadedEntities,
    tabRefs,
    openTab,
    openNewTab,
    reorderTabs,
  } = tabs;

  useTabKeyBindings({ activeTab, tabRefs, closeTab: wrappedCloseTab, openNewTab });

  const handleServerToggle = useCallback(async () => {
    setServerLoading(true);
    if (serverRunning) {
      const res = await window.api.stopWebhookServer();
      if (res.ok) {
        setServerRunning(false);
        setServerError(null);
      } else {
        setServerError(res.error || "Failed to stop server");
      }
    } else {
      const res = await window.api.startWebhookServer();
      if (res.ok) {
        setServerRunning(true);
        setServerError(null);
      } else {
        setServerError(res.error || "Failed to start server");
      }
    }
    setServerLoading(false);
  }, [serverRunning]);

  useEffect(() => {
    window.api.webhookServerStatus().then((status) => {
      setServerRunning(status.running);
      setServerError(status.error || null);
    });
  }, []);

  useEffect(() => {
    if (!historyOpen || !activeTab) return;
    const path = getEntityFilePath(activeTab);
    if (path) onEntityPathChange?.(path);
  }, [activeTab, historyOpen, getEntityFilePath, onEntityPathChange]);

  const mainContent = (
    <div className="flex flex-col flex-1 overflow-hidden">
      {openTabs.length === 0 ? (
        <div className="flex flex-col items-center justify-center flex-1 gap-4 text-center py-16 px-8">
          <div className="opacity-10"><Webhook size={48} /></div>
          <p className="text-sm text-muted-foreground">{strings.webhooks.openToReceive}</p>
          <button
            onClick={openNewTab}
            className="flex items-center gap-2 px-4 py-2 rounded bg-signal/10 hover:bg-signal/20 text-signal text-sm font-medium transition-colors cursor-pointer border border-signal/20"
          >
            <Plus size={14} /> {strings.webhooks.newWebhook}
          </button>
        </div>
      ) : (
        <>
          <WebhookTabs
            openTabs={openTabs}
            activeTab={activeTab}
            dirtyTabs={dirtyTabs}
            tabLabel={tabLabel}
            isDraftId={isDraftId}
            activeTabs={activeTabs}
            openTab={openTab}
            closeTab={wrappedCloseTab}
            openNewTab={openNewTab}
            handleReorderTabs={reorderTabs}
            handleDuplicate={handleDuplicate}
          />
          <div className="flex-1 overflow-hidden">
            {openTabs.map((tabId) => {
              const isTabActive = tabId === activeTab;
              const isDraft = isDraftId(tabId);
              const hook = isDraft ? null : (loadedEntities[tabId] ?? webhooks.find((h) => h.id === tabId) ?? null);
              const isActivated = !isDraft && activeTabs.has(tabId);
              const isAtLimit = activeTabs.size >= MAX_ACTIVE_WEBHOOKS;
              const tabPayloads = isDraft ? [] : (payloadMap[tabId] ?? []);
              const relPath = hook ? getEntityFilePath(tabId) : "";
              const syncStatus = relPath ? entitySyncStatus?.[relPath] : undefined;

              return (
                <WebhookTabContent
                  key={tabId}
                  tabId={tabId}
                  activeTab={activeTab}
                  isDraft={isDraft}
                  hook={hook}
                  webhookPort={webhookPort}
                  handleNewSave={handleNewSave}
                  handleTabSave={handleTabSave}
                  wrappedCloseTab={wrappedCloseTab}
                  folders={folders}
                  tabPayloads={tabPayloads}
                  isActivated={isActivated}
                  isAtLimit={isAtLimit}
                  setDirtyTabs={setDirtyTabs}
                  onPublishItem={onPublishItem}
                  onRestoreItem={onRestoreItem}
                  config={config}
                  setLoadedEntities={setLoadedEntities}
                  tabRefs={tabRefs}
                  onHistoryOpen={onHistoryOpen}
                  relPath={relPath}
                  syncStatus={syncStatus}
                />
              );
            })}
          </div>
        </>
      )}
    </div>
  );

  return (
    <>
      <SidebarLayout
        sidebarOpen={sidebarOpen}
        onToggle={() => setSidebarOpen((v) => !v)}
        sidebar={
          <WebhooksSidebar
            setSidebarOpen={setSidebarOpen}
            draftTabIds={openTabs.filter(isDraftId)}
            activeTab={activeTab}
            setActiveTab={setActiveTab}
            closeTab={wrappedCloseTab}
            tabLabel={tabLabel}
            folders={folders}
            folderViewItems={folderViewItems}
            openTab={openTab}
            handleDelete={handleDelete}
            handleFoldersChange={handleFoldersChange}
            handleDuplicate={handleDuplicate}
            handleMoveItems={handleMoveItems}
            handleMoveFolder={handleMoveFolder}
            openNewTab={openNewTab}
            onHistoryOpen={onHistoryOpen}
            getEntityFilePath={getEntityFilePath}
            entitySyncStatus={entitySyncStatus}
            onPublishItem={onPublishItem}
            onPublishFolder={onPublishFolder}
            onRestoreItem={onRestoreItem}
            folderStatusMap={folderStatusMap}
            handleServerToggle={handleServerToggle}
            serverLoading={serverLoading}
            serverRunning={serverRunning}
            serverError={serverError}
            webhookPort={webhookPort}
          />
        }
        storageKey="webhooks-panel-sidebar"
        collapsedBadge={
          <span className="text-[10px] text-muted-foreground rotate-90 whitespace-nowrap" style={{ writingMode: "vertical-rl" }}>
            {strings.webhooks.title}
          </span>
        }
      >
        {mainContent}
      </SidebarLayout>
    </>
  );
}
