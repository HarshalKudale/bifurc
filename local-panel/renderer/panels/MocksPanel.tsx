import React, { useState, useEffect, useMemo, useCallback, useRef } from "react";
import {
  AppConfig,
  MockRule,
  SavedGraphQLMock,
  SavedGrpcMock,
  SavedSoapMock,
  Folder,
  Environment,
  ApiProtocol,
} from "@/types";
import RestTab from "@/components/rest/RestTab";
import GraphQLTab from "@/components/graphql/GraphQLTab";
import GrpcTab from "@/components/grpc/GrpcTab";
import SoapTab from "@/components/soap/SoapTab";
import ProtocolSelectorTab from "@/components/editor/ProtocolSelectorTab";
import FolderTree, { FolderTreeItem } from "@/components/sidebar/FolderTree";
import DraftsFolder from "@/components/sidebar/DraftsFolder";
import { loadDraft } from "@/lib/useDraftPersist";
import { useEntityTabs } from "@/lib/useEntityTabs";
import { strings } from "@/lib/strings";
import { entityRelPath, calculateFolderStatus, methodColor, methodBg } from "@/lib/utils";
import { findBlocksFolder, ensureBlocksFolderId, buildBlockMock } from "@/lib/blocks";
import { Zap } from "@/lib/icons";
import TabBar from "@/components/editor/TabBar";
import { SidebarLayout, SidebarHeader } from "@/components/ui";
import { useTabKeyBindings } from "@/hooks/useTabKeyBindings";
import { usePersistedState } from "@/lib/usePersistedState";

// -- Draft tab prefix -------------------------------------------------------

const DRAFT_PREFIX = "mock-draft-";
const isDraft = (id: string) =>
  id.startsWith(DRAFT_PREFIX) ||
  id.startsWith("prefill-") ||
  id.startsWith("gql-mock-draft-") ||
  id.startsWith("grpc-mock-draft-") ||
  id.startsWith("soap-mock-draft-");

// -- Props ------------------------------------------------------------------

interface Props {
  config: AppConfig;
  onConfigChange: (cfg: AppConfig) => Promise<void>;
  pendingMockInitial?: Partial<MockRule> | null;
  onPendingConsumed?: () => void;
  activeEnv?: Environment | null;
  onHistoryOpen?: (filePath: string) => void;
  onEntityPathChange?: (filePath: string) => void;
  historyOpen?: boolean;
  onAfterSave?: () => void;
  entitySyncStatus?: Record<string, "clean" | "modified" | "new" | "deleted">;
  onPublishItem?: (id: string) => void;
  onPublishFolder?: (folderId: string | null) => void;
  onRestoreItem?: (id: string) => void;
}

// -- MocksPanel -------------------------------------------------------------

export default function MocksPanel({
  config,
  onConfigChange,
  pendingMockInitial,
  onPendingConsumed,
  activeEnv = null,
  onHistoryOpen,
  onEntityPathChange,
  historyOpen = false,
  onAfterSave,
  entitySyncStatus,
  onPublishItem,
  onPublishFolder,
  onRestoreItem,
}: Props) {
  const restMocks = config.mocks ?? [];
  const graphqlMocks = config.graphqlMocks ?? [];
  const grpcMocks = config.grpcMocks ?? [];
  const soapMocks = config.soapMocks ?? [];

  // Unified folders deduplicated by ID
  const folders = useMemo(() => {
    const map = new Map<string, Folder>();
    (config.mockFolders ?? []).forEach((f) => map.set(f.id, f));
    (config.graphqlMockFolders ?? []).forEach((f) => { if (!map.has(f.id)) map.set(f.id, f); });
    (config.grpcMockFolders ?? []).forEach((f) => { if (!map.has(f.id)) map.set(f.id, f); });
    (config.soapMockFolders ?? []).forEach((f) => { if (!map.has(f.id)) map.set(f.id, f); });
    return Array.from(map.values());
  }, [
    config.mockFolders,
    config.graphqlMockFolders,
    config.grpcMockFolders,
    config.soapMockFolders,
  ]);

  // Map of entity id -> protocol
  const itemProtocolMap = useMemo(() => {
    const map = new Map<string, ApiProtocol>();
    restMocks.forEach((m) => map.set(m.id, "rest"));
    graphqlMocks.forEach((m) => map.set(m.id, "graphql"));
    grpcMocks.forEach((m) => map.set(m.id, "grpc"));
    soapMocks.forEach((m) => map.set(m.id, "soap"));
    return map;
  }, [restMocks, graphqlMocks, grpcMocks, soapMocks]);

  // Unified item metadata map for fast lookups
  const allItemsMap = useMemo(() => {
    const map = new Map<string, { id: string; name: string; folderId?: string | null; protocol: ApiProtocol; summary: string; methodBadge: string; enabled: boolean; relPath: string }>();
    restMocks.forEach((m) => {
      map.set(m.id, {
        id: m.id,
        name: m.name || "",
        folderId: m.folderId ?? null,
        protocol: "rest",
        summary: m.urlPattern || "REST Mock",
        methodBadge: m.method || "GET",
        enabled: m.enabled,
        relPath: entityRelPath("mocks", m, folders),
      });
    });
    graphqlMocks.forEach((m) => {
      map.set(m.id, {
        id: m.id,
        name: m.name || "",
        folderId: m.folderId ?? null,
        protocol: "graphql",
        summary: m.operationName ? `${m.operationType || "query"} ${m.operationName}` : (m.endpointPattern || "GraphQL Mock"),
        methodBadge: "GQL",
        enabled: m.enabled,
        relPath: entityRelPath("graphqlMocks", m, folders),
      });
    });
    grpcMocks.forEach((m) => {
      map.set(m.id, {
        id: m.id,
        name: m.name || "",
        folderId: m.folderId ?? null,
        protocol: "grpc",
        summary: m.serviceName && m.methodName ? `${m.serviceName}/${m.methodName}` : "gRPC Mock",
        methodBadge: "gRPC",
        enabled: m.enabled,
        relPath: entityRelPath("grpcMocks", m, folders),
      });
    });
    soapMocks.forEach((m) => {
      map.set(m.id, {
        id: m.id,
        name: m.name || "",
        folderId: m.folderId ?? null,
        protocol: "soap",
        summary: m.soapActionPattern || m.endpointPattern || "SOAP Mock",
        methodBadge: "SOAP",
        enabled: m.enabled,
        relPath: entityRelPath("soapMocks", m, folders),
      });
    });
    return map;
  }, [restMocks, graphqlMocks, grpcMocks, soapMocks, folders]);

  // Unified entities array for useEntityTabs
  const allEntities = useMemo(() => {
    return [
      ...restMocks,
      ...graphqlMocks,
      ...grpcMocks,
      ...soapMocks,
    ];
  }, [restMocks, graphqlMocks, grpcMocks, soapMocks]);

  const [sidebarOpen, setSidebarOpen] = usePersistedState(`mocks:${config.activeWorkspaceId}:sidebar-open`, true);
  const [selectedFolderId, setSelectedFolderId] = usePersistedState<string | null>(`mocks:${config.activeWorkspaceId}:selected-folder`, null);

  // Track draft protocols per tab ID: "rest" | "graphql" | "grpc" | "soap" | null
  const [draftProtocols, setDraftProtocols] = usePersistedState<Record<string, ApiProtocol | null>>(
    `mocks:${config.activeWorkspaceId}:draft-protocols`,
    {}
  );

  const resolveEntityKind = useCallback((id: string): string => {
    const proto = itemProtocolMap.get(id);
    if (proto === "graphql") return "graphqlMocks";
    if (proto === "grpc") return "grpcMocks";
    if (proto === "soap") return "soapMocks";
    return "mocks";
  }, [itemProtocolMap]);

  const {
    openTabs,
    activeTab,
    setActiveTab,
    loadedEntities,
    setLoadedEntities,
    tabRefs,
    isDraft,
    openTab,
    openNewTab,
    closeTab,
    replaceTab,
    reorderTabs,
    closeOtherTabs,
    closeAllTabs,
  } = useEntityTabs<any>({
    storageKey: "mocks",
    draftPrefix: DRAFT_PREFIX,
    extraDraftPrefixes: ["prefill-", "gql-mock-draft-", "grpc-mock-draft-", "soap-mock-draft-"],
    workspaceId: config.activeWorkspaceId,
    entityKind: "mocks",
    resolveEntityKind,
    entities: allEntities,
  });

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

  useTabKeyBindings({ activeTab, tabRefs, closeTab, openNewTab: openNewTabInFolder });

  const [prefillData, setPrefillData] = useState<Record<string, Partial<MockRule>>>({});

  useEffect(() => {
    if (!pendingMockInitial) return;
    const tabId = `prefill-${Date.now()}`;
    setPrefillData((prev) => ({ ...prev, [tabId]: pendingMockInitial }));
    setDraftProtocols((prev) => ({ ...prev, [tabId]: "rest" }));
    openTab(tabId);
    onPendingConsumed?.();
  }, [pendingMockInitial, openTab, onPendingConsumed, setDraftProtocols]);

  const reloadMocks = useCallback(async () => {
    const fresh = await window.api.getConfig();
    await onConfigChange(fresh);
  }, [onConfigChange]);

  const getEntityFilePath = useCallback((tabId: string): string => {
    if (isDraft(tabId)) return "";
    const item = allItemsMap.get(tabId);
    return item?.relPath ?? "";
  }, [allItemsMap]);

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

  // -- Toggle Enabled handler -------------------------------------------------

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

  // -- Save handlers by protocol ----------------------------------------------

  const handleSaveEntity = useCallback(
    async (tabId: string, protocol: ApiProtocol, data: any) => {
      const isNew = isDraft(tabId);
      let saved: any;
      if (protocol === "rest") {
        saved = isNew
          ? await window.api.addMock(data)
          : await window.api.updateMock({ ...(loadedEntities[tabId] ?? {}), ...data });
      } else if (protocol === "graphql") {
        saved = isNew
          ? await window.api.addGraphQLMock(data)
          : await window.api.updateGraphQLMock({ ...(loadedEntities[tabId] ?? {}), ...data });
      } else if (protocol === "grpc") {
        saved = isNew
          ? await window.api.addGrpcMock(data)
          : await window.api.updateGrpcMock({ ...(loadedEntities[tabId] ?? {}), ...data });
      } else if (protocol === "soap") {
        saved = isNew
          ? await window.api.addSoapMock(data)
          : await window.api.updateSoapMock({ ...(loadedEntities[tabId] ?? {}), ...data });
      }

      await reloadMocks();
      if (isNew && saved?.id) {
        replaceTab(tabId, saved.id);
        setPrefillData((prev) => { const next = { ...prev }; delete next[tabId]; return next; });
        setNewTabInitials((prev) => { const next = { ...prev }; delete next[tabId]; return next; });
        setDraftProtocols((prev) => { const next = { ...prev }; delete next[tabId]; return next; });
      }
      onAfterSave?.();
      return saved;
    },
    [loadedEntities, reloadMocks, replaceTab, onAfterSave, setDraftProtocols]
  );

  const handleDelete = useCallback(
    async (id: string) => {
      closeTab(id);
      const proto = itemProtocolMap.get(id);
      if (proto === "graphql") await window.api.deleteGraphQLMock(id);
      else if (proto === "grpc") await window.api.deleteGrpcMock(id);
      else if (proto === "soap") await window.api.deleteSoapMock(id);
      else await window.api.deleteMock(id);
      await reloadMocks();
    },
    [itemProtocolMap, reloadMocks, closeTab]
  );

  const handleDeleteItems = useCallback(
    async (trackedIds: string[], untrackedIds: string[]) => {
      const allIds = [...trackedIds, ...untrackedIds];
      allIds.forEach((id) => closeTab(id));
      for (const id of allIds) {
        const proto = itemProtocolMap.get(id);
        if (proto === "graphql") await window.api.deleteGraphQLMock(id);
        else if (proto === "grpc") await window.api.deleteGrpcMock(id);
        else if (proto === "soap") await window.api.deleteSoapMock(id);
        else await window.api.deleteMock(id);
      }
      await reloadMocks();
    },
    [itemProtocolMap, reloadMocks, closeTab]
  );

  const handleBeforeDeleteFolder = useCallback(
    (folderId: string) => {
      allEntities
        .filter((m: any) => m.folderId === folderId)
        .forEach((m: any) => closeTab(m.id));
    },
    [allEntities, closeTab]
  );

  const handleDuplicate = useCallback(
    async (id: string) => {
      const proto = itemProtocolMap.get(id) ?? "rest";
      const kind = resolveEntityKind(id);
      let item = loadedEntities[id];
      if (!item) {
        const res = await window.api.loadEntity(config.activeWorkspaceId, kind, id);
        if (res.ok && res.entity) item = res.entity;
      }
      if (!item) return;
      const { id: _id, createdAt: _ca, workspaceId: _ws, ...rest } = item;
      const copyName = item.name ? `${item.name} (copy)` : "Copy";

      if (proto === "graphql") await window.api.addGraphQLMock({ ...rest, name: copyName });
      else if (proto === "grpc") await window.api.addGrpcMock({ ...rest, name: copyName });
      else if (proto === "soap") await window.api.addSoapMock({ ...rest, name: copyName });
      else await window.api.addMock({ ...rest, name: copyName });

      await reloadMocks();
    },
    [itemProtocolMap, resolveEntityKind, loadedEntities, config.activeWorkspaceId, reloadMocks]
  );

  const handleMoveItems = useCallback(
    async (ids: string[], folderId: string | null) => {
      for (const id of ids) {
        const proto = itemProtocolMap.get(id) ?? "rest";
        const kind = resolveEntityKind(id);
        let item = loadedEntities[id];
        if (!item) {
          const res = await window.api.loadEntity(config.activeWorkspaceId, kind, id);
          if (res.ok && res.entity) item = res.entity;
        }
        if (!item) continue;
        const updated = { ...item, folderId: folderId ?? undefined };
        if (proto === "graphql") await window.api.updateGraphQLMock(updated);
        else if (proto === "grpc") await window.api.updateGrpcMock(updated);
        else if (proto === "soap") await window.api.updateSoapMock(updated);
        else await window.api.updateMock(updated);
      }
      await reloadMocks();
    },
    [itemProtocolMap, resolveEntityKind, loadedEntities, config.activeWorkspaceId, reloadMocks]
  );

  const blocksFolder = useMemo(() => findBlocksFolder(folders), [folders]);

  const handleBlockItem = useCallback(
    async (id: string) => {
      const m = restMocks.find((x) => x.id === id);
      if (!m) return;
      const folderId = await ensureBlocksFolderId(folders);
      await window.api.deleteMock(id);
      closeTab(id);
      await window.api.addMock(buildBlockMock(m.method, m.urlPattern, folderId));
      await reloadMocks();
    },
    [restMocks, folders, closeTab, reloadMocks]
  );

  const handleUnblockItem = useCallback(
    async (id: string) => {
      closeTab(id);
      await window.api.deleteMock(id);
      await reloadMocks();
    },
    [closeTab, reloadMocks]
  );

  const draftTabIds = openTabs.filter(isDraft);

  // Tab label resolution
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

  // Tab badge resolution (e.g. GET, POST, GQL, gRPC, SOAP)
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

  // Uniform folder tree items combining all 4 protocols
  const folderViewItems: FolderTreeItem[] = useMemo(() => {
    const items: FolderTreeItem[] = [];

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

  // -- Sidebar ------------------------------------------------------------

  const sidebarContent = (
    <>
      <SidebarHeader onCollapse={() => setSidebarOpen(false)} collapseTitle={strings.mocks.collapseSidebar}>
        <span className="text-xs font-semibold px-1 text-muted-foreground uppercase tracking-wider">
          {strings.nav.mocks}
        </span>
      </SidebarHeader>
      <div className="flex-1 overflow-y-auto overflow-x-auto min-w-0" style={{ display: "flex", flexDirection: "column" }}>
        {draftTabIds.length > 0 && (
          <DraftsFolder
            label={strings.mocks.drafts}
            draftTabIds={draftTabIds}
            activeTab={activeTab}
            onOpenTab={(id) => setActiveTab(id)}
            onCloseTab={closeTab}
            tabLabel={tabLabel}
          />
        )}
        <FolderTree
          kind="mock"
          folders={folders}
          items={folderViewItems}
          onOpenItem={openTab}
          onDeleteItem={handleDelete}
          onDeleteItems={handleDeleteItems}
          onToggleItem={(id) => handleToggle(id)}
          onToggleFolderItems={handleToggleFolderItems}
          onFoldersChange={handleFoldersChange}
          onDuplicateItem={handleDuplicate}
          onMoveItems={handleMoveItems}
          onMoveFolder={handleMoveFolder}
          onOpenNewTab={openNewTabInFolder}
          onSelectedFolderChange={setSelectedFolderId}
          onBeforeCreateFolder={() => true}
          onHistoryItem={onHistoryOpen ? (id) => {
            const path = getEntityFilePath(id);
            if (path) onHistoryOpen(path);
          } : undefined}
          pathStatusMap={entitySyncStatus}
          folderStatusMap={folderStatusMap}
          onPublishItem={onPublishItem}
          onPublishFolder={onPublishFolder}
          onRestoreItem={onRestoreItem}
          onBeforeDeleteFolder={handleBeforeDeleteFolder}
          blocksFolderId={blocksFolder?.id ?? null}
          onBlockItem={handleBlockItem}
          onUnblockItem={handleUnblockItem}
        />
      </div>
    </>
  );

  // -- Main content -------------------------------------------------------

  const mainContent = (
    <div className="flex flex-col flex-1 overflow-hidden min-w-0 h-full">
      <TabBar
        tabs={openTabs.map((id) => {
          const badge = tabBadge(id);
          const color = methodColor(badge);
          const bg = methodBg(badge);
          return {
            id,
            label: tabLabel(id),
            isDraft: isDraft(id),
            isModified: dirtyTabs[id],
            renderTab: () => (
              <div className="flex items-center gap-1.5 min-w-0">
                <span
                  className="text-[9px] font-bold font-mono px-1.5 py-0.5 rounded flex-shrink-0 leading-none"
                  style={{ color, background: bg }}
                >
                  {badge}
                </span>
                <span className="truncate">{tabLabel(id)}</span>
              </div>
            ),
          };
        })}
        activeTab={activeTab}
        onTabClick={setActiveTab}
        onTabClose={closeTab}
        onNewTab={openNewTabInFolder}
        onReorderTabs={reorderTabs}
        newTabTitle={strings.mocks.newTab}
        closeTabTitle={strings.mocks.closeTab}
        onCloseOthers={closeOtherTabs}
        onCloseAll={closeAllTabs}
        onTabDuplicate={handleDuplicate}
      />

      <div className="flex-1 overflow-hidden relative">
        {openTabs.length === 0 ? (
          <div className="flex flex-col items-center justify-center h-full text-center gap-2">
            <div className="opacity-10 mb-1"><Zap size={48} /></div>
            <div className="text-sm font-medium text-foreground">{strings.mocks.noMocksOpen}</div>
            <p className="text-xs text-muted-foreground max-w-xs leading-relaxed">
              {strings.mocks.noMocksOpenHint}
            </p>
          </div>
        ) : (
          openTabs.map((tabId) => {
            const isUnsaved = isDraft(tabId);
            const savedProto = itemProtocolMap.get(tabId);
            const draftProto = draftProtocols[tabId];
            const currentProto = isUnsaved ? draftProto : savedProto;

            // If draft has not yet chosen a protocol, render the ProtocolSelectorTab
            if (isUnsaved && !currentProto) {
              return (
                <div
                  key={tabId}
                  className="absolute inset-0 flex flex-col overflow-hidden"
                  style={{ display: activeTab === tabId ? "flex" : "none" }}
                >
                  <ProtocolSelectorTab
                    mode="mock"
                    onSelect={(proto) => {
                      setDraftProtocols((prev) => ({ ...prev, [tabId]: proto }));
                    }}
                  />
                </div>
              );
            }

            const activeProtocol: ApiProtocol = currentProto ?? "rest";

            const entity = isUnsaved
              ? null
              : (loadedEntities[tabId] ?? allEntities.find((m: any) => m.id === tabId) ?? null);
            const initialData = isUnsaved
              ? (prefillData[tabId] ?? newTabInitials[tabId] ?? null)
              : entity;
            if (!isUnsaved && !entity) return null;

            const itemMeta = allItemsMap.get(tabId);
            const relPath = itemMeta?.relPath ?? "";
            const syncStatus = relPath ? entitySyncStatus?.[relPath] : undefined;

            return (
              <div
                key={tabId}
                className="absolute inset-0 flex flex-col overflow-hidden"
                style={{ display: activeTab === tabId ? "flex" : "none" }}
              >
                {activeProtocol === "rest" && (
                  <RestTab
                    ref={(el) => { tabRefs.current[tabId] = el; }}
                    tabType="mock"
                    tabId={tabId}
                    draftTabId={isUnsaved ? tabId : null}
                    initial={initialData}
                    folders={folders}
                    activeEnv={activeEnv}
                    onSave={(data) => handleSaveEntity(tabId, "rest", data)}
                    onClose={() => closeTab(tabId)}
                    onDirtyChange={(dirty) => setDirtyTabs((prev) => ({ ...prev, [tabId]: dirty }))}
                    showCurlImport={isUnsaved}
                    enabled={isUnsaved ? undefined : itemMeta?.enabled}
                    onToggleEnabled={isUnsaved ? undefined : () => handleToggle(tabId)}
                    onSync={onPublishItem ? async (savedId?: string) => {
                      await onPublishItem(savedId ?? tabId);
                    } : undefined}
                    onRevert={onRestoreItem ? async () => {
                      await onRestoreItem(tabId);
                      const res = await window.api.loadEntity(config.activeWorkspaceId, "mocks", tabId);
                      if (res.ok && res.entity) {
                        setLoadedEntities((prev) => ({ ...prev, [tabId]: res.entity }));
                        tabRefs.current[tabId]?.refresh?.(res.entity);
                      } else if (!res.ok) {
                        closeTab(tabId);
                      }
                      setDirtyTabs((prev) => ({ ...prev, [tabId]: false }));
                    } : undefined}
                    onHistory={onHistoryOpen && relPath && !isUnsaved ? () => onHistoryOpen(relPath) : undefined}
                    syncStatus={syncStatus}
                  />
                )}

                {activeProtocol === "graphql" && (
                  <GraphQLTab
                    ref={(el) => { tabRefs.current[tabId] = el; }}
                    tabType="mock"
                    tabId={tabId}
                    draftTabId={isUnsaved ? tabId : null}
                    initial={initialData}
                    folders={folders}
                    activeEnv={activeEnv}
                    onSave={(data) => handleSaveEntity(tabId, "graphql", data)}
                    onClose={() => closeTab(tabId)}
                    onDirtyChange={(dirty) => setDirtyTabs((prev) => ({ ...prev, [tabId]: dirty }))}
                    onSync={onPublishItem ? async (savedId?: string) => {
                      await onPublishItem(savedId ?? tabId);
                    } : undefined}
                    onRevert={onRestoreItem ? async () => {
                      await onRestoreItem(tabId);
                      const res = await window.api.loadEntity(config.activeWorkspaceId, "graphqlMocks", tabId);
                      if (res.ok && res.entity) {
                        setLoadedEntities((prev) => ({ ...prev, [tabId]: res.entity }));
                        tabRefs.current[tabId]?.refresh?.(res.entity);
                      } else if (!res.ok) {
                        closeTab(tabId);
                      }
                      setDirtyTabs((prev) => ({ ...prev, [tabId]: false }));
                    } : undefined}
                    onHistory={onHistoryOpen && relPath && !isUnsaved ? () => onHistoryOpen(relPath) : undefined}
                    syncStatus={syncStatus}
                  />
                )}

                {activeProtocol === "grpc" && (
                  <GrpcTab
                    ref={(el) => { tabRefs.current[tabId] = el; }}
                    tabType="mock"
                    tabId={tabId}
                    draftTabId={isUnsaved ? tabId : null}
                    initial={initialData}
                    folders={folders}
                    activeEnv={activeEnv}
                    onSave={(data) => handleSaveEntity(tabId, "grpc", data)}
                    onClose={() => closeTab(tabId)}
                    onSync={onPublishItem ? async (savedId?: string) => {
                      await onPublishItem(savedId ?? tabId);
                    } : undefined}
                    onRevert={onRestoreItem ? async () => {
                      await onRestoreItem(tabId);
                      const res = await window.api.loadEntity(config.activeWorkspaceId, "grpcMocks", tabId);
                      if (res.ok && res.entity) {
                        setLoadedEntities((prev) => ({ ...prev, [tabId]: res.entity }));
                        tabRefs.current[tabId]?.refresh?.(res.entity);
                      } else if (!res.ok) {
                        closeTab(tabId);
                      }
                      setDirtyTabs((prev) => ({ ...prev, [tabId]: false }));
                    } : undefined}
                    onHistory={onHistoryOpen && relPath && !isUnsaved ? () => onHistoryOpen(relPath) : undefined}
                    syncStatus={syncStatus}
                  />
                )}

                {activeProtocol === "soap" && (
                  <SoapTab
                    ref={(el) => { tabRefs.current[tabId] = el; }}
                    tabType="mock"
                    tabId={tabId}
                    draftTabId={isUnsaved ? tabId : null}
                    initial={initialData}
                    folders={folders}
                    activeEnv={activeEnv}
                    onSave={(data) => handleSaveEntity(tabId, "soap", data)}
                    onClose={() => closeTab(tabId)}
                    onDirtyChange={(dirty) => setDirtyTabs((prev) => ({ ...prev, [tabId]: dirty }))}
                    onSync={onPublishItem ? async (savedId?: string) => {
                      await onPublishItem(savedId ?? tabId);
                    } : undefined}
                    onRevert={onRestoreItem ? async () => {
                      await onRestoreItem(tabId);
                      const res = await window.api.loadEntity(config.activeWorkspaceId, "soapMocks", tabId);
                      if (res.ok && res.entity) {
                        setLoadedEntities((prev) => ({ ...prev, [tabId]: res.entity }));
                        tabRefs.current[tabId]?.refresh?.(res.entity);
                      } else if (!res.ok) {
                        closeTab(tabId);
                      }
                      setDirtyTabs((prev) => ({ ...prev, [tabId]: false }));
                    } : undefined}
                    onHistory={onHistoryOpen && relPath && !isUnsaved ? () => onHistoryOpen(relPath) : undefined}
                    syncStatus={syncStatus}
                  />
                )}
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
        collapseTitle={strings.mocks.collapseSidebar}
        expandTitle={strings.mocks.expandSidebar}
        storageKey="mocks-panel-sidebar"
        collapsedBadge={allEntities.length > 0 ? (
          <span className="text-[9px] text-muted-foreground font-mono" title={`${allEntities.length} mocks`}
            style={{ writingMode: "vertical-rl", transform: "rotate(180deg)", lineHeight: 1.4 }}>{allEntities.length}</span>
        ) : undefined}
      >
        {mainContent}
      </SidebarLayout>
    </>
  );
}
