import React, { useState, useEffect, useCallback, useRef } from "react";
import { AuditEntry } from "@/types";
import { X, ChevronRight, ChevronDown, GitCommit } from "@/lib/icons";
import { formatFieldLabel } from "@/lib/utils";
import { strings } from "@/lib/strings";

interface Props {
  filePath: string;
  workspaceId: string;
  onClose(): void;
  /** Increment to trigger a reload (e.g. after a save). */
  reloadKey?: number;
  open: boolean;
}

const ACTION_COLORS: Record<string, string> = {
  create: "bg-signal/15 text-signal border border-signal/30",
  update: "bg-amber/15 text-amber border border-amber/30",
  delete: "bg-destructive/15 text-destructive border border-destructive/30",
};

function relativeTime(ts: number): string {
  const diff = Date.now() - ts;
  if (diff < 60_000)  return `${Math.floor(diff / 1000)}s ago`;
  if (diff < 3_600_000) return `${Math.floor(diff / 60_000)}m ago`;
  if (diff < 86_400_000) return `${Math.floor(diff / 3_600_000)}h ago`;
  return `${Math.floor(diff / 86_400_000)}d ago`;
}

function absoluteTime(ts: number): string {
  return new Date(ts).toLocaleString();
}

export default function HistorySidebar({ filePath, workspaceId, onClose, reloadKey, open }: Props) {
  const [entries, setEntries] = useState<AuditEntry[]>([]);
  const [total, setTotal] = useState(0);
  const [loading, setLoading] = useState(false);
  const [expandedHash, setExpandedHash] = useState<string | null>(null);
  const [diffs, setDiffs] = useState<Record<string, { before: unknown | null; after: unknown | null }>>({});
  const [diffLoading, setDiffLoading] = useState<string | null>(null);
  const prevPathRef = useRef<string>("");

  const load = useCallback(async (fp: string, wsId: string) => {
    if (!fp) return;
    setLoading(true);
    setExpandedHash(null);
    setDiffs({});
    try {
      const result = await window.api.listHistory({ workspaceId: wsId, filePath: fp, limit: 100 });
      setEntries(result.entries);
      setTotal(result.total);
    } catch {
      setEntries([]);
      setTotal(0);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    if (filePath !== prevPathRef.current) {
      prevPathRef.current = filePath;
      load(filePath, workspaceId);
    }
  }, [filePath, workspaceId, load]);

  // Initial load
  useEffect(() => {
    load(filePath, workspaceId);
    prevPathRef.current = filePath;
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  // Reload when reloadKey increments (e.g. after a save)
  useEffect(() => {
    if (reloadKey === undefined || reloadKey === 0) return;
    load(filePath, workspaceId);
  }, [reloadKey]); // eslint-disable-line react-hooks/exhaustive-deps

  const handleExpand = async (entry: AuditEntry) => {
    if (expandedHash === entry.commitHash) {
      setExpandedHash(null);
      return;
    }
    setExpandedHash(entry.commitHash);
    if (diffs[entry.commitHash]) return;
    setDiffLoading(entry.commitHash);
    try {
      const result = await window.api.historyDiff(entry.commitHash, filePath, workspaceId);
      setDiffs((prev) => ({ ...prev, [entry.commitHash]: result }));
    } catch {
      setDiffs((prev) => ({ ...prev, [entry.commitHash]: { before: null, after: null } }));
    } finally {
      setDiffLoading(null);
    }
  };

  const fileName = filePath.split("/").pop() ?? filePath;

  const KIND_LABELS: Record<string, string> = {
    requests:     strings.history.kindRequest,
    mocks:        strings.history.kindMock,
    sockets:      strings.history.kindSocket,
    environments: strings.history.kindEnvironment,
    mappings:     strings.history.kindMapping,
    rules:        strings.history.kindRule,
  };
  const firstSegment = filePath.split("/")[0] ?? "";
  const kindLabel = KIND_LABELS[firstSegment] ?? strings.history.kindEntity;
  const historyTitle = strings.history.title.replace("{kind}", kindLabel);

  return (
    <div
      className="history-sidebar"
      style={{ width: open ? 280 : 0, opacity: open ? 1 : 0 }}
    >
    <div className="flex flex-col h-full bg-surface border-l border-border" style={{ width: 280, minWidth: 280 }}>
      {/* Header */}
      <div className="flex items-center gap-2 px-3 py-2.5 border-b border-border flex-shrink-0">
        <GitCommit size={13} className="text-signal flex-shrink-0" />
        <div className="flex-1 min-w-0">
          <div className="text-xs font-semibold text-foreground truncate">{historyTitle}</div>
          <div className="text-[10px] text-muted-foreground truncate font-mono" title={filePath}>{fileName}</div>
        </div>
        <button
          onClick={onClose}
          title={strings.history.closeHistory}
          className="w-6 h-6 flex items-center justify-center rounded hover:bg-card text-muted-foreground hover:text-foreground transition-colors flex-shrink-0 cursor-pointer"
        >
          <X size={12} />
        </button>
      </div>

      {/* Entry count */}
      {!loading && total > 0 && (
        <div className="px-3 py-1.5 border-b border-border flex-shrink-0 bg-background/30">
          <span className="text-[10px] text-muted-foreground">{total} {strings.history.commit.replace("{s}", total !== 1 ? "s" : "")}</span>
        </div>
      )}

      {/* List */}
      <div className="flex-1 overflow-y-auto">
        {loading && (
          <div className="flex items-center justify-center h-20 text-xs text-muted-foreground">{strings.history.loading}</div>
        )}

        {!loading && entries.length === 0 && (
          <div className="flex flex-col items-center justify-center h-24 px-4 text-center">
            <p className="text-xs text-muted-foreground">{strings.history.noHistory}</p>
          </div>
        )}

        {entries.map((entry, i) => {
          const canExpand = entry.action === "update";
          const isExpanded = canExpand && expandedHash === entry.commitHash;
          const isDiffLoading = diffLoading === entry.commitHash;
          const entryDiff = diffs[entry.commitHash];
          const isFirst = i === 0;

          const rowContent = (
            <div className="w-full flex items-center gap-2 px-3 py-2">
              {/* Timeline dot */}
              <div className="flex-shrink-0" style={{ width: 8 }}>
                <div className={`w-2 h-2 rounded-full border ${isFirst ? "border-signal bg-signal/30" : "border-border bg-surface-2"}`} />
              </div>

              {/* Action badge */}
              <span
                className={`text-[9px] font-semibold px-1 py-px rounded uppercase tracking-wide flex-shrink-0 ${ACTION_COLORS[entry.action] ?? "bg-surface-2 text-muted-foreground border border-border"}`}
              >
                {entry.action}
              </span>

              {/* Committer */}
              <span
                className="text-[9px] font-mono bg-card border border-border px-1 py-px rounded text-muted-foreground truncate flex-shrink-0 max-w-[72px]"
                title={entry.actor}
              >
                {entry.actor || strings.history.localActor}
              </span>

              {/* Changed fields or spacer */}
              {entry.action === "update" && entry.changedFields && entry.changedFields.length > 0 ? (
                <span
                  className="text-[10px] text-muted-foreground font-mono truncate flex-1 min-w-0"
                  title={entry.changedFields.map(formatFieldLabel).join(", ")}
                >
                  {entry.changedFields.slice(0, 3).map(formatFieldLabel).join(", ")}
                  {entry.changedFields.length > 3 ? ` +${entry.changedFields.length - 3}` : ""}
                </span>
              ) : (
                <span className="flex-1" />
              )}

              {/* Time */}
              <div className="flex-shrink-0">
                <span className="text-[9px] text-muted-foreground" title={absoluteTime(entry.ts)}>
                  {relativeTime(entry.ts)}
                </span>
              </div>

              {/* Chevron - update only */}
              {canExpand && (
                <span className="text-muted-foreground flex-shrink-0">
                  {isExpanded ? <ChevronDown size={10} /> : <ChevronRight size={10} />}
                </span>
              )}
            </div>
          );

          return (
            <div key={entry.commitHash} className="border-b border-border/50 last:border-0">
              {canExpand ? (
                <button
                  onClick={() => handleExpand(entry)}
                  className="w-full hover:bg-card transition-colors text-left cursor-pointer"
                >
                  {rowContent}
                </button>
              ) : (
                <div className="select-text">{rowContent}</div>
              )}

              {/* Expanded diff - update only */}
              {isExpanded && (
                <div className="px-3 pb-3 pt-1 bg-background/40">
                  <div className="text-[9px] font-mono text-muted-foreground/60 mb-1.5 select-all">{entry.commitHash}</div>
                  {isDiffLoading ? (
                    <div className="text-[10px] text-muted-foreground py-1">{strings.history.loadingDiff}</div>
                  ) : entryDiff ? (
                    <CompactDiff before={entryDiff.before} after={entryDiff.after} />
                  ) : null}
                </div>
              )}
            </div>
          );
        })}
      </div>
    </div>
    </div>
  );
}

// -- Compact diff view ---------------------------------------------------------

interface CompactDiffProps {
  before: unknown | null;
  after: unknown | null;
}

function CompactDiff({ before, after }: CompactDiffProps) {
  const [showAll, setShowAll] = useState(false);

  const beforeObj = (before && typeof before === "object") ? (before as Record<string, unknown>) : {};
  const afterObj  = (after  && typeof after  === "object") ? (after  as Record<string, unknown>) : {};

  if (!before && !after) {
    return <p className="text-[10px] text-muted-foreground">{strings.history.noSnapshot}</p>;
  }

  if (before && !after) {
    return <p className="text-[10px] text-muted-foreground italic">{strings.history.deletedPrior}</p>;
  }

  if (!before && after) {
    return <p className="text-[10px] text-muted-foreground italic">{strings.history.createdNoPrior}</p>;
  }

  const allKeys = Array.from(new Set([...Object.keys(beforeObj), ...Object.keys(afterObj)]))
    .filter((k) => !k.startsWith("_") && k !== "id" && k !== "workspaceId" && k !== "createdAt");

  const changed = allKeys.filter((k) => JSON.stringify(beforeObj[k]) !== JSON.stringify(afterObj[k]));
  const unchanged = allKeys.filter((k) => !changed.includes(k));

  if (changed.length === 0) {
    return <p className="text-[10px] text-muted-foreground">{strings.history.noFieldChanges}</p>;
  }

  const displayKeys = showAll ? allKeys : changed;

  return (
    <div className="flex flex-col gap-1 text-[10px] font-mono">
      {displayKeys.map((k) => {
        const isChanged = changed.includes(k);
        const bVal = JSON.stringify(beforeObj[k] ?? null);
        const aVal = JSON.stringify(afterObj[k]  ?? null);
        return (
          <div key={k} className={`${!isChanged ? "opacity-40" : ""}`}>
            <span className="text-[9px] uppercase text-muted-foreground tracking-wider block mb-0.5">{k}</span>
            {isChanged && (
              <>
                <div className="bg-destructive/5 border border-destructive/20 rounded px-1.5 py-0.5 text-destructive line-through opacity-70 truncate mb-0.5">
                  {bVal}
                </div>
                <div className="bg-signal/5 border border-signal/20 rounded px-1.5 py-0.5 text-signal truncate">
                  {aVal}
                </div>
              </>
            )}
          </div>
        );
      })}
      {unchanged.length > 0 && (
        <button
          className="text-[9px] text-muted-foreground underline text-left mt-0.5 cursor-pointer"
          onClick={() => setShowAll((v) => !v)}
        >
          {showAll ? strings.history.hideUnchanged.replace("{n}", String(unchanged.length)) : strings.history.showUnchanged.replace("{n}", String(unchanged.length))}
        </button>
      )}
    </div>
  );
}
