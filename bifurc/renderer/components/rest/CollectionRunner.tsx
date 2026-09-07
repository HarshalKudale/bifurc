import React from "react";
import { Play, ChevronDown, ChevronUp, GripVertical, Loader2 } from "lucide-react";
import { SavedRequest, Environment } from "@/types";
import { CollectionRunReport } from "@/lib/collectionRunner";
import { strings } from "@/lib/strings";
import { useCollectionRunner } from "./useCollectionRunner";
import { RunnerConfig } from "./RunnerConfig";
import { RunnerProgress } from "./RunnerProgress";
import { RequestResultCard, StatusDot, methodColor } from "./RunnerResults";

export interface CollectionRunnerProps {
  folderId: string;
  folderName: string;
  requests: SavedRequest[];
  activeEnv: Environment | null;
  wsId: string;
  onClose(): void;
  onSaveReport?(report: CollectionRunReport): Promise<void>;
}

export default function CollectionRunner({
  folderId,
  folderName,
  requests,
  activeEnv,
  wsId,
  onSaveReport,
}: CollectionRunnerProps) {
  const {
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
  } = useCollectionRunner({
    wsId,
    folderId,
    folderName,
    requests,
    activeEnv,
    onSaveReport,
  });

  return (
    <div className="flex flex-col h-full overflow-hidden">
      <RunnerConfig
        folderName={folderName}
        requestCount={orderedRequests.length}
        delayMs={delayMs}
        running={running}
        report={report}
        onDelayChange={handleDelayChange}
        onExport={handleExport}
        onRun={handleRun}
        onCancel={handleCancel}
      />

      <RunnerProgress
        running={running}
        progress={progress}
        total={orderedRequests.length}
        report={report}
        results={results}
      />

      <div className="flex flex-1 overflow-hidden">
        {/* Left panel -- request list */}
        <div className="w-72 flex-shrink-0 border-r border-border/60 flex flex-col overflow-hidden bg-background/20">
          <div className="px-3 py-2 border-b border-border/30 flex-shrink-0">
            <span className="text-[10px] font-semibold text-muted-foreground uppercase tracking-wider">{strings.collectionRunner.requestsHeader}</span>
            <span className="text-[10px] text-muted-foreground ml-1">{strings.collectionRunner.dragToReorder}</span>
          </div>
          <div className="flex-1 overflow-y-auto p-2 space-y-1">
            {orderedRequests.length === 0 && (
              <div className="flex items-center justify-center h-20">
                <p className="text-[11px] text-muted-foreground">{strings.collectionRunner.noRequests}</p>
              </div>
            )}
            {orderedRequests.map((req, idx) => {
              const result = results[idx];
              const isRunning = running && idx === progress;
              return (
                <div
                  key={req.id}
                  draggable={!running}
                  onDragStart={() => handleDragStart(idx)}
                  onDragOver={(e) => handleDragOver(e, idx)}
                  onDrop={() => handleDrop(idx)}
                  className="flex items-center gap-2 px-2 py-1.5 rounded border border-border/30 bg-surface/20 hover:bg-surface/50 cursor-grab active:cursor-grabbing group select-none"
                >
                  <GripVertical size={12} className="text-muted-foreground/40 flex-shrink-0" />
                  <span className="text-[10px] text-muted-foreground/60 w-4 text-right font-mono flex-shrink-0">{idx + 1}</span>
                  <span className={`text-[10px] font-bold font-mono w-11 flex-shrink-0 ${methodColor(req.method)}`}>
                    {req.method}
                  </span>
                  <span className="text-[11px] text-foreground truncate flex-1" title={req.name || req.url}>
                    {req.name || strings.collectionRunner.untitled}
                  </span>
                  {isRunning && (
                    <Loader2 size={11} className="animate-spin text-signal flex-shrink-0" />
                  )}
                  {result && <StatusDot result={result} />}
                  {!result && !isRunning && (
                    <div className="flex gap-0.5 opacity-0 group-hover:opacity-100 flex-shrink-0">
                      <button onClick={() => moveUp(idx)} disabled={idx === 0} className="text-muted-foreground hover:text-foreground disabled:opacity-20 cursor-pointer p-0.5">
                        <ChevronUp size={11} />
                      </button>
                      <button onClick={() => moveDown(idx)} disabled={idx === orderedRequests.length - 1} className="text-muted-foreground hover:text-foreground disabled:opacity-20 cursor-pointer p-0.5">
                        <ChevronDown size={11} />
                      </button>
                    </div>
                  )}
                </div>
              );
            })}
          </div>
        </div>

        {/* Right panel -- results */}
        <div className="flex-1 flex flex-col overflow-hidden">
          {results.length === 0 && !running ? (
            <div className="flex-1 flex flex-col items-center justify-center gap-3 text-center p-8">
              <Play size={36} className="opacity-10 text-foreground" />
              <p className="text-xs text-muted-foreground max-w-48">
                {strings.collectionRunner.pressRunPrefix} <span className="font-semibold text-foreground">{strings.collectionRunner.run}</span> {strings.collectionRunner.pressRunSuffix}
              </p>
            </div>
          ) : (
            <div className="flex-1 overflow-y-auto p-3 space-y-2">
              {results.map((r, i) => (
                <RequestResultCard key={i} result={r} index={i} />
              ))}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
