import React, { useState, useMemo, useCallback, useRef } from "react";
import { AppConfig, SavedSoapRequest, Folder, Environment } from "@/types";
import FolderTree, { FolderTreeItem } from "@/components/sidebar/FolderTree";
import SoapTab, { SoapTabHandle } from "@/components/soap/SoapTab";
import DraftsFolder from "@/components/sidebar/DraftsFolder";
import { loadDraft } from "@/lib/useDraftPersist";
import { useEntityTabs } from "@/lib/useEntityTabs";
import { FileCode } from "@/lib/icons";
import TabBar from "@/components/editor/TabBar";
import { SidebarLayout, SidebarHeader } from "@/components/ui";
import { strings } from "@/lib/strings";
import { usePersistedState } from "@/lib/usePersistedState";
import { useTabKeyBindings } from "@/hooks/useTabKeyBindings";
import { entityRelPath } from "@/lib/utils";


// -- Draft tab prefix -------------------------------------------------------

const DRAFT_PREFIX = "soap-req-draft-";
const isDraftCheck = (id: string) => id.startsWith(DRAFT_PREFIX);

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

// -- SoapRequestsPanel ------------------------------------------------------

export default function SoapRequestsPanel({ config, onConfigChange, activeEnv = null, onHistoryOpen, entitySyncStatus, onPublishItem, onRestoreItem }: Props) {
    const requests = config.soapRequests ?? [];
    const folders = config.soapRequestFolders ?? [];

    const [sidebarOpen, setSidebarOpen] = usePersistedState(`soap-requests:${config.activeWorkspaceId}:sidebar-open`, true);

    const {
        openTabs, activeTab, setActiveTab,
        loadedEntities, setLoadedEntities,
        tabRefs, isDraft,
        openTab, openNewTab, closeTab, replaceTab, closeOtherTabs, closeAllTabs,
    } = useEntityTabs<SavedSoapRequest>({
        storageKey: "soapRequests",
        draftPrefix: DRAFT_PREFIX,
        extraDraftPrefixes: [],
        workspaceId: config.activeWorkspaceId,
        entityKind: "soapRequests" as any,
        entities: requests,
    });

    useTabKeyBindings({ activeTab, tabRefs, closeTab, openNewTab });

    const reloadConfig = useCallback(async () => {
        const fresh = await window.api.getConfig();
        await onConfigChange(fresh);
    }, [onConfigChange]);

    const handleNewSave = useCallback(async (tabId: string, data: Omit<SavedSoapRequest, "id" | "createdAt" | "workspaceId">) => {
        const created = await window.api.addSoapRequest(data);
        await reloadConfig();
        replaceTab(tabId, created.id);
        return created;
    }, [reloadConfig, replaceTab]);

    const handleTabSave = useCallback(async (tabId: string, data: Omit<SavedSoapRequest, "id" | "createdAt" | "workspaceId">) => {
        const req = loadedEntities[tabId] ?? requests.find((r) => r.id === tabId);
        if (!req) return;
        const updated = { ...req, ...data };
        setLoadedEntities((prev) => ({ ...prev, [tabId]: updated }));
        await window.api.updateSoapRequest(updated);
        await reloadConfig();
        return updated;
    }, [loadedEntities, requests, reloadConfig]);

    const handleDelete = useCallback(async (id: string) => {
        await window.api.deleteSoapRequest(id);
        await reloadConfig();
        closeTab(id);
    }, [reloadConfig, closeTab]);

    const handleDuplicate = useCallback(async (id: string) => {
        let r = loadedEntities[id];
        if (!r) {
            const res = await window.api.loadEntity(config.activeWorkspaceId, "soapRequests", id);
            if (res.ok && res.entity) r = res.entity as SavedSoapRequest;
        }
        if (!r) return;
        const { id: _id, createdAt: _ca, workspaceId: _ws, ...rest } = r;
        await window.api.addSoapRequest({ ...rest, name: r.name ? strings.soap.copySuffix.replace("{name}", r.name) : "" });
        await reloadConfig();
    }, [loadedEntities, config.activeWorkspaceId, reloadConfig]);

    const handleMoveItems = useCallback(async (ids: string[], folderId: string | null) => {
        for (const id of ids) {
            let r = loadedEntities[id];
            if (!r) {
                const res = await window.api.loadEntity(config.activeWorkspaceId, "soapRequests", id);
                if (res.ok && res.entity) r = res.entity as SavedSoapRequest;
            }
            if (r) await window.api.updateSoapRequest({ ...r, folderId: folderId ?? undefined });
        }
        await reloadConfig();
    }, [loadedEntities, config.activeWorkspaceId, reloadConfig]);

    const handleFoldersChange = useCallback(async () => {
        await reloadConfig();
    }, [reloadConfig]);

    const draftTabIds = openTabs.filter(isDraft);

    interface DraftSnapshot { name?: string; endpointUrl?: string; }

    const tabLabel = (tabId: string) => {
        if (isDraft(tabId)) {
            const d = loadDraft<DraftSnapshot>(tabId);
            if (d?.endpointUrl) {
                try { const u = new URL(d.endpointUrl); return u.pathname || u.host; } catch { return d.endpointUrl.slice(0, 20); }
            }
            return strings.soap.newRequestTab;
        }
        const r = requests.find((x) => x.id === tabId);
        if (!r) return "…";
        if (r.name) return r.name;
        if (r.endpointUrl) {
            try { const u = new URL(r.endpointUrl); return u.pathname || u.host; } catch { return r.endpointUrl.slice(0, 20); }
        }
        return strings.soap.requestLabel;
    };

    const folderViewItems: FolderTreeItem[] = useMemo(() => {
        return requests.map((r): FolderTreeItem => ({
            id: r.id,
            name: r.name || r.endpointUrl?.slice(0, 40) || strings.soap.requestLabel,
            folderId: r.folderId ?? null,
            isActive: activeTab === r.id,
            isEnabled: true,
        }));
    }, [requests, activeTab]);

    // -- Sidebar ------------------------------------------------------------

    const sidebarContent = (
        <>
            <SidebarHeader onCollapse={() => setSidebarOpen(false)} collapseTitle={strings.soap.collapseSidebar}>
                <span className="text-xs font-semibold px-1 text-muted-foreground uppercase tracking-wider">
                    {strings.soap.requests}
                </span>
            </SidebarHeader>
            <div className="flex-1 overflow-y-auto overflow-x-auto min-w-0" style={{ display: "flex", flexDirection: "column" }}>
                {draftTabIds.length > 0 && (
                    <DraftsFolder
                        label={strings.soap.drafts}
                        draftTabIds={draftTabIds}
                        activeTab={activeTab}
                        onOpenTab={(id) => setActiveTab(id)}
                        onCloseTab={closeTab}
                        tabLabel={tabLabel}
                    />
                )}
                <FolderTree
                    kind="soapRequest"
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
                newTabTitle={strings.soap.newRequestTabTitle}
                closeTabTitle={strings.soap.closeTab}
                onCloseOthers={closeOtherTabs}
                onCloseAll={closeAllTabs}
                onTabDuplicate={handleDuplicate}
            />

            <div className="flex-1 overflow-hidden relative">
                {openTabs.length === 0 ? (
                    <div className="flex flex-col items-center justify-center h-full text-center gap-2">
                        <div className="opacity-10 mb-1"><FileCode size={48} /></div>
                        <div className="text-sm font-medium text-foreground">{strings.soap.noRequestsOpen}</div>
                        <p className="text-xs text-muted-foreground max-w-xs leading-relaxed">
                            {strings.soap.noRequestsOpenHint}
                        </p>
                    </div>
                ) : (
                    openTabs.map((tabId) => {
                        const isUnsaved = isDraft(tabId);
                        const req = isUnsaved ? null : (loadedEntities[tabId] ?? requests.find((r) => r.id === tabId) ?? null);
                        const initialData = isUnsaved ? null : req;
                        if (!isUnsaved && !req) return null;
                        const relPath = req ? entityRelPath("soapRequests", req, folders) : "";
                        const syncStatus = relPath ? entitySyncStatus?.[relPath] : undefined;
                        return (
                            <div key={tabId} className="absolute inset-0 flex flex-col overflow-hidden" style={{ display: activeTab === tabId ? "flex" : "none" }}>
                                <SoapTab
                                    ref={(el) => { (tabRefs as any).current[tabId] = el; }}
                                    tabType="request"
                                    tabId={tabId}
                                    draftTabId={isUnsaved ? tabId : null}
                                    initial={initialData}
                                    folders={folders}
                                    activeEnv={activeEnv}
                                    onSave={(data) => isUnsaved
                                        ? handleNewSave(tabId, data as Omit<SavedSoapRequest, "id" | "createdAt" | "workspaceId">)
                                        : handleTabSave(tabId, data as Omit<SavedSoapRequest, "id" | "createdAt" | "workspaceId">)
                                    }
                                    onClose={() => closeTab(tabId)}
                                    onSync={onPublishItem ? async (savedId?: string) => {
                                        const targetId = savedId ?? tabId;
                                        await onPublishItem(targetId);
                                    } : undefined}
                                    onRevert={onRestoreItem ? async () => {
                                        await onRestoreItem(tabId);
                                        const res = await window.api.loadEntity(config.activeWorkspaceId, "soapRequests", tabId);
                                        if (res.ok && res.entity) {
                                            const entity = res.entity as SavedSoapRequest;
                                            setLoadedEntities((prev) => ({ ...prev, [tabId]: entity }));
                                            (tabRefs as any).current[tabId]?.refresh?.(entity);
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
                collapseTitle={strings.soap.collapseSidebar}
                expandTitle={strings.soap.expandSidebar}
                storageKey="soap-requests-panel-sidebar"
                collapsedBadge={requests.length > 0 ? (
                    <span className="text-[9px] text-muted-foreground font-mono" title={strings.soap.requestCount.replace("{count}", String(requests.length))}
                        style={{ writingMode: "vertical-rl", transform: "rotate(180deg)", lineHeight: 1.4 }}>{requests.length}</span>
                ) : undefined}
            >
                {mainContent}
            </SidebarLayout>
        </>
    );
}
