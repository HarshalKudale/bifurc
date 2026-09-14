import React, { useState } from "react";
import { AuditEntry, AuditAction, AuditEntity } from "@/types";
import { ChevronRight, ChevronDown } from "@/lib/icons";
import { formatFieldLabel } from "@/lib/utils";
import { strings } from "@/lib/strings";
import CodeEditor from "@/components/common/CodeEditor";

const ACTION_COLORS: Record<AuditAction, string> = {
  create: "bg-signal/15 text-signal border border-signal/30",
  update: "bg-amber/15 text-amber border border-amber/30",
  delete: "bg-destructive/15 text-destructive border border-destructive/30",
};

const ENTITY_LABELS: Record<AuditEntity, string> = {
  mock: "MOCK",
  mapping: "MAP",
  rule: "RULE",
  environment: "ENV",
  request: "REQ",
  wsConnection: "WS",
  webhook: "HOOK",
  folder: "FOLDER",
  workspace: "WS",
};

function relativeTime(ts: number): string {
  const diff = Date.now() - ts;
  if (diff < 60_000) return `${Math.floor(diff / 1000)}s ago`;
  if (diff < 3_600_000) return `${Math.floor(diff / 60_000)}m ago`;
  if (diff < 86_400_000) return `${Math.floor(diff / 3_600_000)}h ago`;
  return `${Math.floor(diff / 86_400_000)}d ago`;
}

function absoluteTime(ts: number): string {
  return new Date(ts).toLocaleString();
}

interface DiffProps {
  before: unknown | null;
  after: unknown | null;
}

function InlineDiff({ before, after }: DiffProps) {
  const [showUnchanged, setShowUnchanged] = useState(false);

  const beforeObj = (before && typeof before === "object") ? (before as Record<string, unknown>) : {};
  const afterObj = (after && typeof after === "object") ? (after as Record<string, unknown>) : {};

  if (!before && !after) {
    return <p className="text-xs text-muted-foreground">{strings.auditLog.noSnapshot}</p>;
  }

  if (before && !after) {
    return (
      <div className="text-xs text-muted-foreground italic">
        {strings.auditLog.entityDeleted} <span className="font-mono text-[10px] text-muted-foreground/60">{strings.auditLog.beforeStateStored}</span>
      </div>
    );
  }

  if (!before && after) {
    return (
      <div className="text-xs text-muted-foreground italic">
        {strings.auditLog.entityCreated}
      </div>
    );
  }

  const allKeys = Array.from(new Set([...Object.keys(beforeObj), ...Object.keys(afterObj)]))
    .filter((k) => !k.startsWith("_"));

  const changed = allKeys.filter((k) => JSON.stringify(beforeObj[k]) !== JSON.stringify(afterObj[k]));
  const unchanged = allKeys.filter((k) => !changed.includes(k));

  if (changed.length === 0) {
    return <p className="text-xs text-muted-foreground">{strings.auditLog.noFieldChanges}</p>;
  }

  const displayKeys = showUnchanged ? allKeys : changed;

  return (
    <div className="flex flex-col gap-1.5 text-xs font-mono">
      {displayKeys.map((k) => {
        const isChanged = changed.includes(k);
        const bVal = JSON.stringify(beforeObj[k] ?? null, null, 2);
        const aVal = JSON.stringify(afterObj[k] ?? null, null, 2);
        return (
          <div key={k} className={`flex flex-col gap-0.5 ${!isChanged ? "opacity-40" : ""}`}>
            <span className="text-[10px] uppercase text-muted-foreground tracking-wider">{k}</span>
            <div className="flex gap-2">
              <div className="flex-1 bg-destructive/5 border border-destructive/20 rounded overflow-hidden opacity-70" style={{ maxHeight: 128 }}>
                <CodeEditor value={bVal} readOnly language="json" className="h-full" />
              </div>
              <div className="flex-1 bg-signal/5 border border-signal/20 rounded overflow-hidden" style={{ maxHeight: 128 }}>
                <CodeEditor value={aVal} readOnly language="json" className="h-full" />
              </div>
            </div>
          </div>
        );
      })}
      {unchanged.length > 0 && (
        <button
          className="text-[10px] text-muted-foreground underline text-left mt-1 cursor-pointer"
          onClick={() => setShowUnchanged((v) => !v)}
        >
          {showUnchanged ? `Hide ${unchanged.length} unchanged fields` : `Show ${unchanged.length} unchanged fields`}
        </button>
      )}
    </div>
  );
}

interface AuditLogRowProps {
  entry: AuditEntry;
  isExpanded: boolean;
  isDiffLoading: boolean;
  entryDiff?: { before: unknown | null; after: unknown | null };
  onExpand: (entry: AuditEntry) => void;
}

export const AuditLogRow = React.memo(({ entry, isExpanded, isDiffLoading, entryDiff, onExpand }: AuditLogRowProps) => {
  const canExpand = entry.action === "update";

  const rowContent = (
    <div className="w-full flex items-center gap-2.5 px-4 py-2.5">
      {canExpand && (
        <span className="text-muted-foreground flex-shrink-0 w-4">
          {isExpanded ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
        </span>
      )}
      {!canExpand && <span className="w-4 flex-shrink-0" />}

      <span
        className={`text-[10px] font-semibold px-1.5 py-0.5 rounded uppercase tracking-wide flex-shrink-0 ${ACTION_COLORS[entry.action]}`}
      >
        {entry.action}
      </span>

      <span className="text-[10px] font-mono bg-card border border-border px-1.5 py-0.5 rounded text-muted-foreground uppercase tracking-wide flex-shrink-0">
        {ENTITY_LABELS[entry.entity] ?? entry.entity}
      </span>

      {entry.action === "update" && entry.changedFields && entry.changedFields.length > 0 ? (
        <span className="text-xs text-muted-foreground font-mono flex-shrink-0 max-w-[160px] truncate" title={entry.changedFields.map(formatFieldLabel).join(", ")}>
          {entry.changedFields.slice(0, 3).map(formatFieldLabel).join(", ")}
          {entry.changedFields.length > 3 ? ` +${entry.changedFields.length - 3}` : ""}
        </span>
      ) : null}

      <span className="text-sm text-foreground truncate flex-1 min-w-0">
        {entry.entityName}
      </span>

      <span className="text-xs text-signal/80 font-medium flex-shrink-0 hidden sm:block max-w-[100px] truncate" title={entry.actor}>
        {entry.actor}
      </span>

      <span
        className="text-xs text-muted-foreground flex-shrink-0"
        title={absoluteTime(entry.ts)}
      >
        {relativeTime(entry.ts)}
      </span>
    </div>
  );

  return (
    <div className="border-b border-border last:border-0">
      {canExpand ? (
        <button
          onClick={() => onExpand(entry)}
          className="w-full hover:bg-surface transition-colors text-left cursor-pointer"
        >
          {rowContent}
        </button>
      ) : (
        <div className="select-text">{rowContent}</div>
      )}

      {/* Expanded diff - update only */}
      {isExpanded && (
        <div className="px-4 pb-4 pt-1 bg-surface/50">
          <div className="flex items-center gap-3 mb-2">
            <span className="text-[10px] font-mono text-muted-foreground select-all">
              commit {entry.commitHash}
            </span>
            <span className="text-[10px] text-signal/80 font-medium ml-auto">
              {entry.actor}
            </span>
            <span className="text-[10px] text-muted-foreground" title={absoluteTime(entry.ts)}>
              {absoluteTime(entry.ts)}
            </span>
          </div>
          {isDiffLoading ? (
            <div className="text-xs text-muted-foreground py-2">{strings.auditLog.loadingDiff}</div>
          ) : entryDiff ? (
            <InlineDiff before={entryDiff.before} after={entryDiff.after} />
          ) : null}
        </div>
      )}
    </div>
  );
});
