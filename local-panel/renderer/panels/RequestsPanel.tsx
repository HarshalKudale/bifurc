import React from "react";
import { AppConfig, SavedRequest, MockRule, Environment, ApiProtocol } from "@/types";
import CollectionRunner from "@/components/rest/CollectionRunner";
import ProtocolSelectorTab from "@/components/editor/ProtocolSelectorTab";
import { SidebarLayout } from "@/components/ui";
import { strings } from "@/lib/strings";
import { Zap } from "@/lib/icons";
import { useTabKeyBindings } from "@/hooks/useTabKeyBindings";
import RequestTabContent from "./requests/RequestTabContent";
import RequestsSidebar from "./requests/RequestsSidebar";
import RequestTabs from "./requests/RequestTabs";
import { useRequestsPanelState } from "./requests/useRequestsPanelState";

interface Props {
  config: AppConfig;
  onConfigChange: (cfg: AppConfig) => Promise<void>;
  pendingOpenRequest?: Omit<SavedRequest, "id" | "createdAt" | "workspaceId"> | null;
  onPendingConsumed?: () => void;
  onOpenMockEditor?: (initial: Partial<MockRule>) => void;
  activeEnv?: Environment | null;
  onHistoryOpen?: (filePath: string) => void;
  onEntityPathChange?: (filePath: string) => void;
  historyOpen?: boolean;
  onAfterSave?: () => void;
  entitySyncStatus?: Record<string, "clean" | "modified" | "new" | "deleted">;
  onPublishItem?: (id: string) => void;
  onPublishFolder?: (folderId: string | null) => void;
  onRestoreItem?: (id: string) => void;
}

export default function RequestsPanel({
  config,
  onConfigChange,
  pendingOpenRequest,
  onPendingConsumed,
  onOpenMockEditor,
  activeEnv = null,
  onHistoryOpen,
  onEntityPathChange,
  historyOpen = false,
  onAfterSave,
  entitySyncStatus,
  onPublishItem,
  onPublishFolder,
  onRestoreItem,
}: Props) {
  const {
    tabs,
    sidebarOpen,
    setSidebarOpen,
    setSelectedFolderId,
    draftProtocols,
    setDraftProtocols,
    dirtyTabs,
    setDirtyTabs,
    openNewTabInFolder,
    pendingData,
    getEntityFilePath,
    handleFoldersChange,
    handleMoveFolder,
    handleSaveEntity,
    handleDelete,
    handleDeleteItems,
    handleBeforeDeleteFolder,
    handleDuplicateImpl,
    handleMoveItemsImpl,
    handleOpenRunner,
    draftTabIds,
    tabLabel,
    tabBadge,
    folderViewItems,
    folders,
    itemProtocolMap,
    allItemsMap,
    allEntities,
    runnerFolderIds,
    isRunner,
    isDraft,
  } = useRequestsPanelState({
    config,
    onConfigChange,
    pendingOpenRequest,
    onPendingConsumed,
    onEntityPathChange,
    historyOpen,
    onAfterSave,
  });

  const {
    openTabs,
    activeTab,
    setActiveTab,
    loadedEntities,
    setLoadedEntities,
    tabRefs,
    openTab,
    closeTab,
    reorderTabs,
    closeOtherTabs,
    closeAllTabs,
  } = tabs;

  useTabKeyBindings({ activeTab, tabRefs, closeTab, openNewTab: openNewTabInFolder });

  const mainContent = (
    <div className="flex flex-col flex-1 overflow-hidden min-w-0 h-full">
      <RequestTabs
        openTabs={openTabs}
        activeTab={activeTab}
        dirtyTabs={dirtyTabs}
        tabBadge={tabBadge}
        tabLabel={tabLabel}
        isDraft={isDraft}
        isRunner={isRunner}
        setActiveTab={setActiveTab}
        closeTab={closeTab}
        openNewTabInFolder={openNewTabInFolder}
        reorderTabs={reorderTabs}
        closeOtherTabs={closeOtherTabs}
        closeAllTabs={closeAllTabs}
        handleDuplicateImpl={handleDuplicateImpl}
      />

      <div className="flex-1 overflow-hidden relative">
        {openTabs.length === 0 ? (
          <div className="flex flex-col items-center justify-center h-full text-center gap-2">
            <div className="opacity-10 mb-1"><Zap size={48} /></div>
            <div className="text-sm font-medium text-foreground">{strings.requests.noRequestsOpen}</div>
            <p className="text-xs text-muted-foreground max-w-xs leading-relaxed">
              {strings.requests.noRequestsOpenHint}
            </p>
          </div>
        ) : (
          openTabs.map((tabId) => {
            if (isRunner(tabId)) {
              const folderId = runnerFolderIds[tabId];
              return (
                <div
                  key={tabId}
                  className="absolute inset-0 flex flex-col overflow-hidden"
                  style={{ display: activeTab === tabId ? "flex" : "none" }}
                >
                  {folderId && (
                    <CollectionRunner
                      folderId={folderId}
                      folders={folders}
                      activeEnv={activeEnv}
                      onClose={() => closeTab(tabId)}
                    />
                  )}
                </div>
              );
            }

            const isUnsaved = isDraft(tabId);
            const savedProto = itemProtocolMap.get(tabId);
            const draftProto = draftProtocols[tabId];
            const currentProto = isUnsaved ? draftProto : savedProto;

            if (isUnsaved && !currentProto) {
              return (
                <div
                  key={tabId}
                  className="absolute inset-0 flex flex-col overflow-hidden"
                  style={{ display: activeTab === tabId ? "flex" : "none" }}
                >
                  <ProtocolSelectorTab
                    mode="request"
                    onSelect={(proto) => {
                      setDraftProtocols((prev) => ({ ...prev, [tabId]: proto }));
                    }}
                  />
                </div>
              );
            }

            const activeProtocol: ApiProtocol = currentProto ?? "rest";

            const entity = isUnsaved
              ? null
              : (loadedEntities[tabId] ?? allEntities.find((e: any) => e.id === tabId) ?? null);
            const initialData = isUnsaved
              ? (pendingData[tabId] ?? null)
              : entity;
            if (!isUnsaved && !entity) return null;

            const itemMeta = allItemsMap.get(tabId);
            const relPath = itemMeta?.relPath ?? "";
            const syncStatus = relPath ? entitySyncStatus?.[relPath] : undefined;

            return (
              <RequestTabContent
                key={tabId}
                tabId={tabId}
                activeTab={activeTab}
                isUnsaved={isUnsaved}
                activeProtocol={activeProtocol}
                initialData={initialData}
                relPath={relPath}
                syncStatus={syncStatus}
                folders={folders}
                activeEnv={activeEnv}
                config={config}
                tabRefs={tabRefs}
                handleSaveEntity={handleSaveEntity}
                closeTab={closeTab}
                setDirtyTabs={setDirtyTabs}
                onOpenMockEditor={onOpenMockEditor}
                onPublishItem={onPublishItem}
                onRestoreItem={onRestoreItem}
                setLoadedEntities={setLoadedEntities}
                onHistoryOpen={onHistoryOpen}
              />
            );
          })
        )}
      </div>
    </div>
  );

  return (
    <>
      <SidebarLayout
        sidebarOpen={sidebarOpen}
        onToggle={() => setSidebarOpen(true)}
        sidebar={
          <RequestsSidebar
            setSidebarOpen={setSidebarOpen}
            draftTabIds={draftTabIds}
            activeTab={activeTab}
            setActiveTab={setActiveTab}
            closeTab={closeTab}
            tabLabel={tabLabel}
            folders={folders}
            folderViewItems={folderViewItems}
            openTab={openTab}
            handleDelete={handleDelete}
            handleDeleteItems={handleDeleteItems}
            handleFoldersChange={handleFoldersChange}
            handleDuplicateImpl={handleDuplicateImpl}
            handleMoveItemsImpl={handleMoveItemsImpl}
            handleMoveFolder={handleMoveFolder}
            openNewTabInFolder={openNewTabInFolder}
            setSelectedFolderId={setSelectedFolderId}
            onHistoryOpen={onHistoryOpen}
            getEntityFilePath={getEntityFilePath}
            entitySyncStatus={entitySyncStatus}
            onPublishItem={onPublishItem}
            onPublishFolder={onPublishFolder}
            onRestoreItem={onRestoreItem}
            handleOpenRunner={handleOpenRunner}
            handleBeforeDeleteFolder={handleBeforeDeleteFolder}
          />
        }
        collapseTitle={strings.mocks.collapseSidebar}
        expandTitle={strings.mocks.expandSidebar}
        storageKey="requests-panel-sidebar"
        collapsedBadge={allEntities.length > 0 ? (
          <span className="text-[9px] text-muted-foreground font-mono" title={`${allEntities.length} requests`}
            style={{ writingMode: "vertical-rl", transform: "rotate(180deg)", lineHeight: 1.4 }}>{allEntities.length}</span>
        ) : undefined}
      >
        {mainContent}
      </SidebarLayout>
    </>
  );
}
