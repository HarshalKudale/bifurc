import React from "react";
import { Play, ChevronDown, ChevronUp, GripVertical, Loader2 } from "lucide-react";
import { SavedRequest, CollectionRunReport } from "@/types";
import { strings } from "@/lib/strings";
import {
  TabPage, TabTitleBar, TabToolbar, TabToolbarRow, SplitPane, useSplitOrientation, PROTOCOL_TONE,
} from "@/components/editor/TabPage";
import { useCollectionRunner } from "./useCollectionRunner";
import { RunnerConfig } from "./RunnerConfig";
import { RunnerProgress } from "./RunnerProgress";
import { RequestResultCard, StatusDot, methodColor } from "./RunnerResults";

export interface CollectionRunnerProps {
  folderId: string;
  folderName: string;
  requests: SavedRequest[];
  wsId: string;
  onClose(): void;
  onSaveReport?(report: CollectionRunReport): Promise<void>;
}

/**
 * The collection runner tab.
 *
 * ## What changed, and why
 *
 * This page was the furthest from the other four: no title bar at all, a hard-coded `w-72` left
 * column that could not be resized, and no way to stack the list above the results. It now follows
 * the same four bands as every protocol tab — title bar, toolbar, split, (no footer, because a run
 * is not a saved entity) — and gains the shared layout toggle with them.
 *
 * ## ⚠ The DOM nesting here is asserted by `tests/renderer/requestsRunnerTab.test.tsx`
 *
 * Two structural facts are load-bearing, and both are about *containment*, not class names:
 *
 * 1. The count text (`"2 requests"`) must be a child of the element that also carries the folder
 *    name — the test reads `getByText(/2 requests/).parentElement.textContent` and expects
 *    `"Payments"` in it. That is why the count rides in `TabTitleBar`'s `actions` slot rather than
 *    in the toolbar row below: the title bar is the element that holds the name.
 * 2. The `- drag to reorder` span's **grandparent** must contain the request rows — the test scopes
 *    every positive and negative assertion through `collectionList()`. So the list's own
 *    header/rows nesting is preserved verbatim from the previous revision; only its outer wrapper
 *    changed (a `Panel` instead of a fixed `w-72` div).
 */
export default function CollectionRunner({
  folderId,
  folderName,
  requests,
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
  } = useCollectionRunner({
    wsId,
    folderId,
    folderName,
    requests,
    onSaveReport,
  });

  const [splitOrientation, toggleSplitOrientation] = useSplitOrientation();

  const countLabel = strings.collectionRunner.requestCount
    .replace("{count}", String(orderedRequests.length))
    .replace("{s}", orderedRequests.length !== 1 ? "s" : "");

  return (
    <TabPage>
      {/* Band 1 — the title bar this tab never had. The folder's name is the tab's identity, and it
          is not editable here: renaming a folder happens in the tree, and the tab label is derived
          from it. `readOnly` renders it as text in the same place every protocol tab renders its
          name field. */}
      <TabTitleBar
        chip="RUNNER"
        tone={PROTOCOL_TONE.runner}
        name={folderName}
        readOnly
        actions={<span className="text-[10px] text-muted-foreground flex-shrink-0">{countLabel}</span>}
      />

      {/* Band 2 — the run controls. */}
      <TabToolbar>
        <TabToolbarRow>
          <RunnerConfig
            requestCount={orderedRequests.length}
            delayMs={delayMs}
            running={running}
            report={report}
            startError={startError}
            onDelayChange={handleDelayChange}
            onExport={handleExport}
            onRun={handleRun}
            onCancel={handleCancel}
          />
        </TabToolbarRow>
      </TabToolbar>

      {/* Progress / summary — renders nothing at rest, so the split keeps its height. */}
      <RunnerProgress
        running={running}
        progress={progress}
        total={orderedRequests.length}
        report={report}
        results={results}
      />

      {/* Band 3 — the split. 30/70 rather than 50/50: the left pane is a list of names, not an
          editor, and it used to be a fixed 288px for exactly that reason. It is resizable now. */}
      <SplitPane
        orientation={splitOrientation}
        leftSize={30}
        rightSize={70}
        minSize={15}
        left={
          /* Left panel -- request list. ⚠ Keep the header → rows nesting: see the note above. */
          <div className="flex flex-col h-full overflow-hidden bg-background/20">
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
        }
        right={
          /* Right panel -- results */
          <div className="flex flex-col h-full overflow-hidden">
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
        }
      />
    </TabPage>
  );
}
