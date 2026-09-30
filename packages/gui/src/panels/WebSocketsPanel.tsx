import React, { forwardRef, useImperativeHandle, useState, useEffect, useMemo, useCallback, useRef } from "react";
import { AppConfig, SavedWsConnection, Folder as FolderType, Environment } from "@/types";
import FolderTree from "@/components/sidebar/FolderTree";
import { FolderTreeItem } from "@/components/sidebar/FolderTree.types";
import EditorTitleBar from "@/components/editor/EditorTitleBar";
import { UrlBar, TabStrip, BottomBar } from "@/components/editor/RequestTab";
import HeaderTable from "@/components/common/HeaderTable";
import BodyEditor from "@/components/common/BodyEditor";
import CodeEditor from "@/components/common/CodeEditor";
import EnvVarHint from "@/components/editor/EnvVarHint";
import RandomizerHint from "@/components/editor/RandomizerHint";
import {
  KVRow, mkRowId, headersToRows, rowsToHeaders, tryFormat,
} from "@/lib/utils";
import { useEntityPaths } from "@/lib/entityPaths";
import { usePersistedState } from "@/hooks/usePersistedState";
import { useDraftPersist, loadDraft, clearDraft, getDraftIds } from "@/hooks/useDraftPersist";
import { Group as PanelGroup, Panel, Separator as PanelResizeHandle } from "react-resizable-panels";
import { Plus, X, Folder, Zap, Play, Send, Radio } from "@/lib/icons";
import { strings } from "@/lib/strings";
import TabBar from "@/components/editor/TabBar";
import { SidebarLayout, SidebarHeader } from "@/components/ui";
import { useTabKeyBindings } from "@/hooks/useTabKeyBindings";
import WsEditor, { WsEditorHandle } from "./websockets/WsEditor";
import WsSidebar from "./websockets/WsSidebar";
import WsTabHeader from "./websockets/WsTabHeader";
import { useEntityTabs } from "@/hooks/useEntityTabs";

// -- Constants --------------------------------------------------------------

const DRAFT_PREFIX = "ws-draft-";
const isDraft = (id: string) => id.startsWith(DRAFT_PREFIX);

const WS_METHODS = ["WS"];

// -- Draft type -------------------------------------------------------------

export interface WsDraft {
  name: string;
  url: string;
  folderId: string | null;
  headers: Record<string, string>;
}

// -- Tree sidebar -----------------------------------------------------------

// -- Props ------------------------------------------------------------------

interface Props {
  config: AppConfig;
  onConfigChange(cfg: AppConfig): Promise<void>;
  activeEnv?: Environment | null;
  onHistoryOpen?: (filePath: string) => void;
  onEntityPathChange?: (filePath: string) => void;
  historyOpen?: boolean;
  onAfterSave?: () => void;
  entitySyncStatus?: Record<string, "clean" | "modified" | "new" | "deleted">;
  /** The workspace's position relative to its remote — needed for "committed but not pushed". */
  workspaceGitState?: import("@/types").WorkspaceGitState | null;
  onPublishItem?: (id: string) => void;
  /** Publish a whole selection in one commit. */
  onPublishPaths?: (paths: string[]) => void;
  onPublishFolder?: (folderId: string | null) => void;
  onRestoreItem?: (id: string) => void;
}

// -- WebSocketsPanel --------------------------------------------------------

export default function WebSocketsPanel({ config, onConfigChange, activeEnv = null, onHistoryOpen, onEntityPathChange, historyOpen = false, onAfterSave, entitySyncStatus, workspaceGitState, onPublishItem, onPublishPaths, onPublishFolder, onRestoreItem }: Props) {
  const connections = config.wsConnections ?? [];
  const folders = config.wsFolders ?? [];

  const [sidebarOpen, setSidebarOpen] = usePersistedState(`sockets:${config.activeWorkspaceId}:sidebar-open`, true);
  const [dirtyTabs, setDirtyTabs] = useState<Record<string, boolean>>({});

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
    replaceTab,
    reorderTabs: handleReorderTabs,
  } = useEntityTabs<SavedWsConnection>({
    storageKey: `sockets:${config.activeWorkspaceId}`,
    draftPrefix: DRAFT_PREFIX,
    workspaceId: config.activeWorkspaceId,
    entityKind: 'sockets',
    entities: connections,
  });

  /**
   * **Closing a socket tab is a user gesture, so it disconnects — and that is not D29 being violated.**
   *
   * D29 forbids inferring a close from a client going away: a closed window, a reload, a panel switch must all
   * leave the engine's socket alone (`plan/19` WI 6). An *unmount* cannot tell those apart from "the user
   * closed this tab", which is exactly why `useWebSocket` has no disconnect-on-unmount and why the close lives
   * here instead — on the handler the TabBar, the title bar's ✕ and Ctrl+W all end up calling.
   *
   * Without this the socket would be unreachable: the tab is the only place it is rendered, so a socket that
   * outlived its tab would be a connection counting against the limit with nothing to disconnect it.
   */
  const closeSocketTab = useCallback((id: string) => {
    void window.api.wsDisconnect(config.activeWorkspaceId, id);
    closeTab(id);
  }, [config.activeWorkspaceId, closeTab]);

  useTabKeyBindings({ activeTab, tabRefs, closeTab: closeSocketTab, openNewTab });

  const reloadConnections = useCallback(async () => {
    const fresh = await window.api.getConfig();
    await onConfigChange(fresh);
  }, [onConfigChange]);

  /** The engine's `sockets/<id>.json` (`plan/19` WI 5 item 3) — see `useEntityPaths`. */
  const { relPathOf } = useEntityPaths();

  const getEntityFilePath = useCallback((tabId: string): string => {
    if (isDraft(tabId)) return "";
    const c = connections.find((x) => x.id === tabId);
    if (!c) return "";
    return relPathOf(c.id);
  }, [connections, relPathOf]);

  useEffect(() => {
    if (!historyOpen || !activeTab) return;
    const path = getEntityFilePath(activeTab);
    if (path) onEntityPathChange?.(path);
  }, [activeTab, historyOpen, getEntityFilePath, onEntityPathChange]);

  const handleFoldersChange = useCallback(async () => {
    const fresh = await window.api.getConfig();
    await onConfigChange(fresh);
  }, [onConfigChange]);

  const handleNewSave = useCallback(async (tabId: string, data: Omit<SavedWsConnection, "id" | "createdAt" | "workspaceId">) => {
    const created = await window.api.addWsConnection(data);
    await reloadConnections();
    /**
     * `replaceTab`, not a hand-rolled `setOpenTabs`.
     *
     * **This was a live bug.** `setOpenTabs` was never destructured from `useEntityTabs` — the compiler
     * said so as `TS2552: Cannot find name 'setOpenTabs'. Did you mean 'openTabs'?` — so saving a new
     * WebSocket threw `ReferenceError: setOpenTabs is not defined` in a panel `panelFactory` renders.
     *
     * The two lines that were here were a partial copy of `replaceTab`'s body, which does the same thing
     * plus the `clearDraft` and `tabRefs` cleanup they had forgotten. The type error was the whole warning
     * and it was being ignored; see `TESTING.md` on the pre-existing error count.
     */
    replaceTab(tabId, created.id);
    onAfterSave?.();
    return created;
  }, [reloadConnections, onAfterSave, replaceTab]);

  const handleTabSave = useCallback(async (tabId: string, data: Omit<SavedWsConnection, "id" | "createdAt" | "workspaceId">) => {
    const conn = loadedEntities[tabId] ?? connections.find((c) => c.id === tabId);
    if (!conn) return;
    const updated = { ...conn, ...data };
    setLoadedEntities((prev) => ({ ...prev, [tabId]: updated }));
    await window.api.updateWsConnection(updated);
    await reloadConnections();
    onAfterSave?.();
    return updated;
  }, [loadedEntities, connections, reloadConnections, onAfterSave]);

  const handleDelete = useCallback(async (id: string) => {
    await window.api.deleteWsConnection(id);
    await reloadConnections();
    closeTab(id);
  }, [reloadConnections, closeTab]);

  const handleDuplicate = useCallback(async (id: string) => {
    let c = loadedEntities[id];
    if (!c) {
      const res = await window.api.loadEntity(config.activeWorkspaceId, "sockets", id);
      if (res.ok && res.entity) c = res.entity as SavedWsConnection;
    }
    if (!c) return;
    const { id: _id, createdAt: _ca, workspaceId: _ws, ...rest } = c;
    await window.api.addWsConnection({ ...rest, name: c.name ? `${c.name} (copy)` : "" });
    await reloadConnections();
  }, [loadedEntities, config.activeWorkspaceId, reloadConnections]);

  const handleMoveItems = useCallback(async (ids: string[], folderId: string | null) => {
    for (const id of ids) {
      let c = loadedEntities[id] ?? connections.find((x) => x.id === id);
      if (!c) {
        const res = await window.api.loadEntity(config.activeWorkspaceId, "sockets", id);
        if (res.ok && res.entity) c = res.entity as SavedWsConnection;
      }
      if (c) await window.api.updateWsConnection({ ...c, folderId: folderId ?? undefined });
    }
    await reloadConnections();
  }, [loadedEntities, connections, config.activeWorkspaceId, reloadConnections]);

  const tabLabel = (tabId: string) => {
    if (isDraft(tabId)) {
      const d = loadDraft<WsDraft>(tabId);
      if (d?.url) {
        try { const u = new URL(d.url); return d.name || u.host || d.url.slice(0, 20); }
        catch { return d.name || d.url.slice(0, 20); }
      }
      return strings.sockets.newSocket;
    }
    const c = connections.find((x) => x.id === tabId);
    if (!c) return "…";
    return c.name || c.url.slice(0, 30);
  };

  const tabItems = useMemo(() => openTabs.map((id) => ({
    id,
    label: tabLabel(id),
    isDraft: isDraft(id),
    isModified: dirtyTabs[id],
    renderTab: () => (
      <WsTabHeader
        tabId={id}
        label={tabLabel(id)}
      />
    ),
  })), [openTabs, dirtyTabs, connections]); // connections included implicitly via tabLabel

  // -- Main content ---------------------------------------------------------

  const mainContent = (
    <div className="flex flex-col flex-1 overflow-hidden min-w-0 h-full">
      <TabBar
        tabs={tabItems}
        activeTab={activeTab}
        onTabClick={setActiveTab}
        onTabClose={closeSocketTab}
        onNewTab={openNewTab}
        onReorderTabs={handleReorderTabs}
        newTabTitle={strings.sockets.newTab}
        closeTabTitle={strings.common.close}
        onCloseOthers={useCallback((id: string) => {
          openTabs.filter((t) => t !== id).forEach(closeSocketTab);
        }, [openTabs, closeSocketTab])}
        onCloseAll={useCallback(() => {
          [...openTabs].forEach(closeSocketTab);
        }, [openTabs, closeSocketTab])}
      />

      <div className="flex-1 overflow-hidden relative">
        {openTabs.length === 0 ? (
          <div className="flex flex-col items-center justify-center h-full text-center gap-2">
            <div className="opacity-10 mb-1"><Zap size={48} /></div>
            <div className="text-sm font-medium text-foreground">{strings.sockets.noSocketsOpen}</div>
            <p className="text-xs text-muted-foreground max-w-xs leading-relaxed">
              {strings.sockets.noSocketsHintPrefix} <span className="text-signal font-semibold">+</span> {strings.sockets.noSocketsHintSuffix}
            </p>
          </div>
        ) : (
          openTabs.map((tabId) => {
            const isUnsaved = isDraft(tabId);
            const conn = isUnsaved ? null : (loadedEntities[tabId] ?? connections.find((c) => c.id === tabId) ?? null);
            if (!isUnsaved && !conn) return null;
            const relPath = conn ? relPathOf(conn.id) : "";
            const syncStatus = relPath ? entitySyncStatus?.[relPath] : undefined;
            return (
              <div key={tabId} className="absolute inset-0 flex flex-col overflow-hidden" style={{ display: activeTab === tabId ? "flex" : "none" }}>
                <WsEditor
                  ref={(el) => { tabRefs.current[tabId] = el; }}
                  key={tabId}
                  tabId={tabId}
                  workspaceId={config.activeWorkspaceId}
                  initial={conn}
                  isNew={isUnsaved}
                  onSave={isUnsaved ? (data) => handleNewSave(tabId, data) : (data) => handleTabSave(tabId, data)}
                  onClose={() => closeSocketTab(tabId)}
                  folders={folders}
                  activeEnv={activeEnv}
                  onDirtyChange={(dirty) => setDirtyTabs((prev) => ({ ...prev, [tabId]: dirty }))}
                  onSync={onPublishItem ? async (savedId?: string) => {
                    const targetId = savedId ?? tabId;
                    await onPublishItem(targetId);
                  } : undefined}
                  onRevert={onRestoreItem ? async () => {
                    await onRestoreItem(tabId);
                    const res = await window.api.loadEntity(config.activeWorkspaceId, "sockets", tabId);
                    if (res.ok && res.entity) {
                      const entity = res.entity as SavedWsConnection;
                      setLoadedEntities((prev) => ({ ...prev, [tabId]: entity }));
                      tabRefs.current[tabId]?.refresh?.(entity);
                    } else if (!res.ok) {
                      closeTab(tabId);
                    }
                    setDirtyTabs((prev) => ({ ...prev, [tabId]: false }));
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
        sidebar={
          <WsSidebar
            connections={connections}
            folders={folders}
            activeTab={activeTab}
            openTab={openTab}
            handleDelete={handleDelete}
            handleFoldersChange={handleFoldersChange}
            handleDuplicate={handleDuplicate}
            handleMoveItems={handleMoveItems}
            openNewTab={openNewTab}
            onHistoryOpen={onHistoryOpen}
            getEntityFilePath={getEntityFilePath}
            entitySyncStatus={entitySyncStatus}
            workspaceGitState={workspaceGitState}
            onPublishItem={onPublishItem}
            onPublishPaths={onPublishPaths}
            onPublishFolder={onPublishFolder}
            onRestoreItem={onRestoreItem}
            setSidebarOpen={setSidebarOpen}
          />
        }
        collapseTitle={strings.titleBar.collapseSidebar}
        expandTitle={strings.titleBar.expandSidebar}
        storageKey="websockets-panel-sidebar"
        collapsedBadge={connections.length > 0 ? (
          <span className="text-[9px] text-muted-foreground font-mono" title={`${connections.length} sockets`}
            style={{ writingMode: "vertical-rl", transform: "rotate(180deg)", lineHeight: 1.4 }}>{connections.length}</span>
        ) : undefined}
      >
        {mainContent}
      </SidebarLayout>
    </>
  );
}
