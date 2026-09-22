import { Folder as FolderType } from "@/types";

export interface FolderTreeItem {
  id: string;
  name: string;
  method?: string;
  folderId?: string | null;
  isActive?: boolean;
  isEnabled?: boolean;
  relPath?: string;
  isRunner?: boolean;
  isBlock?: boolean;
  hideDot?: boolean;
}

export interface FolderNode {
  folder: FolderType | null;
  children: FolderNode[];
  items: FolderTreeItem[];
}

export type EntitySyncStatus = "clean" | "modified" | "new" | "deleted";
export type FolderStatus = "enabled" | "mixed" | "disabled";

export interface FolderTreeProps {
  kind: "mock" | "request" | "ws" | "webhook" | "rule" | "graphqlRequest" | "graphqlMock" | "soapRequest" | "soapMock" | "grpcRequest" | "grpcMock" | "runner";
  folders: FolderType[];
  items: FolderTreeItem[];
  onOpenItem(id: string): void;
  onDeleteItem(id: string): void;
  onToggleItem?: (id: string) => void;
  onToggleFolderItems?: (folderId: string | null, enable: boolean) => void;
  onFoldersChange(): void;
  onDuplicateItem?: (id: string) => void;
  onMoveItems?: (ids: string[], folderId: string | null) => void;
  onMoveFolder?: (folderId: string, targetParentId: string | null) => void;
  onOpenNewTab?: () => void;
  onHistoryItem?: (id: string) => void;
  pathStatusMap?: Record<string, EntitySyncStatus>;
  entitySyncStatus?: Record<string, EntitySyncStatus>;
  folderStatusMap?: Record<string, FolderStatus>;
  onPublishItem?: (id: string) => void;
  onPublishFolder?: (folderId: string | null) => void;
  onRestoreItem?: (id: string) => void;
  onBeforeCreateFolder?: () => boolean;
  onOpenRunner?: (folderId: string) => void;
  onBeforeDeleteFolder?: (folderId: string) => void;
  blocksFolderId?: string | null;
  onBlockItem?: (id: string) => void;
  onUnblockItem?: (id: string) => void;
  onSelectedFolderChange?: (folderId: string | null) => void;
  onDeleteItems?: (trackedIds: string[], untrackedIds: string[]) => Promise<void>;
  onStartItem?: (id: string) => void;
  onStopItem?: (id: string) => void;
}

export interface TreeContextType {
  expanded: Set<string>;
  selectedItemIds: Set<string>;
  selectedFolderIds: Set<string>;
  dragOver: string | null;
  renaming: string | null;
  newFolderParent: string | null | undefined;
  hoveredItemId: string | null;
  
  onToggleExpand: (id: string) => void;
  onSelectFolder: (id: string, multi: boolean, range: boolean) => void;
  onSelectItem: (id: string, multi: boolean, range: boolean) => void;
  onFolderContextMenu: (e: React.MouseEvent, id: string | null) => void;
  onItemContextMenu: (e: React.MouseEvent, item: FolderTreeItem) => void;
  
  onDragStartFolder: (e: React.DragEvent, id: string) => void;
  onDragStartItem: (e: React.DragEvent, id: string) => void;
  onDragOverNode: (e: React.DragEvent, id: string) => void;
  onDropNode: (e: React.DragEvent, id: string | null) => void;
  onDragLeaveNode: () => void;
  
  onHoverItem: (id: string | null) => void;
  
  handleNewFolder: (name: string, parentId: string | null) => void;
  cancelNewFolder: () => void;
  
  getItemStatus: (item: FolderTreeItem) => EntitySyncStatus | undefined;
  
  folderStatusMap?: Record<string, FolderStatus>;
  onOpenRunner?: (id: string) => void;
  onOpenItem: (id: string) => void;
  setDeletedItemPopup: (item: FolderTreeItem) => void;
  hasMoveItems?: boolean;
  hasMoveFolder?: boolean;
}
