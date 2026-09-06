import React, { useState, useMemo, useCallback } from "react";
import { AppConfig, ProxyRule } from "@/types";
import RuleTab, { RuleTabHandle, RuleSavePayload } from "@/components/rules/RuleTab";
import FolderTree, { FolderTreeItem } from "@/components/sidebar/FolderTree";
import DraftsFolder from "@/components/sidebar/DraftsFolder";
import { loadDraft } from "@/lib/useDraftPersist";
import { useEntityTabs } from "@/lib/useEntityTabs";
import { strings } from "@/lib/strings";
import { entityRelPath, calculateFolderStatus } from "@/lib/utils";
import { Settings } from "@/lib/icons";
import TabBar from "@/components/editor/TabBar";
import { SidebarLayout, SidebarHeader } from "@/components/ui";
import { usePersistedState } from "@/lib/usePersistedState";
import { useTabKeyBindings } from "@/hooks/useTabKeyBindings";

const DRAFT_PREFIX = "rule-draft-";

interface Props {
  config: AppConfig;
  onConfigChange: (cfg: AppConfig) => Promise<void>;
  onHistoryOpen?: (filePath: string) => void;
  entitySyncStatus?: Record<string, "clean" | "modified" | "new" | "deleted">;
  onPublishItem?: (id: string) => void;
  onPublishFolder?: (folderId: string | null) => void;
  onRestoreItem?: (id: string) => void;
}

export default function ProxyRulesPanel({
  config, onConfigChange, onHistoryOpen, entitySyncStatus, onPublishItem, onPublishFolder, onRestoreItem,
}: Props) {
  const rules = config.proxyRules ?? [];
  const folders = config.ruleFolders ?? [];

  const [sidebarOpen, setSidebarOpen] = usePersistedState(`rules:${config.activeWorkspaceId}:sidebar-open`, true);

  const {
    openTabs, activeTab, setActiveTab,
    loadedEntities, setLoadedEntities,
    tabRefs, isDraft,
    openTab, openNewTab, closeTab, replaceTab, reorderTabs, closeOtherTabs, closeAllTabs,
  } = useEntityTabs<ProxyRule>({
    storageKey: "rules",
    draftPrefix: DRAFT_PREFIX,
    workspaceId: config.activeWorkspaceId,
    entityKind: "rules",
    entities: rules,
  });

  useTabKeyBindings({ activeTab, tabRefs, closeTab, openNewTab });

  const reloadRules = useCallback(async () => {
    const fresh = await window.api.getConfig();
    await onConfigChange(fresh);
  }, [onConfigChange]);

  const handleToggle = useCallback(async (rule: ProxyRule) => {
    if (isDraft(rule.id)) return; // drafts cannot be enabled
    await window.api.setEntityEnabled(config.activeWorkspaceId, "rules", rule.id, !rule.enabled);
    await reloadRules();
  }, [rules, isDraft, config.activeWorkspaceId, reloadRules]);

  const handleToggleFolderItems = useCallback(async (folderId: string | null, enable: boolean) => {
    const descendantFolderIds = new Set<string | null>([folderId]);
    const queue = folders.filter((f) => (f.parentId ?? null) === folderId);
    while (queue.length) {
      const f = queue.shift()!;
      descendantFolderIds.add(f.id);
      folders.filter((c) => (c.parentId ?? null) === f.id).forEach((c) => queue.push(c));
    }
    const affected = rules.filter((r) => descendantFolderIds.has(r.folderId ?? null) && !isDraft(r.id));
    for (const stub of affected) {
      if (stub.enabled !== enable) await window.api.setEntityEnabled(config.activeWorkspaceId, "rules", stub.id, enable);
    }
    await reloadRules();
  }, [rules, folders, isDraft, config.activeWorkspaceId, reloadRules]);

  const handleNewRuleSave = useCallback(async (tabId: string, data: RuleSavePayload) => {
    const created = await window.api.addRule({
      ...data,
      workspaceId: config.activeWorkspaceId,
      enabled: false,
    } as Omit<ProxyRule, "id" | "createdAt" | "workspaceId">);
    await reloadRules();
    replaceTab(tabId, created.id);
    return created;
  }, [rules.length, config.activeWorkspaceId, reloadRules, replaceTab]);

  const handleTabSave = useCallback(async (tabId: string, data: RuleSavePayload) => {
    const stub = rules.find((r) => r.id === tabId);
    const existing = loadedEntities[tabId] ?? (stub ? await window.api.loadEntity(config.activeWorkspaceId, "rules", tabId)
      .then((r) => r.ok && r.entity ? r.entity as ProxyRule : null) : null);
    if (!existing) return;
    const updated: ProxyRule = { ...existing, ...data };
    setLoadedEntities((prev) => ({ ...prev, [tabId]: updated }));
    await window.api.updateRule(updated);
    await reloadRules();
    return updated;
  }, [rules, loadedEntities, config.activeWorkspaceId, setLoadedEntities, reloadRules]);

  const handleDelete = useCallback(async (id: string) => {
    await window.api.deleteRule(id);
    await reloadRules();
    closeTab(id);
  }, [reloadRules, closeTab]);

  const handleDuplicate = useCallback(async (id: string) => {
    const full = loadedEntities[id] ?? await window.api.loadEntity(config.activeWorkspaceId, "rules", id)
      .then((r) => r.ok && r.entity ? r.entity as ProxyRule : null);
    if (!full) return;
    const { id: _id, createdAt: _ca, workspaceId: _ws, ...rest } = full;
    await window.api.addRule({ ...rest, name: full.name ? `${full.name}${strings.proxyRules.copySuffix}` : "", enabled: false });
    await reloadRules();
  }, [loadedEntities, config.activeWorkspaceId, reloadRules]);

  const handleMoveItems = useCallback(async (ids: string[], folderId: string | null) => {
    for (const id of ids) {
      let full = loadedEntities[id] ?? await window.api.loadEntity(config.activeWorkspaceId, "rules", id)
        .then((r) => r.ok && r.entity ? r.entity as ProxyRule : null);
      if (!full) continue;
      await window.api.updateRule({ ...full, folderId: folderId ?? undefined });
    }
    await reloadRules();
  }, [loadedEntities, config.activeWorkspaceId, reloadRules]);

  const handleFoldersChange = useCallback(async () => {
    const fresh = await window.api.getConfig();
    await onConfigChange(fresh);
  }, [onConfigChange]);

  // -- Label helpers ------------------------------------------------------

  interface RuleDraftSnapshot { name?: string; pattern?: string; }

  const tabLabel = (tabId: string) => {
    if (isDraft(tabId)) {
      const draft = loadDraft<RuleDraftSnapshot>(tabId);
      if (draft?.name) return draft.name;
      if (draft?.pattern) return draft.pattern.slice(0, 30);
      return strings.proxyRules.newRule;
    }
    const r = rules.find((x) => x.id === tabId);
    if (!r) return "…";
    return r.name || r.pattern.slice(0, 30) || strings.proxyRules.newRule;
  };

  // -- Folder tree items --------------------------------------------------

  const folderViewItems: FolderTreeItem[] = useMemo(() => {
    return rules.map((r): FolderTreeItem => ({
      id: r.id,
      name: r.name || r.pattern.slice(0, 40) || strings.proxyRules.newRule,
      folderId: r.folderId ?? null,
      isActive: activeTab === r.id,
      isEnabled: r.enabled,
      relPath: entityRelPath("rules", r, folders),
    }));
  }, [rules, folders, activeTab]);

  const folderStatusMap = useMemo(() => calculateFolderStatus(rules, folders), [rules, folders]);

  const draftTabIds = openTabs.filter(isDraft);

  // -- Sidebar ------------------------------------------------------------

  const sidebarContent = (
    <>
      <SidebarHeader onCollapse={() => setSidebarOpen(false)} collapseTitle={strings.proxyRules.collapseSidebar}>
        <span className="text-xs font-semibold px-1 text-muted-foreground uppercase tracking-wider">
          {strings.nav.proxyRules}
        </span>
      </SidebarHeader>
      <div className="flex-1 overflow-y-auto overflow-x-auto min-w-0" style={{ display: "flex", flexDirection: "column" }}>
        {draftTabIds.length > 0 && (
          <DraftsFolder
            label={strings.proxyRules.drafts}
            draftTabIds={draftTabIds}
            activeTab={activeTab}
            onOpenTab={(id) => setActiveTab(id)}
            onCloseTab={closeTab}
            tabLabel={tabLabel}
          />
        )}
        <FolderTree
          kind="rule"
          folders={folders}
          items={folderViewItems}
          folderStatusMap={folderStatusMap}
          onOpenItem={openTab}
          onDeleteItem={handleDelete}
          onToggleItem={(id) => { const r = rules.find((x) => x.id === id); if (r) handleToggle(r); }}
          onToggleFolderItems={handleToggleFolderItems}
          onFoldersChange={handleFoldersChange}
          onDuplicateItem={handleDuplicate}
          onMoveItems={handleMoveItems}
          onOpenNewTab={openNewTab}
          onBeforeCreateFolder={() => true}
          onHistoryItem={onHistoryOpen ? (id) => {
            const r = rules.find((x) => x.id === id);
            if (r) onHistoryOpen(entityRelPath("rules", r, folders));
          } : undefined}
          pathStatusMap={entitySyncStatus}
          onPublishItem={onPublishItem}
          onPublishFolder={onPublishFolder}
          onRestoreItem={onRestoreItem}
        />
      </div>
    </>
  );

  // -- Main content -------------------------------------------------------

  const mainContent = (
    <div className="flex flex-col flex-1 overflow-hidden min-w-0 h-full">
      <TabBar
        tabs={openTabs.map((id) => ({ id, label: tabLabel(id), isDraft: isDraft(id) }))}
        activeTab={activeTab}
        onTabClick={setActiveTab}
        onTabClose={closeTab}
        onNewTab={openNewTab}
        onReorderTabs={reorderTabs}
        newTabTitle={strings.proxyRules.newTab}
        closeTabTitle={strings.proxyRules.closeTab}
        onCloseOthers={closeOtherTabs}
        onCloseAll={closeAllTabs}
        onTabDuplicate={handleDuplicate}
      />
      <div className="flex-1 overflow-hidden relative">
        {openTabs.length === 0 ? (
          <div className="flex flex-col items-center justify-center h-full text-center gap-2">
            <div className="opacity-10 mb-1"><Settings size={48} /></div>
            <div className="text-sm font-medium text-foreground">{strings.proxyRules.noRulesOpen}</div>
            <p className="text-xs text-muted-foreground max-w-xs leading-relaxed">
              {strings.proxyRules.noRulesOpenHintPrefix} <span className="text-signal font-semibold">+</span> {strings.proxyRules.noRulesOpenHintSuffix}
            </p>
          </div>
        ) : (
          openTabs.map((tabId) => {
            const isUnsaved = isDraft(tabId);
            const rule = isUnsaved ? null : (loadedEntities[tabId] ?? rules.find((r) => r.id === tabId) ?? null);
            if (!isUnsaved && !rule) return null;
            const relPath = rule ? entityRelPath("rules", rule, folders) : "";
            const syncStatus = relPath ? entitySyncStatus?.[relPath] : undefined;
            return (
              <div key={tabId} className="absolute inset-0 flex flex-col overflow-hidden" style={{ display: activeTab === tabId ? "flex" : "none" }}>
                <RuleTab
                  ref={(el: RuleTabHandle | null) => { (tabRefs as React.MutableRefObject<Record<string, RuleTabHandle | null>>).current[tabId] = el; }}
                  tabId={tabId}
                  draftTabId={isUnsaved ? tabId : null}
                  initial={rule}
                  folders={folders}
                  config={config}
                  onSave={(data) => isUnsaved
                    ? handleNewRuleSave(tabId, data)
                    : handleTabSave(tabId, data)
                  }
                  onClose={() => closeTab(tabId)}
                  enabled={isUnsaved ? undefined : rules.find((r) => r.id === tabId)?.enabled}
                  onToggleEnabled={isUnsaved ? undefined : () => { const r = rules.find((x) => x.id === tabId); if (r) handleToggle(r); }}
                  onSync={onPublishItem ? async (savedId?: string) => {
                    const targetId = savedId ?? tabId;
                    await onPublishItem(targetId);
                  } : undefined}
                  onRevert={onRestoreItem ? async () => {
                    await onRestoreItem(tabId);
                    const res = await window.api.loadEntity(config.activeWorkspaceId, "rules", tabId);
                    if (res.ok && res.entity) {
                      const entity = res.entity as ProxyRule;
                      setLoadedEntities((prev) => ({ ...prev, [tabId]: entity }));
                      (tabRefs as React.MutableRefObject<Record<string, RuleTabHandle | null>>).current[tabId]?.refresh(entity);
                    } else if (!res.ok) {
                      closeTab(tabId);
                    }
                  } : undefined}
                  onHistory={onHistoryOpen && relPath && !isUnsaved ? () => onHistoryOpen(relPath) : undefined}
                  syncStatus={syncStatus}
                />
              </div>
            );
          })
        )}
      </div>
    </div>
  );

  return (
    <>
      <SidebarLayout
        sidebarOpen={sidebarOpen}
        onToggle={() => setSidebarOpen(true)}
        sidebar={sidebarContent}
        collapseTitle={strings.proxyRules.collapseSidebar}
        expandTitle={strings.proxyRules.expandSidebar}
        storageKey="proxy-rules-panel-sidebar"
        collapsedBadge={rules.length > 0 ? (
          <span className="text-[9px] text-muted-foreground font-mono" title={strings.proxyRules.rulesCountTitle.replace("{count}", String(rules.length))}
            style={{ writingMode: "vertical-rl", transform: "rotate(180deg)", lineHeight: 1.4 }}>{rules.length}</span>
        ) : undefined}
      >
        {mainContent}
      </SidebarLayout>
    </>
  );
}
