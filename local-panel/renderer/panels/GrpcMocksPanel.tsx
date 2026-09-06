import React, { useState, useMemo, useCallback, useEffect } from "react";
import { AppConfig, SavedGrpcMock, Folder, Environment } from "@/types";
import FolderTree, { FolderTreeItem } from "@/components/sidebar/FolderTree";
import GrpcTab from "@/components/grpc/GrpcTab";
import DraftsFolder from "@/components/sidebar/DraftsFolder";
import { loadDraft } from "@/lib/useDraftPersist";
import { useEntityTabs } from "@/lib/useEntityTabs";
import { calculateFolderStatus, entityRelPath } from "@/lib/utils";
import { Network, Play, Square } from "@/lib/icons";
import TabBar from "@/components/editor/TabBar";
import { SidebarLayout, SidebarHeader } from "@/components/ui";
import { strings } from "@/lib/strings";
import { usePersistedState } from "@/lib/usePersistedState";
import { useTabKeyBindings } from "@/hooks/useTabKeyBindings";
import type { GrpcTabHandle } from "@/components/grpc/GrpcTab";

import { GrpcMockDraft } from "@/components/grpc/grpcTabReducer";

// -- Constants --------------------------------------------------------------

const DRAFT_PREFIX = "grpc-mock-draft-";
const isDraft = (id: string) => id.startsWith(DRAFT_PREFIX);

// -- Props ------------------------------------------------------------------

interface Props {
    config: AppConfig;
    onConfigChange: (cfg: AppConfig) => Promise<void>;
    activeEnv?: Environment | null;
    onHistoryOpen?: (filePath: string) => void;
    entitySyncStatus?: Record<string, "clean" | "modified" | "new" | "deleted">;
    onPublishItem?: (id: string) => void;
    onRestoreItem?: (id: string) => void;
}

// -- GrpcMocksPanel -----------------------------------------------------

export default function GrpcMocksPanel({ config, onConfigChange, activeEnv = null, onHistoryOpen, entitySyncStatus, onPublishItem, onRestoreItem }: Props) {
    const mocks = config.grpcMocks ?? [];
    const folders = config.grpcMockFolders ?? [];

    const [sidebarOpen, setSidebarOpen] = usePersistedState(`grpc-mocks:${config.activeWorkspaceId}:sidebar-open`, true);
    const [mockServerRunning, setMockServerRunning] = useState(false);
    const [mockServerPort, setMockServerPort] = useState(config.grpcMockServerPort ?? 9102);
    const [serverBusy, setServerBusy] = useState(false);

    // Check mock server status on mount
    useEffect(() => {
        window.api.grpcMockServerStatus().then((s) => {
            setMockServerRunning(s.running);
            setMockServerPort(s.port);
        });
    }, []);

    const toggleMockServer = useCallback(async () => {
        setServerBusy(true);
        try {
            if (mockServerRunning) {
                await window.api.grpcStopMockServer();
                setMockServerRunning(false);
            } else {
                const res = await window.api.grpcStartMockServer();
                if (res.ok) setMockServerRunning(true);
            }
        } finally {
            setServerBusy(false);
        }
    }, [mockServerRunning]);

    const {
        openTabs, activeTab, setActiveTab,
        loadedEntities, setLoadedEntities, tabRefs,
        openTab, openNewTab, closeTab, replaceTab, closeOtherTabs, closeAllTabs,
    } = useEntityTabs<SavedGrpcMock>({
        storageKey: "grpcMocks",
        draftPrefix: DRAFT_PREFIX,
        workspaceId: config.activeWorkspaceId,
        entityKind: "grpcMocks" as any,
        entities: mocks,
    });

    useTabKeyBindings({ activeTab, tabRefs, closeTab, openNewTab });

    const reloadConfig = useCallback(async () => {
        const fresh = await window.api.getConfig();
        await onConfigChange(fresh);
    }, [onConfigChange]);

    const handleNewSave = useCallback(async (tabId: string, data: Omit<SavedGrpcMock, "id" | "createdAt" | "workspaceId">) => {
        const created = await window.api.addGrpcMock(data);
        await reloadConfig();
        replaceTab(tabId, created.id);
        return created;
    }, [reloadConfig, replaceTab]);

    const handleTabSave = useCallback(async (tabId: string, data: Omit<SavedGrpcMock, "id" | "createdAt" | "workspaceId">) => {
        const mock = loadedEntities[tabId] ?? mocks.find((m) => m.id === tabId);
        if (!mock) return;
        const updated = { ...mock, ...data };
        setLoadedEntities((prev) => ({ ...prev, [tabId]: updated }));
        await window.api.updateGrpcMock(updated);
        await reloadConfig();
        return updated;
    }, [loadedEntities, mocks, reloadConfig]);

    const handleDelete = useCallback(async (id: string) => {
        await window.api.deleteGrpcMock(id);
        await reloadConfig();
        closeTab(id);
    }, [reloadConfig, closeTab]);

    const handleDuplicate = useCallback(async (id: string) => {
        let m = loadedEntities[id];
        if (!m) {
            const res = await window.api.loadEntity(config.activeWorkspaceId, "grpcMocks", id);
            if (res.ok && res.entity) m = res.entity as SavedGrpcMock;
        }
        if (!m) return;
        const { id: _id, createdAt: _ca, workspaceId: _ws, ...rest } = m;
        await window.api.addGrpcMock({ ...rest, name: m.name ? strings.grpc.copySuffix.replace("{name}", m.name) : "" });
        await reloadConfig();
    }, [loadedEntities, config.activeWorkspaceId, reloadConfig]);

    const handleFoldersChange = useCallback(async () => {
        await reloadConfig();
    }, [reloadConfig]);

    const handleMoveItems = useCallback(async (ids: string[], folderId: string | null) => {
        for (const id of ids) {
            let m = loadedEntities[id];
            if (!m) {
                const res = await window.api.loadEntity(config.activeWorkspaceId, "grpcMocks", id);
                if (res.ok && res.entity) m = res.entity as SavedGrpcMock;
            }
            if (m) await window.api.updateGrpcMock({ ...m, folderId: folderId ?? undefined } as any);
        }
        await reloadConfig();
    }, [loadedEntities, config.activeWorkspaceId, reloadConfig]);

    const tabLabel = (tabId: string): string => {
        if (isDraft(tabId)) {
            const draft = loadDraft<GrpcMockDraft>(tabId);
            if (draft?.serviceName && draft?.methodName) return `${draft.serviceName}/${draft.methodName}`;
            if (draft?.serviceName) return draft.serviceName;
            return strings.grpc.newMockTab;
        }
        const m = mocks.find((x) => x.id === tabId);
        if (!m) return "…";
        if (m.name) return m.name;
        if (m.serviceName && m.methodName) return `${m.serviceName}/${m.methodName}`;
        return m.serviceName || strings.grpc.mockLabel;
    };

    const folderViewItems: FolderTreeItem[] = useMemo(() => {
        return mocks.map((m): FolderTreeItem => ({
            id: m.id,
            name: m.name || `${m.serviceName}/${m.methodName}` || strings.grpc.unnamed,
            folderId: m.folderId ?? null,
            isActive: activeTab === m.id,
            isEnabled: m.enabled,
        }));
    }, [mocks, activeTab]);

    const folderStatusMap = useMemo(() => calculateFolderStatus(mocks, folders), [mocks, folders]);

    const draftTabIds = openTabs.filter(isDraft);

    // -- Sidebar ------------------------------------------------------------

    const sidebarContent = (
        <>
            <SidebarHeader onCollapse={() => setSidebarOpen(false)} collapseTitle={strings.grpc.collapseSidebar}>
                <span className="text-xs font-semibold px-1 text-muted-foreground uppercase tracking-wider">
                    {strings.grpc.mocks}
                </span>
            </SidebarHeader>
            {/* Mock server toggle */}
            <div className="flex items-center gap-2 px-3 py-2 border-b border-border">
                <button
                    onClick={toggleMockServer}
                    disabled={serverBusy}
                    className={`flex items-center justify-center w-6 h-6 rounded border transition-all duration-150 cursor-pointer disabled:opacity-40 disabled:cursor-not-allowed flex-shrink-0 ${mockServerRunning
                        ? "border-signal/40 bg-signal/10 hover:bg-destructive/15 hover:border-destructive/40 text-signal hover:text-destructive"
                        : "border-border bg-card hover:bg-signal/15 hover:border-signal/40 text-muted-foreground hover:text-signal"
                        }`}
                    title={mockServerRunning ? strings.grpc.stopMockServer : strings.grpc.startMockServer}
                >
                    {serverBusy ? (
                        <span className="inline-block w-2.5 h-2.5 border-2 border-current/30 border-t-current rounded-full animate-spin" />
                    ) : mockServerRunning ? (
                        <Square size={8} fill="currentColor" />
                    ) : (
                        <Play size={8} fill="currentColor" />
                    )}
                </button>
                <span className="text-[10px] text-muted-foreground">
                    {mockServerRunning ? strings.grpc.mockServerRunning.replace("{port}", String(mockServerPort)) : strings.grpc.mockServerStopped}
                </span>
            </div>
            <div className="flex-1 overflow-y-auto overflow-x-auto min-w-0" style={{ display: "flex", flexDirection: "column" }}>
                {draftTabIds.length > 0 && (
                    <DraftsFolder
                        label={strings.grpc.drafts}
                        draftTabIds={draftTabIds}
                        activeTab={activeTab}
                        onOpenTab={(id) => setActiveTab(id)}
                        onCloseTab={closeTab}
                        tabLabel={tabLabel}
                    />
                )}
                <FolderTree
                    kind="grpcMock"
                    folders={folders}
                    items={folderViewItems}
                    folderStatusMap={folderStatusMap}
                    onOpenItem={openTab}
                    onDeleteItem={handleDelete}
                    onFoldersChange={handleFoldersChange}
                    onDuplicateItem={handleDuplicate}
                    onMoveItems={handleMoveItems}
                    onOpenNewTab={openNewTab}
                    onBeforeCreateFolder={() => true}
                    pathStatusMap={entitySyncStatus}
                    onPublishItem={onPublishItem}
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
                newTabTitle={strings.grpc.newMockTabTitle}
                closeTabTitle={strings.grpc.closeTab}
                onCloseOthers={closeOtherTabs}
                onCloseAll={closeAllTabs}
                onTabDuplicate={handleDuplicate}
            />

            <div className="flex-1 overflow-hidden relative">
                {openTabs.length === 0 ? (
                    <div className="flex flex-col items-center justify-center h-full text-center gap-2">
                        <div className="opacity-10 mb-1"><Network size={48} /></div>
                        <div className="text-sm font-medium text-foreground">{strings.grpc.noMocksOpen}</div>
                        <p className="text-xs text-muted-foreground max-w-xs leading-relaxed">
                            {strings.grpc.noMocksOpenHint}
                        </p>
                    </div>
                ) : (
                    openTabs.map((tabId) => {
                        const isUnsaved = isDraft(tabId);
                        const mock = isUnsaved ? null : (loadedEntities[tabId] ?? mocks.find((m) => m.id === tabId) ?? null);
                        const initialData = isUnsaved ? null : mock;
                        if (!isUnsaved && !mock) return null;
                        const relPath = mock ? entityRelPath("grpcMocks", mock, folders) : "";
                        const syncStatus = relPath ? entitySyncStatus?.[relPath] : undefined;
                        return (
                            <div key={tabId} className="absolute inset-0 flex flex-col overflow-hidden" style={{ display: activeTab === tabId ? "flex" : "none" }}>
                                <GrpcTab
                                    ref={(el: GrpcTabHandle | null) => { tabRefs.current[tabId] = el; }}
                                    tabType="mock"
                                    tabId={tabId}
                                    draftTabId={isUnsaved ? tabId : null}
                                    initial={initialData}
                                    folders={folders}
                                    activeEnv={activeEnv}
                                    onSave={(data) => isUnsaved
                                        ? handleNewSave(tabId, data as Omit<SavedGrpcMock, "id" | "createdAt" | "workspaceId">)
                                        : handleTabSave(tabId, data as Omit<SavedGrpcMock, "id" | "createdAt" | "workspaceId">)
                                    }
                                    onClose={() => closeTab(tabId)}
                                    onSync={onPublishItem ? async (savedId?: string) => {
                                        const targetId = savedId ?? tabId;
                                        await onPublishItem(targetId);
                                    } : undefined}
                                    onRevert={onRestoreItem ? async () => {
                                        await onRestoreItem(tabId);
                                        const res = await window.api.loadEntity(config.activeWorkspaceId, "grpcMocks", tabId);
                                        if (res.ok && res.entity) {
                                            const entity = res.entity as SavedGrpcMock;
                                            setLoadedEntities((prev) => ({ ...prev, [tabId]: entity }));
                                            tabRefs.current[tabId]?.refresh?.(entity);
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
                collapseTitle={strings.grpc.collapseSidebar}
                expandTitle={strings.grpc.expandSidebar}
                storageKey="grpc-mocks-panel-sidebar"
                collapsedBadge={mocks.length > 0 ? (
                    <span className="text-[9px] text-muted-foreground font-mono" title={strings.grpc.mockCount.replace("{count}", String(mocks.length))}
                        style={{ writingMode: "vertical-rl", transform: "rotate(180deg)", lineHeight: 1.4 }}>{mocks.length}</span>
                ) : undefined}
            >
                {mainContent}
            </SidebarLayout>
        </>
    );
}
