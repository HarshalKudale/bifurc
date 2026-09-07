import React, { useState } from "react";
import { CheckCircle2, XCircle, ChevronUp, ChevronDown, AlertTriangle, Terminal } from "lucide-react";
import { strings } from "@/lib/strings";
import { RunnerRequestResult } from "@/lib/collectionRunner";
import { statusColor } from "@/lib/utils";

export function methodColor(method: string): string {
  switch (method.toUpperCase()) {
    case "GET": return "text-signal";
    case "POST": return "text-amber";
    case "PUT": return "text-blue";
    case "PATCH": return "text-orange";
    case "DELETE": return "text-destructive";
    default: return "text-signal";
  }
}

export function StatusDot({ result }: { result: RunnerRequestResult }) {
  const hasError = !!result.error;
  const hasTests = result.tests.length > 0;
  const allPassed = result.tests.every((t) => t.passed);
  const isGood = !hasError && (!hasTests ? (result.status !== null && result.status >= 200 && result.status < 300) : allPassed);
  return isGood
    ? <CheckCircle2 size={13} className="text-signal flex-shrink-0" />
    : <XCircle size={13} className="text-destructive flex-shrink-0" />;
}

function getResultCardStyle(result: RunnerRequestResult): { border: string; bg: string; indicator: string } {
  const hasError = !!result.error;
  const hasTests = result.tests.length > 0;
  const allPassed = result.tests.every((t) => t.passed);
  if (hasError || (hasTests && !allPassed)) return { border: "border-destructive/25", bg: "bg-destructive/5", indicator: "bg-destructive" };
  if (hasTests && allPassed) return { border: "border-signal/25", bg: "bg-signal/5", indicator: "bg-signal" };
  if (result.status !== null && result.status >= 200 && result.status < 300) return { border: "border-signal/25", bg: "bg-signal/5", indicator: "bg-signal" };
  if (result.status !== null && result.status >= 400) return { border: "border-destructive/25", bg: "bg-destructive/5", indicator: "bg-destructive" };
  return { border: "border-border/40", bg: "bg-surface/20", indicator: "bg-amber" };
}

export function RequestResultCard({ result, index }: { result: RunnerRequestResult; index: number }) {
  const hasExpandable = result.tests.length > 0 || !!result.error || !!result.preScriptError || !!result.postScriptError || result.testLogs.length > 0;
  const [expanded, setExpanded] = useState(false);
  const hasTests = result.tests.length > 0;
  const allPassed = result.tests.every((t) => t.passed);
  const style = getResultCardStyle(result);

  return (
    <div className={`rounded-md border ${style.border} ${style.bg} overflow-hidden`}>
      {/* Card header row */}
      <div
        className={`flex items-center gap-2.5 px-3 py-2 ${hasExpandable ? "cursor-pointer hover:bg-black/10" : ""}`}
        onClick={() => hasExpandable && setExpanded((v) => !v)}
      >
        {/* Status indicator bar */}
        <span className={`w-0.5 h-5 rounded-full flex-shrink-0 ${style.indicator}`} />

        <span className="text-[10px] font-mono text-muted-foreground w-4 text-right flex-shrink-0">{index + 1}</span>
        <span className={`text-[10px] font-bold font-mono w-12 flex-shrink-0 ${methodColor(result.method)}`}>{result.method}</span>

        <div className="flex-1 min-w-0">
          <div className="text-[12px] text-foreground font-medium truncate">{result.requestName}</div>
          <div className="text-[10px] text-muted-foreground/70 truncate font-mono">{result.url}</div>
        </div>

        {/* Status code */}
        {result.status !== null ? (
          <span className="text-xs font-bold font-mono flex-shrink-0" style={{ color: statusColor(result.status) }}>
            {result.status}
          </span>
        ) : (
          result.error && <span className="text-[10px] text-destructive flex-shrink-0">{strings.collectionRunner.error}</span>
        )}

        {/* Response time */}
        <span className="text-[10px] text-muted-foreground/70 font-mono flex-shrink-0">{result.responseTime}ms</span>

        {/* Test summary badge */}
        {hasTests && (
          <span className={`text-[10px] font-semibold px-1.5 py-0.5 rounded-full flex-shrink-0 ${allPassed ? "bg-signal/15 text-signal" : "bg-destructive/15 text-destructive"}`}>
            {result.tests.filter((t) => t.passed).length}/{result.tests.length}
          </span>
        )}

        {hasExpandable && (
          expanded ? <ChevronUp size={13} className="text-muted-foreground/50 flex-shrink-0" /> : <ChevronDown size={13} className="text-muted-foreground/50 flex-shrink-0" />
        )}
      </div>

      {/* Expanded details */}
      {expanded && hasExpandable && (
        <div className="border-t border-border/25 px-3 py-2 space-y-2">
          {/* Errors */}
          {result.error && (
            <div className="flex items-start gap-1.5 text-[11px] text-destructive font-mono bg-destructive/5 rounded px-2 py-1">
              <AlertTriangle size={11} className="mt-0.5 flex-shrink-0" />
              {result.error}
            </div>
          )}
          {result.preScriptError && (
            <div className="text-[11px] text-amber font-mono bg-amber/5 rounded px-2 py-1">
              <span className="text-muted-foreground mr-1">{strings.collectionRunner.preScriptLabel}</span>{result.preScriptError}
            </div>
          )}
          {result.postScriptError && (
            <div className="text-[11px] text-amber font-mono bg-amber/5 rounded px-2 py-1">
              <span className="text-muted-foreground mr-1">{strings.collectionRunner.postScriptLabel}</span>{result.postScriptError}
            </div>
          )}

          {/* Test results */}
          {result.tests.length > 0 && (
            <div className="space-y-0.5">
              {result.tests.map((t, ti) => (
                <div key={ti} className={`flex items-start gap-2 text-[11px] font-mono py-0.5 ${t.passed ? "text-signal" : "text-destructive"}`}>
                  {t.passed
                    ? <CheckCircle2 size={12} className="flex-shrink-0 mt-px" />
                    : <XCircle size={12} className="flex-shrink-0 mt-px" />
                  }
                  <span className="flex-1">{t.name}</span>
                  {t.error && <span className="text-[10px] opacity-70 truncate max-w-48" title={t.error}>{t.error}</span>}
                </div>
              ))}
            </div>
          )}

          {/* Console logs */}
          {result.testLogs.length > 0 && (
            <div className="border-t border-border/20 pt-2 space-y-0.5">
              <div className="flex items-center gap-1 text-[9px] text-muted-foreground uppercase tracking-wider mb-1">
                <Terminal size={9} /> {strings.collectionRunner.console}
              </div>
              {result.testLogs.map((log, li) => (
                <div key={li} className="text-[10px] text-muted-foreground font-mono">{log}</div>
              ))}
            </div>
          )}
        </div>
      )}
    </div>
  );
}
