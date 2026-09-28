import React, { useMemo } from "react";
import { ProxyRule, Folder } from "@/types";
import { strings } from "@/lib/strings";
import { entityRelPath } from "@/lib/utils";
import { Settings, History, Copy, Trash2 } from "@/lib/icons";
import { Button, IconButton, DataTable, EmptyState, StatusDot, Switch } from "@/components/ui";
import type { TableColumn } from "@/components/ui";
import { entityGitView, entityGitDotColor, type EntityGitView } from "@/lib/entityGit";

interface Props {
  rules: ProxyRule[];
  folders: Folder[];
  selectedRuleId: string | null;
  isDetailsOpen: boolean;
  entitySyncStatus?: Record<string, "clean" | "modified" | "new" | "deleted">;
  /** The workspace's position relative to its remote — needed for "committed but not pushed". */
  workspaceGitState?: import("@/types").WorkspaceGitState | null;
  onSelectRule: (id: string) => void;
  onToggleRule: (rule: ProxyRule, enabled?: boolean) => void;
  onPublishItem?: (id: string) => void;
  onRestoreItem?: (id: string) => void;
  onHistoryOpen?: (filePath: string) => void;
  onDuplicate: (id: string) => void;
  onDelete: (id: string) => void;
  onOpenAdd: () => void;
}

/** Delegates to the one implementation — see `lib/entityGit.ts`. */
const syncDotColor = (syncSt: string | undefined) => entityGitDotColor(syncSt as any);

export default function ProxyRulesTable({
  rules,
  folders,
  selectedRuleId,
  isDetailsOpen,
  entitySyncStatus,
  workspaceGitState,
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

  /**
   * The uniform git view for a rule — what the row's dot, its action buttons and its History gate all
   * read. The table used to compute three different conditions from the raw status, and its History
   * button was rendered **unconditionally**, so a never-published rule opened an empty History panel.
   */
  const getView = (r: ProxyRule): EntityGitView =>
    entityGitView(getSyncStatus(r), workspaceGitState);

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
          const view = getView(r);
          const relPath = entityRelPath("rules", r, folders);
          return (
            <div className="flex items-center justify-end gap-1" onClick={(e) => e.stopPropagation()}>
              {/* One primary button, labelled Publish or Sync by the rule's state. */}
              {onPublishItem && view.primary && (
                <Button variant="ghost" size="sm" title={view.primaryTitle} onClick={() => onPublishItem(r.id)}>
                  {view.primaryLabel}
                </Button>
              )}
              {onRestoreItem && view.canRevert && (
                <button
                  onClick={() => onRestoreItem(r.id)}
                  title={strings.entityGit.revertTooltip}
                  className="px-2.5 py-1 rounded text-amber hover:bg-amber/10 text-xs font-medium transition-all cursor-pointer"
                >
                  {strings.entityGit.revert}
                </button>
              )}
              {/* History only for a rule that has commits — it used to render unconditionally. */}
              {onHistoryOpen && view.canViewHistory && (
                <IconButton
                  icon={<History size={11} />}
                  title={strings.entityGit.historyTooltip}
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
