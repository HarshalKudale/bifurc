import React from "react";
import { SidebarHeader } from "@/components/ui";
import { strings } from "@/lib/strings";
import DraftsFolder from "@/components/sidebar/DraftsFolder";
import FolderTree, { FolderTreeItem } from "@/components/sidebar/FolderTree";
import { Folder } from "@/types";
import { Play, Square } from "@/lib/icons";

interface Props {
  setSidebarOpen: React.Dispatch<React.SetStateAction<boolean>>;
  draftTabIds: string[];
  activeTab: string | null;
  setActiveTab: (id: string) => void;
  closeTab: (id: string) => void;
  tabLabel: (id: string) => string;
  folders: Folder[];
  folderViewItems: FolderTreeItem[];
  openTab: (id: string) => void;
  handleDelete: (id: string) => Promise<void>;
  handleFoldersChange: () => Promise<void>;
  handleDuplicate: (id: string) => Promise<void>;
  handleMoveItems: (ids: string[], folderId: string | null) => Promise<void>;
  handleMoveFolder: (folderId: string, parentId: string | null) => Promise<void>;
  openNewTab: () => void;
  onHistoryOpen?: (id: string) => void;
  getEntityFilePath: (id: string) => string;
  entitySyncStatus?: Record<string, any>;
  onPublishItem?: (id: string) => void;
  onPublishFolder?: (id: string | null) => void;
  onRestoreItem?: (id: string) => void;
  folderStatusMap: Record<string, any>;
  handleServerToggle: () => void;
  serverLoading: boolean;
  serverRunning: boolean;
  serverError: string | null;
  webhookPort: number;
}

export default function WebhooksSidebar({
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
  handleFoldersChange,
  handleDuplicate,
  handleMoveItems,
  handleMoveFolder,
  openNewTab,
  onHistoryOpen,
  getEntityFilePath,
  entitySyncStatus,
  onPublishItem,
  onPublishFolder,
  onRestoreItem,
  folderStatusMap,
  handleServerToggle,
  serverLoading,
  serverRunning,
  serverError,
  webhookPort,
}: Props) {
  return (
    <>
      <SidebarHeader onCollapse={() => setSidebarOpen(false)} collapseTitle={strings.webhooks.collapseSidebar}>
        <span className="text-xs font-semibold px-1 text-muted-foreground uppercase tracking-wider">
          {strings.nav.webhooks}
        </span>
      </SidebarHeader>

      <div className="flex items-center gap-2 px-3 py-2 border-b border-border">
        <button
          onClick={handleServerToggle}
          disabled={serverLoading}
          className={`flex items-center justify-center w-6 h-6 rounded border transition-all duration-150 cursor-pointer disabled:opacity-40 disabled:cursor-not-allowed flex-shrink-0 ${serverRunning
            ? "border-signal/40 bg-signal/10 hover:bg-destructive/15 hover:border-destructive/40 text-signal hover:text-destructive"
            : "border-border bg-card hover:bg-signal/15 hover:border-signal/40 text-muted-foreground hover:text-signal"
            }`}
          title={serverRunning ? strings.webhooks.stopServer : strings.webhooks.startServer}
        >
          {serverLoading ? (
            <span className="inline-block w-2.5 h-2.5 border-2 border-current/30 border-t-current rounded-full animate-spin" />
          ) : serverRunning ? (
            <Square size={8} fill="currentColor" />
          ) : (
            <Play size={8} fill="currentColor" />
          )}
        </button>
        <span className="text-[10px] text-muted-foreground">
          {serverRunning ? `${strings.webhooks.serverLabel} :${webhookPort}` : strings.webhooks.serverStopped}
        </span>
        {serverError && (
          <span className="text-[9px] text-destructive truncate max-w-[120px]" title={serverError}>
            {serverError}
          </span>
        )}
      </div>

      <div className="flex-1 overflow-y-auto overflow-x-auto min-w-0" style={{ display: "flex", flexDirection: "column" }}>
        {draftTabIds.length > 0 && (
          <DraftsFolder
            label={strings.webhooks.drafts}
            draftTabIds={draftTabIds}
            activeTab={activeTab}
            onOpenTab={(id) => setActiveTab(id)}
            onCloseTab={closeTab}
            tabLabel={tabLabel}
          />
        )}
        <FolderTree
          kind="webhook"
          folders={folders}
          items={folderViewItems}
          onOpenItem={openTab}
          onDeleteItem={handleDelete}
          onFoldersChange={handleFoldersChange}
          onDuplicateItem={handleDuplicate}
          onMoveItems={handleMoveItems}
          onMoveFolder={handleMoveFolder}
          onOpenNewTab={openNewTab}
          onHistoryItem={onHistoryOpen ? (id) => {
            const path = getEntityFilePath(id);
            if (path) onHistoryOpen(path);
          } : undefined}
          pathStatusMap={entitySyncStatus}
          onPublishItem={onPublishItem}
          onPublishFolder={onPublishFolder}
          onRestoreItem={onRestoreItem}
          folderStatusMap={folderStatusMap}
          onBeforeDeleteFolder={(folderId) => {
            // omitted
          }}
        />
      </div>
    </>
  );
}
