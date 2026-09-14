import React from "react";
import { SidebarHeader } from "@/components/ui";
import { strings } from "@/lib/strings";
import DraftsFolder from "@/components/sidebar/DraftsFolder";
import FolderTree, { FolderTreeItem } from "@/components/sidebar/FolderTree";
import { Folder } from "@/types";

interface Props {
  setSidebarOpen: (v: boolean) => void;
  draftTabIds: string[];
  activeTab: string | null;
  setActiveTab: (id: string) => void;
  closeTab: (id: string) => void;
  tabLabel: (id: string) => string;
  folders: Folder[];
  folderViewItems: FolderTreeItem[];
  openTab: (id: string) => void;
  handleDelete: (id: string) => Promise<void>;
  handleDeleteItems: (tracked: string[], untracked: string[]) => Promise<void>;
  handleToggle: (id: string) => Promise<void>;
  handleToggleFolderItems: (folderId: string | null, enable: boolean) => Promise<void>;
  handleFoldersChange: () => Promise<void>;
  handleDuplicateImpl: (id: string) => Promise<void>;
  handleMoveItemsImpl: (ids: string[], folderId: string | null) => Promise<void>;
  handleMoveFolder: (folderId: string, parentId: string | null) => Promise<void>;
  openNewTabInFolder: () => void;
  setSelectedFolderId: (id: string | null) => void;
  onHistoryOpen?: (id: string) => void;
  getEntityFilePath: (id: string) => string;
  entitySyncStatus?: Record<string, any>;
  folderStatusMap: Record<string, any>;
  onPublishItem?: (id: string) => void;
  onPublishFolder?: (id: string | null) => void;
  onRestoreItem?: (id: string) => void;
  handleBeforeDeleteFolder: (id: string) => void;
  blocksFolderId: string | null;
  handleBlockItem: (id: string) => Promise<void>;
  handleUnblockItem: (id: string) => Promise<void>;
}

export default function MocksSidebar({
  setSidebarOpen,
  draftTabIds,
  activeTab,
  setActiveTab,
  closeTab,
  tabLabel,
  folders,
  folderViewItems,
  openTab,
  handleDelete,
  handleDeleteItems,
  handleToggle,
  handleToggleFolderItems,
  handleFoldersChange,
  handleDuplicateImpl,
  handleMoveItemsImpl,
  handleMoveFolder,
  openNewTabInFolder,
  setSelectedFolderId,
  onHistoryOpen,
  getEntityFilePath,
  entitySyncStatus,
  folderStatusMap,
  onPublishItem,
  onPublishFolder,
  onRestoreItem,
  handleBeforeDeleteFolder,
  blocksFolderId,
  handleBlockItem,
  handleUnblockItem,
}: Props) {
  return (
    <>
      <SidebarHeader onCollapse={() => setSidebarOpen(false)} collapseTitle={strings.mocks.collapseSidebar}>
        <span className="text-xs font-semibold px-1 text-muted-foreground uppercase tracking-wider">
          {strings.nav.mocks}
        </span>
      </SidebarHeader>
      <div className="flex-1 overflow-y-auto overflow-x-auto min-w-0" style={{ display: "flex", flexDirection: "column" }}>
        {draftTabIds.length > 0 && (
          <DraftsFolder
            label={strings.mocks.drafts}
            draftTabIds={draftTabIds}
            activeTab={activeTab}
            onOpenTab={(id) => setActiveTab(id)}
            onCloseTab={closeTab}
            tabLabel={tabLabel}
          />
        )}
        <FolderTree
          kind="mock"
          folders={folders}
          items={folderViewItems}
          onOpenItem={openTab}
          onDeleteItem={handleDelete}
          onDeleteItems={handleDeleteItems}
          onToggleItem={(id) => handleToggle(id)}
          onToggleFolderItems={handleToggleFolderItems}
          onFoldersChange={handleFoldersChange}
          onDuplicateItem={handleDuplicateImpl}
          onMoveItems={handleMoveItemsImpl}
          onMoveFolder={handleMoveFolder}
          onOpenNewTab={openNewTabInFolder}
          onSelectedFolderChange={setSelectedFolderId}
          onBeforeCreateFolder={() => true}
          onHistoryItem={onHistoryOpen ? (id) => {
            const path = getEntityFilePath(id);
            if (path) onHistoryOpen(path);
          } : undefined}
          pathStatusMap={entitySyncStatus}
          folderStatusMap={folderStatusMap}
          onPublishItem={onPublishItem}
          onPublishFolder={onPublishFolder}
          onRestoreItem={onRestoreItem}
          onBeforeDeleteFolder={handleBeforeDeleteFolder}
          blocksFolderId={blocksFolderId}
          onBlockItem={handleBlockItem}
          onUnblockItem={handleUnblockItem}
        />
      </div>
    </>
  );
}
