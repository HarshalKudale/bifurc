import React, { forwardRef, useImperativeHandle, useState, useEffect, useMemo, useCallback, useRef } from 'react';
import { SavedWebhook, Folder, WebhookPayload } from '@/types';
import { loadDraft, useDraftPersist } from '@/hooks/useDraftPersist';
import { Webhook } from '@/lib/icons';
import { strings } from '@/lib/strings';
import CodeEditor from '@/components/common/CodeEditor';
import EditorTitleBar from '@/components/editor/EditorTitleBar';
import { BottomBar } from '@/components/editor/RequestTab';

const DRAFT_PREFIX = 'wh-draft-';
const isDraftId = (id: string) => id.startsWith(DRAFT_PREFIX);
const MAX_ACTIVE_WEBHOOKS = 5;
const BASE_URL_SEGMENT = '/localpanel/webhooks/';

// -- Webhook editor ---------------------------------------------------------

interface WebhookEditorProps {
  tabId: string;
  webhookId: string | null;
  initial: Partial<SavedWebhook> | null;
  isNew: boolean;
  webhookPort: number;
  onSave(data: Omit<SavedWebhook, "id" | "createdAt" | "workspaceId">): Promise<any>;
  onClose(): void;
  folders?: Folder[];
  payloads: WebhookPayload[];
  isActive: boolean;
  isAtLimit: boolean;
  onDirtyChange?: (dirty: boolean) => void;
  onSync?: (savedId?: string) => Promise<void>;
  onRevert?: () => Promise<void>;
  syncStatus?: "clean" | "modified" | "new" | "deleted";
  onHistory?: () => void;
}

export interface WebhookEditorHandle {
  save(): Promise<any> | void;
  refresh?(hook: SavedWebhook): void;
}

const WebhookEditor = forwardRef<WebhookEditorHandle, WebhookEditorProps>(function WebhookEditor({
  tabId, webhookId, initial, isNew,
  webhookPort, onSave, onClose, folders = [],
  payloads, isActive, isAtLimit, onDirtyChange,
  onSync, onRevert, syncStatus, onHistory,
}: WebhookEditorProps, ref) {
  const draft = isDraftId(tabId) ? loadDraft<WebhookDraft>(tabId) : null;
  const src = draft ?? initial;

  const [name, setName] = useState(src?.name ?? "");
  const [urlSuffix, setUrlSuffix] = useState(src?.urlSuffix ?? "");
  const [folderId, setFolderId] = useState<string | null>(() => (src as SavedWebhook | null)?.folderId ?? null);
  const [saving, setSaving] = useState(false);
  const [saveErr, setSaveErr] = useState<string | null>(null);
  const [selectedPayload, setSelectedPayload] = useState<WebhookPayload | null>(null);

  const bottomRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [payloads.length]);

  // Keep selected payload in sync when new ones arrive
  useEffect(() => {
    if (!selectedPayload && payloads.length > 0) {
      setSelectedPayload(payloads[payloads.length - 1]);
    }
  }, [payloads, selectedPayload]);

  const isEmptyDraft = useCallback(
    () => !name.trim() && !urlSuffix.trim(),
    [name, urlSuffix],
  );

  const { markSaved } = useDraftPersist(
    isDraftId(tabId) ? tabId : null,
    () => ({ name, urlSuffix, folderId } satisfies WebhookDraft),
    isEmptyDraft,
  );

  // Dirty detection
  const savedName = (initial as SavedWebhook | null)?.name ?? "";
  const savedUrlSuffix = (initial as SavedWebhook | null)?.urlSuffix ?? "";
  const savedFolderId = (initial as SavedWebhook | null)?.folderId ?? null;
  const isDirtyWh = isNew
    ? (name.trim() !== "" || urlSuffix.trim() !== "")
    : (name !== savedName || urlSuffix !== savedUrlSuffix || folderId !== savedFolderId);
  useEffect(() => { onDirtyChange?.(isDirtyWh); }, [isDirtyWh, onDirtyChange]);

  const handleSave = useCallback(async () => {
    setSaving(true); setSaveErr(null);
    try {
      const res = await onSave({ name: name.trim(), urlSuffix: urlSuffix.trim(), folderId: folderId ?? null });
      markSaved();
      return res;
    } catch (e) {
      setSaveErr(e instanceof Error ? e.message : "Save failed");
      throw e;
    } finally {
      setSaving(false);
    }
  }, [name, urlSuffix, folderId, onSave, markSaved]);

  useImperativeHandle(ref, () => ({
    save() {
      return handleSave();
    },
    refresh(hook: SavedWebhook) {
      setName(hook.name ?? "");
      setUrlSuffix(hook.urlSuffix ?? "");
      setFolderId(hook.folderId ?? null);
    },
  }), [handleSave]);

  const [syncing, setSyncing] = useState(false);
  const [reverting, setReverting] = useState(false);

  const handleSyncClick = useCallback(async () => {
    if (!onSync || syncing) return;
    setSyncing(true);
    try {
      let targetId = tabId;
      if (isDirtyWh || isNew) {
        const saved: any = await handleSave();
        if (saved && typeof saved === "object" && saved.id) {
          targetId = saved.id;
        }
      }
      await onSync(targetId);
    } finally {
      setSyncing(false);
    }
  }, [onSync, syncing, isDirtyWh, isNew, handleSave, tabId]);

  const hasChanges = !isNew && Boolean(isDirtyWh || (syncStatus && syncStatus !== "clean"));

  const handleRevertClick = useCallback(async () => {
    if (!onRevert || reverting) return;
    setReverting(true);
    try {
      await onRevert();
    } finally {
      setReverting(false);
    }
  }, [onRevert, reverting]);

  const syncDisabled = !hasChanges || syncing;
  const revertDisabled = !hasChanges || reverting;
  const syncTitle = !hasChanges ? strings.common.noChangesToSync : strings.common.syncTooltip;
  const revertTitle = !hasChanges ? strings.common.noChangesToRevert : strings.common.revertTooltip;

  const fullUrl = `http://localhost:${webhookPort}${BASE_URL_SEGMENT}${urlSuffix.replace(/^\/+/, "")}`;

  const formattedBody = useMemo(() => {
    if (!selectedPayload) return "";
    const body = selectedPayload.body;
    if (!body) return strings.webhooks.emptyBody;
    const t = body.trimStart();
    if (t.startsWith("{") || t.startsWith("[")) {
      try { return JSON.stringify(JSON.parse(body), null, 2); } catch { /* fall through */ }
    }
    return body;
  }, [selectedPayload]);

  const isBodyJson = useMemo(() => {
    if (!selectedPayload?.body) return false;
    const t = selectedPayload.body.trimStart();
    return t.startsWith("{") || t.startsWith("[");
  }, [selectedPayload]);

  return (
    <div className="flex flex-col h-full overflow-hidden bg-surface">
      <EditorTitleBar
        label="WEBHOOK"
        namePlaceholder={strings.webhooks.namePlaceholder}
        name={name}
        onNameChange={setName}
        onClose={onClose}
      />

      {/* URL bar */}
      <div className="px-4 py-3 border-b border-border flex-shrink-0">
        <div className="flex items-stretch rounded border border-border focus-within:border-signal transition-colors overflow-hidden" style={{ background: "var(--c-card)" }}>
          {/* Fixed base - not editable */}
          <span className="bg-surface-2 border-r border-border text-xs font-mono px-3 flex items-center flex-shrink-0 text-muted-foreground whitespace-nowrap select-all">
            {`localhost:${webhookPort}${BASE_URL_SEGMENT}`}
          </span>
          {/* User-editable suffix */}
          <input
            className="flex-1 bg-transparent px-3 py-2.5 text-sm font-mono text-foreground outline-none placeholder:text-muted-foreground min-w-0"
            placeholder="your-webhook-path"
            value={urlSuffix}
            onChange={(e) => {
              // Strip leading slashes - base already ends with /
              setUrlSuffix(e.target.value.replace(/^\/+/, ""));
            }}
          />
        </div>
        <div className="mt-1.5 flex items-center gap-2">
          <span className="text-[10px] text-muted-foreground font-mono truncate flex-1">{fullUrl}</span>
          {/* Status indicator */}
          <span className={`text-[10px] font-semibold px-2 py-0.5 rounded-full flex-shrink-0 ${isActive
            ? "bg-signal/10 text-signal"
            : isAtLimit
              ? "bg-amber/10 text-amber"
              : "bg-muted-foreground/10 text-muted-foreground"
            }`}>
            {isActive ? strings.webhooks.statusActive : isAtLimit ? strings.webhooks.statusAtLimit : strings.webhooks.statusInactive}
          </span>
        </div>
        {isAtLimit && !isActive && (
          <p className="mt-1 text-[11px] text-amber">
            {strings.webhooks.maxActive.replace("{n}", String(MAX_ACTIVE_WEBHOOKS))}
          </p>
        )}
      </div>

      {/* Payloads area */}
      <div className="flex flex-1 min-h-0 overflow-hidden">
        {/* Left: payload list */}
        <div className="flex flex-col border-r border-border flex-shrink-0 overflow-hidden" style={{ width: 220 }}>
          <div className="px-3 py-2 border-b border-border flex-shrink-0 flex items-center justify-between">
            <span className="text-[10px] font-semibold uppercase tracking-widest text-muted-foreground">{strings.webhooks.received}</span>
            {payloads.length > 0 && (
              <span className="text-[10px] text-muted-foreground">{payloads.length}</span>
            )}
          </div>
          <div className="flex-1 overflow-y-auto min-h-0">
            {payloads.length === 0 ? (
              <div className="flex flex-col items-center justify-center h-full text-center gap-2 py-8 px-3">
                <div className="opacity-15"><Webhook size={28} /></div>
                <p className="text-xs text-muted-foreground">
                  {isActive ? strings.webhooks.waitingForPost : strings.webhooks.activateToReceive}
                </p>
              </div>
            ) : (
              [...payloads].reverse().map((p, i) => {
                const isSelected = p === selectedPayload;
                const t = new Date(p.ts).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" });
                return (
                  <button
                    key={i}
                    onClick={() => setSelectedPayload(p)}
                    className={`w-full text-left px-3 py-2 border-b border-border/50 transition-colors cursor-pointer ${isSelected ? "bg-signal/10 text-signal" : "hover:bg-card text-muted-foreground hover:text-foreground"
                      }`}
                  >
                    <div className="flex items-center gap-2 text-[10px]">
                      <span className="font-semibold">{p.method}</span>
                      <span className="font-mono text-muted-foreground truncate flex-1">{t}</span>
                    </div>
                    <div className="text-[10px] font-mono text-muted-foreground mt-0.5 truncate">
                      {p.body ? p.body.slice(0, 30) : strings.webhooks.empty}
                    </div>
                  </button>
                );
              })
            )}
            <div ref={bottomRef} />
          </div>
        </div>

        {/* Right: payload detail */}
        <div className="flex flex-col flex-1 overflow-hidden min-w-0">
          {selectedPayload ? (
            <>
              {/* Headers strip */}
              <div className="px-4 py-2 border-b border-border flex-shrink-0 flex items-center gap-3 text-[10px] text-muted-foreground font-mono">
                <span className="text-signal font-semibold">{selectedPayload.method}</span>
                <span>{new Date(selectedPayload.ts).toLocaleString()}</span>
                {Object.keys(selectedPayload.headers).length > 0 && (
                  <span>{Object.keys(selectedPayload.headers).length} headers</span>
                )}
              </div>
              {/* Body */}
              <div className="flex-1 overflow-hidden min-h-0">
                <CodeEditor
                  value={formattedBody}
                  readOnly
                  language={isBodyJson ? "json" : "text"}
                  className="h-full"
                />
              </div>
            </>
          ) : (
            <div className="flex flex-col items-center justify-center h-full text-center gap-2 py-8 px-6">
              <div className="opacity-15"><Webhook size={28} /></div>
              <p className="text-xs text-muted-foreground">{strings.webhooks.selectPayload}</p>
            </div>
          )}
        </div>
      </div>

      <BottomBar
        folders={folders}
        folderId={folderId}
        onFolderChange={setFolderId}
        onCancel={onClose}
        onSave={handleSave}
        saveLabel={isNew ? strings.webhooks.saveWebhook : strings.webhooks.updateWebhook}
        saveDisabled={!isNew && !isDirtyWh}
        saving={saving}
        savingLabel={strings.server.saving}
        extraLeft={saveErr ? <span className="text-xs text-destructive">{saveErr}</span> : undefined}
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

export default WebhookEditor;