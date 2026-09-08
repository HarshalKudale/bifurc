import { useCallback, useState, useEffect, useMemo } from "react";
import { AppConfig, SavedRequest, Environment, ApiProtocol } from "@/types";
import { useMultiplexedEntities } from "@/hooks/useMultiplexedEntities";
import { useEntityTabs } from "@/hooks/useEntityTabs";
import { usePersistedState } from "@/hooks/usePersistedState";
import { loadDraft } from "@/hooks/useDraftPersist";
import { strings } from "@/lib/strings";

const DRAFT_PREFIX = "req-draft-";
const RUNNER_PREFIX = "runner-";

export function useRequestsPanelState({
  config,
  onConfigChange,
  pendingOpenRequest,
  onPendingConsumed,
  onEntityPathChange,
  historyOpen,
  onAfterSave,
}: {
  config: AppConfig;
  onConfigChange: (cfg: AppConfig) => Promise<void>;
  pendingOpenRequest?: Omit<SavedRequest, "id" | "createdAt" | "workspaceId"> | null;
  onPendingConsumed?: () => void;
  onEntityPathChange?: (filePath: string) => void;
  historyOpen?: boolean;
  onAfterSave?: () => void;
}) {
  const reloadRequests = useCallback(async () => {
    const fresh = await window.api.getConfig();
    await onConfigChange(fresh);
  }, [onConfigChange]);

  const multiplex = useMultiplexedEntities({
    mode: "request",
    config,
    loadedEntities: {}, 
    reload: reloadRequests,
  });

  const {
    folders,
    itemProtocolMap,
    allItemsMap,
    allEntities,
    resolveEntityKind,
    handleSaveEntity: _handleSaveEntity,
    handleDelete: _handleDelete,
    handleDeleteItems: _handleDeleteItems,
    handleDuplicate,
    handleMoveItems,
  } = multiplex;

  const [sidebarOpen, setSidebarOpen] = usePersistedState(`requests:${config.activeWorkspaceId}:sidebar-open`, true);
  const [selectedFolderId, setSelectedFolderId] = usePersistedState<string | null>(`requests:${config.activeWorkspaceId}:selected-folder`, null);

  const [draftProtocols, setDraftProtocols] = usePersistedState<Record<string, ApiProtocol | null>>(
    `requests:${config.activeWorkspaceId}:draft-protocols`,
    {}
  );

  const tabs = useEntityTabs<any>({
    storageKey: "requests",
    draftPrefix: DRAFT_PREFIX,
    extraDraftPrefixes: ["pending-", "gql-req-draft-", "grpc-req-draft-", "soap-req-draft-", RUNNER_PREFIX],
    workspaceId: config.activeWorkspaceId,
    entityKind: "requests",
    resolveEntityKind,
    entities: allEntities,
  });

  const {
    openTabs,
    activeTab,
    setActiveTab,
    loadedEntities,
    setLoadedEntities,
    tabRefs,
    isDraft,
    openTab,
    closeTab,
    replaceTab,
  } = tabs;
  const isRunner = (id: string) => id.startsWith(RUNNER_PREFIX);

  const [newTabInitials, setNewTabInitials] = useState<Record<string, { folderId?: string | null }>>({});
  const [dirtyTabs, setDirtyTabs] = useState<Record<string, boolean>>({});

  const openNewTabInFolder = useCallback(() => {
    const tabId = `${DRAFT_PREFIX}${Date.now()}`;
    if (selectedFolderId) {
      setNewTabInitials((prev) => ({ ...prev, [tabId]: { folderId: selectedFolderId } }));
    }
    setDraftProtocols((prev) => ({ ...prev, [tabId]: null }));
    openTab(tabId);
  }, [openTab, selectedFolderId, setDraftProtocols]);

  const [pendingData, setPendingData] = useState<Record<string, Partial<SavedRequest>>>({});
  useEffect(() => {
    if (!pendingOpenRequest) return;
    const tabId = `pending-${Date.now()}`;
    setPendingData((prev) => ({ ...prev, [tabId]: pendingOpenRequest }));
    setDraftProtocols((prev) => ({ ...prev, [tabId]: "rest" }));
    openTab(tabId);
    onPendingConsumed?.();
  }, [pendingOpenRequest, openTab, onPendingConsumed, setDraftProtocols]);

  const getEntityFilePath = useCallback((tabId: string): string => {
    if (isDraft(tabId) || isRunner(tabId)) return "";
    const item = allItemsMap.get(tabId);
    return item?.relPath ?? "";
  }, [allItemsMap, isDraft, isRunner]);

  useEffect(() => {
    if (!historyOpen || !activeTab) return;
    const path = getEntityFilePath(activeTab);
    if (path) onEntityPathChange?.(path);
  }, [activeTab, historyOpen, getEntityFilePath, onEntityPathChange]);

  const handleFoldersChange = useCallback(async () => {
    const fresh = await window.api.getConfig();
    await onConfigChange(fresh);
  }, [onConfigChange]);

  const handleMoveFolder = useCallback(async (folderId: string, targetParentId: string | null) => {
    await window.api.moveFolder("request", folderId, targetParentId);
    await handleFoldersChange();
  }, [handleFoldersChange]);

  const handleSaveEntity = useCallback(
    async (tabId: string, protocol: ApiProtocol, data: any) => {
      const isNew = isDraft(tabId);
      const saved = await _handleSaveEntity(tabId, protocol, data, isNew, loadedEntities);

      if (isNew && saved?.id) {
        replaceTab(tabId, saved.id);
        setPendingData((prev) => { const next = { ...prev }; delete next[tabId]; return next; });
        setNewTabInitials((prev) => { const next = { ...prev }; delete next[tabId]; return next; });
        setDraftProtocols((prev) => { const next = { ...prev }; delete next[tabId]; return next; });
      }
      onAfterSave?.();
      return saved;
    },
    [_handleSaveEntity, loadedEntities, replaceTab, onAfterSave, setDraftProtocols, isDraft]
  );

  const handleDelete = useCallback(async (id: string) => await _handleDelete(id, closeTab), [_handleDelete, closeTab]);
  const handleDeleteItems = useCallback(async (t: string[], u: string[]) => await _handleDeleteItems(t, u, closeTab), [_handleDeleteItems, closeTab]);
  const handleBeforeDeleteFolder = useCallback((folderId: string) => allEntities.filter((m: any) => m.folderId === folderId).forEach((m: any) => closeTab(m.id)), [allEntities, closeTab]);
  const handleDuplicateImpl = useCallback(async (id: string) => await handleDuplicate(id, loadedEntities), [handleDuplicate, loadedEntities]);
  const handleMoveItemsImpl = useCallback(async (ids: string[], folderId: string | null) => await handleMoveItems(ids, folderId, loadedEntities), [handleMoveItems, loadedEntities]);

  const [runnerFolderIds, setRunnerFolderIds] = useState<Record<string, string>>({});
  const handleOpenRunner = useCallback((folderId: string) => {
    const tabId = `${RUNNER_PREFIX}${folderId}`;
    setRunnerFolderIds((prev) => ({ ...prev, [tabId]: folderId }));
    openTab(tabId);
  }, [openTab]);

  const draftTabIds = openTabs.filter(isDraft).filter((id) => !isRunner(id));

  const tabLabel = (tabId: string) => {
    if (isRunner(tabId)) {
      const folderId = runnerFolderIds[tabId];
      const f = folders.find((x) => x.id === folderId);
      return f ? `${f.name} Runner` : "Runner";
    }
    if (isDraft(tabId)) {
      if (tabId.startsWith("pending-")) {
        const pd = pendingData[tabId];
        if (!pd) return strings.requests.newRequest;
        if (pd.name) return pd.name;
        if (pd.method && pd.url) {
          try { const u = new URL(pd.url); const last = u.pathname.split("/").filter(Boolean).pop() ?? u.host; return `${pd.method} /${last}`; }
          catch { return `${pd.method} ${(pd.url ?? "").slice(0, 18)}`; }
        }
        return strings.requests.newRequest;
      }
      const proto = draftProtocols[tabId];
      if (!proto) return "New Request";
      const draft = loadDraft<any>(tabId);
      if (draft?.name) return draft.name;
      if (proto === "rest" && draft?.url) {
        try { const u = new URL(draft.url); const last = u.pathname.split("/").filter(Boolean).pop() ?? u.host; return `${draft.method ?? "GET"} /${last}`; }
        catch { return `${draft.method ?? "GET"} ${(draft.url ?? "").slice(0, 18)}`; }
      }
      if (proto === "graphql") return draft?.operationName || "New GraphQL Request";
      if (proto === "grpc") return draft?.methodName ? `${draft.serviceName || ""}/${draft.methodName}` : "New gRPC Request";
      if (proto === "soap") return draft?.operationName || "New SOAP Request";
      return `New ${proto.toUpperCase()} Request`;
    }
    const item = allItemsMap.get(tabId);
    if (!item) return "…";
    if (item.name) return item.name;
    return item.summary || "Request";
  };

  const tabBadge = (tabId: string) => {
    if (isRunner(tabId)) return "▶";
    if (isDraft(tabId)) {
      const proto = draftProtocols[tabId];
      if (!proto) return "+";
      if (proto === "graphql") return "GQL";
      if (proto === "grpc") return "gRPC";
      if (proto === "soap") return "SOAP";
      return "REST";
    }
    const item = allItemsMap.get(tabId);
    return item?.methodBadge || "REQ";
  };

  const folderViewItems = useMemo(() => {
    const items: any[] = [];
    allItemsMap.forEach((it) => {
      items.push({
        id: it.id,
        name: it.name || it.summary,
        method: it.methodBadge,
        folderId: it.folderId ?? null,
        isActive: activeTab === it.id,
        isEnabled: true,
        relPath: it.relPath,
      });
    });
    const runnerItems = openTabs.filter(isRunner).map((id) => {
      const fId = runnerFolderIds[id];
      const f = folders.find((x) => x.id === fId);
      return {
        id,
        name: f ? `${f.name} Runner` : "Runner",
        method: "▶",
        folderId: fId ?? null,
        isActive: activeTab === id,
        isEnabled: true,
        relPath: "",
      };
    });
    return [...items, ...runnerItems];
  }, [allItemsMap, folders, activeTab, runnerFolderIds, openTabs, isRunner]);

  return {
    multiplex,
    tabs,
    sidebarOpen,
    setSidebarOpen,
    selectedFolderId,
    setSelectedFolderId,
    draftProtocols,
    setDraftProtocols,
    newTabInitials,
    dirtyTabs,
    setDirtyTabs,
    openNewTabInFolder,
    pendingData,
    getEntityFilePath,
    handleFoldersChange,
    handleMoveFolder,
    handleSaveEntity,
    handleDelete,
    handleDeleteItems,
    handleBeforeDeleteFolder,
    handleDuplicateImpl,
    handleMoveItemsImpl,
    handleOpenRunner,
    draftTabIds,
    tabLabel,
    tabBadge,
    folderViewItems,
    folders,
    itemProtocolMap,
    allItemsMap,
    allEntities,
    runnerFolderIds,
    isRunner,
    isDraft,
  };
}
