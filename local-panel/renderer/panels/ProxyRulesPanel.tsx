import React, { useState, useEffect, useMemo, useCallback } from "react";
import { AppConfig, ProxyRule } from "@/types";
import PanelHeader from "@/components/layout/PanelHeader";
import ProxyRuleDetailsPanel, { RuleSavePayload } from "@/components/rules/ProxyRuleDetailsPanel";
import { strings } from "@/lib/strings";
import { entityRelPath } from "@/lib/utils";
import { Settings, History, Copy, Trash2 } from "@/lib/icons";
import { Button, IconButton, DataTable, EmptyState, StatusDot, Switch } from "@/components/ui";
import { useConfirmDialog } from "@/hooks/useConfirmDialog";
import type { TableColumn } from "@/components/ui";

interface Props {
  config: AppConfig;
  onConfigChange: (cfg: AppConfig) => Promise<void>;
  onHistoryOpen?: (filePath: string) => void;
  entitySyncStatus?: Record<string, "clean" | "modified" | "new" | "deleted">;
  onPublishItem?: (id: string) => void;
  onPublishFolder?: (folderId: string | null) => void;
  onRestoreItem?: (id: string) => void;
  pendingRuleId?: string | null;
  onPendingRuleConsumed?: () => void;
}

export default function ProxyRulesPanel({
  config,
  onConfigChange,
  onHistoryOpen,
  entitySyncStatus,
  onPublishItem,
  onRestoreItem,
  pendingRuleId,
  onPendingRuleConsumed,
}: Props) {
  const { confirm, ConfirmDialogElement } = useConfirmDialog();
  const rules = config.proxyRules ?? [];
  const folders = config.ruleFolders ?? [];

  const [selectedRuleId, setSelectedRuleId] = useState<string | null>(null);
  const [isCreatingNew, setIsCreatingNew] = useState(false);
  const [loadedEntities, setLoadedEntities] = useState<Record<string, ProxyRule>>({});

  // Auto-select rule from search or navigation
  useEffect(() => {
    if (pendingRuleId) {
      setSelectedRuleId(pendingRuleId);
      setIsCreatingNew(false);
      onPendingRuleConsumed?.();
    }
  }, [pendingRuleId, onPendingRuleConsumed]);

  // Listen to localpanel:select-rule event
  useEffect(() => {
    const handler = (e: Event) => {
      const custom = e as CustomEvent<{ ruleId?: string }>;
      if (custom.detail?.ruleId) {
        setSelectedRuleId(custom.detail.ruleId);
        setIsCreatingNew(false);
      }
    };
    window.addEventListener("localpanel:select-rule", handler);
    return () => window.removeEventListener("localpanel:select-rule", handler);
  }, []);

  // Reload rules from config
  const reloadRules = useCallback(async () => {
    const fresh = await window.api.getConfig();
    await onConfigChange(fresh);
  }, [onConfigChange]);

  // Load full entity data whenever selectedRuleId changes
  useEffect(() => {
    if (!selectedRuleId) return;
    let cancelled = false;
    window.api
      .loadEntity(config.activeWorkspaceId, "rules", selectedRuleId)
      .then((res) => {
        if (!cancelled && res.ok && res.entity) {
          setLoadedEntities((prev) => ({ ...prev, [selectedRuleId]: res.entity as ProxyRule }));
        }
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [selectedRuleId, config.activeWorkspaceId]);

  // Clean up selectedRuleId if the rule was removed
  useEffect(() => {
    if (selectedRuleId && !rules.some((r) => r.id === selectedRuleId) && !isCreatingNew) {
      setSelectedRuleId(null);
    }
  }, [rules, selectedRuleId, isCreatingNew]);

  const handleToggle = useCallback(
    async (rule: ProxyRule, enabled?: boolean) => {
      const nextEnabled = enabled !== undefined ? enabled : !rule.enabled;
      await window.api.setEntityEnabled(config.activeWorkspaceId, "rules", rule.id, nextEnabled);
      await reloadRules();
    },
    [config.activeWorkspaceId, reloadRules]
  );

  const handleOpenAdd = () => {
    setSelectedRuleId(null);
    setIsCreatingNew(true);
  };

  const handleSelectRule = (id: string) => {
    setSelectedRuleId(id);
    setIsCreatingNew(false);
  };

  const handleCloseDetails = () => {
    setSelectedRuleId(null);
    setIsCreatingNew(false);
  };

  const handleDelete = async (id: string) => {
    const ok = await confirm(strings.proxyRules.deleteConfirm);
    if (!ok) return;
    await window.api.deleteRule(id);
    if (selectedRuleId === id) {
      setSelectedRuleId(null);
      setIsCreatingNew(false);
    }
    await reloadRules();
  };

  const handleDuplicate = async (id: string) => {
    const full =
      loadedEntities[id] ??
      (await window.api
        .loadEntity(config.activeWorkspaceId, "rules", id)
        .then((r) => (r.ok && r.entity ? (r.entity as ProxyRule) : null))) ??
      rules.find((r) => r.id === id);

    if (!full) return;
    const { id: _id, createdAt: _ca, workspaceId: _ws, ...rest } = full;
    const newRule = await window.api.addRule({
      ...rest,
      name: full.name ? `${full.name}${strings.proxyRules.copySuffix}` : "",
      enabled: false,
      workspaceId: config.activeWorkspaceId,
    });
    await reloadRules();
    setSelectedRuleId(newRule.id);
    setIsCreatingNew(false);
  };

  const handleSaveRule = async (data: RuleSavePayload) => {
    if (isCreatingNew) {
      const created = await window.api.addRule({
        ...data,
        workspaceId: config.activeWorkspaceId,
        enabled: false,
      } as Omit<ProxyRule, "id" | "createdAt" | "workspaceId">);
      await reloadRules();
      setLoadedEntities((prev) => ({ ...prev, [created.id]: created }));
      setSelectedRuleId(created.id);
      setIsCreatingNew(false);
      return created;
    } else if (selectedRuleId) {
      const existing =
        loadedEntities[selectedRuleId] ??
        (await window.api
          .loadEntity(config.activeWorkspaceId, "rules", selectedRuleId)
          .then((r) => (r.ok && r.entity ? (r.entity as ProxyRule) : null))) ??
        rules.find((r) => r.id === selectedRuleId);

      const updated: ProxyRule = {
        ...(existing ?? {}),
        ...data,
        id: selectedRuleId,
        workspaceId: config.activeWorkspaceId,
        enabled: existing?.enabled ?? false,
        createdAt: existing?.createdAt ?? Date.now(),
      };
      setLoadedEntities((prev) => ({ ...prev, [selectedRuleId]: updated }));
      await window.api.updateRule(updated);
      await reloadRules();
      return updated;
    }
  };

  const syncDotColor = (syncSt: string | undefined) => {
    if (syncSt === "new" || syncSt === "deleted") return "red" as const;
    if (syncSt === "modified") return "yellow" as const;
    return "green" as const;
  };

  const getSyncStatus = (r: ProxyRule) => {
    const relPath = entityRelPath("rules", r, folders);
    return entitySyncStatus?.[relPath];
  };

  const isDetailsOpen = isCreatingNew || selectedRuleId !== null;

  // Currently selected rule for details panel
  const activeRule = useMemo(() => {
    if (isCreatingNew) return null;
    if (!selectedRuleId) return null;
    return loadedEntities[selectedRuleId] ?? rules.find((r) => r.id === selectedRuleId) ?? null;
  }, [isCreatingNew, selectedRuleId, loadedEntities, rules]);

  // -- Column definitions (dynamically adjust when right panel is open) --
  const columns: TableColumn<ProxyRule>[] = useMemo(() => {
    if (isDetailsOpen) {
      // Streamlined columns when right side panel is open
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
              <Switch checked={r.enabled} onChange={(v) => handleToggle(r, v)} />
            </div>
          ),
        },
      ];
    }

    // Full columns when right side panel is closed
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
            <Switch checked={r.enabled} onChange={(v) => handleToggle(r, v)} />
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
                onClick={() => handleDuplicate(r.id)}
                className="hover:text-signal"
              />
              <IconButton
                icon={<Trash2 size={12} />}
                title={strings.common.delete}
                onClick={() => handleDelete(r.id)}
                className="hover:text-destructive"
              />
            </div>
          );
        },
      },
    ];
  }, [
    isDetailsOpen,
    folders,
    entitySyncStatus,
    onPublishItem,
    onRestoreItem,
    onHistoryOpen,
    handleToggle,
  ]);

  const emptyNode = (
    <EmptyState
      icon={<Settings size={36} />}
      title={strings.proxyRules.noRulesYet}
      description={strings.proxyRules.noRulesYetHint}
      action={
        <Button variant="primary" onClick={handleOpenAdd}>
          {strings.proxyRules.addRule}
        </Button>
      }
    />
  );

  const selectedRuleSyncStatus = activeRule ? getSyncStatus(activeRule) : undefined;
  const selectedRuleRelPath = activeRule ? entityRelPath("rules", activeRule, folders) : "";

  return (
    <>
      {ConfirmDialogElement}
      <div className="flex flex-1 overflow-hidden h-full">
        {/* Rules Table Area */}
        <div className="flex flex-col flex-1 overflow-hidden min-w-0">
          <PanelHeader
            title={strings.proxyRules.title}
            subtitle={strings.proxyRules.subtitle}
            actions={
              <Button variant="primary" onClick={handleOpenAdd}>
                {strings.proxyRules.addRule}
              </Button>
            }
          />

          <div className="flex-1 overflow-y-auto p-6">
            <DataTable
              columns={columns}
              data={rules}
              rowKey={(r) => r.id}
              emptyState={emptyNode}
              onRowClick={(r) => handleSelectRule(r.id)}
              rowClassName={(r) =>
                r.id === selectedRuleId && isDetailsOpen ? "bg-card border-l-2 border-l-signal" : ""
              }
            />
          </div>
        </div>

        {/* Right-Side Proxy Details Panel */}
        {isDetailsOpen && (
          <aside className="w-[500px] max-w-[50vw] min-w-[360px] border-l border-border bg-surface flex flex-col h-full flex-shrink-0 z-10 shadow-lg">
            <ProxyRuleDetailsPanel
              rule={activeRule}
              isNew={isCreatingNew}
              config={config}
              onSave={handleSaveRule}
              onClose={handleCloseDetails}
              onDelete={activeRule ? () => handleDelete(activeRule.id) : undefined}
              onDuplicate={activeRule ? () => handleDuplicate(activeRule.id) : undefined}
              enabled={activeRule?.enabled}
              onToggleEnabled={activeRule ? () => handleToggle(activeRule) : undefined}
              onSync={onPublishItem && activeRule ? async () => onPublishItem(activeRule.id) : undefined}
              onRevert={onRestoreItem && activeRule ? async () => onRestoreItem(activeRule.id) : undefined}
              onHistory={
                onHistoryOpen && selectedRuleRelPath
                  ? () => onHistoryOpen(selectedRuleRelPath)
                  : undefined
              }
              syncStatus={selectedRuleSyncStatus}
            />
          </aside>
        )}
      </div>
    </>
  );
}
