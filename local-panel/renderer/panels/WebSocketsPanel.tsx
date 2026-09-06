import React, { forwardRef, useImperativeHandle, useState, useEffect, useMemo, useCallback, useRef } from "react";
import { AppConfig, SavedWsConnection, Folder as FolderType, Environment } from "@/types";
import FolderTree, { FolderTreeItem } from "@/components/sidebar/FolderTree";
import EditorTitleBar from "@/components/editor/EditorTitleBar";
import { UrlBar, TabStrip, BottomBar } from "@/components/editor/RequestTab";
import HeaderTable from "@/components/editor/HeaderTable";
import BodyEditor from "@/components/editor/BodyEditor";
import CodeEditor from "@/components/common/CodeEditor";
import EnvVarHint from "@/components/editor/EnvVarHint";
import RandomizerHint from "@/components/editor/RandomizerHint";
import { resolveVars } from "@/lib/resolveVars";
import {
  KVRow, mkRowId, headersToRows, rowsToHeaders, tryFormat, entityRelPath,
} from "@/lib/utils";
import { usePersistedState } from "@/lib/usePersistedState";
import { useDraftPersist, loadDraft, clearDraft, getDraftIds } from "@/lib/useDraftPersist";
import { useWebSocket, MAX_WS_CONNECTIONS, WsMessage } from "@/lib/useWebSocket";
import { Group as PanelGroup, Panel, Separator as PanelResizeHandle } from "react-resizable-panels";
import { Plus, X, Folder, Zap, Play, Send, Radio } from "@/lib/icons";
import { strings } from "@/lib/strings";
import TabBar from "@/components/editor/TabBar";
import { SidebarLayout, SidebarHeader } from "@/components/ui";
import { useTabKeyBindings } from "@/hooks/useTabKeyBindings";

// -- Constants --------------------------------------------------------------

const DRAFT_PREFIX = "ws-draft-";
const isDraft = (id: string) => id.startsWith(DRAFT_PREFIX);

const WS_METHODS = ["WS"];

// -- Draft type -------------------------------------------------------------

interface WsDraft {
  name: string;
  url: string;
  folderId: string | null;
  headers: Record<string, string>;
}

// -- WebSocket editor -------------------------------------------------------

interface WsEditorProps {
  tabId: string;
  initial: Partial<SavedWsConnection> | null;
  isNew: boolean;
  onSave(data: Omit<SavedWsConnection, "id" | "createdAt" | "workspaceId">): Promise<any>;
  onClose(): void;
  folders?: FolderType[];
  activeEnv?: Environment | null;
  onDirtyChange?: (dirty: boolean) => void;
  onSync?: (savedId?: string) => Promise<void>;
  onRevert?: () => Promise<void>;
  syncStatus?: "clean" | "modified" | "new" | "deleted";
  onHistory?: () => void;
}

export interface WsEditorHandle {
  save(): Promise<any> | void;
  refresh?(conn: SavedWsConnection): void;
}

const WsEditor = forwardRef<WsEditorHandle, WsEditorProps>(function WsEditor({ tabId, initial, isNew, onSave, onClose, folders = [], activeEnv = null, onDirtyChange, onSync, onRevert, syncStatus, onHistory }: WsEditorProps, ref) {
  const draft = isDraft(tabId) ? loadDraft<WsDraft>(tabId) : null;
  const src = draft ?? initial;

  const [name, setName] = useState(src?.name ?? "");
  const [url, setUrl] = useState(src?.url ?? "");
  const [folderId, setFolderId] = useState<string | null>(() => (src as SavedWsConnection | null)?.folderId ?? null);
  const [headers, setHeaders] = useState<KVRow[]>(() => headersToRows((draft?.headers ?? (initial as SavedWsConnection | null)?.headers) ?? {}));
  const [reqTab, setReqTab] = useState<"headers">("headers");

  const [outgoingInput, setOutgoingInput] = useState("");
  const [sendErr, setSendErr] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [saveErr, setSaveErr] = useState<string | null>(null);

  const [activePane, setActivePane] = useState<"outputstream" | "inputstream">("outputstream");

  const { status, error: wsError, messages, connect, disconnect, send, clearMessages, isAtConnectionLimit } = useWebSocket({ tabId, activeEnv });

  const outgoingMessages = useMemo(() => messages.filter((m) => m.direction === "sent"), [messages]);
  const incomingMessages = useMemo(() => messages.filter((m) => m.direction === "received"), [messages]);

  const isConnected = status === "connected";
  const isConnecting = status === "connecting";
  const isDisconnected = status === "disconnected" || status === "error";

  // auto-scroll refs
  const outEndRef = useRef<HTMLDivElement>(null);
  const inEndRef = useRef<HTMLDivElement>(null);
  useEffect(() => { outEndRef.current?.scrollIntoView({ behavior: "smooth" }); }, [outgoingMessages.length]);
  useEffect(() => { inEndRef.current?.scrollIntoView({ behavior: "smooth" }); }, [incomingMessages.length]);

  // Draft persistence
  const isEmptyDraft = useCallback(
    () => !name.trim() && !url.trim() && headers.filter((r) => r.enabled && r.key.trim()).length === 0,
    [name, url, headers],
  );
  const { markSaved } = useDraftPersist(
    isDraft(tabId) ? tabId : null,
    () => ({ name, url, folderId, headers: rowsToHeaders(headers) } satisfies WsDraft),
    isEmptyDraft,
  );

  // Dirty detection: compare current values to the initial saved values
  const savedName = (initial as SavedWsConnection | null)?.name ?? "";
  const savedUrl = (initial as SavedWsConnection | null)?.url ?? "";
  const savedFolderId = (initial as SavedWsConnection | null)?.folderId ?? null;
  const savedHeaders = JSON.stringify((initial as SavedWsConnection | null)?.headers ?? {});
  const isDirtyWs = isNew
    ? (name.trim() !== "" || url.trim() !== "")
    : (name !== savedName || url !== savedUrl || folderId !== savedFolderId || JSON.stringify(rowsToHeaders(headers)) !== savedHeaders);
  useEffect(() => { onDirtyChange?.(isDirtyWs); }, [isDirtyWs, onDirtyChange]);

  const handleConnect = useCallback(() => {
    setSendErr(null);
    connect(url.trim(), rowsToHeaders(headers));
  }, [url, headers, connect]);

  const handleDisconnect = useCallback(() => {
    disconnect();
  }, [disconnect]);

  const handleSend = useCallback(() => {
    if (!outgoingInput.trim()) return;
    setSendErr(null);
    try {
      send(outgoingInput);
      setOutgoingInput("");
    } catch (e) {
      setSendErr(e instanceof Error ? e.message : "Send failed");
    }
  }, [outgoingInput, send]);

  const handleSave = useCallback(async () => {
    if (!url.trim()) return;
    setSaving(true); setSaveErr(null);
    try {
      const res = await onSave({ name: name.trim(), url: url.trim(), headers: rowsToHeaders(headers), folderId: folderId ?? null });
      markSaved();
      return res;
    } catch (e) {
      setSaveErr(e instanceof Error ? e.message : "Save failed");
      throw e;
    } finally {
      setSaving(false);
    }
  }, [name, url, headers, folderId, onSave, markSaved]);

  useImperativeHandle(ref, () => ({
    save() {
      return handleSave();
    },
    refresh(conn: SavedWsConnection) {
      setName(conn.name ?? "");
      setUrl(conn.url ?? "");
      setFolderId(conn.folderId ?? null);
      setHeaders(headersToRows(conn.headers ?? {}));
    },
  }), [handleSave]);

  const [syncing, setSyncing] = useState(false);
  const [reverting, setReverting] = useState(false);

  const handleSyncClick = useCallback(async () => {
    if (!onSync || syncing) return;
    setSyncing(true);
    try {
      let targetId = tabId;
      if (isDirtyWs || isNew) {
        const saved: any = await handleSave();
        if (saved && typeof saved === "object" && saved.id) {
          targetId = saved.id;
        }
      }
      await onSync(targetId);
    } finally {
      setSyncing(false);
    }
  }, [onSync, syncing, isDirtyWs, isNew, handleSave, tabId]);

  const hasChanges = !isNew && Boolean(isDirtyWs || (syncStatus && syncStatus !== "clean"));

  const handleRevertClick = useCallback(async () => {
    if (!onRevert || reverting) return;
    setReverting(true);
    try {
      await onRevert();
    } finally {
      setReverting(false);
    }
  }, [onRevert, reverting]);

  const canSave = Boolean(url.trim());
  const syncDisabled = !hasChanges || (!canSave && isDirtyWs) || syncing;
  const revertDisabled = !hasChanges || reverting;
  const syncTitle = !hasChanges ? strings.common.noChangesToSync : strings.common.syncTooltip;
  const revertTitle = !hasChanges ? strings.common.noChangesToRevert : strings.common.revertTooltip;

  const headerCount = headers.filter((r) => r.enabled && r.key.trim()).length;

  const resolvedUrl = useMemo(() => resolveVars(url.trim(), activeEnv), [url, activeEnv]);

  // Status indicator
  const statusDot = (
    <span
      style={{
        display: "inline-block", width: 8, height: 8, borderRadius: "50%", flexShrink: 0,
        background: isConnected ? "var(--c-signal)" : status === "connecting" ? "var(--c-amber)" : status === "error" ? "var(--c-destructive)" : "var(--c-muted-foreground)",
      }}
    />
  );

  const statusLabel = isConnected ? "Connected" : isConnecting ? "Connecting…" : status === "error" ? "Error" : "Disconnected";

  return (
    <div className="flex flex-col h-full overflow-hidden bg-surface">
      {/* Title bar */}
      <EditorTitleBar
        label="WEBSOCKET"
        namePlaceholder={strings.sockets.namePlaceholder}
        name={name}
        onNameChange={setName}
        onClose={onClose}
      />

      {/* URL bar - Connect/Disconnect button inline */}
      <div className="px-4 py-2.5 border-b border-border flex-shrink-0 flex items-center gap-2">
        <div
          className="flex items-stretch rounded border border-border focus-within:border-signal transition-colors overflow-hidden flex-1"
          style={{ background: "var(--c-card)" }}
        >
          <span className="bg-surface-2 border-r border-border text-xs font-bold font-mono px-3 py-2.5 flex-shrink-0 flex items-center" style={{ color: "var(--c-signal)", minWidth: 56 }}>
            WS
          </span>
          <input
            className="flex-1 bg-transparent px-3 py-2.5 text-sm font-mono text-foreground outline-none placeholder:text-muted-foreground min-w-0"
            placeholder="ws://localhost:8080 or wss://…"
            value={url}
            onChange={(e) => setUrl(e.target.value)}
            onKeyDown={(e) => { if (e.key === "Enter" && isDisconnected) handleConnect(); }}
            disabled={isConnected || isConnecting}
          />
        </div>

        {/* Status dot + label */}
        <div className="flex items-center gap-1.5 flex-shrink-0 text-[10px] text-muted-foreground">
          {statusDot}
          <span>{statusLabel}</span>
        </div>

        {isDisconnected ? (
          <button
            onClick={handleConnect}
            disabled={!url.trim() || (isAtConnectionLimit && !isConnected)}
            title={isAtConnectionLimit && !isConnected ? strings.sockets.maxConnectionsTitle.replace("{n}", String(MAX_WS_CONNECTIONS)) : strings.sockets.connect}
            className="px-4 py-2.5 rounded bg-signal hover:bg-signal/80 disabled:opacity-40 disabled:cursor-not-allowed text-background text-xs font-semibold transition-all cursor-pointer flex-shrink-0"
          >
            <Play size={10} className="inline mr-1" fill="currentColor" /> {strings.sockets.connect}
          </button>
        ) : (
          <button
            onClick={handleDisconnect}
            className="px-4 py-2.5 rounded bg-destructive/80 hover:bg-destructive text-white text-xs font-semibold transition-all cursor-pointer flex-shrink-0"
          >
            {strings.sockets.disconnect}
          </button>
        )}
      </div>

      {/* Connection limit warning */}
      {isAtConnectionLimit && isDisconnected && (
        <div className="px-4 py-1.5 border-b border-border bg-amber/5 flex-shrink-0">
          <span className="text-[11px] text-amber">
            {strings.sockets.connectionsActive.replace("{n}", String(MAX_WS_CONNECTIONS))}
          </span>
        </div>
      )}

      {/* Error / WS error */}
      {(wsError || saveErr) && (
        <div className="px-4 py-1.5 border-b border-border bg-destructive/5 flex-shrink-0">
          <span className="text-xs text-destructive font-mono">{wsError ?? saveErr}</span>
        </div>
      )}

      {/* Main 50/50 split: OutputStream (left) | InputStream (right) */}
      <PanelGroup orientation="horizontal" className="flex flex-1 min-h-0 overflow-hidden">

        {/* -- OutputStream (left) ------------------------------------------- */}
        <Panel defaultSize={50} minSize={20} className="flex flex-col overflow-hidden">
          <div className="flex flex-col h-full overflow-hidden">
            <TabStrip
              tabs={[
                { id: "headers" as const, label: `Headers${headerCount > 0 ? ` (${headerCount})` : ""}` },
              ]}
              active={reqTab}
              onChange={(t) => setReqTab(t as "headers")}
              prefix={
                <span className="px-4 py-2.5 text-[10px] font-semibold uppercase tracking-widest text-muted-foreground border-r border-border whitespace-nowrap">
                  {strings.sockets.outputStream}
                </span>
              }
            />

            <div className="flex-1 overflow-y-auto min-h-0">
              {!(isConnected || isConnecting) && (
                <div className="flex items-center gap-1.5 px-3 py-1.5 border-b border-border/40 bg-background/10 flex-shrink-0 justify-end">
                  <span className="text-[9px] text-muted-foreground/60 uppercase tracking-wider mr-1">Insert</span>
                  <EnvVarHint
                    env={activeEnv}
                    onInsert={(token) => {
                      setHeaders((prev) => {
                        if (prev.length === 0) return [{ id: mkRowId(), enabled: true, key: "", value: token }];
                        return prev.map((r, i) => i === prev.length - 1 ? { ...r, value: r.value + token } : r);
                      });
                    }}
                  />
                  <RandomizerHint
                    onInsert={(token) => {
                      setHeaders((prev) => {
                        if (prev.length === 0) return [{ id: mkRowId(), enabled: true, key: "", value: token }];
                        return prev.map((r, i) => i === prev.length - 1 ? { ...r, value: r.value + token } : r);
                      });
                    }}
                  />
                </div>
              )}
              <HeaderTable
                rows={headers}
                onChange={setHeaders}
                readOnly={isConnected || isConnecting}
                emptyMessage={isConnected || isConnecting ? strings.common.noHeaders : undefined}
              />
            </div>

            {/* Send message section */}
            <div className="border-t border-border flex-shrink-0">
              {/* Sent messages list */}
              <div className="max-h-40 overflow-y-auto px-4 py-2 space-y-1">
                {outgoingMessages.length === 0 ? (
                  <p className="text-[10px] text-muted-foreground italic py-1">{strings.sockets.noMessagesSent}</p>
                ) : (
                  outgoingMessages.map((m) => (
                    <MessageRow key={m.id} msg={m} />
                  ))
                )}
                <div ref={outEndRef} />
              </div>

              {/* Token hints for message input */}
              {isConnected && (
                <div className="flex items-center gap-1.5 px-3 py-1 border-t border-border/40 bg-background/10 justify-end">
                  <span className="text-[9px] text-muted-foreground/60 uppercase tracking-wider mr-1">Insert</span>
                  <EnvVarHint
                    env={activeEnv}
                    onInsert={(token) => setOutgoingInput((v) => v + token)}
                  />
                  <RandomizerHint
                    onInsert={(token) => setOutgoingInput((v) => v + token)}
                  />
                </div>
              )}

              {/* Input row */}
              <div className="flex items-center gap-2 px-4 py-2.5 border-t border-border">
                <input
                  className="flex-1 bg-card border border-border focus:border-signal rounded px-3 py-2 text-xs font-mono text-foreground outline-none placeholder:text-muted-foreground/60 transition-colors"
                  placeholder={isConnected ? strings.sockets.typeMessage : strings.sockets.connectToSend}
                  value={outgoingInput}
                  onChange={(e) => setOutgoingInput(e.target.value)}
                  onKeyDown={(e) => { if (e.key === "Enter") handleSend(); }}
                  disabled={!isConnected}
                />
                <button
                  onClick={handleSend}
                  disabled={!isConnected || !outgoingInput.trim()}
                  className="px-4 py-2 rounded bg-signal hover:bg-signal/80 disabled:opacity-40 disabled:cursor-not-allowed text-background text-xs font-semibold transition-all cursor-pointer flex-shrink-0"
                >
                  <Send size={12} />
                </button>
              </div>
              {sendErr && (
                <div className="px-4 pb-2">
                  <span className="text-[10px] text-destructive font-mono">{sendErr}</span>
                </div>
              )}
            </div>
          </div>
        </Panel>

        <PanelResizeHandle className="w-1 bg-border hover:bg-signal/40 active:bg-signal/60 transition-colors cursor-col-resize flex-shrink-0" />

        {/* -- InputStream (right) ------------------------------------------- */}
        <Panel defaultSize={50} minSize={20} className="flex flex-col overflow-hidden">
          <div className="flex flex-col h-full overflow-hidden">
            <TabStrip
              tabs={[{ id: "inputstream" as const, label: "InputStream" }]}
              active="inputstream"
              onChange={() => { }}
              prefix={
                <span className="px-4 py-2.5 text-[10px] font-semibold uppercase tracking-widest text-muted-foreground border-r border-border whitespace-nowrap">
                  {isConnected
                    ? <span className="flex items-center gap-1.5">
                      <span style={{ display: "inline-block", width: 6, height: 6, borderRadius: "50%", background: "var(--c-signal)", boxShadow: "0 0 5px var(--c-signal)" }} />
                      {strings.sockets.live}
                    </span>
                    : strings.sockets.waiting}
                </span>
              }
              suffix={
                incomingMessages.length > 0
                  ? <button
                    onClick={clearMessages}
                    className="px-3 text-[10px] text-muted-foreground hover:text-foreground cursor-pointer transition-colors"
                    title="Clear all messages"
                  >{strings.sockets.clear}</button>
                  : undefined
              }
            />
            <div className="flex-1 overflow-y-auto min-h-0 px-4 py-2 space-y-1">
              {incomingMessages.length === 0 ? (
                <div className="flex flex-col items-center justify-center h-full text-center gap-2 py-8">
                  <div className="opacity-15"><Radio size={28} /></div>
                  <p className="text-xs text-muted-foreground">
                    {isConnected ? strings.sockets.waitingForMessages : strings.sockets.connectToReceive}
                  </p>
                </div>
              ) : (
                incomingMessages.map((m) => (
                  <MessageRow key={m.id} msg={m} />
                ))
              )}
              <div ref={inEndRef} />
            </div>
          </div>
        </Panel>
      </PanelGroup>

      {/* Bottom bar */}
      <BottomBar
        folders={folders}
        folderId={folderId}
        onFolderChange={setFolderId}
        onCancel={onClose}
        onSave={handleSave}
        saveLabel={isNew ? strings.sockets.saveSocket : strings.sockets.updateSocket}
        saveDisabled={!url.trim() || (!isNew && !isDirtyWs)}
        saving={saving}
        savingLabel={strings.server.saving}
        onSync={onSync ? handleSyncClick : undefined}
        onRevert={onRevert ? handleRevertClick : undefined}
        onHistory={onHistory}
        historyDisabled={!onHistory || isNew}
        syncDisabled={syncDisabled}
        revertDisabled={revertDisabled}
        syncing={syncing}
        reverting={reverting}
        syncTitle={syncTitle}
        revertTitle={revertTitle}
      />
    </div>
  );
});

// -- Message row ------------------------------------------------------------

function looksLikeJson(s: string): boolean {
  const t = s.trimStart();
  return (t.startsWith("{") || t.startsWith("[")) && t.length > 10;
}

function MessageRow({ msg }: { msg: WsMessage }) {
  const time = new Date(msg.ts).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" });
  const useEditor = looksLikeJson(msg.data) || msg.data.length > 200;
  const displayData = looksLikeJson(msg.data) ? tryFormat(msg.data) : msg.data;
  return (
    <div className="flex items-start gap-2 py-1 border-b border-border/30 last:border-0 group">
      <span className="text-[9px] font-mono text-muted-foreground flex-shrink-0 mt-0.5 w-16">{time}</span>
      {useEditor
        ? <div className="flex-1 overflow-hidden border border-border/30 rounded" style={{ maxHeight: 192 }}>
          <CodeEditor
            value={displayData}
            readOnly
            language={looksLikeJson(msg.data) ? "json" : "text"}
            className="h-full"
          />
        </div>
        : <pre className="text-[11px] font-mono text-foreground flex-1 whitespace-pre-wrap break-all leading-relaxed">{msg.data}</pre>
      }
    </div>
  );
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
  onPublishItem?: (id: string) => void;
  onPublishFolder?: (folderId: string | null) => void;
  onRestoreItem?: (id: string) => void;
}

// -- WebSocketsPanel --------------------------------------------------------

export default function WebSocketsPanel({ config, onConfigChange, activeEnv = null, onHistoryOpen, onEntityPathChange, historyOpen = false, onAfterSave, entitySyncStatus, onPublishItem, onPublishFolder, onRestoreItem }: Props) {
  const connections = config.wsConnections ?? [];
  const folders = config.wsFolders ?? [];

  const [sidebarOpen, setSidebarOpen] = usePersistedState(`sockets:${config.activeWorkspaceId}:sidebar-open`, true);
  const [dirtyTabs, setDirtyTabs] = useState<Record<string, boolean>>({});
  const tabRefs = useRef<Record<string, WsEditorHandle | null>>({});

  const [openTabs, setOpenTabs] = usePersistedState<string[]>(
    "ws:openTabs", [],
    (tabs) => tabs.filter((id) => {
      if (isDraft(id)) return getDraftIds(DRAFT_PREFIX).includes(id);
      return connections.some((c) => c.id === id);
    }),
  );
  const [activeTab, setActiveTab] = usePersistedState<string | null>(
    "ws:activeTab", null,
    (id) => {
      if (id === null) return null;
      if (isDraft(id)) return getDraftIds(DRAFT_PREFIX).includes(id) ? id : null;
      return connections.some((c) => c.id === id) ? id : null;
    },
  );

  // Full entity cache: loaded on-demand when tab is opened (config.wsConnections only has stubs)
  const [loadedEntities, setLoadedEntities] = useState<Record<string, SavedWsConnection>>({});

  useEffect(() => {
    if (!activeTab || isDraft(activeTab)) return;
    if (loadedEntities[activeTab]) return;
    window.api.loadEntity(config.activeWorkspaceId, "sockets", activeTab).then((res) => {
      if (res.ok && res.entity) setLoadedEntities((prev) => ({ ...prev, [activeTab]: res.entity as SavedWsConnection }));
    }).catch(() => { });
  }, [activeTab, config.activeWorkspaceId]);

  const openTab = useCallback((id: string) => {
    setOpenTabs((prev) => (prev.includes(id) ? prev : [...prev, id]));
    setActiveTab(id);
  }, []);

  useEffect(() => {
    const handler = (e: Event) => {
      const custom = e as CustomEvent<{ panel: string; tabId: string }>;
      if (custom.detail?.panel === "sockets" && custom.detail.tabId) {
        openTab(custom.detail.tabId);
      }
    };
    window.addEventListener("localpanel:open-tab", handler);
    return () => window.removeEventListener("localpanel:open-tab", handler);
  }, [openTab]);

  const openNewTab = useCallback(() => {
    const existingEmpty = openTabs.find((id) => isDraft(id) && !loadDraft(id));
    if (existingEmpty) { setActiveTab(existingEmpty); return; }
    const tabId = `${DRAFT_PREFIX}${Date.now()}`;
    setOpenTabs((p) => [...p, tabId]);
    setActiveTab(tabId);
  }, [openTabs]);

  const closeTab = useCallback((tabId: string) => {
    if (isDraft(tabId)) clearDraft(tabId);
    setOpenTabs((prev) => {
      const next = prev.filter((id) => id !== tabId);
      setActiveTab((cur) => { if (cur !== tabId) return cur; return next.length > 0 ? next[next.length - 1] : null; });
      return next;
    });
  }, []);

  const handleReorderTabs = useCallback((fromId: string, toId: string, edge: "left" | "right" = "left") => {
    if (fromId === toId) return;
    setOpenTabs((prev) => {
      const fromIndex = prev.indexOf(fromId);
      if (fromIndex === -1) return prev;
      const withoutFrom = prev.filter((id) => id !== fromId);
      let targetIndex = withoutFrom.indexOf(toId);
      if (targetIndex === -1) return prev;
      if (edge === "right") targetIndex += 1;
      const next = [...withoutFrom];
      next.splice(targetIndex, 0, fromId);
      return next;
    });
  }, []);

  useTabKeyBindings({ activeTab, tabRefs, closeTab, openNewTab });

  const reloadConnections = useCallback(async () => {
    const fresh = await window.api.getConfig();
    await onConfigChange(fresh);
  }, [onConfigChange]);

  const getEntityFilePath = useCallback((tabId: string): string => {
    if (isDraft(tabId)) return "";
    const c = connections.find((x) => x.id === tabId);
    if (!c) return "";
    return entityRelPath("sockets", c, folders);
  }, [connections, folders]);

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
    setOpenTabs((prev) => [...prev.filter((id) => id !== tabId), created.id]);
    setActiveTab(created.id);
    onAfterSave?.();
    return created;
  }, [reloadConnections, onAfterSave]);

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

  // Folder view items
  const folderViewItems: FolderTreeItem[] = useMemo(() =>
    connections.map((c): FolderTreeItem => ({
      id: c.id,
      name: c.name || c.url.slice(0, 40),
      folderId: c.folderId ?? null,
      isActive: activeTab === c.id,
      isEnabled: true,
      relPath: entityRelPath("sockets", c, folders),
    })),
    [connections, folders, activeTab],
  );

  // -- Sidebar --------------------------------------------------------------

  const sidebarContent = (
    <>
      <SidebarHeader onCollapse={() => setSidebarOpen(false)} collapseTitle={strings.titleBar.collapseSidebar}>
        <span className="text-xs font-semibold px-1 text-muted-foreground uppercase tracking-wider">
          {strings.panels.sectionWebsocket}
        </span>
      </SidebarHeader>

      <div className="flex-1 overflow-y-auto overflow-x-auto min-w-0" style={{ display: "flex", flexDirection: "column" }}>
        <FolderTree
          kind="ws"
          folders={folders}
          items={folderViewItems}
          onOpenItem={openTab}
          onDeleteItem={handleDelete}
          onFoldersChange={handleFoldersChange}
          onDuplicateItem={handleDuplicate}
          onMoveItems={handleMoveItems}
          onOpenNewTab={openNewTab}
          onHistoryItem={onHistoryOpen ? (id) => {
            const path = getEntityFilePath(id);
            if (path) onHistoryOpen(path);
          } : undefined}
          pathStatusMap={entitySyncStatus}
          onPublishItem={onPublishItem}
          onPublishFolder={onPublishFolder}
          onRestoreItem={onRestoreItem}
          onBeforeCreateFolder={() => true}
        />
      </div>

    </>
  );

  // -- Main content ---------------------------------------------------------

  const mainContent = (
    <div className="flex flex-col flex-1 overflow-hidden min-w-0 h-full">
      <TabBar
        tabs={openTabs.map((id) => ({
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
        }))}
        activeTab={activeTab}
        onTabClick={setActiveTab}
        onTabClose={closeTab}
        onNewTab={openNewTab}
        onReorderTabs={handleReorderTabs}
        newTabTitle={strings.sockets.newTab}
        closeTabTitle={strings.common.close}
        onCloseOthers={(id) => {
          openTabs.filter((t) => t !== id).forEach(closeTab);
        }}
        onCloseAll={() => {
          [...openTabs].forEach(closeTab);
        }}
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
            const relPath = conn ? entityRelPath("sockets", conn, folders) : "";
            const syncStatus = relPath ? entitySyncStatus?.[relPath] : undefined;
            return (
              <div key={tabId} className="absolute inset-0 flex flex-col overflow-hidden" style={{ display: activeTab === tabId ? "flex" : "none" }}>
                <WsEditor
                  ref={(el) => { tabRefs.current[tabId] = el; }}
                  key={tabId}
                  tabId={tabId}
                  initial={conn}
                  isNew={isUnsaved}
                  onSave={isUnsaved ? (data) => handleNewSave(tabId, data) : (data) => handleTabSave(tabId, data)}
                  onClose={() => closeTab(tabId)}
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
        sidebar={sidebarContent}
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

// -- WsTabHeader - green/red dot based on connection status -----------------

function WsTabHeader({ tabId, label }: { tabId: string; label: string }) {
  const [dotColor, setDotColor] = useState("var(--c-muted-foreground)");

  useEffect(() => {
    const handler = (e: CustomEvent<{ tabId: string; color: string }>) => {
      if (e.detail.tabId === tabId) setDotColor(e.detail.color);
    };
    window.addEventListener("ws:statuscolor" as any, handler as any);
    return () => window.removeEventListener("ws:statuscolor" as any, handler as any);
  }, [tabId]);

  return (
    <div className="flex items-center gap-1.5 min-w-0">
      <span style={{ display: "inline-block", width: 6, height: 6, borderRadius: "50%", background: dotColor, flexShrink: 0 }} />
      <span className="truncate">{label}</span>
    </div>
  );
}
