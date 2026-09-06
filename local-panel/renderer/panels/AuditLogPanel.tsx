import React, { useState, useEffect, useCallback, useRef } from "react";
import { AuditEntry, AuditListOptions, AuditAction, AuditEntity } from "@/types";
import { ChevronRight, ChevronDown, Download } from "@/lib/icons";
import { formatFieldLabel } from "@/lib/utils";
import { strings } from "@/lib/strings";
import CodeEditor from "@/components/common/CodeEditor";
import { Button, Input, Select } from "@/components/ui";


interface Props {
  activeWorkspaceId: string;
  embedded?: boolean;
}

const ENTITY_OPTIONS: { value: AuditEntity | ""; label: string }[] = [
  { value: "", label: strings.auditLog.allEntities },
  { value: "mock", label: strings.auditLog.entityMock },
  { value: "mapping", label: strings.auditLog.entityMapping },
  { value: "rule", label: strings.auditLog.entityRule },
  { value: "environment", label: strings.auditLog.entityEnvironment },
  { value: "request", label: strings.auditLog.entityRequest },
  { value: "wsConnection", label: strings.auditLog.entityWebSocket },
  { value: "folder", label: strings.auditLog.entityFolder },
  { value: "workspace", label: strings.auditLog.entityWorkspace },
];

const ACTION_OPTIONS: { value: AuditAction | ""; label: string }[] = [
  { value: "", label: strings.auditLog.allActions },
  { value: "create", label: strings.auditLog.actionCreate },
  { value: "update", label: strings.auditLog.actionUpdate },
  { value: "delete", label: strings.auditLog.actionDelete },
];

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

export default function AuditLogPanel({ activeWorkspaceId, embedded }: Props) {
  const [entries, setEntries] = useState<AuditEntry[]>([]);
  const [total, setTotal] = useState(0);
  const [loading, setLoading] = useState(false);
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const [diff, setDiff] = useState<Record<string, { before: unknown | null; after: unknown | null }>>({});
  const [diffLoading, setDiffLoading] = useState<string | null>(null);
  const [exporting, setExporting] = useState(false);


  // Filter state
  const [entityFilter, setEntityFilter] = useState<AuditEntity | "">("");
  const [actionFilter, setActionFilter] = useState<AuditAction | "">("");
  const [searchText, setSearchText] = useState("");
  const [fromDate, setFromDate] = useState("");
  const [toDate, setToDate] = useState("");
  const [offset, setOffset] = useState(0);
  const LIMIT = 100;

  const load = useCallback(async (newOffset = 0) => {
    setLoading(true);
    try {
      const opts: AuditListOptions = {
        workspaceId: activeWorkspaceId,
        limit: LIMIT,
        offset: newOffset,
      };
      if (entityFilter) opts.entity = entityFilter;
      if (actionFilter) opts.action = actionFilter;
      if (searchText.trim()) opts.search = searchText.trim();
      if (fromDate) opts.fromTs = new Date(fromDate).getTime();
      if (toDate) opts.toTs = new Date(toDate + "T23:59:59").getTime();

      const result = await window.api.listAudit(opts);
      setEntries(result.entries);
      setTotal(result.total);
      setOffset(newOffset);
    } catch {
      // ignore
    } finally {
      setLoading(false);
    }
  }, [activeWorkspaceId, entityFilter, actionFilter, searchText, fromDate, toDate]);

  useEffect(() => {
    load(0);
  }, [load]);

  const handleExpand = async (entry: AuditEntry) => {
    const key = entry.commitHash;
    if (expandedId === key) {
      setExpandedId(null);
      return;
    }
    setExpandedId(key);
    if (diff[key]) return;
    setDiffLoading(key);
    try {
      const result = await window.api.auditDiff(
        entry.commitHash,
        entry.entity,
        entry.entityId,
        entry.workspaceId,
      );
      setDiff((prev) => ({ ...prev, [key]: result }));
    } catch {
      setDiff((prev) => ({ ...prev, [key]: { before: null, after: null } }));
    } finally {
      setDiffLoading(null);
    }
  };

  const handleExport = async (format: "json" | "csv") => {
    setExporting(true);
    try {
      await window.api.exportAudit(format);
    } finally {
      setExporting(false);
    }
  };

  return (
    <>
      <div className="flex flex-col flex-1 overflow-hidden min-w-0">
        {/* Header */}
        <div className={`${embedded ? "px-4 py-3" : "px-6 py-4"} border-b border-border flex items-center gap-3 flex-shrink-0 bg-surface/50`}>
          <div className="flex-1 min-w-0">
            <h1 className={`${embedded ? "text-sm" : "text-base"} font-semibold text-foreground`}>
              {strings.auditLog.title}
            </h1>
            <p className="text-xs text-muted-foreground mt-0.5">
              {embedded
                ? `${total} ${strings.auditLog.entries}`
                : `${strings.auditLog.subtitle} ${total} ${strings.auditLog.entries}`}
            </p>
          </div>
          <div className="flex items-center gap-1.5 flex-shrink-0">
            <Button variant="secondary" size="sm" icon={<Download size={11} />} onClick={() => handleExport("json")} disabled={exporting}>JSON</Button>
            <Button variant="secondary" size="sm" icon={<Download size={11} />} onClick={() => handleExport("csv")} disabled={exporting}>CSV</Button>
          </div>
        </div>

        {/* Filters */}
        <div className="flex items-center gap-2 px-4 py-2.5 border-b border-border flex-shrink-0 flex-wrap bg-surface">
          <Select inputSize="sm" value={entityFilter} onChange={(e) => setEntityFilter(e.target.value as AuditEntity | "")}>
            {ENTITY_OPTIONS.map((o) => <option key={o.value} value={o.value}>{o.label}</option>)}
          </Select>

          <Select inputSize="sm" value={actionFilter} onChange={(e) => setActionFilter(e.target.value as AuditAction | "")}>
            {ACTION_OPTIONS.map((o) => <option key={o.value} value={o.value}>{o.label}</option>)}
          </Select>

          <Input inputSize="sm" type="date" value={fromDate} onChange={(e) => setFromDate(e.target.value)} />
          <span className="text-muted-foreground text-xs">–</span>
          <Input inputSize="sm" type="date" value={toDate} onChange={(e) => setToDate(e.target.value)} />

          {(entityFilter || actionFilter || fromDate || toDate) && (
            <button
              onClick={() => { setEntityFilter(""); setActionFilter(""); setFromDate(""); setToDate(""); }}
              className="text-xs text-muted-foreground hover:text-foreground underline cursor-pointer"
            >
              Clear
            </button>
          )}
        </div>

        {/* List */}
        <div className="flex-1 overflow-y-auto">
          {loading && entries.length === 0 && (
            <div className="flex items-center justify-center h-32 text-xs text-muted-foreground">
              {strings.auditLog.loading}
            </div>
          )}

          {!loading && entries.length === 0 && (
            <div className="flex items-center justify-center h-32 text-xs text-muted-foreground">
              {strings.auditLog.noEntries}
            </div>
          )}

          {entries.map((entry) => {
            const canExpand = entry.action === "update";
            const isExpanded = canExpand && expandedId === entry.commitHash;
            const isDiffLoading = diffLoading === entry.commitHash;
            const entryDiff = diff[entry.commitHash];

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
              <div key={entry.commitHash} className="border-b border-border last:border-0">
                {canExpand ? (
                  <button
                    onClick={() => handleExpand(entry)}
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
          })}

          {/* Pagination */}
          {total > LIMIT && (
            <div className="flex items-center justify-between px-4 py-3 border-t border-border text-xs text-muted-foreground">
              <span>
                {offset + 1}–{Math.min(offset + LIMIT, total)} of {total}
              </span>
              <div className="flex gap-2">
                <Button variant="secondary" size="sm" onClick={() => load(Math.max(0, offset - LIMIT))} disabled={offset === 0}>← Prev</Button>
                <Button variant="secondary" size="sm" onClick={() => load(offset + LIMIT)} disabled={offset + LIMIT >= total}>Next →</Button>
              </div>
            </div>
          )}
        </div>
      </div>
    </>
  );
}

// -- Inline diff ----------------------------------------------------------------

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
