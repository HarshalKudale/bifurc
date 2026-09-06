import React, { useState, useEffect, useMemo, useCallback } from "react";
import { AppConfig, ProxyRule } from "@/types";
import PanelHeader from "@/components/layout/PanelHeader";
import ProxyRuleDetailsPanel, { RuleSavePayload } from "@/components/rules/ProxyRuleDetailsPanel";
import ProxyRulesTable from "./ProxyRulesTable";
import { strings } from "@/lib/strings";
import { entityRelPath } from "@/lib/utils";
import { Button } from "@/components/ui";
import { useConfirmDialog } from "@/hooks/useConfirmDialog";

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

  const getSyncStatus = useCallback((r: ProxyRule) => {
    const relPath = entityRelPath("rules", r, folders);
    return entitySyncStatus?.[relPath];
  }, [entitySyncStatus, folders]);

  const isDetailsOpen = isCreatingNew || !!selectedRuleId;

  const activeRule: ProxyRule | null = useMemo(() => {
    if (isCreatingNew) return null;
    if (!selectedRuleId) return null;
    return loadedEntities[selectedRuleId] ?? rules.find((r) => r.id === selectedRuleId) ?? null;
  }, [isCreatingNew, selectedRuleId, loadedEntities, rules]);

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
            <ProxyRulesTable
              rules={rules}
              folders={folders}
              selectedRuleId={selectedRuleId}
              isDetailsOpen={isDetailsOpen}
              entitySyncStatus={entitySyncStatus}
              onSelectRule={handleSelectRule}
              onToggleRule={handleToggle}
              onPublishItem={onPublishItem}
              onRestoreItem={onRestoreItem}
              onHistoryOpen={onHistoryOpen}
              onDuplicate={handleDuplicate}
              onDelete={handleDelete}
              onOpenAdd={handleOpenAdd}
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
