import React, { forwardRef, useImperativeHandle, useState, useEffect, useMemo, useCallback, useRef } from 'react';
import { SavedWsConnection, Folder as FolderType, Environment } from '@/types';
import EditorTitleBar from '@/components/editor/EditorTitleBar';
import { TabStrip, BottomBar, UrlBar } from '@/components/editor/RequestTab';
import HeaderTable from '@/components/common/HeaderTable';
import CodeEditor from '@/components/common/CodeEditor';
import EnvVarHint from '@/components/editor/EnvVarHint';
import RandomizerHint from '@/components/editor/RandomizerHint';
import { resolveVars } from '@/lib/resolveVars';
import { KVRow, mkRowId, headersToRows, rowsToHeaders, tryFormat } from '@/lib/utils';
import { useDraftPersist, loadDraft, clearDraft, getDraftIds } from '@/hooks/useDraftPersist';
import { useWebSocket, MAX_WS_CONNECTIONS, WsMessage } from '@/hooks/useWebSocket';
import { Group as PanelGroup, Panel, Separator as PanelResizeHandle } from 'react-resizable-panels';
import { Play, Send, Radio } from '@/lib/icons';
import { strings } from '@/lib/strings';
import WsStreams from './WsStreams';

const DRAFT_PREFIX = 'ws-draft-';
const isDraft = (id: string) => id.startsWith(DRAFT_PREFIX);

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
      <WsStreams
        reqTab={reqTab}
        setReqTab={setReqTab}
        headerCount={headerCount}
        headers={headers}
        setHeaders={setHeaders}
        activeEnv={activeEnv}
        isConnected={isConnected}
        isConnecting={isConnecting}
        outgoingMessages={outgoingMessages}
        outgoingInput={outgoingInput}
        setOutgoingInput={setOutgoingInput}
        handleSend={handleSend}
        sendErr={sendErr}
        outEndRef={outEndRef}
        incomingMessages={incomingMessages}
        clearMessages={clearMessages}
        inEndRef={inEndRef}
      />

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

export default WsEditor;