import type { Folder as FolderType, WorkspaceGitState, EntitySyncStatus } from "@/types";

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

/**
 * An entity's position relative to HEAD. Re-exported from `@/types` so the tree's own modules can
 * import it from here — the definition itself lives beside `WorkspaceGitState` in `types/config.ts`,
 * because it is a mirror of a protocol type rather than a tree concern.
 */
export type { EntitySyncStatus } from "@/types";

/** A folder's enable/disable rollup — **not** a git status. See the note on `folderStatusMap`. */
export type FolderStatus = "enabled" | "mixed" | "disabled";

export interface FolderTreeProps {
  kind: "mock" | "request" | "ws" | "webhook" | "rule" | "graphqlRequest" | "soapRequest" | "grpcRequest" | "runner";
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
  /** Per-file git status, keyed by workspace-relative path. */
  pathStatusMap?: Record<string, EntitySyncStatus>;
  /** Per-entity git status, keyed by entity id — a fallback for an item with no `relPath`. */
  entitySyncStatus?: Record<string, EntitySyncStatus>;
  /**
   * A folder's **enable/disable** rollup, keyed by folder id.
   *
   * The name is a collision worth naming: this is *not* git status, and `folderStatusMap[id]` is
   * `"enabled" | "mixed" | "disabled"`. Git status for a folder is derived on demand by
   * `folderGitRollup()` from `pathStatusMap`, which is why there is no second map here.
   */
  folderStatusMap?: Record<string, FolderStatus>;
  /**
   * The workspace's position relative to its remote. Without it a committed-but-unpushed entity
   * reads `clean` and is offered no action at all — see `lib/entityGit.ts`.
   */
  workspaceGitState?: WorkspaceGitState | null;
  /** Publish one entity. Labelled Publish or Sync by its state. */
  onPublishItem?: (id: string) => void;
  /**
   * Publish a whole selection in **one commit**.
   *
   * Distinct from calling `onPublishItem` N times, which is what the multi-selection menu used to do
   * and which produced N commits for one gesture. Falls back to per-item publishes when absent.
   */
  onPublishPaths?: (paths: string[]) => void;
  onPublishFolder?: (folderId: string | null) => void;
  /** Revert one entity to its last synced version. Also backs "Recover" for a deleted entity. */
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
  /** The uniform view of an item's git state — what every menu and row renders from. */
  getItemView: (item: FolderTreeItem) => import("@/lib/entityGit").EntityGitView;
  /** The rollup for a folder, or for the whole kind when `folderId` is `null`. */
  getFolderRollup: (folderId: string | null) => import("@/lib/entityGit").FolderGitRollup;
  
  folderStatusMap?: Record<string, FolderStatus>;
  onOpenRunner?: (id: string) => void;
  onOpenItem: (id: string) => void;
  setDeletedItemPopup: (item: FolderTreeItem) => void;
  hasMoveItems?: boolean;
  hasMoveFolder?: boolean;
}
