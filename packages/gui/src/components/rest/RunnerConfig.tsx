import React from "react";
import { Play, Square, Download } from "lucide-react";
import { strings } from "@/lib/strings";
import { CollectionRunReport } from "@/types";

interface RunnerConfigProps {
  /**
   * The number of requests the run will cover.
   *
   * Used **only** to gate the Run button — the visible count lives in the title bar now, so that it
   * sits beside the folder name it belongs to. Rendering it here as well would put "2 requests" on
   * screen twice, which is both clutter and a second element for a locator to trip over.
   */
  requestCount: number;
  delayMs: number;
  running: boolean;
  report: CollectionRunReport | null;
  /**
   * Why the engine refused to start a run, or `null`.
   *
   * **This is D30 made visible.** A second run of a folder already in flight is refused rather than queued,
   * and "the button did nothing" is the one outcome a Run button must not produce silently — so the refusal
   * is rendered here rather than swallowed in the hook.
   */
  startError: string | null;
  onDelayChange: (val: number) => void;
  onExport: () => void;
  onRun: () => void;
  onCancel: () => void;
}

/**
 * The runner's toolbar row — delay on the left, run controls on the right.
 *
 * ⚠ This returns a **fragment**, not a framed row: `CollectionRunner` renders it inside a
 * `TabToolbarRow`, which owns the `flex items-center gap-2` and the band's padding. Adding an outer
 * `<div>` here would nest a second flex container and reintroduce the extra padding and divider this
 * refactor removed. `ml-auto` on the right cluster is what keeps the two groups apart.
 *
 * `folderName` used to be a prop of this component and is gone: the folder's name is the runner
 * tab's *title*, and it now lives in the title bar with every other tab page's name.
 */
export function RunnerConfig({
  requestCount,
  delayMs,
  running,
  report,
  startError,
  onDelayChange,
  onExport,
  onRun,
  onCancel,
}: RunnerConfigProps) {
  return (
    <>
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

      {startError && (
        <span className="text-[10px] text-destructive truncate max-w-64" title={startError}>
          {startError}
        </span>
      )}

      <div className="ml-auto flex items-center gap-2">
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
    </>
  );
}
