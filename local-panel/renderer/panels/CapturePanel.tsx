import React, { useEffect, useRef, useState, useCallback, useMemo } from "react";
import { Group as PanelGroup, Panel, Separator as ResizeHandle } from "react-resizable-panels";
import { RequestLogEntry, MockRule, SavedRequest, AppConfig } from "@/types";
import PanelHeader from "@/components/layout/PanelHeader";
import CaptureTable from "@/components/capture/CaptureTable";
import CaptureDetail from "@/components/capture/CaptureDetail";
import CaptureTypeTabs, { TypeFilter } from "@/components/capture/CaptureTypeTabs";
import {
  CaptureType, deriveType, fulfilledBy, buildMockInitial, reqToHeadersBody,
} from "@/components/capture/captureUtils";
import {
  blockKey, blockedKeySet, findBlocksFolder, ensureBlocksFolderId, buildBlockMock,
} from "@/lib/blocks";
import { strings } from "@/lib/strings";
import {
  Play, Pause, Clipboard, Zap, Download, Share2, Ban, Trash2, ArrowUpRight,
} from "@/lib/icons";
import { Button, ContextMenu, ContextMenuItem } from "@/components/ui";
import { useCaptureActions } from "@/components/capture/useCaptureActions";
import { CaptureToolbar } from "@/components/capture/CaptureToolbar";
import { useCaptureContextMenu } from "@/components/capture/useCaptureContextMenu";

export interface CaptureStats {
  total: number;
  shown: number;
  paused: boolean;
}

const MAX_ENTRIES = 200;
const storageKey = (wsId: string) => `capture:entries:${wsId}`;

function loadPersistedEntries(wsId: string): RequestLogEntry[] {
  try {
    const raw = localStorage.getItem(storageKey(wsId));
    if (!raw) return [];
    return JSON.parse(raw) as RequestLogEntry[];
  } catch {
    return [];
  }
}

function persistEntries(wsId: string, entries: RequestLogEntry[]) {
  try {
    localStorage.setItem(storageKey(wsId), JSON.stringify(entries));
  } catch { /* quota */ }
}

interface Props {
  activeWorkspaceId: string;
  wsConfig: AppConfig;
  onConfigChange: (next: AppConfig) => Promise<void>;
  onOpenInMocks: (initial: Partial<MockRule>) => void;
  onOpenInRequests: (req: Omit<SavedRequest, "id" | "createdAt" | "workspaceId">) => void;
  onStatsChange?: (stats: CaptureStats) => void;
}

interface CtxMenuState {
  x: number;
  y: number;
  multi: boolean;
  entry: RequestLogEntry;
}

export default function CapturePanel({ activeWorkspaceId, wsConfig, onConfigChange, onOpenInMocks, onOpenInRequests, onStatsChange }: Props) {
  const [entries, setEntries] = useState<RequestLogEntry[]>(() => loadPersistedEntries(activeWorkspaceId));
  const [typeFilter, setTypeFilter] = useState<TypeFilter>("all");
  const [paused, setPaused] = useState(false);
  const pausedRef = useRef(false);
  pausedRef.current = paused;

  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const [anchorId, setAnchorId] = useState<string | null>(null);
  const [activeId, setActiveId] = useState<string | null>(null);
  const [ctxMenu, setCtxMenu] = useState<CtxMenuState | null>(null);

  useEffect(() => {
    setEntries(loadPersistedEntries(activeWorkspaceId));
    setSelectedIds(new Set());
    setAnchorId(null);
    setActiveId(null);
  }, [activeWorkspaceId]);

  useEffect(() => {
    persistEntries(activeWorkspaceId, entries);
  }, [activeWorkspaceId, entries]);

  useEffect(() => {
    const unsub = window.api.onLogEntry((entry) => {
      if (pausedRef.current) return;
      setEntries((prev) => {
        const next = [entry, ...prev];
        return next.length > MAX_ENTRIES ? next.slice(0, MAX_ENTRIES) : next;
      });
    });
    return unsub;
  }, []);

  // Streaming chunk accumulation
  useEffect(() => {
    const unsub = window.api.onLogChunk((chunk) => {
      if (pausedRef.current) return;
      if (chunk.done) return; // Final log:entry will have the full body
      setEntries((prev) => prev.map((e) => {
        if (e.id !== chunk.logId) return e;
        return { ...e, resBody: e.resBody + chunk.chunk };
      }));
    });
    return unsub;
  }, []);

  // Prune stale selection / active id when entries shrink
  useEffect(() => {
    const ids = new Set(entries.map((e) => e.id));
    setSelectedIds((prev) => {
      const next = new Set([...prev].filter((id) => ids.has(id)));
      return next.size === prev.size ? prev : next;
    });
    setActiveId((prev) => (prev && !ids.has(prev) ? null : prev));
  }, [entries]);

  const clear = useCallback(() => {
    setEntries([]);
    setSelectedIds(new Set());
    setAnchorId(null);
    setActiveId(null);
    localStorage.removeItem(storageKey(activeWorkspaceId));
  }, [activeWorkspaceId]);

  const removeEntries = useCallback((ids: Set<string>) => {
    setEntries((prev) => prev.filter((e) => !ids.has(e.id)));
  }, []);

  const visible = useMemo(
    () => entries.filter((e) => {
      if (typeFilter !== "all" && deriveType(e) !== typeFilter) return false;
      return true;
    }),
    [entries, typeFilter],
  );

  const typeCounts = useMemo(() => {
    const counts = { all: entries.length } as Record<TypeFilter, number>;
    for (const e of entries) {
      const t = deriveType(e) as CaptureType;
      counts[t] = (counts[t] ?? 0) + 1;
    }
    return counts;
  }, [entries]);

  const activeEntry = activeId ? entries.find((e) => e.id === activeId) ?? null : null;

  // -- Selection handlers ------------------------------------------------------
  const handleRowClick = useCallback((entry: RequestLogEntry, ev: React.MouseEvent) => {
    if (ev.shiftKey && anchorId) {
      const from = visible.findIndex((e) => e.id === anchorId);
      const to = visible.findIndex((e) => e.id === entry.id);
      if (from !== -1 && to !== -1) {
        const [lo, hi] = from < to ? [from, to] : [to, from];
        setSelectedIds(new Set(visible.slice(lo, hi + 1).map((e) => e.id)));
        return;
      }
    }
    if (ev.ctrlKey || ev.metaKey) {
      setSelectedIds((prev) => {
        const next = new Set(prev);
        next.has(entry.id) ? next.delete(entry.id) : next.add(entry.id);
        return next;
      });
      setAnchorId(entry.id);
      return;
    }
    setActiveId(entry.id);
    setAnchorId(entry.id);
  }, [anchorId, visible]);

  const handleToggleCheck = useCallback((id: string) => {
    setSelectedIds((prev) => {
      const next = new Set(prev);
      next.has(id) ? next.delete(id) : next.add(id);
      return next;
    });
    setAnchorId(id);
  }, []);

  const handleToggleAll = useCallback(() => {
    setSelectedIds((prev) => (
      visible.length > 0 && visible.every((e) => prev.has(e.id))
        ? new Set()
        : new Set(visible.map((e) => e.id))
    ));
  }, [visible]);

  const reloadConfig = useCallback(async () => {
    const fresh = await window.api.getConfig();
    await onConfigChange(fresh);
  }, [onConfigChange]);

  const {
    blockedKeys,
    mockEntries,
    saveEntries,
    blockEntries,
    unblockEntries,
    shareEntries,
  } = useCaptureActions({ wsConfig, reloadConfig });

  // -- Context menu -------------------------------------------------------------
  const handleContextMenu = useCallback((entry: RequestLogEntry, ev: React.MouseEvent) => {
    ev.preventDefault();
    const multi = selectedIds.size > 1 && selectedIds.has(entry.id);
    setCtxMenu({ x: ev.clientX, y: ev.clientY, multi, entry });
  }, [selectedIds]);

  const ctxItems = useCaptureContextMenu({
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
  });

  // Notify parent of current stats so the global footer can display them
  useEffect(() => {
    onStatsChange?.({ total: entries.length, shown: visible.length, paused });
  }, [entries.length, visible.length, paused, onStatsChange]);

  const list = (
    <div className="flex-1 overflow-y-auto font-mono text-xs">
      {visible.length === 0 ? (
        <div className="flex flex-col items-center justify-center h-full text-center py-16">
          <div className="opacity-15 mb-3"><Clipboard size={36} /></div>
          <div className="text-sm font-medium text-foreground font-sans mb-1">{strings.capture.emptyTitle}</div>
          <p className="text-xs text-muted-foreground font-sans">
            {entries.length === 0
              ? paused ? strings.capture.emptyPausedHint : strings.capture.emptyLiveHint
              : strings.capture.emptyNoMatch}
          </p>
        </div>
      ) : (
        <CaptureTable
          entries={visible}
          selectedIds={selectedIds}
          activeId={activeId}
          blockedKeys={blockedKeys}
          onRowClick={handleRowClick}
          onToggleCheck={handleToggleCheck}
          onToggleAll={handleToggleAll}
          onContextMenu={handleContextMenu}
        />
      )}
    </div>
  );

  return (
    <div className="flex flex-col flex-1 overflow-hidden">
      <CaptureToolbar
        selectedCount={selectedIds.size}
        paused={paused}
        onTogglePause={() => setPaused((v) => !v)}
        onClear={clear}
        hasEntries={entries.length > 0}
        onMockAll={() => mockEntries(visible, true)}
        onSaveAll={() => saveEntries(visible, true)}
        maxEntries={MAX_ENTRIES}
      />

      <CaptureTypeTabs active={typeFilter} counts={typeCounts} onChange={setTypeFilter} />

      {activeEntry ? (
        <PanelGroup orientation="horizontal" className="flex flex-1 min-h-0 overflow-hidden">
          <Panel defaultSize={60} minSize={30} className="flex flex-col overflow-hidden">
            {list}
          </Panel>
          <ResizeHandle className="w-1 bg-border hover:bg-signal/40 active:bg-signal/60 transition-colors cursor-col-resize flex-shrink-0" />
          <Panel defaultSize={40} minSize={25} className="flex flex-col overflow-hidden">
            <CaptureDetail key={activeEntry.id} entry={activeEntry} onClose={() => setActiveId(null)} />
          </Panel>
        </PanelGroup>
      ) : (
        list
      )}

      {ctxMenu && (
        <ContextMenu x={ctxMenu.x} y={ctxMenu.y} items={ctxItems} onClose={() => setCtxMenu(null)} />
      )}
    </div>
  );
}
