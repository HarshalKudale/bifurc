import { useCallback, useState, useEffect, useMemo } from "react";
import { AppConfig, MockRule, Environment, ApiProtocol } from "@/types";
import { useMultiplexedEntities } from "@/hooks/useMultiplexedEntities";
import { useEntityTabs } from "@/hooks/useEntityTabs";
import { usePersistedState } from "@/hooks/usePersistedState";
import { loadDraft } from "@/hooks/useDraftPersist";
import { strings } from "@/lib/strings";
import { calculateFolderStatus, methodColor, methodBg } from "@/lib/utils";
import { findBlocksFolder, ensureBlocksFolderId, buildBlockMock } from "@/lib/blocks";

const DRAFT_PREFIX = "mock-draft-";

export function useMocksPanelState({
  config,
  onConfigChange,
  pendingMockInitial,
  onPendingConsumed,
  onEntityPathChange,
  historyOpen,
  onAfterSave,
}: {
  config: AppConfig;
  onConfigChange: (cfg: AppConfig) => Promise<void>;
  pendingMockInitial?: Partial<MockRule> | null;
  onPendingConsumed?: () => void;
  onEntityPathChange?: (filePath: string) => void;
  historyOpen?: boolean;
  onAfterSave?: () => void;
}) {
  const reloadMocks = useCallback(async () => {
    const fresh = await window.api.getConfig();
    await onConfigChange(fresh);
  }, [onConfigChange]);

  const multiplex = useMultiplexedEntities({
    mode: "mock",
    config,
    reload: reloadMocks,
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
    restItems: restMocks,
  } = multiplex;

  const [sidebarOpen, setSidebarOpen] = usePersistedState(`mocks:${config.activeWorkspaceId}:sidebar-open`, true);
  const [selectedFolderId, setSelectedFolderId] = usePersistedState<string | null>(`mocks:${config.activeWorkspaceId}:selected-folder`, null);

  const [draftProtocols, setDraftProtocols] = usePersistedState<Record<string, ApiProtocol | null>>(
    `mocks:${config.activeWorkspaceId}:draft-protocols`,
    {}
  );

  const tabs = useEntityTabs<any>({
    storageKey: "mocks",
    draftPrefix: DRAFT_PREFIX,
    extraDraftPrefixes: ["prefill-", "gql-mock-draft-", "grpc-mock-draft-", "soap-mock-draft-"],
    workspaceId: config.activeWorkspaceId,
    entityKind: "mocks",
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

  const [prefillData, setPrefillData] = useState<Record<string, Partial<MockRule>>>({});

  useEffect(() => {
    if (!pendingMockInitial) return;
    const tabId = `prefill-${Date.now()}`;
    setPrefillData((prev) => ({ ...prev, [tabId]: pendingMockInitial }));
    setDraftProtocols((prev) => ({ ...prev, [tabId]: "rest" }));
    openTab(tabId);
    onPendingConsumed?.();
  }, [pendingMockInitial, openTab, onPendingConsumed, setDraftProtocols]);

  const getEntityFilePath = useCallback((tabId: string): string => {
    if (isDraft(tabId)) return "";
    const item = allItemsMap.get(tabId);
    return item?.relPath ?? "";
  }, [allItemsMap, isDraft]);

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
    await window.api.moveFolder("mock", folderId, targetParentId);
    await handleFoldersChange();
  }, [handleFoldersChange]);

  const handleToggle = useCallback(
    async (id: string) => {
      const item = allItemsMap.get(id);
      if (!item) return;
      const kind = resolveEntityKind(id);
      await window.api.setEntityEnabled(config.activeWorkspaceId, kind, id, !item.enabled);
      await reloadMocks();
    },
    [allItemsMap, resolveEntityKind, config.activeWorkspaceId, reloadMocks]
  );

  const handleToggleFolderItems = useCallback(
    async (folderId: string | null, enable: boolean) => {
      const descendantFolderIds = new Set<string | null>([folderId]);
      const queue = folders.filter((f) => (f.parentId ?? null) === folderId);
      while (queue.length) {
        const f = queue.shift()!;
        descendantFolderIds.add(f.id);
        folders.filter((c) => (c.parentId ?? null) === f.id).forEach((c) => queue.push(c));
      }

      allItemsMap.forEach((item) => {
        if (descendantFolderIds.has(item.folderId ?? null) && item.enabled !== enable) {
          const kind = resolveEntityKind(item.id);
          window.api.setEntityEnabled(config.activeWorkspaceId, kind, item.id, enable);
        }
      });
      await reloadMocks();
    },
    [folders, allItemsMap, resolveEntityKind, config.activeWorkspaceId, reloadMocks]
  );

  const handleSaveEntity = useCallback(
    async (tabId: string, protocol: ApiProtocol, data: any) => {
      const isNew = isDraft(tabId);
      const saved = await _handleSaveEntity(tabId, protocol, data, isNew, loadedEntities);

      if (isNew && saved?.id) {
        replaceTab(tabId, saved.id);
        setPrefillData((prev) => { const next = { ...prev }; delete next[tabId]; return next; });
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

  const blocksFolder = useMemo(() => findBlocksFolder(folders), [folders]);
  const handleBlockItem = useCallback(async (id: string) => {
    const m = restMocks.find((x) => x.id === id);
    if (!m) return;
    const folderId = await ensureBlocksFolderId(folders);
    await window.api.deleteMock(id);
    closeTab(id);
    await window.api.addMock(buildBlockMock(m.method, m.urlPattern, folderId));
    await reloadMocks();
  }, [restMocks, folders, closeTab, reloadMocks]);

  const handleUnblockItem = useCallback(async (id: string) => {
    closeTab(id);
    await window.api.deleteMock(id);
    await reloadMocks();
  }, [closeTab, reloadMocks]);

  const draftTabIds = openTabs.filter(isDraft);

  const tabLabel = (tabId: string) => {
    if (isDraft(tabId)) {
      if (tabId.startsWith("prefill-")) {
        const pf = prefillData[tabId];
        if (!pf) return strings.mocks.newMock;
        if (pf.name) return pf.name;
        if (pf.method && pf.urlPattern) {
          try { const u = new URL(pf.urlPattern); const last = u.pathname.split("/").filter(Boolean).pop() ?? u.host; return `${pf.method} /${last}`; }
          catch { return `${pf.method} ${(pf.urlPattern ?? "").slice(0, 18)}`; }
        }
        return strings.mocks.newMock;
      }
      const proto = draftProtocols[tabId];
      if (!proto) return "New Mock";
      const draft = loadDraft<any>(tabId);
      if (draft?.name) return draft.name;
      if (proto === "rest" && draft?.urlPattern) {
        try { const u = new URL(draft.urlPattern); const last = u.pathname.split("/").filter(Boolean).pop() ?? u.host; return `${draft.method ?? "GET"} /${last}`; }
        catch { return `${draft.method ?? "GET"} ${(draft.urlPattern ?? "").slice(0, 18)}`; }
      }
      if (proto === "graphql") return draft?.operationName || "New GraphQL Mock";
      if (proto === "grpc") return draft?.methodName ? `${draft.serviceName || ""}/${draft.methodName}` : "New gRPC Mock";
      if (proto === "soap") return draft?.operationName || "New SOAP Mock";
      return `New ${proto.toUpperCase()} Mock`;
    }
    const item = allItemsMap.get(tabId);
    if (!item) return "…";
    if (item.name) return item.name;
    return item.summary || "Mock";
  };

  const tabBadge = (tabId: string) => {
    if (isDraft(tabId)) {
      const proto = draftProtocols[tabId];
      if (!proto) return "+";
      if (proto === "graphql") return "GQL";
      if (proto === "grpc") return "gRPC";
      if (proto === "soap") return "SOAP";
      return "REST";
    }
    const item = allItemsMap.get(tabId);
    return item?.methodBadge || "MOCK";
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
        isEnabled: it.enabled,
        isBlock: !!blocksFolder && it.folderId === blocksFolder.id,
        relPath: it.relPath,
      });
    });
    return items;
  }, [allItemsMap, blocksFolder, activeTab]);

  const folderStatusMap = useMemo(() => calculateFolderStatus(allEntities as any, folders), [allEntities, folders]);

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
    prefillData,
    getEntityFilePath,
    handleFoldersChange,
    handleMoveFolder,
    handleToggle,
    handleToggleFolderItems,
    handleSaveEntity,
    handleDelete,
    handleDeleteItems,
    handleBeforeDeleteFolder,
    handleDuplicateImpl,
    handleMoveItemsImpl,
    blocksFolder,
    handleBlockItem,
    handleUnblockItem,
    draftTabIds,
    tabLabel,
    tabBadge,
    folderViewItems,
    folderStatusMap,
    folders,
    itemProtocolMap,
    allItemsMap,
    allEntities,
  };
}
