import { useCallback, useState, useMemo } from "react";
import { AppConfig, SavedWebhook, WebhookPayload } from "@/types";
import { usePersistedState } from "@/hooks/usePersistedState";
import { useEntityTabs } from "@/hooks/useEntityTabs";
import { loadDraft, clearDraft } from "@/hooks/useDraftPersist";
import { entityRelPath, calculateFolderStatus } from "@/lib/utils";
import { strings } from "@/lib/strings";

const DRAFT_PREFIX = "wh-draft-";
const isDraftId = (id: string) => id.startsWith(DRAFT_PREFIX);
const MAX_ACTIVE_WEBHOOKS = 5;

export function useWebhooksPanelState({
  config,
  onConfigChange,
  onAfterSave,
  deregisterWebhook,
}: {
  config: AppConfig;
  onConfigChange: (cfg: AppConfig) => Promise<void>;
  onAfterSave?: () => void;
  deregisterWebhook: (id: string) => void;
}) {
  const webhooks = config.webhooks ?? [];
  const folders = config.webhookFolders ?? [];
  const webhookPort = config.webhookPort ?? 9101;

  const [sidebarOpen, setSidebarOpen] = usePersistedState(`webhooks:${config.activeWorkspaceId}:sidebar-open`, true);
  const [dirtyTabs, setDirtyTabs] = useState<Record<string, boolean>>({});
  const [serverRunning, setServerRunning] = useState(false);
  const [serverError, setServerError] = useState<string | null>(null);
  const [serverLoading, setServerLoading] = useState(false);
  const [payloadMap, setPayloadMap] = useState<Record<string, WebhookPayload[]>>({});
  const [activeTabs, setActiveTabs] = useState<Set<string>>(new Set());

  const reloadWebhooks = useCallback(async () => {
    const fresh = await window.api.getConfig();
    await onConfigChange(fresh);
  }, [onConfigChange]);

  const tabs = useEntityTabs<SavedWebhook>({
    storageKey: `webhooks:${config.activeWorkspaceId}`,
    draftPrefix: DRAFT_PREFIX,
    workspaceId: config.activeWorkspaceId,
    entityKind: 'webhooks',
    entities: webhooks,
  });

  const {
    openTabs,
    activeTab,
    setActiveTab,
    loadedEntities,
    setLoadedEntities,
    tabRefs,
    openTab,
    openNewTab,
    closeTab,
    reorderTabs,
    setOpenTabs,
  } = tabs;

  const wrappedCloseTab = useCallback((tabId: string) => {
    deregisterWebhook(tabId);
    closeTab(tabId);
  }, [deregisterWebhook, closeTab]);

  const handleNewSave = useCallback(async (tabId: string, data: Omit<SavedWebhook, "id" | "createdAt" | "workspaceId">) => {
    const created = await window.api.addWebhook(data);
    await reloadWebhooks();
    setOpenTabs((prev) => [...prev.filter((id) => id !== tabId), created.id]);
    setActiveTab(created.id);
    if (isDraftId(tabId)) clearDraft(tabId);
    onAfterSave?.();
    return created;
  }, [reloadWebhooks, onAfterSave, setOpenTabs, setActiveTab]);

  const handleTabSave = useCallback(async (tabId: string, data: Omit<SavedWebhook, "id" | "createdAt" | "workspaceId">) => {
    const hook = loadedEntities[tabId] ?? webhooks.find((h) => h.id === tabId);
    if (!hook) return;
    const updated = { ...hook, ...data };
    setLoadedEntities((prev) => ({ ...prev, [tabId]: updated }));
    if (activeTabs.has(tabId) && hook.urlSuffix !== data.urlSuffix) {
      await window.api.unregisterActiveWebhook(tabId);
      await window.api.registerActiveWebhook(tabId, data.urlSuffix);
    }
    await window.api.updateWebhook(updated);
    await reloadWebhooks();
    onAfterSave?.();
    return updated;
  }, [loadedEntities, webhooks, activeTabs, reloadWebhooks, onAfterSave, setLoadedEntities]);

  const handleDelete = useCallback(async (id: string) => {
    deregisterWebhook(id);
    await window.api.deleteWebhook(id);
    await reloadWebhooks();
    wrappedCloseTab(id);
  }, [deregisterWebhook, reloadWebhooks, wrappedCloseTab]);

  const handleDuplicate = useCallback(async (id: string) => {
    let h = loadedEntities[id];
    if (!h) {
      const res = await window.api.loadEntity(config.activeWorkspaceId, "webhooks", id);
      if (res.ok && res.entity) h = res.entity as SavedWebhook;
    }
    if (!h) return;
    const { id: _id, createdAt: _ca, workspaceId: _ws, ...rest } = h;
    await window.api.addWebhook({ ...rest, name: h.name ? `${h.name} (copy)` : "", urlSuffix: "" });
    await reloadWebhooks();
  }, [loadedEntities, config.activeWorkspaceId, reloadWebhooks]);

  const handleMoveItems = useCallback(async (ids: string[], folderId: string | null) => {
    for (const id of ids) {
      let h = loadedEntities[id] ?? webhooks.find((x) => x.id === id);
      if (!h) {
        const res = await window.api.loadEntity(config.activeWorkspaceId, "webhooks", id);
        if (res.ok && res.entity) h = res.entity as SavedWebhook;
      }
      if (h) await window.api.updateWebhook({ ...h, folderId: folderId ?? undefined });
    }
    await reloadWebhooks();
  }, [loadedEntities, webhooks, config.activeWorkspaceId, reloadWebhooks]);

  const handleFoldersChange = useCallback(async () => {
    await reloadWebhooks();
  }, [reloadWebhooks]);

  const handleMoveFolder = useCallback(async (folderId: string, targetParentId: string | null) => {
    await window.api.moveFolder("webhook", folderId, targetParentId);
    await reloadWebhooks();
  }, [reloadWebhooks]);

  const getEntityFilePath = useCallback((id: string) => {
    if (isDraftId(id)) return "";
    const h = webhooks.find((x) => x.id === id);
    return h ? entityRelPath("webhooks", h, folders) : "";
  }, [webhooks, folders]);

  const tabLabel = (tabId: string) => {
    if (isDraftId(tabId)) {
      const d = loadDraft<any>(tabId);
      return d?.name || d?.urlSuffix || strings.webhooks.newWebhook;
    }
    const h = webhooks.find((x) => x.id === tabId);
    if (!h) return "…";
    return h.name || h.urlSuffix || strings.webhooks.webhook;
  };

  const folderViewItems = useMemo(() =>
    webhooks.map((h): any => ({
      id: h.id,
      name: h.name || h.urlSuffix || strings.webhooks.webhook,
      folderId: h.folderId ?? null,
      isActive: activeTab === h.id,
      isEnabled: activeTabs.has(h.id),
      relPath: entityRelPath("webhooks", h, folders),
    })),
    [webhooks, folders, activeTab, activeTabs],
  );

  const folderStatusMap = useMemo(() => {
    const itemsWithEnabled = webhooks.map((h) => ({
      ...h,
      isEnabled: activeTabs.has(h.id),
    }));
    return calculateFolderStatus(itemsWithEnabled, folders);
  }, [webhooks, folders, activeTabs]);

  return {
    webhooks,
    folders,
    webhookPort,
    sidebarOpen,
    setSidebarOpen,
    dirtyTabs,
    setDirtyTabs,
    serverRunning,
    setServerRunning,
    serverError,
    setServerError,
    serverLoading,
    setServerLoading,
    payloadMap,
    setPayloadMap,
    activeTabs,
    setActiveTabs,
    tabs,
    wrappedCloseTab,
    handleNewSave,
    handleTabSave,
    handleDelete,
    handleDuplicate,
    handleMoveItems,
    handleFoldersChange,
    handleMoveFolder,
    getEntityFilePath,
    tabLabel,
    folderViewItems,
    folderStatusMap,
    isDraftId,
    MAX_ACTIVE_WEBHOOKS,
  };
}
