import React, { useMemo } from "react";
import { Folder as FolderType, SavedWsConnection } from "@/types";
import FolderTree, { FolderTreeItem } from "@/components/sidebar/FolderTree";
import { SidebarHeader } from "@/components/ui";
import { strings } from "@/lib/strings";
import { entityRelPath } from "@/lib/utils";

interface WsSidebarProps {
  connections: SavedWsConnection[];
  folders: FolderType[];
  activeTab: string | null;
  openTab: (id: string) => void;
  handleDelete: (id: string) => Promise<void>;
  handleFoldersChange: () => Promise<void>;
  handleDuplicate: (id: string) => Promise<void>;
  handleMoveItems: (ids: string[], folderId: string | null) => Promise<void>;
  openNewTab: () => void;
  onHistoryOpen?: (filePath: string) => void;
  getEntityFilePath: (id: string) => string;
  entitySyncStatus?: Record<string, "clean" | "modified" | "new" | "deleted">;
  onPublishItem?: (id: string) => void;
  onPublishFolder?: (folderId: string | null) => void;
  onRestoreItem?: (id: string) => void;
  setSidebarOpen: (open: boolean) => void;
}

export default function WsSidebar({
  connections, folders, activeTab, openTab, handleDelete, handleFoldersChange,
  handleDuplicate, handleMoveItems, openNewTab, onHistoryOpen, getEntityFilePath,
  entitySyncStatus, onPublishItem, onPublishFolder, onRestoreItem, setSidebarOpen
}: WsSidebarProps) {
  // Folder view items
  const folderViewItems: FolderTreeItem[] = useMemo(() =>
    connections.map((c): FolderTreeItem => ({
      id: c.id,
      name: c.name || c.url.slice(0, 40),
      folderId: c.folderId ?? null,
      isActive: activeTab === c.id,
      isEnabled: true,
      relPath: entityRelPath("sockets", c, folders),
    })),
    [connections, folders, activeTab],
  );

  return (
    <>
      <SidebarHeader onCollapse={() => setSidebarOpen(false)} collapseTitle={strings.titleBar.collapseSidebar}>
        <span className="text-xs font-semibold px-1 text-muted-foreground uppercase tracking-wider">
          {strings.panels.sectionWebsocket}
        </span>
      </SidebarHeader>

      <div className="flex-1 overflow-y-auto overflow-x-auto min-w-0" style={{ display: "flex", flexDirection: "column" }}>
        <FolderTree
          kind="ws"
          folders={folders}
          items={folderViewItems}
          onOpenItem={openTab}
          onDeleteItem={handleDelete}
          onFoldersChange={handleFoldersChange}
          onDuplicateItem={handleDuplicate}
          onMoveItems={handleMoveItems}
          onOpenNewTab={openNewTab}
          onHistoryItem={onHistoryOpen ? (id) => {
            const path = getEntityFilePath(id);
            if (path) onHistoryOpen(path);
          } : undefined}
          pathStatusMap={entitySyncStatus}
          onPublishItem={onPublishItem}
          onPublishFolder={onPublishFolder}
          onRestoreItem={onRestoreItem}
          onBeforeCreateFolder={() => true}
        />
      </div>
    </>
  );
}
