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

  // -- Bulk actions (also used by toolbar Mock All / Save All) ------------------
  const mockEntries = useCallback(async (list: RequestLogEntry[], inFolder: boolean) => {
    if (list.length === 0) return;
    let folderId: string | null = null;
    if (inFolder) {
      const folderName = `Capture ${new Date().toLocaleString().replace(/[/:]/g, "-")}`;
      const folder = await window.api.addFolder("mock", { name: folderName, parentId: null });
      folderId = folder.id;
    }
    for (const entry of list) {
      const mock = buildMockInitial(entry);
      await window.api.addMock({
        name: `${entry.method} ${entry.url}`,
        method: mock.method ?? "GET",
        urlPattern: mock.urlPattern ?? entry.url,
        useRegex: mock.useRegex ?? false,
        responseStatus: mock.responseStatus ?? 200,
        responseHeaders: mock.responseHeaders ?? {},
        responseBody: mock.responseBody ?? "",
        responseBodyEncoding: mock.responseBodyEncoding,
        enabled: true,
        folderId,
      } as any);
    }
    window.api.getConfig();
  }, []);

  const saveEntries = useCallback(async (list: RequestLogEntry[], inFolder: boolean) => {
    if (list.length === 0) return;
    let folderId: string | null = null;
    if (inFolder) {
      const folderName = `Capture ${new Date().toLocaleString().replace(/[/:]/g, "-")}`;
      const folder = await window.api.addFolder("request", { name: folderName, parentId: null });
      folderId = folder.id;
    }
    for (const entry of list) {
      await window.api.addRequest({ ...reqToHeadersBody(entry), name: `${entry.method} ${entry.url}`, folderId } as any);
    }
    window.api.getConfig();
  }, []);

  const blockedKeys = useMemo(() => blockedKeySet(wsConfig), [wsConfig.mocks, wsConfig.mockFolders]);

  const reloadConfig = useCallback(async () => {
    const fresh = await window.api.getConfig();
    await onConfigChange(fresh);
  }, [onConfigChange]);

  const blockEntries = useCallback(async (list: RequestLogEntry[]) => {
    if (list.length === 0) return;
    const folderId = await ensureBlocksFolderId(wsConfig.mockFolders ?? []);
    const existing = blockedKeySet(wsConfig);
    for (const entry of list) {
      if (existing.has(blockKey(entry.method, entry.url))) continue;
      await window.api.addMock(buildBlockMock(entry.method, entry.url, folderId) as any);
    }
    await reloadConfig();
  }, [wsConfig, reloadConfig]);

  const unblockEntries = useCallback(async (list: RequestLogEntry[]) => {
    const blocks = findBlocksFolder(wsConfig.mockFolders ?? []);
    if (!blocks) return;
    const keys = new Set(list.map((e) => blockKey(e.method, e.url)));
    const toDelete = (wsConfig.mocks ?? []).filter(
      (m) => m.folderId === blocks.id && keys.has(blockKey(m.method, m.urlPattern)),
    );
    for (const m of toDelete) await window.api.deleteMock(m.id);
    await reloadConfig();
  }, [wsConfig, reloadConfig]);

  const shareEntries = useCallback((list: RequestLogEntry[]) => {
    if (list.length === 0) return;
    const name = list.length === 1
      ? `capture-${list[0].method}-${list[0].id}.json`
      : `captured-requests-${list.length}.json`;
    window.api.shareCaptureJson(list, name);
  }, []);

  // -- Context menu -------------------------------------------------------------
  const handleContextMenu = useCallback((entry: RequestLogEntry, ev: React.MouseEvent) => {
    ev.preventDefault();
    const multi = selectedIds.size > 1 && selectedIds.has(entry.id);
    setCtxMenu({ x: ev.clientX, y: ev.clientY, multi, entry });
  }, [selectedIds]);

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
  }, [ctxMenu, entries, selectedIds, blockedKeys, mockEntries, saveEntries, blockEntries, unblockEntries, shareEntries, removeEntries, onOpenInMocks, onOpenInRequests]);

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
      <PanelHeader
        title={strings.capture.title}
        subtitle={strings.capture.subtitle.replace("{count}", String(MAX_ENTRIES))}
        actions={
          <>
            {selectedIds.size > 0 && (
              <span className="text-xs text-muted-foreground whitespace-nowrap">
                {strings.capture.selectedCount.replace("{count}", String(selectedIds.size))}
              </span>
            )}
            <button
              onClick={() => setPaused((v) => !v)}
              className={`flex items-center gap-1.5 px-3 py-1.5 rounded border text-xs font-medium transition-all cursor-pointer whitespace-nowrap ${paused
                ? "border-amber bg-amber/10 text-amber"
                : "border-signal/40 bg-signal/10 text-signal hover:bg-signal/20"
                }`}
            >
              {paused ? <><Play size={10} fill="currentColor" /> {strings.capture.start}</> : <><Pause size={10} fill="currentColor" /> {strings.capture.pause}</>}
            </button>
            <Button variant="secondary" onClick={clear}>{strings.capture.clear}</Button>
            {entries.length > 0 && (
              <>
                <Button variant="secondary" onClick={() => mockEntries(visible, true)} icon={<Zap size={10} />}>{strings.capture.mockAll}</Button>
                <Button variant="secondary" onClick={() => saveEntries(visible, true)} icon={<Download size={10} />}>{strings.capture.saveAll}</Button>
              </>
            )}
          </>
        }
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
