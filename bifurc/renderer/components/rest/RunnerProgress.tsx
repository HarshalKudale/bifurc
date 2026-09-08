import React from "react";
import { Loader2, CheckCircle2, XCircle } from "lucide-react";
import { strings } from "@/lib/strings";
import { CollectionRunReport, RunnerRequestResult } from "@/lib/collectionRunner";

interface RunnerProgressProps {
  running: boolean;
  progress: number;
  total: number;
  report: CollectionRunReport | null;
  results: RunnerRequestResult[];
}

export function RunnerProgress({
  running,
  progress,
  total,
  report,
  results,
}: RunnerProgressProps) {
  if (running) {
    return (
      <div className="px-4 py-1.5 border-b border-border/40 bg-background/30 flex-shrink-0">
        <div className="flex items-center gap-2 mb-1">
          <Loader2 size={10} className="animate-spin text-signal" />
          <span className="text-[10px] text-muted-foreground">{strings.collectionRunner.running}</span>
          <span className="text-[10px] text-foreground font-mono">
            {progress}/{total}
          </span>
        </div>
        <div className="w-full h-1 bg-card rounded overflow-hidden">
          <div
            className="h-full bg-signal transition-all duration-300"
            style={{ width: `${(progress / Math.max(total, 1)) * 100}%` }}
          />
        </div>
      </div>
    );
  }

  if (report && !running) {
    const totalTests = results.reduce((acc, r) => acc + r.tests.length, 0);
    const passedTests = results.reduce((acc, r) => acc + r.tests.filter((t) => t.passed).length, 0);
    const failedTests = totalTests - passedTests;
    const allGood = failedTests === 0 && results.every((r) => !r.error);

    return (
      <div className="flex items-center gap-5 px-4 py-1.5 border-b border-border/40 bg-background/30 flex-shrink-0">
        {allGood ? (
          <span className="flex items-center gap-1.5 text-[11px] font-semibold text-signal">
            <CheckCircle2 size={13} /> {strings.collectionRunner.allPassed}
          </span>
        ) : (
          <span className="flex items-center gap-1.5 text-[11px] font-semibold text-destructive">
            <XCircle size={13} /> {strings.collectionRunner.someFailed}
          </span>
        )}
        <span className="text-[11px] text-muted-foreground">
          <span className="text-foreground font-mono">{results.length}</span> {strings.collectionRunner.requests}
        </span>
        {totalTests > 0 && (
          <span className="text-[11px] text-muted-foreground">
            <span className="text-signal font-mono">{passedTests}</span>
            {failedTests > 0 && (
              <>
                <span className="text-muted-foreground mx-1">/</span>
                <span className="text-destructive font-mono">
                  {failedTests} {strings.collectionRunner.failed}
                </span>
              </>
            )}
            {" "}
            {strings.collectionRunner.tests}
          </span>
        )}
        <span className="text-[11px] text-muted-foreground ml-auto font-mono">
          {((report.completedAt - report.startedAt) / 1000).toFixed(2)}s
        </span>
      </div>
    );
  }

  return null;
}
