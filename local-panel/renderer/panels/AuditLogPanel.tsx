import React, { useState, useEffect, useCallback } from "react";
import { AuditEntry, AuditListOptions, AuditAction, AuditEntity } from "@/types";
import { Download } from "@/lib/icons";
import { strings } from "@/lib/strings";
import { Button, Input, Select } from "@/components/ui";
import { AuditLogRow } from "./AuditLogRow";

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

  const handleExpand = useCallback(async (entry: AuditEntry) => {
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
  }, [diff, expandedId]);

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

          {entries.map((entry) => (
            <AuditLogRow
              key={entry.commitHash}
              entry={entry}
              isExpanded={expandedId === entry.commitHash}
              isDiffLoading={diffLoading === entry.commitHash}
              entryDiff={diff[entry.commitHash]}
              onExpand={handleExpand}
            />
          ))}

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
