import React, { useState, useEffect, useRef, useCallback, useMemo } from "react";
import ConfirmDialog from "@/components/ui/ConfirmDialog";
import ContextMenu from "@/components/ui/ContextMenu";
import { strings } from "@/lib/strings";
import { FolderTreeProps, FolderNode, TreeContextType, FolderTreeItem, EntitySyncStatus } from "./FolderTree.types";
import { FolderTreeContext } from "./FolderTreeContext";
import { FolderTreeNode } from "./FolderTreeNode";
import { MoveDialog } from "./modals/MoveDialog";
import { RenameDialog } from "./modals/RenameDialog";
import { DeletedItemDialog } from "./modals/DeletedItemDialog";
import { useFolderTreeContextMenu } from "./useFolderTreeContextMenu";

export default function FolderTree({
  kind, folders, items, onOpenItem, onDeleteItem, onToggleItem, onToggleFolderItems, onFoldersChange,
  onDuplicateItem, onMoveItems, onMoveFolder, onOpenNewTab, onHistoryItem,
  pathStatusMap, entitySyncStatus, folderStatusMap, onPublishItem, onPublishFolder, onRestoreItem,
  onBeforeCreateFolder, onOpenRunner, onBeforeDeleteFolder,
  blocksFolderId, onBlockItem, onUnblockItem, onSelectedFolderChange, onDeleteItems,
  onStartItem, onStopItem,
}: FolderTreeProps) {
  const [expanded, setExpanded] = useState<Set<string>>(() => new Set(["__root__"]));
  const [renaming, setRenaming] = useState<string | null>(null);
  const [newFolderParent, setNewFolderParent] = useState<string | null | undefined>(undefined);
  const [selectedItemIds, setSelectedItemIds] = useState<Set<string>>(new Set());
  const [selectedFolderIds, setSelectedFolderIds] = useState<Set<string>>(new Set());
  const [showMove, setShowMove] = useState(false);
  const [pendingDelete, setPendingDelete] = useState<{ itemIds: string[]; folderIds: string[]; hasTracked?: boolean } | null>(null);
  const [hoveredItemId, setHoveredItemId] = useState<string | null>(null);
  const [deletedItemPopup, setDeletedItemPopup] = useState<FolderTreeItem | null>(null);
  const [dragOver, setDragOver] = useState<string | null>(null);
  const lastClickedRef = useRef<{ id: string; kind: "folder" | "item" } | null>(null);

  const clearSelection = useCallback(() => {
    setSelectedItemIds(new Set());
    setSelectedFolderIds(new Set());
    lastClickedRef.current = null;
  }, []);

  const getItemStatus = useCallback((item: FolderTreeItem): EntitySyncStatus | undefined => {
    return (item.relPath ? pathStatusMap?.[item.relPath] : undefined) ?? entitySyncStatus?.[item.id];
  }, [pathStatusMap, entitySyncStatus]);

  const toggleExpand = useCallback((id: string) => setExpanded((p) => { const s = new Set(p); s.has(id) ? s.delete(id) : s.add(id); return s; }), []);
  const expandAll = useCallback(() => setExpanded(new Set(["__root__", ...folders.map((f) => f.id)])), [folders]);
  const collapseAll = useCallback(() => setExpanded(new Set(["__root__"])), []);

  const { ctxMenu, closeMenu, openEmptySpaceMenu, openFolderMenu, openItemMenu, openMultiMenu } = useFolderTreeContextMenu({
    expandAll, collapseAll, setNewFolderParent, setExpanded, onOpenNewTab, blocksFolderId,
    onPublishFolder, onToggleFolderItems, setRenaming, setPendingDelete, clearSelection,
    onOpenRunner, onUnblockItem, getItemStatus, onOpenItem, onPublishItem, onRestoreItem,
    onToggleItem, onStartItem, onStopItem, onDuplicateItem, onHistoryItem, onBlockItem,
    selectedItemIds, selectedFolderIds, items, onMoveItems, setShowMove
  });

  useEffect(() => {
    if (!ctxMenu) return;
    const h = () => closeMenu();
    window.addEventListener("click", h);
    return () => window.removeEventListener("click", h);
  }, [ctxMenu, closeMenu]);

  const buildTree = useCallback((): FolderNode => {
    const nodeMap = new Map<string | null, FolderNode>([[null, { folder: null, children: [], items: [] }]]);
    for (const f of folders) nodeMap.set(f.id, { folder: f, children: [], items: [] });
    for (const f of folders) (nodeMap.get(f.parentId ?? null) ?? nodeMap.get(null)!).children.push(nodeMap.get(f.id)!);
    for (const item of items) (nodeMap.get(item.folderId ?? null) ?? nodeMap.get(null)!).items.push(item);
    return nodeMap.get(null)!;
  }, [folders, items]);

  const getVisibleOrder = useCallback((root: FolderNode) => {
    const result: { id: string; kind: "folder" | "item" }[] = [];
    function walk(node: FolderNode) {
      if (node.folder !== null) result.push({ id: node.folder!.id, kind: "folder" });
      if (expanded.has(node.folder === null ? "__root__" : node.folder!.id)) {
        node.children.forEach(walk);
        [...node.items].sort((a, b) => (getItemStatus(a) === "deleted" ? 1 : 0) - (getItemStatus(b) === "deleted" ? 1 : 0)).forEach(item => result.push({ id: item.id, kind: "item" }));
      }
    }
    walk(root); return result;
  }, [expanded, getItemStatus]);

  const handleNewFolder = useCallback(async (name: string, parentId: string | null) => {
    if (onBeforeCreateFolder && !onBeforeCreateFolder()) { setNewFolderParent(undefined); return; }
    await window.api.addFolder(kind, { name, parentId });
    onFoldersChange(); setNewFolderParent(undefined);
    setExpanded((p) => { const s = new Set(p); s.add(parentId === null ? "__root__" : parentId); return s; });
  }, [kind, onBeforeCreateFolder, onFoldersChange]);

  const handleDeleteFolder = useCallback(async (id: string) => {
    onBeforeDeleteFolder?.(id); await window.api.deleteFolder(kind, id); onFoldersChange();
  }, [kind, onBeforeDeleteFolder, onFoldersChange]);

  const doBulkDelete = useCallback(async () => {
    if (!pendingDelete) return;
    const { itemIds, folderIds } = pendingDelete;
    clearSelection(); setPendingDelete(null);
    if (itemIds.length > 0) {
      if (onDeleteItems) {
        const tracked = itemIds.filter(id => { const it = items.find(i => i.id === id); return it?.relPath && pathStatusMap?.[it.relPath] && pathStatusMap[it.relPath] !== "new"; });
        const untracked = itemIds.filter(id => { const it = items.find(i => i.id === id); return !it?.relPath || !pathStatusMap?.[it.relPath] || pathStatusMap[it.relPath] === "new"; });
        await onDeleteItems(tracked, untracked);
      } else { for (const id of itemIds) await onDeleteItem(id); }
    }
    for (const id of folderIds) handleDeleteFolder(id);
  }, [pendingDelete, clearSelection, items, pathStatusMap, onDeleteItems, onDeleteItem, handleDeleteFolder]);

  const handleSelectFolder = useCallback((id: string, multi: boolean, range: boolean) => {
    if (id === "__root__") { clearSelection(); toggleExpand(id); onSelectedFolderChange?.(null); return; }
    if (range && lastClickedRef.current) {
      const order = getVisibleOrder(buildTree());
      const a = order.findIndex((n) => n.id === lastClickedRef.current!.id); const t = order.findIndex((n) => n.id === id);
      if (a !== -1 && t !== -1) {
        const r = order.slice(Math.min(a, t), Math.max(a, t) + 1);
        setSelectedFolderIds(new Set(r.filter(n => n.kind === "folder").map(n => n.id)));
        setSelectedItemIds(new Set(r.filter(n => n.kind === "item").map(n => n.id)));
      }
    } else if (multi) {
      setSelectedFolderIds(p => { const s = new Set(p); s.has(id) ? s.delete(id) : s.add(id); return s; });
      lastClickedRef.current = { id, kind: "folder" };
    } else { clearSelection(); toggleExpand(id); onSelectedFolderChange?.(id); lastClickedRef.current = { id, kind: "folder" }; }
  }, [buildTree, clearSelection, getVisibleOrder, onSelectedFolderChange, toggleExpand]);

  const handleSelectItem = useCallback((id: string, multi: boolean, range: boolean) => {
    if (range && lastClickedRef.current) {
      const order = getVisibleOrder(buildTree());
      const a = order.findIndex((n) => n.id === lastClickedRef.current!.id); const t = order.findIndex((n) => n.id === id);
      if (a !== -1 && t !== -1) {
        const r = order.slice(Math.min(a, t), Math.max(a, t) + 1);
        setSelectedFolderIds(new Set(r.filter(n => n.kind === "folder").map(n => n.id)));
        setSelectedItemIds(new Set(r.filter(n => n.kind === "item").map(n => n.id)));
      }
    } else if (multi) {
      setSelectedItemIds(p => { const s = new Set(p); s.has(id) ? s.delete(id) : s.add(id); return s; });
      lastClickedRef.current = { id, kind: "item" };
    } else { clearSelection(); onOpenItem(id); lastClickedRef.current = { id, kind: "item" }; }
  }, [buildTree, clearSelection, getVisibleOrder, onOpenItem]);

  const contextValue: TreeContextType = useMemo(() => ({
    expanded, selectedItemIds, selectedFolderIds, dragOver, renaming, newFolderParent, hoveredItemId,
    onToggleExpand: toggleExpand, onSelectFolder: handleSelectFolder, onSelectItem: handleSelectItem,
    onFolderContextMenu: (e, id) => {
      e.preventDefault(); e.stopPropagation(); const t = selectedItemIds.size + selectedFolderIds.size; const nk = id === null ? "__root__" : id;
      if (t > 1 && id !== null && selectedFolderIds.has(nk)) openMultiMenu(e.clientX, e.clientY);
      else { setSelectedItemIds(new Set()); setSelectedFolderIds(new Set(id === null ? [] : [nk])); openFolderMenu(e.clientX, e.clientY, id); }
    },
    onItemContextMenu: (e, item) => {
      e.preventDefault(); e.stopPropagation(); if (item.isRunner) return; const t = selectedItemIds.size + selectedFolderIds.size;
      if (t > 1 && selectedItemIds.has(item.id)) openMultiMenu(e.clientX, e.clientY);
      else { setSelectedItemIds(new Set([item.id])); setSelectedFolderIds(new Set()); openItemMenu(e.clientX, e.clientY, item); }
    },
    onDragStartFolder: (e, id) => { e.dataTransfer.setData("text/x-folder-id", id); e.dataTransfer.effectAllowed = "move"; },
    onDragStartItem: (e, id) => { e.dataTransfer.setData("text/x-item-id", id); e.dataTransfer.effectAllowed = "move"; },
    onDragOverNode: (e, id) => { if (!e.dataTransfer.types.includes("text/x-folder-id") && !e.dataTransfer.types.includes("text/x-item-id")) return; e.preventDefault(); e.dataTransfer.dropEffect = "move"; setDragOver(id); },
    onDropNode: (e, id) => {
      e.preventDefault(); setDragOver(null);
      const itemId = e.dataTransfer.getData("text/x-item-id"); if (itemId && onMoveItems) { onMoveItems([itemId], id); return; }
      const folderId = e.dataTransfer.getData("text/x-folder-id"); if (folderId && onMoveFolder && folderId !== id) onMoveFolder(folderId, id);
    },
    onDragLeaveNode: () => setDragOver(null),
    onHoverItem: setHoveredItemId, handleNewFolder, cancelNewFolder: () => setNewFolderParent(undefined), getItemStatus,
    folderStatusMap, onOpenRunner, onOpenItem, setDeletedItemPopup,
    hasMoveItems: !!onMoveItems, hasMoveFolder: !!onMoveFolder
  }), [expanded, selectedItemIds, selectedFolderIds, dragOver, renaming, newFolderParent, hoveredItemId, toggleExpand, handleSelectFolder, handleSelectItem, openMultiMenu, openFolderMenu, openItemMenu, handleNewFolder, getItemStatus, folderStatusMap, onOpenRunner, onOpenItem, onMoveItems, onMoveFolder]);

  return (
    <FolderTreeContext.Provider value={contextValue}>
      <div style={{ display: "flex", flexDirection: "column", flex: 1, width: "100%" }} onContextMenu={(e) => { e.preventDefault(); clearSelection(); openEmptySpaceMenu(e.clientX, e.clientY); }}>
        <div style={{ width: "100%", overflow: "hidden" }}><FolderTreeNode node={buildTree()} depth={0} /></div>
        <div style={{ flex: 1, minHeight: 24 }} onClick={clearSelection} />
        {ctxMenu && <ContextMenu x={ctxMenu.x} y={ctxMenu.y} items={ctxMenu.items} onClose={closeMenu} />}
        {showMove && <MoveDialog folders={folders} onMove={(id) => { onMoveItems?.([...selectedItemIds], id); clearSelection(); setShowMove(false); }} onCancel={() => setShowMove(false)} />}
        <ConfirmDialog open={!!pendingDelete} onConfirm={doBulkDelete} onCancel={() => setPendingDelete(null)} message={pendingDelete ? (() => {
          const what = [(pendingDelete.itemIds.length > 0 && `${pendingDelete.itemIds.length} item(s)`), (pendingDelete.folderIds.length > 0 && `${pendingDelete.folderIds.length} folder(s)`)].filter(Boolean).join(strings.folderTree.and);
          return pendingDelete.hasTracked ? strings.folderTree.deleteConfirmTracked.replace("{what}", what) : strings.folderTree.deleteConfirmUntracked.replace("{what}", what);
        })() : ""} />
        {renaming && (() => { const f = folders.find((f) => f.id === renaming); return f ? <RenameDialog currentName={f.name} onSave={async (name) => { await window.api.renameFolder(kind, f.id, name); onFoldersChange(); setRenaming(null); }} onCancel={() => setRenaming(null)} /> : null; })()}
        {deletedItemPopup && <DeletedItemDialog item={deletedItemPopup} onRestore={onRestoreItem} onPublish={onPublishItem} onClose={() => setDeletedItemPopup(null)} />}
      </div>
    </FolderTreeContext.Provider>
  );
}
