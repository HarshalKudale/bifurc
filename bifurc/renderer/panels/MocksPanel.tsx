import React from "react";
import {
  AppConfig,
  MockRule,
  Environment,
} from "@/types";
import { Zap } from "@/lib/icons";
import { SidebarLayout } from "@/components/ui";
import { strings } from "@/lib/strings";
import { useTabKeyBindings } from "@/hooks/useTabKeyBindings";
import MockTabContent from "./mocks/MockTabContent";
import MocksSidebar from "./mocks/MocksSidebar";
import MockTabs from "./mocks/MockTabs";
import { useMocksPanelState } from "./mocks/useMocksPanelState";

// -- Draft tab prefix -------------------------------------------------------

const DRAFT_PREFIX = "mock-draft-";
const isDraft = (id: string) =>
  id.startsWith(DRAFT_PREFIX) ||
  id.startsWith("prefill-") ||
  id.startsWith("gql-mock-draft-") ||
  id.startsWith("grpc-mock-draft-") ||
  id.startsWith("soap-mock-draft-");

// -- Props ------------------------------------------------------------------

interface Props {
  config: AppConfig;
  onConfigChange: (cfg: AppConfig) => Promise<void>;
  pendingMockInitial?: Partial<MockRule> | null;
  onPendingConsumed?: () => void;
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

// -- MocksPanel -------------------------------------------------------------

export default function MocksPanel({
  config,
  onConfigChange,
  pendingMockInitial,
  onPendingConsumed,
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
    getEntityFilePath,
    handleFoldersChange,
    handleMoveFolder,
    handleToggle,
    handleToggleFolderItems,
    handleSaveEntity,
    handleDelete,
    handleDeleteItems,
    handleBeforeDeleteFolder,
    handleDuplicateImpl,
    handleMoveItemsImpl,
    handleBlockItem,
    handleUnblockItem,
    draftTabIds,
    tabLabel,
    tabBadge,
    folderViewItems,
    folderStatusMap,
    folders,
    itemProtocolMap,
    allItemsMap,
    allEntities,
    blocksFolder,
    prefillData,
    newTabInitials
  } = useMocksPanelState({
    config,
    onConfigChange,
    pendingMockInitial,
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
    isDraft,
    openTab,
    closeTab,
    reorderTabs,
    closeOtherTabs,
    closeAllTabs,
  } = tabs;

  useTabKeyBindings({ activeTab, tabRefs, closeTab, openNewTab: openNewTabInFolder });

  // -- Main content -------------------------------------------------------

  const mainContent = (
    <div className="flex flex-col flex-1 overflow-hidden min-w-0 h-full">
      <MockTabs
        openTabs={openTabs}
        activeTab={activeTab}
        dirtyTabs={dirtyTabs}
        tabBadge={tabBadge}
        tabLabel={tabLabel}
        isDraft={isDraft}
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
            <div className="text-sm font-medium text-foreground">{strings.mocks.noMocksOpen}</div>
            <p className="text-xs text-muted-foreground max-w-xs leading-relaxed">
              {strings.mocks.noMocksOpenHint}
            </p>
          </div>
        ) : (
          openTabs.map((tabId) => {
            const isUnsaved = isDraft(tabId);
            const savedProto = itemProtocolMap.get(tabId);
            const draftProto = draftProtocols[tabId];
            const currentProto = isUnsaved ? draftProto : savedProto;

            const entity = isUnsaved
              ? null
              : (loadedEntities[tabId] ?? allEntities.find((m: any) => m.id === tabId) ?? null);
            const initialData = isUnsaved
              ? (prefillData[tabId] ?? newTabInitials[tabId] ?? null)
              : entity;

            const itemMeta = allItemsMap.get(tabId);
            const relPath = itemMeta?.relPath ?? "";
            const syncStatus = relPath && entitySyncStatus ? entitySyncStatus[relPath] : undefined;

            return (
              <MockTabContent
                key={tabId}
                tabId={tabId}
                activeTab={activeTab}
                isUnsaved={isUnsaved}
                currentProto={currentProto}
                initialData={initialData}
                itemMeta={itemMeta}
                relPath={relPath}
                syncStatus={syncStatus}
                folders={folders}
                activeEnv={activeEnv}
                config={config}
                tabRefs={tabRefs}
                setDraftProtocols={setDraftProtocols}
                handleSaveEntity={handleSaveEntity}
                closeTab={closeTab}
                setDirtyTabs={setDirtyTabs}
                handleToggle={handleToggle}
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
          <MocksSidebar
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
            handleToggle={handleToggle}
            handleToggleFolderItems={handleToggleFolderItems}
            handleFoldersChange={handleFoldersChange}
            handleDuplicateImpl={handleDuplicateImpl}
            handleMoveItemsImpl={handleMoveItemsImpl}
            handleMoveFolder={handleMoveFolder}
            openNewTabInFolder={openNewTabInFolder}
            setSelectedFolderId={setSelectedFolderId}
            onHistoryOpen={onHistoryOpen}
            getEntityFilePath={getEntityFilePath}
            entitySyncStatus={entitySyncStatus}
            folderStatusMap={folderStatusMap}
            onPublishItem={onPublishItem}
            onPublishFolder={onPublishFolder}
            onRestoreItem={onRestoreItem}
            handleBeforeDeleteFolder={handleBeforeDeleteFolder}
            blocksFolderId={blocksFolder?.id ?? null}
            handleBlockItem={handleBlockItem}
            handleUnblockItem={handleUnblockItem}
          />
        }
        collapseTitle={strings.mocks.collapseSidebar}
        expandTitle={strings.mocks.expandSidebar}
        storageKey="mocks-panel-sidebar"
        collapsedBadge={allEntities.length > 0 ? (
          <span className="text-[9px] text-muted-foreground font-mono" title={`${allEntities.length} mocks`}
            style={{ writingMode: "vertical-rl", transform: "rotate(180deg)", lineHeight: 1.4 }}>{allEntities.length}</span>
        ) : undefined}
      >
        {mainContent}
      </SidebarLayout>
    </>
  );
}
