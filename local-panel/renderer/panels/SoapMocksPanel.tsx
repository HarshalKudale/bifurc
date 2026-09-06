import React, { useState, useMemo, useCallback } from "react";
import { AppConfig, SavedSoapMock, Folder, Environment } from "@/types";
import FolderTree, { FolderTreeItem } from "@/components/sidebar/FolderTree";
import SoapTab, { SoapTabHandle } from "@/components/soap/SoapTab";
import DraftsFolder from "@/components/sidebar/DraftsFolder";
import { loadDraft } from "@/lib/useDraftPersist";
import { useEntityTabs } from "@/lib/useEntityTabs";
import { calculateFolderStatus, entityRelPath } from "@/lib/utils";
import { FileCode } from "@/lib/icons";
import TabBar from "@/components/editor/TabBar";
import { SidebarLayout, SidebarHeader } from "@/components/ui";
import { strings } from "@/lib/strings";
import { usePersistedState } from "@/lib/usePersistedState";
import { useTabKeyBindings } from "@/hooks/useTabKeyBindings";


// -- Draft tab prefix -------------------------------------------------------

const DRAFT_PREFIX = "soap-mock-draft-";
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

// -- SoapMocksPanel ---------------------------------------------------------

export default function SoapMocksPanel({ config, onConfigChange, activeEnv = null, onHistoryOpen, entitySyncStatus, onPublishItem, onRestoreItem }: Props) {
    const mocks = config.soapMocks ?? [];
    const folders = config.soapMockFolders ?? [];

    const [sidebarOpen, setSidebarOpen] = usePersistedState(`soap-mocks:${config.activeWorkspaceId}:sidebar-open`, true);

    const {
        openTabs, activeTab, setActiveTab,
        loadedEntities, setLoadedEntities,
        tabRefs, isDraft,
        openTab, openNewTab, closeTab, replaceTab, closeOtherTabs, closeAllTabs,
    } = useEntityTabs<SavedSoapMock>({
        storageKey: "soapMocks",
        draftPrefix: DRAFT_PREFIX,
        extraDraftPrefixes: [],
        workspaceId: config.activeWorkspaceId,
        entityKind: "soapMocks" as any,
        entities: mocks,
    });

    useTabKeyBindings({ activeTab, tabRefs, closeTab, openNewTab });

    const reloadConfig = useCallback(async () => {
        const fresh = await window.api.getConfig();
        await onConfigChange(fresh);
    }, [onConfigChange]);

    const handleNewSave = useCallback(async (tabId: string, data: Omit<SavedSoapMock, "id" | "createdAt" | "workspaceId">) => {
        const created = await window.api.addSoapMock(data);
        await reloadConfig();
        replaceTab(tabId, created.id);
        return created;
    }, [reloadConfig, replaceTab]);

    const handleTabSave = useCallback(async (tabId: string, data: Omit<SavedSoapMock, "id" | "createdAt" | "workspaceId">) => {
        const mock = loadedEntities[tabId] ?? mocks.find((m) => m.id === tabId);
        if (!mock) return;
        const updated = { ...mock, ...data };
        setLoadedEntities((prev) => ({ ...prev, [tabId]: updated }));
        await window.api.updateSoapMock(updated);
        await reloadConfig();
        return updated;
    }, [loadedEntities, mocks, reloadConfig]);

    const handleDelete = useCallback(async (id: string) => {
        await window.api.deleteSoapMock(id);
        await reloadConfig();
        closeTab(id);
    }, [reloadConfig, closeTab]);

    const handleDuplicate = useCallback(async (id: string) => {
        let m = loadedEntities[id];
        if (!m) {
            const res = await window.api.loadEntity(config.activeWorkspaceId, "soapMocks", id);
            if (res.ok && res.entity) m = res.entity as SavedSoapMock;
        }
        if (!m) return;
        const { id: _id, createdAt: _ca, workspaceId: _ws, ...rest } = m;
        await window.api.addSoapMock({ ...rest, name: m.name ? strings.soap.copySuffix.replace("{name}", m.name) : "" });
        await reloadConfig();
    }, [loadedEntities, config.activeWorkspaceId, reloadConfig]);

    const handleMoveItems = useCallback(async (ids: string[], folderId: string | null) => {
        for (const id of ids) {
            let m = loadedEntities[id];
            if (!m) {
                const res = await window.api.loadEntity(config.activeWorkspaceId, "soapMocks", id);
                if (res.ok && res.entity) m = res.entity as SavedSoapMock;
            }
            if (m) await window.api.updateSoapMock({ ...m, folderId: folderId ?? undefined });
        }
        await reloadConfig();
    }, [loadedEntities, config.activeWorkspaceId, reloadConfig]);

    const handleFoldersChange = useCallback(async () => {
        await reloadConfig();
    }, [reloadConfig]);

    const draftTabIds = openTabs.filter(isDraft);

    interface DraftSnapshot { name?: string; soapActionPattern?: string; }

    const tabLabel = (tabId: string) => {
        if (isDraft(tabId)) {
            const d = loadDraft<DraftSnapshot>(tabId);
            if (d?.soapActionPattern) return d.soapActionPattern.slice(0, 30);
            return strings.soap.newMockTab;
        }
        const m = mocks.find((x) => x.id === tabId);
        if (!m) return "…";
        return m.name || m.soapActionPattern || strings.soap.mockLabel;
    };

    const folderViewItems: FolderTreeItem[] = useMemo(() => {
        return mocks.map((m): FolderTreeItem => ({
            id: m.id,
            name: m.name || m.soapActionPattern || strings.soap.mockLabel,
            folderId: m.folderId ?? null,
            isActive: activeTab === m.id,
            isEnabled: m.enabled,
        }));
    }, [mocks, activeTab]);

    const folderStatusMap = useMemo(() => calculateFolderStatus(mocks, folders), [mocks, folders]);

    // -- Sidebar ------------------------------------------------------------

    const sidebarContent = (
        <>
            <SidebarHeader onCollapse={() => setSidebarOpen(false)} collapseTitle={strings.soap.collapseSidebar}>
                <span className="text-xs font-semibold px-1 text-muted-foreground uppercase tracking-wider">
                    {strings.soap.mocks}
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
                    kind="soapMock"
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
                newTabTitle={strings.soap.newMockTabTitle}
                closeTabTitle={strings.soap.closeTab}
                onCloseOthers={closeOtherTabs}
                onCloseAll={closeAllTabs}
                onTabDuplicate={handleDuplicate}
            />

            <div className="flex-1 overflow-hidden relative">
                {openTabs.length === 0 ? (
                    <div className="flex flex-col items-center justify-center h-full text-center gap-2">
                        <div className="opacity-10 mb-1"><FileCode size={48} /></div>
                        <div className="text-sm font-medium text-foreground">{strings.soap.noMocksOpen}</div>
                        <p className="text-xs text-muted-foreground max-w-xs leading-relaxed">
                            {strings.soap.noMocksOpenHintBefore} <span className="text-signal font-semibold">+</span> {strings.soap.noMocksOpenHintAfter}
                        </p>
                    </div>
                ) : (
                    openTabs.map((tabId) => {
                        const isUnsaved = isDraft(tabId);
                        const mock = isUnsaved ? null : (loadedEntities[tabId] ?? mocks.find((m) => m.id === tabId) ?? null);
                        const initialData = isUnsaved ? null : mock;
                        if (!isUnsaved && !mock) return null;
                        const relPath = mock ? entityRelPath("soapMocks", mock, folders) : "";
                        const syncStatus = relPath ? entitySyncStatus?.[relPath] : undefined;
                        return (
                            <div key={tabId} className="absolute inset-0 flex flex-col overflow-hidden" style={{ display: activeTab === tabId ? "flex" : "none" }}>
                                <SoapTab
                                    ref={(el) => { (tabRefs as any).current[tabId] = el; }}
                                    tabType="mock"
                                    tabId={tabId}
                                    draftTabId={isUnsaved ? tabId : null}
                                    initial={initialData}
                                    folders={folders}
                                    activeEnv={activeEnv}
                                    onSave={(data) => isUnsaved
                                        ? handleNewSave(tabId, data as Omit<SavedSoapMock, "id" | "createdAt" | "workspaceId">)
                                        : handleTabSave(tabId, data as Omit<SavedSoapMock, "id" | "createdAt" | "workspaceId">)
                                    }
                                    onClose={() => closeTab(tabId)}
                                    onSync={onPublishItem ? async (savedId?: string) => {
                                        const targetId = savedId ?? tabId;
                                        await onPublishItem(targetId);
                                    } : undefined}
                                    onRevert={onRestoreItem ? async () => {
                                        await onRestoreItem(tabId);
                                        const res = await window.api.loadEntity(config.activeWorkspaceId, "soapMocks", tabId);
                                        if (res.ok && res.entity) {
                                            const entity = res.entity as SavedSoapMock;
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
                storageKey="soap-mocks-panel-sidebar"
                collapsedBadge={mocks.length > 0 ? (
                    <span className="text-[9px] text-muted-foreground font-mono" title={strings.soap.mockCount.replace("{count}", String(mocks.length))}
                        style={{ writingMode: "vertical-rl", transform: "rotate(180deg)", lineHeight: 1.4 }}>{mocks.length}</span>
                ) : undefined}
            >
                {mainContent}
            </SidebarLayout>
        </>
    );
}
