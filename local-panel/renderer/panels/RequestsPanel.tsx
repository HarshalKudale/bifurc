import React, { useState, useEffect, useMemo, useCallback, useRef } from "react";
import {
  AppConfig,
  SavedRequest,
  SavedGraphQLRequest,
  SavedGrpcRequest,
  SavedSoapRequest,
  MockRule,
  Folder,
  Environment,
  ApiProtocol,
} from "@/types";
import FolderTree, { FolderTreeItem } from "@/components/sidebar/FolderTree";
import RestTab from "@/components/rest/RestTab";
import GraphQLTab from "@/components/graphql/GraphQLTab";
import GrpcTab from "@/components/grpc/GrpcTab";
import SoapTab from "@/components/soap/SoapTab";
import CollectionRunner from "@/components/rest/CollectionRunner";
import ProtocolSelectorTab from "@/components/editor/ProtocolSelectorTab";
import DraftsFolder from "@/components/sidebar/DraftsFolder";
import { loadDraft } from "@/lib/useDraftPersist";
import { useEntityTabs } from "@/lib/useEntityTabs";
import { strings } from "@/lib/strings";
import { entityRelPath, methodColor, methodBg } from "@/lib/utils";
import { Zap } from "@/lib/icons";
import TabBar from "@/components/editor/TabBar";
import { SidebarLayout, SidebarHeader } from "@/components/ui";
import { useTabKeyBindings } from "@/hooks/useTabKeyBindings";
import { usePersistedState } from "@/lib/usePersistedState";

// -- Draft tab prefix -------------------------------------------------------

const DRAFT_PREFIX = "req-draft-";
const RUNNER_PREFIX = "runner-";
const isDraft = (id: string) =>
  id.startsWith(DRAFT_PREFIX) ||
  id.startsWith("pending-") ||
  id.startsWith("gql-req-draft-") ||
  id.startsWith("grpc-req-draft-") ||
  id.startsWith("soap-req-draft-");
const isRunner = (id: string) => id.startsWith(RUNNER_PREFIX);

// -- Props ------------------------------------------------------------------

interface Props {
  config: AppConfig;
  onConfigChange: (cfg: AppConfig) => Promise<void>;
  pendingOpenRequest?: Omit<SavedRequest, "id" | "createdAt" | "workspaceId"> | null;
  onPendingConsumed?: () => void;
  onOpenMockEditor?: (initial: Partial<MockRule>) => void;
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

// -- RequestsPanel ----------------------------------------------------------

export default function RequestsPanel({
  config,
  onConfigChange,
  pendingOpenRequest,
  onPendingConsumed,
  onOpenMockEditor,
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
  const restRequests = config.requests ?? [];
  const graphqlRequests = config.graphqlRequests ?? [];
  const grpcRequests = config.grpcRequests ?? [];
  const soapRequests = config.soapRequests ?? [];

  // Unified folders deduplicated by ID
  const folders = useMemo(() => {
    const map = new Map<string, Folder>();
    (config.requestFolders ?? []).forEach((f) => map.set(f.id, f));
    (config.graphqlRequestFolders ?? []).forEach((f) => { if (!map.has(f.id)) map.set(f.id, f); });
    (config.grpcRequestFolders ?? []).forEach((f) => { if (!map.has(f.id)) map.set(f.id, f); });
    (config.soapRequestFolders ?? []).forEach((f) => { if (!map.has(f.id)) map.set(f.id, f); });
    return Array.from(map.values());
  }, [
    config.requestFolders,
    config.graphqlRequestFolders,
    config.grpcRequestFolders,
    config.soapRequestFolders,
  ]);

  // Map of entity id -> protocol
  const itemProtocolMap = useMemo(() => {
    const map = new Map<string, ApiProtocol>();
    restRequests.forEach((r) => map.set(r.id, "rest"));
    graphqlRequests.forEach((r) => map.set(r.id, "graphql"));
    grpcRequests.forEach((r) => map.set(r.id, "grpc"));
    soapRequests.forEach((r) => map.set(r.id, "soap"));
    return map;
  }, [restRequests, graphqlRequests, grpcRequests, soapRequests]);

  // Unified item metadata map for fast lookups
  const allItemsMap = useMemo(() => {
    const map = new Map<string, { id: string; name: string; folderId?: string | null; protocol: ApiProtocol; summary: string; methodBadge: string; relPath: string }>();
    restRequests.forEach((r) => {
      map.set(r.id, {
        id: r.id,
        name: r.name || "",
        folderId: r.folderId ?? null,
        protocol: "rest",
        summary: r.url || "REST Request",
        methodBadge: r.method || "GET",
        relPath: entityRelPath("requests", r, folders),
      });
    });
    graphqlRequests.forEach((r) => {
      map.set(r.id, {
        id: r.id,
        name: r.name || "",
        folderId: r.folderId ?? null,
        protocol: "graphql",
        summary: r.endpointUrl || "GraphQL Operation",
        methodBadge: "GQL",
        relPath: entityRelPath("graphqlRequests", r, folders),
      });
    });
    grpcRequests.forEach((r) => {
      map.set(r.id, {
        id: r.id,
        name: r.name || "",
        folderId: r.folderId ?? null,
        protocol: "grpc",
        summary: r.serviceName && r.methodName ? `${r.serviceName}/${r.methodName}` : "gRPC Call",
        methodBadge: "gRPC",
        relPath: entityRelPath("grpcRequests", r, folders),
      });
    });
    soapRequests.forEach((r) => {
      map.set(r.id, {
        id: r.id,
        name: r.name || "",
        folderId: r.folderId ?? null,
        protocol: "soap",
        summary: r.soapAction || r.endpointUrl || "SOAP Request",
        methodBadge: "SOAP",
        relPath: entityRelPath("soapRequests", r, folders),
      });
    });
    return map;
  }, [restRequests, graphqlRequests, grpcRequests, soapRequests, folders]);

  // Unified entities array for useEntityTabs
  const allEntities = useMemo(() => {
    return [
      ...restRequests,
      ...graphqlRequests,
      ...grpcRequests,
      ...soapRequests,
    ];
  }, [restRequests, graphqlRequests, grpcRequests, soapRequests]);

  const [sidebarOpen, setSidebarOpen] = usePersistedState(`requests:${config.activeWorkspaceId}:sidebar-open`, true);
  const [selectedFolderId, setSelectedFolderId] = usePersistedState<string | null>(`requests:${config.activeWorkspaceId}:selected-folder`, null);
  const [runnerFolderIds, setRunnerFolderIds] = useState<Set<string>>(new Set());

  // Track draft protocols per tab ID: "rest" | "graphql" | "grpc" | "soap" | null
  const [draftProtocols, setDraftProtocols] = usePersistedState<Record<string, ApiProtocol | null>>(
    `requests:${config.activeWorkspaceId}:draft-protocols`,
    {}
  );

  // Load which folders have saved runner configs
  const loadRunnerFolderIds = useCallback(async () => {
    try {
      const ids = await window.api.listRunnerFolderIds(config.activeWorkspaceId);
      setRunnerFolderIds(new Set(ids));
    } catch { /* ignore */ }
  }, [config.activeWorkspaceId]);

  useEffect(() => { loadRunnerFolderIds(); }, [loadRunnerFolderIds]);

  const resolveEntityKind = useCallback((id: string): string => {
    const proto = itemProtocolMap.get(id);
    if (proto === "graphql") return "graphqlRequests";
    if (proto === "grpc") return "grpcRequests";
    if (proto === "soap") return "soapRequests";
    return "requests";
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
    storageKey: "requests",
    draftPrefix: DRAFT_PREFIX,
    extraDraftPrefixes: ["pending-", RUNNER_PREFIX, "gql-req-draft-", "grpc-req-draft-", "soap-req-draft-"],
    workspaceId: config.activeWorkspaceId,
    entityKind: "requests",
    resolveEntityKind,
    entities: allEntities,
  });

  const [pendingData, setPendingData] = useState<Record<string, Omit<SavedRequest, "id" | "createdAt" | "workspaceId">>>({});
  const [newTabInitials, setNewTabInitials] = useState<Record<string, { folderId?: string | null }>>({});
  const [dirtyTabs, setDirtyTabs] = useState<Record<string, boolean>>({});

  const openNewTabInFolder = useCallback(() => {
    const tabId = `${DRAFT_PREFIX}${Date.now()}`;
    if (selectedFolderId) {
      setNewTabInitials((prev) => ({ ...prev, [tabId]: { folderId: selectedFolderId } }));
    }
    // New tab starts with no protocol selected so user chooses first
    setDraftProtocols((prev) => ({ ...prev, [tabId]: null }));
    openTab(tabId);
  }, [openTab, selectedFolderId, setDraftProtocols]);

  useTabKeyBindings({ activeTab, tabRefs, closeTab, openNewTab: openNewTabInFolder });

  // Open a pending request in a new draft tab (pre-set to REST)
  useEffect(() => {
    if (!pendingOpenRequest) return;
    const tabId = `pending-${Date.now()}`;
    setPendingData((prev) => ({ ...prev, [tabId]: pendingOpenRequest }));
    setDraftProtocols((prev) => ({ ...prev, [tabId]: "rest" }));
    openTab(tabId);
    onPendingConsumed?.();
  }, [pendingOpenRequest, openTab, onPendingConsumed, setDraftProtocols]);

  const reloadRequests = useCallback(async () => {
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
    await window.api.moveFolder("request", folderId, targetParentId);
    await handleFoldersChange();
  }, [handleFoldersChange]);

  // -- Save handlers by protocol ----------------------------------------------

  const handleSaveEntity = useCallback(
    async (tabId: string, protocol: ApiProtocol, data: any) => {
      const isNew = isDraft(tabId);
      let saved: any;
      if (protocol === "rest") {
        saved = isNew
          ? await window.api.addRequest(data)
          : await window.api.updateRequest({ ...(loadedEntities[tabId] ?? {}), ...data });
      } else if (protocol === "graphql") {
        saved = isNew
          ? await window.api.addGraphQLRequest(data)
          : await window.api.updateGraphQLRequest({ ...(loadedEntities[tabId] ?? {}), ...data });
      } else if (protocol === "grpc") {
        saved = isNew
          ? await window.api.addGrpcRequest(data)
          : await window.api.updateGrpcRequest({ ...(loadedEntities[tabId] ?? {}), ...data });
      } else if (protocol === "soap") {
        saved = isNew
          ? await window.api.addSoapRequest(data)
          : await window.api.updateSoapRequest({ ...(loadedEntities[tabId] ?? {}), ...data });
      }

      await reloadRequests();
      if (isNew && saved?.id) {
        replaceTab(tabId, saved.id);
        setPendingData((prev) => { const next = { ...prev }; delete next[tabId]; return next; });
        setNewTabInitials((prev) => { const next = { ...prev }; delete next[tabId]; return next; });
        setDraftProtocols((prev) => { const next = { ...prev }; delete next[tabId]; return next; });
      }
      onAfterSave?.();
      return saved;
    },
    [loadedEntities, reloadRequests, replaceTab, onAfterSave, setDraftProtocols]
  );

  const handleDelete = useCallback(
    async (id: string) => {
      closeTab(id);
      const proto = itemProtocolMap.get(id);
      if (proto === "graphql") await window.api.deleteGraphQLRequest(id);
      else if (proto === "grpc") await window.api.deleteGrpcRequest(id);
      else if (proto === "soap") await window.api.deleteSoapRequest(id);
      else await window.api.deleteRequest(id);
      await reloadRequests();
    },
    [itemProtocolMap, reloadRequests, closeTab]
  );

  const handleDeleteItems = useCallback(
    async (trackedIds: string[], untrackedIds: string[]) => {
      const allIds = [...trackedIds, ...untrackedIds];
      allIds.forEach((id) => closeTab(id));
      for (const id of allIds) {
        const proto = itemProtocolMap.get(id);
        if (proto === "graphql") await window.api.deleteGraphQLRequest(id);
        else if (proto === "grpc") await window.api.deleteGrpcRequest(id);
        else if (proto === "soap") await window.api.deleteSoapRequest(id);
        else await window.api.deleteRequest(id);
      }
      await reloadRequests();
    },
    [itemProtocolMap, reloadRequests, closeTab]
  );

  const handleBeforeDeleteFolder = useCallback(
    (folderId: string) => {
      allEntities
        .filter((r: any) => r.folderId === folderId)
        .forEach((r: any) => closeTab(r.id));
      closeTab(`${RUNNER_PREFIX}${folderId}`);
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

      if (proto === "graphql") await window.api.addGraphQLRequest({ ...rest, name: copyName });
      else if (proto === "grpc") await window.api.addGrpcRequest({ ...rest, name: copyName });
      else if (proto === "soap") await window.api.addSoapRequest({ ...rest, name: copyName });
      else await window.api.addRequest({ ...rest, name: copyName });

      await reloadRequests();
    },
    [itemProtocolMap, resolveEntityKind, loadedEntities, config.activeWorkspaceId, reloadRequests]
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
        if (proto === "graphql") await window.api.updateGraphQLRequest(updated);
        else if (proto === "grpc") await window.api.updateGrpcRequest(updated);
        else if (proto === "soap") await window.api.updateSoapRequest(updated);
        else await window.api.updateRequest(updated);
      }
      await reloadRequests();
    },
    [itemProtocolMap, resolveEntityKind, loadedEntities, config.activeWorkspaceId, reloadRequests]
  );

  const handleOpenRunner = useCallback(
    (folderId: string) => {
      const tabId = `${RUNNER_PREFIX}${folderId}`;
      openTab(tabId);
      setTimeout(() => loadRunnerFolderIds(), 500);
    },
    [openTab, loadRunnerFolderIds]
  );

  const handleSaveRunnerReport = useCallback(
    async (report: any) => {
      await window.api.saveRunnerReport(config.activeWorkspaceId, report);
    },
    [config.activeWorkspaceId]
  );

  const draftTabIds = openTabs.filter(isDraft);

  // Tab label resolution
  const tabLabel = (tabId: string) => {
    if (isRunner(tabId)) {
      const fId = tabId.slice(RUNNER_PREFIX.length);
      const folder = folders.find((f) => f.id === fId);
      return `Runner: ${folder?.name ?? strings.requests.collectionFallback}`;
    }
    if (isDraft(tabId)) {
      const proto = draftProtocols[tabId];
      if (!proto) return "New Request";
      const draft = loadDraft<any>(tabId);
      if (draft?.name) return draft.name;
      if (proto === "rest" && draft?.url) {
        try { const u = new URL(draft.url); const last = u.pathname.split("/").filter(Boolean).pop() ?? u.host; return `${draft.method ?? "GET"} /${last}`; }
        catch { return draft.method ?? "New REST"; }
      }
      if (proto === "graphql") return draft?.operationName || "New GraphQL";
      if (proto === "grpc") return draft?.methodName ? `${draft.serviceName || ""}/${draft.methodName}` : "New gRPC";
      if (proto === "soap") return draft?.operationName || "New SOAP";
      return `New ${proto.toUpperCase()}`;
    }
    const item = allItemsMap.get(tabId);
    if (!item) return "…";
    if (item.name) return item.name;
    return item.summary || "Request";
  };

  // Tab badge resolution (e.g. GET, POST, GQL, gRPC, SOAP)
  const tabBadge = (tabId: string) => {
    if (isRunner(tabId)) return "RUN";
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

  // Uniform folder tree items combining all 4 protocols + runners
  const folderViewItems: FolderTreeItem[] = useMemo(() => {
    const items: FolderTreeItem[] = [];

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

    // Add runner nodes for folders with saved runner.json
    const runnerItems: FolderTreeItem[] = folders
      .filter((f) => runnerFolderIds.has(f.id))
      .map((f): FolderTreeItem => ({
        id: `${RUNNER_PREFIX}${f.id}`,
        name: "Run Collection",
        folderId: f.id,
        isActive: activeTab === `${RUNNER_PREFIX}${f.id}`,
        isEnabled: true,
        isRunner: true,
      }));

    return [...items, ...runnerItems];
  }, [allItemsMap, folders, activeTab, runnerFolderIds]);

  // -- Sidebar ------------------------------------------------------------

  const sidebarContent = (
    <>
      <SidebarHeader onCollapse={() => setSidebarOpen(false)} collapseTitle={strings.mocks.collapseSidebar}>
        <span className="text-xs font-semibold px-1 text-muted-foreground uppercase tracking-wider">
          {strings.nav.requests}
        </span>
      </SidebarHeader>
      <div className="flex-1 overflow-y-auto overflow-x-auto min-w-0" style={{ display: "flex", flexDirection: "column" }}>
        {draftTabIds.length > 0 && (
          <DraftsFolder
            label={strings.requests.drafts}
            draftTabIds={draftTabIds}
            activeTab={activeTab}
            onOpenTab={(id) => setActiveTab(id)}
            onCloseTab={closeTab}
            tabLabel={tabLabel}
          />
        )}
        <FolderTree
          kind="request"
          folders={folders}
          items={folderViewItems}
          onOpenItem={openTab}
          onDeleteItem={handleDelete}
          onDeleteItems={handleDeleteItems}
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
          onPublishItem={onPublishItem}
          onPublishFolder={onPublishFolder}
          onRestoreItem={onRestoreItem}
          onOpenRunner={handleOpenRunner}
          onBeforeDeleteFolder={handleBeforeDeleteFolder}
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
        newTabTitle={strings.requests.newTab}
        closeTabTitle={strings.requests.closeTab}
        onCloseOthers={closeOtherTabs}
        onCloseAll={closeAllTabs}
        onTabDuplicate={handleDuplicate}
      />

      <div className="flex-1 overflow-hidden relative">
        {openTabs.length === 0 ? (
          <div className="flex flex-col items-center justify-center h-full text-center gap-2">
            <div className="opacity-10 mb-1"><Zap size={48} /></div>
            <div className="text-sm font-medium text-foreground">{strings.requests.noRequestsOpen}</div>
            <p className="text-xs text-muted-foreground max-w-xs leading-relaxed">
              {strings.requests.noRequestsOpenHint.replace("+", "")}
              <span className="text-signal font-semibold">+</span>
              {" to create a new one."}
            </p>
          </div>
        ) : (
          openTabs.map((tabId) => {
            // Runner tab
            if (isRunner(tabId)) {
              const fId = tabId.slice(RUNNER_PREFIX.length);
              const folder = folders.find((f) => f.id === fId);
              const folderRequests = restRequests.filter((r) => r.folderId === fId);
              return (
                <div key={tabId} className="absolute inset-0 flex flex-col overflow-hidden" style={{ display: activeTab === tabId ? "flex" : "none" }}>
                  <CollectionRunner
                    folderId={fId}
                    folderName={folder?.name ?? "Collection"}
                    requests={folderRequests}
                    activeEnv={activeEnv}
                    wsId={config.activeWorkspaceId}
                    onClose={() => closeTab(tabId)}
                    onSaveReport={handleSaveRunnerReport}
                  />
                </div>
              );
            }

            const isUnsaved = isDraft(tabId);
            // Protocol determination
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
                    mode="request"
                    onSelect={(proto) => {
                      setDraftProtocols((prev) => ({ ...prev, [tabId]: proto }));
                    }}
                  />
                </div>
              );
            }

            const activeProtocol: ApiProtocol = currentProto ?? "rest";

            // Saved entity or draft initial data
            const entity = isUnsaved
              ? null
              : (loadedEntities[tabId] ?? allEntities.find((e: any) => e.id === tabId) ?? null);
            const initialData = isUnsaved
              ? (pendingData[tabId] ?? newTabInitials[tabId] ?? null)
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
                    tabType="request"
                    tabId={tabId}
                    draftTabId={isUnsaved ? tabId : null}
                    initial={initialData}
                    folders={folders}
                    activeEnv={activeEnv}
                    onSave={(data) => handleSaveEntity(tabId, "rest", data)}
                    onCreateMock={(initial) => onOpenMockEditor?.(initial)}
                    onClose={() => closeTab(tabId)}
                    onDirtyChange={(dirty) => setDirtyTabs((prev) => ({ ...prev, [tabId]: dirty }))}
                    showCurlImport={isUnsaved}
                    onSync={onPublishItem ? async (savedId?: string) => {
                      await onPublishItem(savedId ?? tabId);
                    } : undefined}
                    onRevert={onRestoreItem ? async () => {
                      await onRestoreItem(tabId);
                      const res = await window.api.loadEntity(config.activeWorkspaceId, "requests", tabId);
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
                    tabType="request"
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
                      const res = await window.api.loadEntity(config.activeWorkspaceId, "graphqlRequests", tabId);
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
                    tabType="request"
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
                      const res = await window.api.loadEntity(config.activeWorkspaceId, "grpcRequests", tabId);
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
                    tabType="request"
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
                      const res = await window.api.loadEntity(config.activeWorkspaceId, "soapRequests", tabId);
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
        storageKey="requests-panel-sidebar"
        collapsedBadge={allEntities.length > 0 ? (
          <span className="text-[9px] text-muted-foreground font-mono" title={`${allEntities.length} requests`}
            style={{ writingMode: "vertical-rl", transform: "rotate(180deg)", lineHeight: 1.4 }}>{allEntities.length}</span>
        ) : undefined}
      >
        {mainContent}
      </SidebarLayout>
    </>
  );
}
