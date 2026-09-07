import React from "react";
import PanelHeader from "@/components/layout/PanelHeader";
import { Button } from "@/components/ui";
import { Play, Pause, Zap, Download } from "@/lib/icons";
import { strings } from "@/lib/strings";
import { RequestLogEntry } from "@/types";

interface CaptureToolbarProps {
  selectedCount: number;
  paused: boolean;
  onTogglePause: () => void;
  onClear: () => void;
  hasEntries: boolean;
  onMockAll: () => void;
  onSaveAll: () => void;
  maxEntries: number;
}

export function CaptureToolbar({
  selectedCount,
  paused,
  onTogglePause,
  onClear,
  hasEntries,
  onMockAll,
  onSaveAll,
  maxEntries,
}: CaptureToolbarProps) {
  return (
    <PanelHeader
      title={strings.capture.title}
      subtitle={strings.capture.subtitle.replace("{count}", String(maxEntries))}
      actions={
        <>
          {selectedCount > 0 && (
            <span className="text-xs text-muted-foreground whitespace-nowrap">
              {strings.capture.selectedCount.replace("{count}", String(selectedCount))}
            </span>
          )}
          <button
            onClick={onTogglePause}
            className={`flex items-center gap-1.5 px-3 py-1.5 rounded border text-xs font-medium transition-all cursor-pointer whitespace-nowrap ${
              paused
                ? "border-amber bg-amber/10 text-amber"
                : "border-signal/40 bg-signal/10 text-signal hover:bg-signal/20"
            }`}
          >
            {paused ? (
              <>
                <Play size={10} fill="currentColor" /> {strings.capture.start}
              </>
            ) : (
              <>
                <Pause size={10} fill="currentColor" /> {strings.capture.pause}
              </>
            )}
          </button>
          <Button variant="secondary" onClick={onClear}>
            {strings.capture.clear}
          </Button>
          {hasEntries && (
            <>
              <Button variant="secondary" onClick={onMockAll} icon={<Zap size={10} />}>
                {strings.capture.mockAll}
              </Button>
              <Button variant="secondary" onClick={onSaveAll} icon={<Download size={10} />}>
                {strings.capture.saveAll}
              </Button>
            </>
          )}
        </>
      }
    />
  );
}
