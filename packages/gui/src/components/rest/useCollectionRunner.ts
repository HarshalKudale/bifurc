import { useState, useCallback, useRef, useEffect } from "react";
import { SavedRequest, CollectionRunReport, RunnerStatusResult } from "@/types";
import { strings } from "@/lib/strings";

interface UseCollectionRunnerProps {
  wsId: string;
  folderId: string;
  folderName: string;
  requests: SavedRequest[];
  onSaveReport?: (report: CollectionRunReport) => Promise<void>;
}

/**
 * The collection runner tab's state — `plan/19` WI 4.
 *
 * ## What this hook no longer is
 *
 * It used to **run** the collection: `runCollection()` looped the folder's requests, executed each one's five
 * steps, and accumulated the results in React state. That is exactly D25's subject — *"run state (running,
 * current index, results, cancellation) is engine state for the lifetime of the run, not React state"* — and
 * the engine owns it now (`runner/runCollection.ts` + `runner/runRegistry.ts`, decisions D29/D30).
 *
 * ## What it is now: a subscription
 *
 * Three calls and two events:
 *
 * - **`runnerStatus` on mount**, so a tab opened *while a run is going* shows it. That is D29 visible in the
 *   UI: the run outlives the window, so a tab that appears mid-run has to be able to attach to one.
 * - **`runnerStart` / `runnerCancel`** for the two buttons.
 * - **`onRunnerProgress` / `onRunnerFinished`** for everything the engine reports.
 *
 * ## ⚠ The `run` state below is a MIRROR, not the authority
 *
 * React needs state to re-render, so the hook holds the last snapshot the engine reported. What matters is
 * what it is *used* for: **nothing here decides anything.** Whether a run is going, how far it has got, and
 * whether it was cancelled all come from the engine; the cancel is a request to the engine rather than a flag
 * this hook sets. `plan/19` §2's test — *would this differ if the UI were a terminal?* — is the one to apply:
 * the state would not, only its rendering does.
 *
 * ## The folder's ordering and delay stay UI-owned, and that is not an oversight
 *
 * `runner.json` is a *configuration*, the storage half of the `runner.*` namespace, and its contract is
 * unchanged. The hook reads it on mount and writes it on every reorder or delay change exactly as before —
 * what it no longer does is use either to drive a loop.
 */
export function useCollectionRunner({
  wsId,
  folderId,
  folderName,
  requests,
  onSaveReport,
}: UseCollectionRunnerProps) {
  const [orderedRequests, setOrderedRequests] = useState<SavedRequest[]>(requests);
  const [delayMs, setDelayMs] = useState(0);
  const [configLoaded, setConfigLoaded] = useState(false);
  /** The engine's last report of this folder's run, or `null` if it has never been asked to run one. */
  const [run, setRun] = useState<RunnerStatusResult | null>(null);
  /** D30's refusal — a second run of a folder already in flight — surfaced rather than swallowed. */
  const [startError, setStartError] = useState<string | null>(null);

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

  /**
   * Ask the engine what it knows about this folder's run.
   *
   * Called on mount and again after a successful start, because **the engine's answer is the authority** —
   * the alternative is to construct a snapshot here, which is the renderer owning run state again by the back
   * door.
   */
  const refreshStatus = useCallback(async () => {
    try {
      setRun(await window.api.runnerStatus(wsId, folderId));
    } catch { /* leave the last snapshot rather than blanking the pane */ }
  }, [wsId, folderId]);

  useEffect(() => { void refreshStatus(); }, [refreshStatus]);

  useEffect(() => {
    // **Filtered by folder.** The events are workspace-wide and two folders may run at once (D30), so a tab
    // that did not filter would show another folder's results interleaved with its own.
    const mine = (p: { workspaceId: string; folderId: string }) =>
      p.workspaceId === wsId && p.folderId === folderId;

    const offProgress = window.api.onRunnerProgress((p) => {
      if (!mine(p)) return;
      setRun((prev) =>
        prev
          ? { ...prev, completed: p.index + 1, results: [...prev.results, p.result] }
          : {
              // **An event has to be able to ESTABLISH the state, not only update one.** A client that
              // attaches mid-run has no snapshot yet, and a progress event is the first thing it sees — this
              // is precisely what D29 means by "a reconnecting client can attach", and it is what `total`
              // riding on every event is for: without it this branch could not fill the field in.
              workspaceId: p.workspaceId,
              folderId: p.folderId,
              // ⚠ A **guess**, and the only one in this hook: a progress event carries no start time.
              // `startedAt` feeds the elapsed-time readout, so a client that attached mid-run reports the
              // time since *it* attached rather than since the run began. Fixing it properly means putting a
              // start time on the event; recorded rather than papered over.
              startedAt: Date.now(),
              total: p.total,
              completed: p.index + 1,
              running: true,
              cancelled: false,
              results: [p.result],
            },
      );
    });

    const offFinished = window.api.onRunnerFinished((p) => {
      if (!mine(p)) return;
      setRun((prev) => ({
        // The report is the authority for everything the run produced, so an event that arrives with no
        // snapshot establishes one — which is the disconnected-client case D29 exists for.
        workspaceId: p.workspaceId,
        folderId: p.folderId,
        startedAt: prev?.startedAt ?? p.report.startedAt,
        total: prev?.total ?? p.report.totalRequests,
        completed: p.report.results.length,
        running: false,
        cancelled: prev?.cancelled ?? false,
        completedAt: p.report.completedAt,
        // The report's results rather than the accumulated ones: a client that was disconnected for the whole
        // run has no accumulated ones, and this is the event that gives it everything.
        results: p.report.results,
        report: p.report,
      }));
      onSaveReport?.(p.report);
    });

    return () => { offProgress(); offFinished(); };
  }, [wsId, folderId, onSaveReport]);

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
    setStartError(null);
    try {
      const res = await window.api.runnerStart(wsId, folderId);
      if (!res.ok) {
        // D30's refusal. Shown rather than swallowed: "nothing happened" is the one outcome a Run button must
        // not produce silently.
        setStartError(res.error ?? strings.collectionRunner.startFailed);
        return;
      }
      await refreshStatus();
    } catch (e) {
      setStartError(e instanceof Error ? e.message : strings.collectionRunner.startFailed);
    }
  }, [wsId, folderId, refreshStatus]);

  const handleCancel = useCallback(async () => {
    // **The engine is the only thing that can stop a run** (D29). The UI asks; it does not decide — which is
    // why there is no local `cancelled` flag to set, and why closing this tab does not stop anything.
    try {
      await window.api.runnerCancel(wsId, folderId);
    } catch { /* the run may already have finished */ }
  }, [wsId, folderId]);

  const handleExport = useCallback(async () => {
    if (!run?.report) return;
    await window.api.exportRunnerReport(run.report);
  }, [run]);

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
    running: run?.running ?? false,
    progress: run?.completed ?? 0,
    results: run?.results ?? [],
    report: run?.report ?? null,
    startError,
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
