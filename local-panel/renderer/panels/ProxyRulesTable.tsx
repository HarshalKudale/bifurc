import React, { useMemo } from "react";
import { ProxyRule, Folder } from "@/types";
import { strings } from "@/lib/strings";
import { entityRelPath } from "@/lib/utils";
import { Settings, History, Copy, Trash2 } from "@/lib/icons";
import { Button, IconButton, DataTable, EmptyState, StatusDot, Switch } from "@/components/ui";
import type { TableColumn } from "@/components/ui";

interface Props {
  rules: ProxyRule[];
  folders: Folder[];
  selectedRuleId: string | null;
  isDetailsOpen: boolean;
  entitySyncStatus?: Record<string, "clean" | "modified" | "new" | "deleted">;
  onSelectRule: (id: string) => void;
  onToggleRule: (rule: ProxyRule, enabled?: boolean) => void;
  onPublishItem?: (id: string) => void;
  onRestoreItem?: (id: string) => void;
  onHistoryOpen?: (filePath: string) => void;
  onDuplicate: (id: string) => void;
  onDelete: (id: string) => void;
  onOpenAdd: () => void;
}

const syncDotColor = (syncSt: string | undefined) => {
  if (syncSt === "new" || syncSt === "deleted") return "red" as const;
  if (syncSt === "modified") return "yellow" as const;
  return "green" as const;
};

export default function ProxyRulesTable({
  rules,
  folders,
  selectedRuleId,
  isDetailsOpen,
  entitySyncStatus,
  onSelectRule,
  onToggleRule,
  onPublishItem,
  onRestoreItem,
  onHistoryOpen,
  onDuplicate,
  onDelete,
  onOpenAdd,
}: Props) {
  const getSyncStatus = (r: ProxyRule) => {
    const relPath = entityRelPath("rules", r, folders);
    return entitySyncStatus?.[relPath];
  };

  const columns: TableColumn<ProxyRule>[] = useMemo(() => {
    if (isDetailsOpen) {
      return [
        {
          key: "nameAndPattern",
          header: strings.proxyRules.columnName,
          render: (r) => {
            const syncSt = getSyncStatus(r);
            return (
              <div className="flex items-center gap-2 min-w-0">
                {syncSt && <StatusDot color={syncDotColor(syncSt)} />}
                <div className="flex flex-col gap-0.5 min-w-0">
                  <span className="text-xs font-medium text-foreground truncate">{r.name || "Untitled Rule"}</span>
                  <div className="flex items-center gap-1.5 min-w-0">
                    <span
                      className={`text-[9px] font-mono px-1 py-0.2 rounded uppercase font-semibold flex-shrink-0 ${
                        r.useRegex ? "bg-signal/15 text-signal" : "bg-surface-2 text-muted-foreground"
                      }`}
                    >
                      {r.useRegex ? "regex" : "exact"}
                    </span>
                    <span className="text-[11px] font-mono text-muted-foreground truncate" title={r.pattern}>
                      {r.pattern}
                    </span>
                  </div>
                </div>
              </div>
            );
          },
        },
        {
          key: "on",
          header: strings.mappings.columnOn,
          align: "center",
          width: "w-14",
          render: (r) => (
            <div className="flex items-center justify-center" onClick={(e) => e.stopPropagation()}>
              <Switch checked={r.enabled} onChange={(v) => onToggleRule(r, v)} />
            </div>
          ),
        },
      ];
    }

    return [
      {
        key: "name",
        header: strings.proxyRules.columnName,
        render: (r) => {
          const syncSt = getSyncStatus(r);
          return (
            <div className="flex items-center gap-2 min-w-0">
              {syncSt && <StatusDot color={syncDotColor(syncSt)} />}
              <span className="text-xs font-medium text-foreground truncate">{r.name || "—"}</span>
            </div>
          );
        },
      },
      {
        key: "pattern",
        header: strings.proxyRules.columnPattern,
        render: (r) => (
          <div className="flex items-center gap-2 min-w-0">
            <span
              className={`text-[10px] font-mono px-1.5 py-0.5 rounded uppercase font-semibold tracking-wider flex-shrink-0 ${
                r.useRegex
                  ? "bg-signal/15 text-signal border border-signal/30"
                  : "bg-surface-2 text-muted-foreground border border-border"
              }`}
            >
              {r.useRegex ? "regex" : "exact"}
            </span>
            <span className="font-mono text-xs text-foreground truncate max-w-md" title={r.pattern}>
              {r.pattern}
            </span>
          </div>
        ),
      },
      {
        key: "on",
        header: strings.mappings.columnOn,
        align: "center",
        width: "w-14",
        render: (r) => (
          <div className="flex items-center justify-center" onClick={(e) => e.stopPropagation()}>
            <Switch checked={r.enabled} onChange={(v) => onToggleRule(r, v)} />
          </div>
        ),
      },
      {
        key: "actions",
        width: "w-32",
        render: (r) => {
          const syncSt = getSyncStatus(r);
          const relPath = entityRelPath("rules", r, folders);
          return (
            <div className="flex items-center justify-end gap-1" onClick={(e) => e.stopPropagation()}>
              {onPublishItem && syncSt && syncSt !== "clean" && (
                <Button variant="ghost" size="sm" onClick={() => onPublishItem(r.id)}>
                  {strings.mappings.publish}
                </Button>
              )}
              {onRestoreItem && syncSt && syncSt !== "clean" && (
                <button
                  onClick={() => onRestoreItem(r.id)}
                  className="px-2.5 py-1 rounded text-amber hover:bg-amber/10 text-xs font-medium transition-all cursor-pointer"
                >
                  {strings.mappings.revert}
                </button>
              )}
              {onHistoryOpen && (
                <IconButton
                  icon={<History size={11} />}
                  title={strings.mappings.viewHistory}
                  onClick={() => onHistoryOpen(relPath)}
                  className="hover:text-signal"
                />
              )}
              <IconButton
                icon={<Copy size={12} />}
                title={strings.folderTree.duplicate}
                onClick={() => onDuplicate(r.id)}
                className="hover:text-signal"
              />
              <IconButton
                icon={<Trash2 size={12} />}
                title={strings.common.delete}
                onClick={() => onDelete(r.id)}
                className="hover:text-destructive"
              />
            </div>
          );
        },
      },
    ];
  }, [isDetailsOpen, folders, entitySyncStatus, onPublishItem, onRestoreItem, onHistoryOpen, onToggleRule, onDuplicate, onDelete]);

  const emptyNode = (
    <EmptyState
      icon={<Settings size={36} />}
      title={strings.proxyRules.noRulesYet}
      description={strings.proxyRules.noRulesYetHint}
      action={
        <Button variant="primary" onClick={onOpenAdd}>
          {strings.proxyRules.addRule}
        </Button>
      }
    />
  );

  return (
    <DataTable
      columns={columns}
      data={rules}
      rowKey={(r) => r.id}
      emptyState={emptyNode}
      onRowClick={(r) => onSelectRule(r.id)}
      rowClassName={(r) =>
        r.id === selectedRuleId && isDetailsOpen ? "bg-card border-l-2 border-l-signal" : ""
      }
    />
  );
}
