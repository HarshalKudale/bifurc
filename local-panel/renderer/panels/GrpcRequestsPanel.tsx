import React, { useState, useMemo, useCallback } from "react";
import { AppConfig, SavedGrpcRequest, Folder, Environment } from "@/types";
import FolderTree, { FolderTreeItem } from "@/components/sidebar/FolderTree";
import GrpcTab from "@/components/grpc/GrpcTab";
import DraftsFolder from "@/components/sidebar/DraftsFolder";
import { loadDraft } from "@/lib/useDraftPersist";
import { useEntityTabs } from "@/lib/useEntityTabs";
import { Network } from "@/lib/icons";
import TabBar from "@/components/editor/TabBar";
import { SidebarLayout, SidebarHeader } from "@/components/ui";
import { strings } from "@/lib/strings";
import { usePersistedState } from "@/lib/usePersistedState";
import { useTabKeyBindings } from "@/hooks/useTabKeyBindings";
import type { GrpcTabHandle } from "@/components/grpc/GrpcTab";
import { entityRelPath } from "@/lib/utils";

import { GrpcRequestDraft } from "@/components/grpc/grpcTabReducer";

// -- Constants --------------------------------------------------------------

const DRAFT_PREFIX = "grpc-req-draft-";
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

// -- GrpcRequestsPanel --------------------------------------------------

export default function GrpcRequestsPanel({ config, onConfigChange, activeEnv = null, onHistoryOpen, entitySyncStatus, onPublishItem, onRestoreItem }: Props) {
    const requests = config.grpcRequests ?? [];
    const folders = config.grpcRequestFolders ?? [];

    const [sidebarOpen, setSidebarOpen] = usePersistedState(`grpc-requests:${config.activeWorkspaceId}:sidebar-open`, true);

    const {
        openTabs, activeTab, setActiveTab,
        loadedEntities, setLoadedEntities, tabRefs,
        openTab, openNewTab, closeTab, replaceTab, closeOtherTabs, closeAllTabs,
    } = useEntityTabs<SavedGrpcRequest>({
        storageKey: "grpcRequests",
        draftPrefix: DRAFT_PREFIX,
        workspaceId: config.activeWorkspaceId,
        entityKind: "grpcRequests" as any,
        entities: requests,
    });

    useTabKeyBindings({ activeTab, tabRefs, closeTab, openNewTab });

    const reloadConfig = useCallback(async () => {
        const fresh = await window.api.getConfig();
        await onConfigChange(fresh);
    }, [onConfigChange]);

    const handleNewSave = useCallback(async (tabId: string, data: Omit<SavedGrpcRequest, "id" | "createdAt" | "workspaceId">) => {
        const created = await window.api.addGrpcRequest(data);
        await reloadConfig();
        replaceTab(tabId, created.id);
        return created;
    }, [reloadConfig, replaceTab]);

    const handleTabSave = useCallback(async (tabId: string, data: Omit<SavedGrpcRequest, "id" | "createdAt" | "workspaceId">) => {
        const req = loadedEntities[tabId] ?? requests.find((r) => r.id === tabId);
        if (!req) return;
        const updated = { ...req, ...data };
        setLoadedEntities((prev) => ({ ...prev, [tabId]: updated }));
        await window.api.updateGrpcRequest(updated);
        await reloadConfig();
        return updated;
    }, [loadedEntities, requests, reloadConfig]);

    const handleDelete = useCallback(async (id: string) => {
        await window.api.deleteGrpcRequest(id);
        await reloadConfig();
        closeTab(id);
    }, [reloadConfig, closeTab]);

    const handleDuplicate = useCallback(async (id: string) => {
        let r = loadedEntities[id];
        if (!r) {
            const res = await window.api.loadEntity(config.activeWorkspaceId, "grpcRequests", id);
            if (res.ok && res.entity) r = res.entity as SavedGrpcRequest;
        }
        if (!r) return;
        const { id: _id, createdAt: _ca, workspaceId: _ws, ...rest } = r;
        await window.api.addGrpcRequest({ ...rest, name: r.name ? strings.grpc.copySuffix.replace("{name}", r.name) : "" });
        await reloadConfig();
    }, [loadedEntities, config.activeWorkspaceId, reloadConfig]);

    const handleFoldersChange = useCallback(async () => {
        await reloadConfig();
    }, [reloadConfig]);

    const handleMoveItems = useCallback(async (ids: string[], folderId: string | null) => {
        for (const id of ids) {
            let r = loadedEntities[id];
            if (!r) {
                const res = await window.api.loadEntity(config.activeWorkspaceId, "grpcRequests", id);
                if (res.ok && res.entity) r = res.entity as SavedGrpcRequest;
            }
            if (r) await window.api.updateGrpcRequest({ ...r, folderId: folderId ?? undefined } as any);
        }
        await reloadConfig();
    }, [loadedEntities, config.activeWorkspaceId, reloadConfig]);

    const tabLabel = (tabId: string): string => {
        if (isDraft(tabId)) {
            const draft = loadDraft<GrpcRequestDraft>(tabId);
            if (draft?.serviceName && draft?.methodName) return `${draft.serviceName}/${draft.methodName}`;
            if (draft?.serviceName) return draft.serviceName;
            return strings.grpc.newRequestTab;
        }
        const r = requests.find((x) => x.id === tabId);
        if (!r) return "…";
        if (r.name) return r.name;
        if (r.serviceName && r.methodName) return `${r.serviceName}/${r.methodName}`;
        return r.serviceName || strings.grpc.requestLabel;
    };

    const folderViewItems: FolderTreeItem[] = useMemo(() => {
        return requests.map((r): FolderTreeItem => ({
            id: r.id,
            name: r.name || `${r.serviceName}/${r.methodName}` || strings.grpc.unnamed,
            folderId: r.folderId ?? null,
            isActive: activeTab === r.id,
            isEnabled: true,
        }));
    }, [requests, activeTab]);

    const draftTabIds = openTabs.filter(isDraft);

    // -- Sidebar ------------------------------------------------------------

    const sidebarContent = (
        <>
            <SidebarHeader onCollapse={() => setSidebarOpen(false)} collapseTitle={strings.grpc.collapseSidebar}>
                <span className="text-xs font-semibold px-1 text-muted-foreground uppercase tracking-wider">
                    {strings.grpc.requests}
                </span>
            </SidebarHeader>
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
                    kind="grpcRequest"
                    folders={folders}
                    items={folderViewItems}
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
                newTabTitle={strings.grpc.newRequestTabTitle}
                closeTabTitle={strings.grpc.closeTab}
                onCloseOthers={closeOtherTabs}
                onCloseAll={closeAllTabs}
                onTabDuplicate={handleDuplicate}
            />

            <div className="flex-1 overflow-hidden relative">
                {openTabs.length === 0 ? (
                    <div className="flex flex-col items-center justify-center h-full text-center gap-2">
                        <div className="opacity-10 mb-1"><Network size={48} /></div>
                        <div className="text-sm font-medium text-foreground">{strings.grpc.noRequestsOpen}</div>
                        <p className="text-xs text-muted-foreground max-w-xs leading-relaxed">
                            {strings.grpc.noRequestsOpenHint}
                        </p>
                    </div>
                ) : (
                    openTabs.map((tabId) => {
                        const isUnsaved = isDraft(tabId);
                        const req = isUnsaved ? null : (loadedEntities[tabId] ?? requests.find((r) => r.id === tabId) ?? null);
                        const initialData = isUnsaved ? null : req;
                        if (!isUnsaved && !req) return null;
                        const relPath = req ? entityRelPath("grpcRequests", req, folders) : "";
                        const syncStatus = relPath ? entitySyncStatus?.[relPath] : undefined;
                        return (
                            <div key={tabId} className="absolute inset-0 flex flex-col overflow-hidden" style={{ display: activeTab === tabId ? "flex" : "none" }}>
                                <GrpcTab
                                    ref={(el: GrpcTabHandle | null) => { tabRefs.current[tabId] = el; }}
                                    tabType="request"
                                    tabId={tabId}
                                    draftTabId={isUnsaved ? tabId : null}
                                    initial={initialData}
                                    folders={folders}
                                    activeEnv={activeEnv}
                                    onSave={(data) => isUnsaved
                                        ? handleNewSave(tabId, data as Omit<SavedGrpcRequest, "id" | "createdAt" | "workspaceId">)
                                        : handleTabSave(tabId, data as Omit<SavedGrpcRequest, "id" | "createdAt" | "workspaceId">)
                                    }
                                    onClose={() => closeTab(tabId)}
                                    onSync={onPublishItem ? async (savedId?: string) => {
                                        const targetId = savedId ?? tabId;
                                        await onPublishItem(targetId);
                                    } : undefined}
                                    onRevert={onRestoreItem ? async () => {
                                        await onRestoreItem(tabId);
                                        const res = await window.api.loadEntity(config.activeWorkspaceId, "grpcRequests", tabId);
                                        if (res.ok && res.entity) {
                                            const entity = res.entity as SavedGrpcRequest;
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
                storageKey="grpc-requests-panel-sidebar"
                collapsedBadge={requests.length > 0 ? (
                    <span className="text-[9px] text-muted-foreground font-mono" title={strings.grpc.requestCount.replace("{count}", String(requests.length))}
                        style={{ writingMode: "vertical-rl", transform: "rotate(180deg)", lineHeight: 1.4 }}>{requests.length}</span>
                ) : undefined}
            >
                {mainContent}
            </SidebarLayout>
        </>
    );
}
