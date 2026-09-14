import { useState, useCallback, useRef, useEffect } from "react";
import { SavedRequest, Environment } from "@/types";
import { runCollection, CollectionRunReport, RunnerRequestResult } from "@/lib/collectionRunner";

interface UseCollectionRunnerProps {
  wsId: string;
  folderId: string;
  folderName: string;
  requests: SavedRequest[];
  activeEnv: Environment | null;
  onSaveReport?: (report: CollectionRunReport) => Promise<void>;
}

export function useCollectionRunner({
  wsId,
  folderId,
  folderName,
  requests,
  activeEnv,
  onSaveReport,
}: UseCollectionRunnerProps) {
  const [orderedRequests, setOrderedRequests] = useState<SavedRequest[]>(requests);
  const [delayMs, setDelayMs] = useState(0);
  const [running, setRunning] = useState(false);
  const [progress, setProgress] = useState(0);
  const [results, setResults] = useState<RunnerRequestResult[]>([]);
  const [report, setReport] = useState<CollectionRunReport | null>(null);
  const [configLoaded, setConfigLoaded] = useState(false);
  
  const cancelledRef = useRef(false);
  const dragIndexRef = useRef<number | null>(null);
  const dragOverRef = useRef<number | null>(null);

  useEffect(() => {
    (async () => {
      try {
        const cfg = await window.api.loadRunnerConfig(wsId, folderId);
        if (cfg) {
          setDelayMs(cfg.delayMs ?? 0);
          if (cfg.requestOrder?.length) {
            const orderMap = new Map(cfg.requestOrder.map((id, idx) => [id, idx]));
            const sorted = [...requests].sort((a, b) => {
              const ai = orderMap.get(a.id) ?? 9999;
              const bi = orderMap.get(b.id) ?? 9999;
              return ai - bi;
            });
            setOrderedRequests(sorted);
          }
        }
      } catch { /* ignore */ }
      setConfigLoaded(true);
    })();
  }, [wsId, folderId]);

  useEffect(() => {
    if (!configLoaded) return;
    const currentIds = new Set(orderedRequests.map((r) => r.id));
    const newReqs = requests.filter((r) => !currentIds.has(r.id));
    const validOrdered = orderedRequests.filter((r) => requests.some((x) => x.id === r.id));
    if (newReqs.length > 0 || validOrdered.length !== orderedRequests.length) {
      setOrderedRequests([...validOrdered, ...newReqs]);
    }
  }, [requests, configLoaded, orderedRequests]);

  const saveConfig = useCallback(async (reqs: SavedRequest[], delay: number) => {
    try {
      await window.api.saveRunnerConfig(wsId, folderId, {
        requestOrder: reqs.map((r) => r.id),
        delayMs: delay,
      });
    } catch { /* ignore */ }
  }, [wsId, folderId]);

  const handleReorder = useCallback((fromIdx: number, toIdx: number) => {
    setOrderedRequests((prev) => {
      const next = [...prev];
      const [item] = next.splice(fromIdx, 1);
      next.splice(toIdx, 0, item);
      saveConfig(next, delayMs);
      return next;
    });
  }, [delayMs, saveConfig]);

  const handleDelayChange = useCallback((val: number) => {
    setDelayMs(val);
    saveConfig(orderedRequests, val);
  }, [orderedRequests, saveConfig]);

  const moveUp = useCallback((idx: number) => { if (idx > 0) handleReorder(idx, idx - 1); }, [handleReorder]);
  const moveDown = useCallback((idx: number) => { if (idx < orderedRequests.length - 1) handleReorder(idx, idx + 1); }, [handleReorder, orderedRequests.length]);

  const handleRun = useCallback(async () => {
    if (orderedRequests.length === 0) return;
    cancelledRef.current = false;
    setRunning(true);
    setResults([]);
    setProgress(0);
    setReport(null);

    const fullRequests: SavedRequest[] = await Promise.all(
      orderedRequests.map(async (stub) => {
        try {
          const res = await window.api.loadEntity(wsId, "requests", stub.id);
          if (res.ok && res.entity) return res.entity as SavedRequest;
        } catch { /* fall through */ }
        return stub;
      }),
    );

    const runReport = await runCollection(
      fullRequests,
      activeEnv,
      folderId,
      folderName,
      {
        onRequestStart(index) { setProgress(index); },
        onRequestDone(index, result) {
          setResults((prev) => [...prev, result]);
          setProgress(index + 1);
        },
        isCancelled() { return cancelledRef.current; },
      },
      delayMs,
    );

    setRunning(false);
    setReport(runReport);
    onSaveReport?.(runReport);
  }, [orderedRequests, activeEnv, folderId, folderName, onSaveReport, delayMs, wsId]);

  const handleCancel = useCallback(() => { cancelledRef.current = true; }, []);

  const handleExport = useCallback(async () => {
    if (!report) return;
    await window.api.exportRunnerReport(report);
  }, [report]);

  const handleDragStart = useCallback((idx: number) => { dragIndexRef.current = idx; }, []);
  const handleDragOver = useCallback((e: React.DragEvent, idx: number) => {
    e.preventDefault();
    e.dataTransfer.dropEffect = "move";
    dragOverRef.current = idx;
  }, []);
  const handleDrop = useCallback((idx: number) => {
    const from = dragIndexRef.current;
    if (from !== null && from !== idx) handleReorder(from, idx);
    dragIndexRef.current = null;
    dragOverRef.current = null;
  }, [handleReorder]);

  return {
    orderedRequests,
    delayMs,
    running,
    progress,
    results,
    report,
    handleDelayChange,
    moveUp,
    moveDown,
    handleRun,
    handleCancel,
    handleExport,
    handleDragStart,
    handleDragOver,
    handleDrop,
  };
}
