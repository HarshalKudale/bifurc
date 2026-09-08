import React from "react";
import { Play, Square, Download } from "lucide-react";
import { strings } from "@/lib/strings";
import { CollectionRunReport } from "@/lib/collectionRunner";

interface RunnerConfigProps {
  folderName: string;
  requestCount: number;
  delayMs: number;
  running: boolean;
  report: CollectionRunReport | null;
  onDelayChange: (val: number) => void;
  onExport: () => void;
  onRun: () => void;
  onCancel: () => void;
}

export function RunnerConfig({
  folderName,
  requestCount,
  delayMs,
  running,
  report,
  onDelayChange,
  onExport,
  onRun,
  onCancel,
}: RunnerConfigProps) {
  return (
    <div className="flex items-center gap-3 px-4 py-2.5 border-b border-border bg-background/60 flex-shrink-0">
      <div className="flex-1 min-w-0">
        <span className="text-xs font-semibold text-foreground">{folderName}</span>
        <span className="text-[10px] text-muted-foreground ml-2">
          {strings.collectionRunner.requestCount.replace("{count}", String(requestCount)).replace("{s}", requestCount !== 1 ? "s" : "")}
        </span>
      </div>
      <label className="flex items-center gap-1.5 text-[10px] text-muted-foreground">
        {strings.collectionRunner.delay}
        <input
          type="number"
          min={0}
          step={100}
          value={delayMs}
          onChange={(e) => onDelayChange(Math.max(0, Number(e.target.value)))}
          disabled={running}
          className="w-16 px-1.5 py-0.5 rounded bg-card border border-border/60 text-foreground text-[10px] font-mono text-center focus:outline-none focus:border-signal/60"
        />
        <span>{strings.collectionRunner.ms}</span>
      </label>
      {report && !running && (
        <button
          onClick={onExport}
          className="flex items-center gap-1.5 px-2.5 py-1.5 rounded border border-border/60 hover:border-signal/50 text-muted-foreground hover:text-foreground text-xs cursor-pointer transition-colors"
          title={strings.collectionRunner.exportTitle}
        >
          <Download size={12} /> {strings.common.export}
        </button>
      )}
      {!running ? (
        <button
          onClick={onRun}
          disabled={requestCount === 0}
          className="flex items-center gap-1.5 px-3 py-1.5 rounded bg-signal hover:bg-signal/80 text-background text-xs font-semibold cursor-pointer disabled:opacity-40 disabled:cursor-not-allowed transition-colors"
        >
          <Play size={12} />
          {report ? strings.collectionRunner.runAgain : strings.collectionRunner.run}
        </button>
      ) : (
        <button
          onClick={onCancel}
          className="flex items-center gap-1.5 px-3 py-1.5 rounded bg-destructive/15 hover:bg-destructive/25 text-destructive text-xs font-semibold cursor-pointer border border-destructive/30 transition-colors"
        >
          <Square size={12} /> {strings.common.cancel}
        </button>
      )}
    </div>
  );
}
