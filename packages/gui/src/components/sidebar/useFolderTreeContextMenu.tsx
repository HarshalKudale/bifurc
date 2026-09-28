import React, { useState, useMemo, useCallback } from "react";
import { Plus, ChevronsUpDown, ArrowUp, ToggleLeft, ToggleRight, Trash2, Play, Square, ExternalLink, History, Copy, Ban, Pencil, RotateCcw } from "@/lib/icons";
import { ContextMenuItem } from "@/components/ui/ContextMenu";
import { strings } from "@/lib/strings";
import { FolderTreeItem } from "./FolderTree.types";
import type { EntityGitView, FolderGitRollup } from "@/lib/entityGit";

/**
 * The tree's context menus.
 *
 * ## Every git affordance here comes from `EntityGitView`
 *
 * This file used to derive its own conditions from the raw `EntitySyncStatus`, and so did the other two
 * menus in it — with different answers. The item menu showed History for an *unknown* status; the multi
 * menu offered no Revert at all; the folder menu offered "Commit & Push Folder" unconditionally, so a
 * clean folder still had the item. The words were a third vocabulary ("Commit & Push",
 * "Discard Changes", "Restore") on top of the editors' ("Sync", "Revert") and the mapping cards'
 * ("Publish", "Revert").
 *
 * Now the menus ask `entityGitView()` / `folderGitRollup()` and render the answer, so the *decision*
 * lives in one place and only the *layout* lives here.
 */
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
  /** The uniform git view for one item. */
  getItemView: (item: FolderTreeItem) => EntityGitView;
  /** The git rollup for a folder — or the whole kind when the id is `null`. */
  getFolderRollup: (folderId: string | null) => FolderGitRollup;
  onOpenItem: (id: string) => void;
  onPublishItem?: (id: string) => void;
  /** Publish a selection in one commit. */
  onPublishIds?: (ids: string[]) => void;
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
  onOpenRunner, onUnblockItem, getItemView, getFolderRollup, onOpenItem, onPublishItem,
  onPublishIds, onRestoreItem, onToggleItem, onStartItem, onStopItem, onDuplicateItem,
  onHistoryItem, onBlockItem, selectedItemIds, selectedFolderIds, items, onMoveItems, setShowMove
}: UseContextMenuProps) {
  const [ctxMenu, setCtxMenu] = useState<{ x: number; y: number; items: ContextMenuItem[] } | null>(null);

  const closeMenu = useCallback(() => setCtxMenu(null), []);
  const sep: ContextMenuItem = useMemo(() => ({ sep: true, action: () => { } }), []);
  const expandAllItem: ContextMenuItem = useMemo(() => ({ label: strings.folderTree.expandAll, icon: <ChevronsUpDown size={11} />, action: () => { expandAll(); closeMenu(); } }), [expandAll, closeMenu]);
  const collapseAllItem: ContextMenuItem = useMemo(() => ({ label: strings.folderTree.collapseAll, icon: <ChevronsUpDown size={11} />, action: () => { collapseAll(); closeMenu(); } }), [collapseAll, closeMenu]);

  /**
   * The two git write items for one entity, from its view.
   *
   * `primary` is Publish for an untracked entity and Sync for anything else outstanding — including a
   * *clean* entity whose commits are not on the remote yet, which is the case that had no item at all
   * before (`syncSt !== "clean"` was the gate). Revert appears beside it only when there is a diff
   * against HEAD to throw away.
   */
  const gitItemsFor = useCallback((view: EntityGitView, id: string): ContextMenuItem[] => {
    const out: ContextMenuItem[] = [];
    if (view.status === "deleted") {
      if (onRestoreItem) out.push({ label: strings.entityGit.recover, icon: <RotateCcw size={11} />, action: () => { onRestoreItem(id); closeMenu(); } });
      if (onPublishItem) out.push({ label: strings.entityGit.syncDelete, icon: <ArrowUp size={11} />, action: () => { onPublishItem(id); closeMenu(); } });
      return out;
    }
    if (onPublishItem && view.primary) {
      out.push({
        label: view.primaryLabel,
        icon: <ArrowUp size={11} />,
        action: () => { onPublishItem(id); closeMenu(); },
      });
    }
    if (onRestoreItem && view.canRevert) {
      out.push({ label: strings.entityGit.revert, icon: <RotateCcw size={11} />, action: () => { onRestoreItem(id); closeMenu(); } });
    }
    return out;
  }, [onPublishItem, onRestoreItem, closeMenu]);

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

    // ── Publish All / Sync All ────────────────────────────────────────────────
    //
    // Gated on the folder's rollup, and offered for the **root** as well. The root row is the "whole
    // kind" case the spec calls folder details, and the old menu excluded it entirely
    // (`folderId !== null && onPublishFolder`), so the only way to publish everything at the top level
    // was the footer button. A clean folder no longer offers the item at all, because there is nothing
    // for it to do — `folderGitRollup().primary` is `null` exactly then.
    const rollup = getFolderRollup(folderId);
    if (onPublishFolder && rollup.primary) {
      menuItems.push({
        label: rollup.primary === "publish" ? strings.entityGit.publishAll : strings.entityGit.syncAll,
        icon: <ArrowUp size={11} />,
        action: () => { onPublishFolder(folderId); closeMenu(); },
      });
    }

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
  }, [blocksFolderId, onPublishFolder, onToggleFolderItems, onOpenRunner, expandAllItem, collapseAllItem, sep, closeMenu, clearSelection, setNewFolderParent, setExpanded, setRenaming, setPendingDelete, getFolderRollup]);

  const openItemMenu = useCallback((x: number, y: number, item: FolderTreeItem) => {
    if (item.isBlock) {
      const blockMenu: ContextMenuItem[] = [];
      if (onUnblockItem) blockMenu.push({ label: strings.folderTree.unblock, icon: <Ban size={11} />, action: () => { onUnblockItem(item.id); closeMenu(); } });
      blockMenu.push(sep, expandAllItem, collapseAllItem);
      setCtxMenu({ x, y, items: blockMenu });
      return;
    }
    const view = getItemView(item);
    const isDeleted = view.status === "deleted";
    const isEnabled = item.isEnabled !== false;

    const menuItems: ContextMenuItem[] = isDeleted
      ? []
      : [{ label: strings.folderTree.open, icon: <ExternalLink size={11} />, action: () => { onOpenItem(item.id); closeMenu(); } }];

    // The git pair, before the entity-level toggles — one vocabulary, from one place.
    menuItems.push(...gitItemsFor(view, item.id));

    if (!isDeleted && onToggleItem) menuItems.push({ label: isEnabled ? strings.folderTree.disable : strings.folderTree.enable, icon: isEnabled ? <ToggleLeft size={11} /> : <ToggleRight size={11} />, action: () => { onToggleItem(item.id); closeMenu(); } });
    if (!isDeleted && onStartItem) menuItems.push({ label: strings.folderTree.start, icon: <Play size={11} />, action: () => { onStartItem(item.id); closeMenu(); } });
    if (!isDeleted && onStopItem) menuItems.push({ label: strings.folderTree.stop, icon: <Square size={11} />, action: () => { onStopItem(item.id); closeMenu(); } });
    if (!isDeleted && onDuplicateItem) menuItems.push({ label: strings.folderTree.duplicate, icon: <Copy size={11} />, action: () => { onDuplicateItem(item.id); closeMenu(); } });
    // History is gated on the entity having commits, from the same view every other surface uses.
    if (!isDeleted && onHistoryItem && view.canViewHistory) menuItems.push({ label: strings.entityGit.history, icon: <History size={11} />, action: () => { onHistoryItem(item.id); closeMenu(); } });
    if (!isDeleted && onBlockItem) menuItems.push({ label: strings.folderTree.block, icon: <Ban size={11} />, action: () => { onBlockItem(item.id); closeMenu(); } });
    if (!isDeleted) menuItems.push(
      { label: strings.folderTree.delete, icon: <Trash2 size={11} />, danger: true, action: () => { setPendingDelete({ itemIds: [item.id], folderIds: [], recoverable: view.deleteIsRecoverable }); clearSelection(); closeMenu(); } },
      sep, expandAllItem, collapseAllItem,
    );
    else menuItems.push(sep, expandAllItem, collapseAllItem);
    setCtxMenu({ x, y, items: menuItems });
  }, [getItemView, gitItemsFor, onOpenItem, onToggleItem, onStartItem, onStopItem, onDuplicateItem, onHistoryItem, onBlockItem, onUnblockItem, expandAllItem, collapseAllItem, sep, closeMenu, clearSelection, setPendingDelete]);

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
      const views = itemIds
        .map((id) => { const it = items.find((i) => i.id === id); return it ? { id, view: getItemView(it) } : null; })
        .filter((v): v is { id: string; view: EntityGitView } => v !== null);
      const publishable = views.filter((v) => v.view.primary);
      const revertable = views.filter((v) => v.view.canRevert);

      // One commit for the whole selection — `onPublishIds` resolves every item's path and hands the
      // array to `entity.publish`. Firing `onPublishItem` per id produced one commit per entity.
      if (publishable.length > 0) {
        const label = publishable.every((v) => v.view.primary === "publish")
          ? `${strings.entityGit.publish} ${items_(publishable.length)}`
          : `${strings.entityGit.sync} ${items_(publishable.length)}`;
        menuItems.push({
          label,
          icon: <ArrowUp size={11} />,
          action: () => {
            const ids = publishable.map((v) => v.id);
            if (onPublishIds) onPublishIds(ids);
            else ids.forEach((id) => onPublishItem?.(id));
            clearSelection(); closeMenu();
          },
        });
      }
      // Revert for the selection. The multi menu had **no** revert item at all, so a bulk edit could
      // only be undone one entity at a time from the item menu.
      if (revertable.length > 0 && onRestoreItem) {
        menuItems.push({
          label: `${strings.entityGit.revert} ${items_(revertable.length)}`,
          icon: <RotateCcw size={11} />,
          action: () => { revertable.forEach((v) => onRestoreItem(v.id)); clearSelection(); closeMenu(); },
        });
      }
      if (onToggleItem) menuItems.push(
        { label: `${strings.folderTree.enable} ${items_(ni)}`, icon: <ToggleRight size={11} />, action: () => { itemIds.forEach(id => { const it = items.find(i => i.id === id); if (it && it.isEnabled === false) onToggleItem(id); }); clearSelection(); closeMenu(); } },
        { label: `${strings.folderTree.disable} ${items_(ni)}`, icon: <ToggleLeft size={11} />, action: () => { itemIds.forEach(id => { const it = items.find(i => i.id === id); if (it && it.isEnabled !== false) onToggleItem(id); }); clearSelection(); closeMenu(); } },
      );
      if (onMoveItems) menuItems.push({ label: `${strings.folderTree.move} ${items_(ni)}…`, action: () => { closeMenu(); setShowMove(true); } });
      menuItems.push({ label: `${strings.folderTree.delete} ${items_(ni)}`, icon: <Trash2 size={11} />, danger: true, action: () => { closeMenu(); setPendingDelete({ itemIds, folderIds: [], recoverable: views.some((v) => v.view.deleteIsRecoverable) }); } });
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
  }, [selectedItemIds, selectedFolderIds, items, getItemView, onPublishItem, onPublishIds, onRestoreItem, onToggleItem, onMoveItems, expandAllItem, collapseAllItem, sep, closeMenu, clearSelection, setExpanded, setPendingDelete, setShowMove]);

  return { ctxMenu, setCtxMenu, closeMenu, openEmptySpaceMenu, openFolderMenu, openItemMenu, openMultiMenu };
}
