import React, { useState, useMemo, useCallback } from "react";
import { Plus, ChevronsUpDown, ArrowUp, ToggleLeft, ToggleRight, Trash2, Play, Square, ExternalLink, History, Copy, Ban, Pencil } from "@/lib/icons";
import { ContextMenuItem } from "@/components/ui/ContextMenu";
import { strings } from "@/lib/strings";
import { FolderTreeItem, EntitySyncStatus, FolderTreeProps } from "./FolderTree.types";
import { Folder as FolderType } from "@/types";

interface UseContextMenuProps {
  expandAll: () => void;
  collapseAll: () => void;
  setNewFolderParent: (id: string | null) => void;
  setExpanded: React.Dispatch<React.SetStateAction<Set<string>>>;
  onOpenNewTab?: () => void;
  blocksFolderId?: string | null;
  onPublishFolder?: (folderId: string | null) => void;
  onToggleFolderItems?: (folderId: string | null, enable: boolean) => void;
  setRenaming: (id: string) => void;
  setPendingDelete: (v: any) => void;
  clearSelection: () => void;
  onOpenRunner?: (id: string) => void;
  onUnblockItem?: (id: string) => void;
  getItemStatus: (item: FolderTreeItem) => EntitySyncStatus | undefined;
  onOpenItem: (id: string) => void;
  onPublishItem?: (id: string) => void;
  onRestoreItem?: (id: string) => void;
  onToggleItem?: (id: string) => void;
  onStartItem?: (id: string) => void;
  onStopItem?: (id: string) => void;
  onDuplicateItem?: (id: string) => void;
  onHistoryItem?: (id: string) => void;
  onBlockItem?: (id: string) => void;
  selectedItemIds: Set<string>;
  selectedFolderIds: Set<string>;
  items: FolderTreeItem[];
  onMoveItems?: (ids: string[], folderId: string | null) => void;
  setShowMove: (v: boolean) => void;
}

export function useFolderTreeContextMenu({
  expandAll, collapseAll, setNewFolderParent, setExpanded, onOpenNewTab, blocksFolderId,
  onPublishFolder, onToggleFolderItems, setRenaming, setPendingDelete, clearSelection,
  onOpenRunner, onUnblockItem, getItemStatus, onOpenItem, onPublishItem, onRestoreItem,
  onToggleItem, onStartItem, onStopItem, onDuplicateItem, onHistoryItem, onBlockItem,
  selectedItemIds, selectedFolderIds, items, onMoveItems, setShowMove
}: UseContextMenuProps) {
  const [ctxMenu, setCtxMenu] = useState<{ x: number; y: number; items: ContextMenuItem[] } | null>(null);

  const closeMenu = useCallback(() => setCtxMenu(null), []);
  const sep: ContextMenuItem = useMemo(() => ({ sep: true, action: () => { } }), []);
  const expandAllItem: ContextMenuItem = useMemo(() => ({ label: strings.folderTree.expandAll, icon: <ChevronsUpDown size={11} />, action: () => { expandAll(); closeMenu(); } }), [expandAll, closeMenu]);
  const collapseAllItem: ContextMenuItem = useMemo(() => ({ label: strings.folderTree.collapseAll, icon: <ChevronsUpDown size={11} />, action: () => { collapseAll(); closeMenu(); } }), [collapseAll, closeMenu]);

  const openEmptySpaceMenu = useCallback((x: number, y: number) => {
    const menuItems: ContextMenuItem[] = [
      { label: strings.folderTree.newSubfolder, icon: <Plus size={11} />, action: () => { setNewFolderParent(null); setExpanded((p) => { const s = new Set(p); s.add("__root__"); return s; }); closeMenu(); } },
    ];
    if (onOpenNewTab) menuItems.push({ label: strings.folderTree.newTab, icon: <Plus size={11} />, action: () => { onOpenNewTab(); closeMenu(); } });
    menuItems.push(sep, expandAllItem, collapseAllItem);
    setCtxMenu({ x, y, items: menuItems });
  }, [onOpenNewTab, expandAllItem, collapseAllItem, sep, closeMenu, setNewFolderParent, setExpanded]);

  const openFolderMenu = useCallback((x: number, y: number, folderId: string | null) => {
    if (folderId !== null && folderId === blocksFolderId) {
      setCtxMenu({ x, y, items: [expandAllItem, collapseAllItem] });
      return;
    }
    const menuItems: ContextMenuItem[] = [
      { label: strings.folderTree.newSubfolder, icon: <Plus size={11} />, action: () => { setNewFolderParent(folderId); setExpanded((p) => { const s = new Set(p); s.add(folderId === null ? "__root__" : folderId); return s; }); closeMenu(); } },
    ];
    if (folderId !== null && onPublishFolder) menuItems.push({ label: strings.folderTree.commitPushFolder, icon: <ArrowUp size={11} />, action: () => { onPublishFolder(folderId); closeMenu(); } });
    if (folderId !== null && onToggleFolderItems) menuItems.push(
      { label: strings.folderTree.enableAllInFolder, icon: <ToggleRight size={11} />, action: () => { onToggleFolderItems(folderId, true); closeMenu(); } },
      { label: strings.folderTree.disableAllInFolder, icon: <ToggleLeft size={11} />, action: () => { onToggleFolderItems(folderId, false); closeMenu(); } },
    );
    if (folderId !== null) menuItems.push(
      { label: strings.folderTree.renameFolder, icon: <Pencil size={11} />, action: () => { setRenaming(folderId); closeMenu(); } },
      { label: strings.folderTree.deleteFolder, icon: <Trash2 size={11} />, danger: true, action: () => { setPendingDelete({ itemIds: [], folderIds: [folderId] }); clearSelection(); closeMenu(); } },
    );
    if (folderId !== null && onOpenRunner) menuItems.push({ label: strings.folderTree.runCollection, icon: <Play size={11} />, action: () => { onOpenRunner(folderId); closeMenu(); } });
    menuItems.push(sep, expandAllItem, collapseAllItem);
    setCtxMenu({ x, y, items: menuItems });
  }, [blocksFolderId, onPublishFolder, onToggleFolderItems, onOpenRunner, expandAllItem, collapseAllItem, sep, closeMenu, clearSelection, setNewFolderParent, setExpanded, setRenaming, setPendingDelete]);

  const openItemMenu = useCallback((x: number, y: number, item: FolderTreeItem) => {
    if (item.isBlock) {
      const blockMenu: ContextMenuItem[] = [];
      if (onUnblockItem) blockMenu.push({ label: strings.folderTree.unblock, icon: <Ban size={11} />, action: () => { onUnblockItem(item.id); closeMenu(); } });
      blockMenu.push(sep, expandAllItem, collapseAllItem);
      setCtxMenu({ x, y, items: blockMenu });
      return;
    }
    const isEnabled = item.isEnabled !== false;
    const syncSt = getItemStatus(item);
    const isDeleted = syncSt === "deleted";
    const isTracked = !syncSt || syncSt === "clean" || syncSt === "modified" || syncSt === "deleted";
    const isNew = syncSt === "new";
    const menuItems: ContextMenuItem[] = isDeleted ? [] : [{ label: strings.folderTree.open, icon: <ExternalLink size={11} />, action: () => { onOpenItem(item.id); closeMenu(); } }];
    if (onPublishItem && syncSt && syncSt !== "clean") menuItems.push({ label: isDeleted ? strings.folderTree.commitDelete : strings.folderTree.commitPush, icon: <ArrowUp size={11} />, action: () => { onPublishItem!(item.id); closeMenu(); } });
    if (onRestoreItem && syncSt && syncSt !== "clean") menuItems.push({ label: isDeleted ? strings.folderTree.restore : strings.folderTree.discardChanges, icon: <History size={11} />, action: () => { onRestoreItem!(item.id); closeMenu(); } });
    if (!isDeleted && onToggleItem) menuItems.push({ label: isEnabled ? strings.folderTree.disable : strings.folderTree.enable, icon: isEnabled ? <ToggleLeft size={11} /> : <ToggleRight size={11} />, action: () => { onToggleItem(item.id); closeMenu(); } });
    if (!isDeleted && onStartItem) menuItems.push({ label: strings.folderTree.start, icon: <Play size={11} />, action: () => { onStartItem(item.id); closeMenu(); } });
    if (!isDeleted && onStopItem) menuItems.push({ label: strings.folderTree.stop, icon: <Square size={11} />, action: () => { onStopItem(item.id); closeMenu(); } });
    if (!isDeleted && onDuplicateItem) menuItems.push({ label: strings.folderTree.duplicate, icon: <Copy size={11} />, action: () => { onDuplicateItem(item.id); closeMenu(); } });
    if (!isDeleted && onHistoryItem && isTracked) menuItems.push({ label: strings.folderTree.history, icon: <History size={11} />, action: () => { onHistoryItem(item.id); closeMenu(); } });
    if (!isDeleted && onBlockItem) menuItems.push({ label: strings.folderTree.block, icon: <Ban size={11} />, action: () => { onBlockItem(item.id); closeMenu(); } });
    if (!isDeleted) menuItems.push(
      { label: strings.folderTree.delete, icon: <Trash2 size={11} />, danger: true, action: () => { setPendingDelete({ itemIds: [item.id], folderIds: [], hasTracked: !isNew }); clearSelection(); closeMenu(); } },
      sep, expandAllItem, collapseAllItem,
    );
    else menuItems.push(sep, expandAllItem, collapseAllItem);
    setCtxMenu({ x, y, items: menuItems });
  }, [getItemStatus, onOpenItem, onPublishItem, onRestoreItem, onToggleItem, onStartItem, onStopItem, onDuplicateItem, onHistoryItem, onBlockItem, onUnblockItem, expandAllItem, collapseAllItem, sep, closeMenu, clearSelection, setPendingDelete]);

  const openMultiMenu = useCallback((x: number, y: number) => {
    const itemIds = [...selectedItemIds];
    const folderIds = [...selectedFolderIds];
    const hasItems = itemIds.length > 0;
    const hasFolders = folderIds.length > 0;
    const ni = itemIds.length;
    const nf = folderIds.length;
    const fmt = (tpl: string, n: number) => tpl.replace(/\{n\}/g, String(n)).replace(/\{s\}/g, n !== 1 ? "s" : "");
    const items_ = (n: number) => fmt(strings.folderTree.nItems, n);
    const folders_ = (n: number) => fmt(strings.folderTree.nFolders, n);

    const menuItems: ContextMenuItem[] = [];
    if (hasItems && !hasFolders) {
      const dirtyItems = onPublishItem ? itemIds.filter((id) => { const it = items.find((i) => i.id === id); const st = it ? getItemStatus(it) : undefined; return st && st !== "clean"; }) : [];
      if (dirtyItems.length > 0 && onPublishItem) menuItems.push({ label: `${strings.folderTree.commitPush} ${items_(dirtyItems.length)}`, icon: <ArrowUp size={11} />, action: () => { dirtyItems.forEach((id) => onPublishItem!(id)); clearSelection(); closeMenu(); } });
      if (onToggleItem) menuItems.push(
        { label: `${strings.folderTree.enable} ${items_(ni)}`, icon: <ToggleRight size={11} />, action: () => { itemIds.forEach(id => { const it = items.find(i => i.id === id); if (it && it.isEnabled === false) onToggleItem(id); }); clearSelection(); closeMenu(); } },
        { label: `${strings.folderTree.disable} ${items_(ni)}`, icon: <ToggleLeft size={11} />, action: () => { itemIds.forEach(id => { const it = items.find(i => i.id === id); if (it && it.isEnabled !== false) onToggleItem(id); }); clearSelection(); closeMenu(); } },
      );
      if (onMoveItems) menuItems.push({ label: `${strings.folderTree.move} ${items_(ni)}…`, action: () => { closeMenu(); setShowMove(true); } });
      menuItems.push({ label: `${strings.folderTree.delete} ${items_(ni)}`, icon: <Trash2 size={11} />, danger: true, action: () => { closeMenu(); setPendingDelete({ itemIds, folderIds: [] }); } });
    } else if (hasFolders && !hasItems) {
      menuItems.push(
        { label: `${strings.folderTree.expand} ${folders_(nf)}`, action: () => { setExpanded((p) => { const s = new Set(p); folderIds.forEach(id => s.add(id)); return s; }); closeMenu(); } },
        { label: `${strings.folderTree.collapse} ${folders_(nf)}`, action: () => { setExpanded((p) => { const s = new Set(p); folderIds.forEach(id => s.delete(id)); return s; }); closeMenu(); } },
        { label: `${strings.folderTree.delete} ${folders_(nf)}`, danger: true, action: () => { closeMenu(); setPendingDelete({ itemIds: [], folderIds }); } },
      );
    } else {
      if (hasItems && onToggleItem) menuItems.push(
        { label: `${strings.folderTree.enable} ${items_(ni)}`, action: () => { itemIds.forEach(id => { const it = items.find(i => i.id === id); if (it && it.isEnabled === false) onToggleItem(id); }); clearSelection(); closeMenu(); } },
        { label: `${strings.folderTree.disable} ${items_(ni)}`, action: () => { itemIds.forEach(id => { const it = items.find(i => i.id === id); if (it && it.isEnabled !== false) onToggleItem(id); }); clearSelection(); closeMenu(); } },
      );
      menuItems.push({ label: `${strings.folderTree.delete} ${fmt(strings.folderTree.nSelected, ni + nf)}`, danger: true, action: () => { closeMenu(); setPendingDelete({ itemIds, folderIds }); } });
    }
    menuItems.push(sep, expandAllItem, collapseAllItem);
    setCtxMenu({ x, y, items: menuItems });
  }, [selectedItemIds, selectedFolderIds, items, getItemStatus, onPublishItem, onToggleItem, onMoveItems, expandAllItem, collapseAllItem, sep, closeMenu, clearSelection, setExpanded, setPendingDelete, setShowMove]);

  return { ctxMenu, setCtxMenu, closeMenu, openEmptySpaceMenu, openFolderMenu, openItemMenu, openMultiMenu };
}
