import React, { useMemo } from "react";
import { Zap, Download, Share2, Ban, Trash2, ArrowUpRight } from "@/lib/icons";
import { strings } from "@/lib/strings";
import { RequestLogEntry, MockRule, SavedRequest } from "@/types";
import { ContextMenuItem } from "@/components/ui";
import { buildMockInitial, reqToHeadersBody } from "./captureUtils";
import { blockKey } from "@/lib/blocks";

interface CtxMenuState {
  x: number;
  y: number;
  multi: boolean;
  entry: RequestLogEntry;
}

interface UseCaptureContextMenuProps {
  ctxMenu: CtxMenuState | null;
  setCtxMenu: (ctx: CtxMenuState | null) => void;
  entries: RequestLogEntry[];
  selectedIds: Set<string>;
  blockedKeys: Set<string>;
  mockEntries: (list: RequestLogEntry[], inFolder: boolean) => void;
  saveEntries: (list: RequestLogEntry[], inFolder: boolean) => void;
  blockEntries: (list: RequestLogEntry[]) => void;
  unblockEntries: (list: RequestLogEntry[]) => void;
  shareEntries: (list: RequestLogEntry[]) => void;
  removeEntries: (ids: Set<string>) => void;
  setSelectedIds: (ids: Set<string>) => void;
  onOpenInMocks: (initial: Partial<MockRule>) => void;
  onOpenInRequests: (req: Omit<SavedRequest, "id" | "createdAt" | "workspaceId">) => void;
}

export function useCaptureContextMenu({
  ctxMenu,
  setCtxMenu,
  entries,
  selectedIds,
  blockedKeys,
  mockEntries,
  saveEntries,
  blockEntries,
  unblockEntries,
  shareEntries,
  removeEntries,
  setSelectedIds,
  onOpenInMocks,
  onOpenInRequests,
}: UseCaptureContextMenuProps) {
  const ctxItems = useMemo<ContextMenuItem[]>(() => {
    if (!ctxMenu) return [];
    const close = () => setCtxMenu(null);
    const run = (fn: () => void) => () => { fn(); close(); };
    if (ctxMenu.multi) {
      const selected = entries.filter((e) => selectedIds.has(e.id));
      const allBlocked = selected.length > 0 && selected.every((e) => blockedKeys.has(blockKey(e.method, e.url)));
      return [
        { label: strings.capture.ctxMockMany, icon: <Zap size={14} />, action: run(() => mockEntries(selected, true)) },
        { label: strings.capture.ctxSaveMany, icon: <Download size={14} />, action: run(() => saveEntries(selected, true)) },
        allBlocked
          ? { label: strings.capture.ctxUnblockMany, icon: <Ban size={14} />, action: run(() => unblockEntries(selected)) }
          : { label: strings.capture.ctxBlockMany, icon: <Ban size={14} />, action: run(() => blockEntries(selected)) },
        { label: strings.capture.ctxShareMany, icon: <Share2 size={14} />, action: run(() => shareEntries(selected)) },
        { sep: true },
        { label: strings.capture.ctxDeleteMany, icon: <Trash2 size={14} />, danger: true, action: run(() => { removeEntries(selectedIds); setSelectedIds(new Set()); }) },
      ];
    }
    const e = ctxMenu.entry;
    const isBlocked = blockedKeys.has(blockKey(e.method, e.url));
    return [
      { label: strings.capture.ctxMock, icon: <Zap size={14} />, action: run(() => onOpenInMocks(buildMockInitial(e))) },
      { label: strings.capture.ctxOpen, icon: <ArrowUpRight size={14} />, action: run(() => onOpenInRequests(reqToHeadersBody(e))) },
      { label: strings.capture.ctxSave, icon: <Download size={14} />, action: run(() => saveEntries([e], false)) },
      { sep: true },
      isBlocked
        ? { label: strings.capture.ctxUnblock, icon: <Ban size={14} />, action: run(() => unblockEntries([e])) }
        : { label: strings.capture.ctxBlock, icon: <Ban size={14} />, action: run(() => blockEntries([e])) },
      { label: strings.capture.ctxShare, icon: <Share2 size={14} />, action: run(() => shareEntries([e])) },
      { sep: true },
      { label: strings.capture.ctxDelete, icon: <Trash2 size={14} />, danger: true, action: run(() => removeEntries(new Set([e.id]))) },
    ];
  }, [ctxMenu, entries, selectedIds, blockedKeys, mockEntries, saveEntries, blockEntries, unblockEntries, shareEntries, removeEntries, setSelectedIds, onOpenInMocks, onOpenInRequests, setCtxMenu]);

  return ctxItems;
}
