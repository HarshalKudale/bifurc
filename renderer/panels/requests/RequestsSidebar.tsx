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
  handleFoldersChange: () => Promise<void>;
  handleDuplicateImpl: (id: string) => Promise<void>;
  handleMoveItemsImpl: (ids: string[], folderId: string | null) => Promise<void>;
  handleMoveFolder: (folderId: string, parentId: string | null) => Promise<void>;
  openNewTabInFolder: () => void;
  setSelectedFolderId: (id: string | null) => void;
  onHistoryOpen?: (id: string) => void;
  getEntityFilePath: (id: string) => string;
  entitySyncStatus?: Record<string, any>;
  onPublishItem?: (id: string) => void;
  onPublishFolder?: (id: string | null) => void;
  onRestoreItem?: (id: string) => void;
  handleOpenRunner: (folderId: string) => void;
  handleBeforeDeleteFolder: (id: string) => void;
}

export default function RequestsSidebar({
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
  handleFoldersChange,
  handleDuplicateImpl,
  handleMoveItemsImpl,
  handleMoveFolder,
  openNewTabInFolder,
  setSelectedFolderId,
  onHistoryOpen,
  getEntityFilePath,
  entitySyncStatus,
  onPublishItem,
  onPublishFolder,
  onRestoreItem,
  handleOpenRunner,
  handleBeforeDeleteFolder,
}: Props) {
  return (
    <>
      <SidebarHeader onCollapse={() => setSidebarOpen(false)} collapseTitle={strings.mocks.collapseSidebar}>
        <span className="text-xs font-semibold px-1 text-muted-foreground uppercase tracking-wider">
          {strings.nav.requests}
        </span>
      </SidebarHeader>
      <div className="flex-1 overflow-y-auto overflow-x-auto min-w-0" style={{ display: "flex", flexDirection: "column" }}>
        {draftTabIds.length > 0 && (
          <DraftsFolder
            label={strings.requests.drafts}
            draftTabIds={draftTabIds}
            activeTab={activeTab}
            onOpenTab={(id) => setActiveTab(id)}
            onCloseTab={closeTab}
            tabLabel={tabLabel}
          />
        )}
        <FolderTree
          kind="request"
          folders={folders}
          items={folderViewItems}
          onOpenItem={openTab}
          onDeleteItem={handleDelete}
          onDeleteItems={handleDeleteItems}
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
          onPublishItem={onPublishItem}
          onPublishFolder={onPublishFolder}
          onRestoreItem={onRestoreItem}
          onOpenRunner={handleOpenRunner}
          onBeforeDeleteFolder={handleBeforeDeleteFolder}
        />
      </div>
    </>
  );
}
