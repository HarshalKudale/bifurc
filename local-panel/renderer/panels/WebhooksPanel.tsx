import React, { forwardRef, useImperativeHandle, useState, useEffect, useMemo, useCallback, useRef } from "react";
import { AppConfig, SavedWebhook, Folder, WebhookPayload } from "@/types";
import SearchInput from "@/components/common/SearchInput";
import FolderTree, { FolderTreeItem } from "@/components/sidebar/FolderTree";
import DraftsFolder from "@/components/sidebar/DraftsFolder";
import { loadDraft, useDraftPersist, clearDraft, getDraftIds } from "@/lib/useDraftPersist";
import { usePersistedState } from "@/lib/usePersistedState";
import { entityRelPath, calculateFolderStatus } from "@/lib/utils";
import { Plus, X, Play, Square, Webhook } from "@/lib/icons";
import { strings } from "@/lib/strings";
import TabBar from "@/components/editor/TabBar";
import { SidebarLayout, SidebarHeader } from "@/components/ui";
import CodeEditor from "@/components/common/CodeEditor";
import EditorTitleBar from "@/components/editor/EditorTitleBar";
import { BottomBar } from "@/components/editor/RequestTab";
import { useTabKeyBindings } from "@/hooks/useTabKeyBindings";

// -- Constants --------------------------------------------------------------

const DRAFT_PREFIX = "wh-draft-";
const isDraftId = (id: string) => id.startsWith(DRAFT_PREFIX);
const MAX_ACTIVE_WEBHOOKS = 5;
const BASE_URL_SEGMENT = "/localpanel/webhooks/";

// -- Draft type -------------------------------------------------------------

interface WebhookDraft {
  name: string;
  urlSuffix: string;
  folderId: string | null;
}

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

// -- Props ------------------------------------------------------------------

interface Props {
  config: AppConfig;
  onConfigChange: (cfg: AppConfig) => Promise<void>;
  onHistoryOpen?: (filePath: string) => void;
  onEntityPathChange?: (filePath: string) => void;
  historyOpen?: boolean;
  onAfterSave?: () => void;
  entitySyncStatus?: Record<string, "clean" | "modified" | "new" | "deleted">;
  onPublishItem?: (id: string) => void;
  onPublishFolder?: (folderId: string | null) => void;
  onRestoreItem?: (id: string) => void;
}

// -- WebhooksPanel ----------------------------------------------------------

export default function WebhooksPanel({
  config, onConfigChange,
  onHistoryOpen, onEntityPathChange, historyOpen = false,
  onAfterSave, entitySyncStatus, onPublishItem, onPublishFolder, onRestoreItem,
}: Props) {
  const webhooks = config.webhooks ?? [];
  const folders = config.webhookFolders ?? [];
  const webhookPort = config.webhookPort ?? 9101;

  const [search, setSearch] = usePersistedState(`webhooks:${config.activeWorkspaceId}:search`, "");
  const [sidebarOpen, setSidebarOpen] = usePersistedState(`webhooks:${config.activeWorkspaceId}:sidebar-open`, true);
  const [dirtyTabs, setDirtyTabs] = useState<Record<string, boolean>>({});
  const tabRefs = useRef<Record<string, WebhookEditorHandle | null>>({});
  const [serverRunning, setServerRunning] = useState(false);
  const [serverError, setServerError] = useState<string | null>(null);
  const [serverLoading, setServerLoading] = useState(false);
  // webhookId -> payload list (runtime, in-memory)
  const [payloadMap, setPayloadMap] = useState<Record<string, WebhookPayload[]>>({});
  // Set of webhookIds currently open in a tab (active)
  const [activeTabs, setActiveTabs] = useState<Set<string>>(new Set());

  // Tab state (persisted)
  const [openTabs, setOpenTabs] = usePersistedState<string[]>(
    "webhooks:openTabs", [],
    (tabs) => tabs.filter((id) => {
      if (isDraftId(id)) return getDraftIds(DRAFT_PREFIX).includes(id);
      return webhooks.some((h) => h.id === id);
    }),
  );
  const [activeTab, setActiveTab] = usePersistedState<string | null>(
    "webhooks:activeTab", null,
    (id) => {
      if (id === null) return null;
      if (isDraftId(id)) return getDraftIds(DRAFT_PREFIX).includes(id) ? id : null;
      return webhooks.some((h) => h.id === id) ? id : null;
    },
  );

  // Loaded full entities (stubs in config, full data on demand)
  const [loadedEntities, setLoadedEntities] = useState<Record<string, SavedWebhook>>({});

  // -- Bootstrap server status ----------------------------------------------

  useEffect(() => {
    window.api.webhookServerStatus().then((s) => {
      setServerRunning(s.running);
      setServerError(s.error);
    }).catch(() => { });
  }, []);

  // -- Listen for incoming webhook payloads ---------------------------------

  useEffect(() => {
    const unsub = window.api.onWebhookPayload((payload) => {
      setPayloadMap((prev) => {
        const existing = prev[payload.webhookId] ?? [];
        // Keep last 100 payloads per webhook
        const next = [...existing, payload].slice(-100);
        return { ...prev, [payload.webhookId]: next };
      });
    });
    return unsub;
  }, []);

  // -- Active tab registration -----------------------------------------------

  useEffect(() => {
    if (!activeTab || isDraftId(activeTab)) return;

    const hook = webhooks.find((h) => h.id === activeTab);
    if (!hook) return;

    const isCurrentlyActive = activeTabs.has(activeTab);
    const isAtLimit = activeTabs.size >= MAX_ACTIVE_WEBHOOKS;

    if (!isCurrentlyActive && !isAtLimit) {
      window.api.registerActiveWebhook(activeTab, hook.urlSuffix).catch(() => { });
      setActiveTabs((prev) => new Set([...prev, activeTab]));
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [activeTab]);

  // Load entity on-demand
  useEffect(() => {
    if (!activeTab || isDraftId(activeTab)) return;
    if (loadedEntities[activeTab]) return;
    window.api.loadEntity(config.activeWorkspaceId, "webhooks", activeTab).then((res) => {
      if (res.ok && res.entity) {
        setLoadedEntities((prev) => ({ ...prev, [activeTab]: res.entity as SavedWebhook }));
      }
    }).catch(() => { });
  }, [activeTab, config.activeWorkspaceId, loadedEntities]);

  // -- Deregister when tab closes --------------------------------------------

  const deregisterWebhook = useCallback((tabId: string) => {
    if (isDraftId(tabId)) return;
    if (activeTabs.has(tabId)) {
      window.api.unregisterActiveWebhook(tabId).catch(() => { });
      setActiveTabs((prev) => { const s = new Set(prev); s.delete(tabId); return s; });
    }
  }, [activeTabs]);

  // -- Server controls -------------------------------------------------------

  const handleServerToggle = useCallback(async () => {
    setServerLoading(true);
    try {
      if (serverRunning) {
        await window.api.stopWebhookServer();
        setServerRunning(false);
        setServerError(null);
      } else {
        await window.api.startWebhookServer();
        const status = await window.api.webhookServerStatus();
        setServerRunning(status.running);
        setServerError(status.error);
      }
    } finally {
      setServerLoading(false);
    }
  }, [serverRunning]);

  // -- Data helpers ----------------------------------------------------------

  const reloadWebhooks = useCallback(async () => {
    const fresh = await window.api.getConfig();
    await onConfigChange(fresh);
  }, [onConfigChange]);

  const getEntityFilePath = useCallback((tabId: string): string => {
    if (isDraftId(tabId)) return "";
    const h = webhooks.find((x) => x.id === tabId);
    if (!h) return "";
    return entityRelPath("webhooks", h, folders);
  }, [webhooks, folders]);

  useEffect(() => {
    if (!historyOpen || !activeTab) return;
    const path = getEntityFilePath(activeTab);
    if (path) onEntityPathChange?.(path);
  }, [activeTab, historyOpen, getEntityFilePath, onEntityPathChange]);

  // -- Tab management --------------------------------------------------------

  const openTab = useCallback((id: string) => {
    setOpenTabs((prev) => (prev.includes(id) ? prev : [...prev, id]));
    setActiveTab(id);
  }, []);

  const openNewTab = useCallback(() => {
    const existingEmpty = openTabs.find((id) => isDraftId(id) && !loadDraft(id));
    if (existingEmpty) { setActiveTab(existingEmpty); return; }
    const tabId = `${DRAFT_PREFIX}${Date.now()}`;
    setOpenTabs((p) => [...p, tabId]);
    setActiveTab(tabId);
  }, [openTabs]);

  const closeTab = useCallback((tabId: string) => {
    deregisterWebhook(tabId);
    if (isDraftId(tabId)) clearDraft(tabId);
    setOpenTabs((prev) => {
      const next = prev.filter((id) => id !== tabId);
      setActiveTab((cur) => { if (cur !== tabId) return cur; return next.length > 0 ? next[next.length - 1] : null; });
      return next;
    });
  }, [deregisterWebhook]);

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

  // -- Save handlers ---------------------------------------------------------

  const handleNewSave = useCallback(async (tabId: string, data: Omit<SavedWebhook, "id" | "createdAt" | "workspaceId">) => {
    const created = await window.api.addWebhook(data);
    await reloadWebhooks();
    // Replace draft tab with real id
    setOpenTabs((prev) => [...prev.filter((id) => id !== tabId), created.id]);
    setActiveTab(created.id);
    // Deregister draft (won't do anything, but clean state)
    if (isDraftId(tabId)) clearDraft(tabId);
    onAfterSave?.();
    return created;
  }, [reloadWebhooks, onAfterSave]);

  const handleTabSave = useCallback(async (tabId: string, data: Omit<SavedWebhook, "id" | "createdAt" | "workspaceId">) => {
    const hook = loadedEntities[tabId] ?? webhooks.find((h) => h.id === tabId);
    if (!hook) return;
    const updated = { ...hook, ...data };
    setLoadedEntities((prev) => ({ ...prev, [tabId]: updated }));
    // If urlSuffix changed, re-register
    if (activeTabs.has(tabId) && hook.urlSuffix !== data.urlSuffix) {
      await window.api.unregisterActiveWebhook(tabId);
      await window.api.registerActiveWebhook(tabId, data.urlSuffix);
    }
    await window.api.updateWebhook(updated);
    await reloadWebhooks();
    onAfterSave?.();
    return updated;
  }, [loadedEntities, webhooks, activeTabs, reloadWebhooks, onAfterSave]);

  const handleDelete = useCallback(async (id: string) => {
    deregisterWebhook(id);
    await window.api.deleteWebhook(id);
    await reloadWebhooks();
    closeTab(id);
  }, [deregisterWebhook, reloadWebhooks, closeTab]);

  const handleDuplicate = useCallback(async (id: string) => {
    let h = loadedEntities[id];
    if (!h) {
      const res = await window.api.loadEntity(config.activeWorkspaceId, "webhooks", id);
      if (res.ok && res.entity) h = res.entity as SavedWebhook;
    }
    if (!h) return;
    const { id: _id, createdAt: _ca, workspaceId: _ws, ...rest } = h;
    await window.api.addWebhook({ ...rest, name: h.name ? `${h.name} (copy)` : "", urlSuffix: "" });
    await reloadWebhooks();
  }, [loadedEntities, config.activeWorkspaceId, reloadWebhooks]);

  const handleMoveItems = useCallback(async (ids: string[], folderId: string | null) => {
    for (const id of ids) {
      let h = loadedEntities[id] ?? webhooks.find((x) => x.id === id);
      if (!h) {
        const res = await window.api.loadEntity(config.activeWorkspaceId, "webhooks", id);
        if (res.ok && res.entity) h = res.entity as SavedWebhook;
      }
      if (h) await window.api.updateWebhook({ ...h, folderId: folderId ?? undefined });
    }
    await reloadWebhooks();
  }, [loadedEntities, webhooks, config.activeWorkspaceId, reloadWebhooks]);

  // -- Tab labels ------------------------------------------------------------

  const tabLabel = (tabId: string) => {
    if (isDraftId(tabId)) {
      const d = loadDraft<WebhookDraft>(tabId);
      return d?.name || d?.urlSuffix || strings.webhooks.newWebhook;
    }
    const h = webhooks.find((x) => x.id === tabId);
    if (!h) return "…";
    return h.name || h.urlSuffix || strings.webhooks.webhook;
  };

  // -- Folder view items -----------------------------------------------------

  const folderViewItems: FolderTreeItem[] = useMemo(() =>
    (search.trim()
      ? webhooks.filter((h) => h.name.toLowerCase().includes(search.toLowerCase()) || h.urlSuffix.toLowerCase().includes(search.toLowerCase()))
      : webhooks
    ).map((h): FolderTreeItem => ({
      id: h.id,
      name: h.name || h.urlSuffix || strings.webhooks.webhook,
      folderId: h.folderId ?? null,
      isActive: activeTab === h.id,
      isEnabled: activeTabs.has(h.id),
      relPath: entityRelPath("webhooks", h, folders),
    })),
    [webhooks, folders, search, activeTab, activeTabs],
  );

  const folderStatusMap = useMemo(() => {
    // For webhooks, isEnabled means "is active" (registered/listening)
    const itemsWithEnabled = webhooks.map((h) => ({
      ...h,
      isEnabled: activeTabs.has(h.id),
    }));
    return calculateFolderStatus(itemsWithEnabled, folders);
  }, [webhooks, folders, activeTabs]);

  // -- Sidebar ---------------------------------------------------------------

  const sidebarContent = (
    <>
      <SidebarHeader onCollapse={() => setSidebarOpen(false)} collapseTitle={strings.titleBar.collapseSidebar}>
        <SearchInput value={search} onChange={setSearch} placeholder={strings.webhooks.searchPlaceholder} />
      </SidebarHeader>
      {/* Webhook server toggle */}
      <div className="flex items-center gap-2 px-3 py-2 border-b border-border">
        <button
          onClick={handleServerToggle}
          disabled={serverLoading}
          className={`flex items-center justify-center w-6 h-6 rounded border transition-all duration-150 cursor-pointer disabled:opacity-40 disabled:cursor-not-allowed flex-shrink-0 ${serverRunning
            ? "border-signal/40 bg-signal/10 hover:bg-destructive/15 hover:border-destructive/40 text-signal hover:text-destructive"
            : "border-border bg-card hover:bg-signal/15 hover:border-signal/40 text-muted-foreground hover:text-signal"
            }`}
          title={serverRunning ? strings.webhooks.stopServer : strings.webhooks.startServer}
        >
          {serverLoading ? (
            <span className="inline-block w-2.5 h-2.5 border-2 border-current/30 border-t-current rounded-full animate-spin" />
          ) : serverRunning ? (
            <Square size={8} fill="currentColor" />
          ) : (
            <Play size={8} fill="currentColor" />
          )}
        </button>
        <span className="text-[10px] text-muted-foreground">
          {serverRunning ? `${strings.webhooks.serverLabel} :${webhookPort}` : strings.webhooks.serverStopped}
        </span>
        {serverError && (
          <span className="text-[9px] text-destructive truncate max-w-[120px]" title={serverError}>
            {serverError}
          </span>
        )}
      </div>

      <div className="flex-1 overflow-y-auto overflow-x-auto min-w-0" style={{ display: "flex", flexDirection: "column" }}>
        {openTabs.some(isDraftId) && (
          <DraftsFolder
            label={strings.webhooks.drafts}
            draftTabIds={openTabs.filter(isDraftId)}
            activeTab={activeTab}
            onOpenTab={openTab}
            onCloseTab={closeTab}
            tabLabel={(id) => {
              const d = loadDraft<WebhookDraft>(id);
              return d?.name || d?.urlSuffix || strings.webhooks.newWebhook;
            }}
          />
        )}
        <FolderTree
          kind="webhook"
          items={folderViewItems}
          folders={folders}
          pathStatusMap={entitySyncStatus}
          folderStatusMap={folderStatusMap}
          onOpenItem={openTab}
          onDeleteItem={handleDelete}
          onDuplicateItem={handleDuplicate}
          onMoveItems={handleMoveItems}
          onFoldersChange={async () => { const fresh = await window.api.getConfig(); await onConfigChange(fresh); }}
          onOpenNewTab={openNewTab}
          onBeforeCreateFolder={() => true}
          onHistoryItem={onHistoryOpen ? (id) => {
            const path = getEntityFilePath(id);
            if (path) onHistoryOpen(path);
          } : undefined}
          onPublishItem={onPublishItem}
          onPublishFolder={onPublishFolder}
          onRestoreItem={onRestoreItem}
        />
      </div>

    </>
  );

  // -- Main content area -----------------------------------------------------

  const mainContent = (
    <div className="flex flex-col flex-1 overflow-hidden">
      {openTabs.length === 0 ? (
        <div className="flex flex-col items-center justify-center flex-1 gap-4 text-center py-16 px-8">
          <div className="opacity-10"><Webhook size={48} /></div>
          <p className="text-sm text-muted-foreground">{strings.webhooks.openToReceive}</p>
          <button
            onClick={openNewTab}
            className="flex items-center gap-2 px-4 py-2 rounded bg-signal/10 hover:bg-signal/20 text-signal text-sm font-medium transition-colors cursor-pointer border border-signal/20"
          >
            <Plus size={14} /> {strings.webhooks.newWebhook}
          </button>
        </div>
      ) : (
        <>
          <TabBar
            tabs={openTabs.map((id) => ({
              id,
              label: tabLabel(id),
              isDraft: isDraftId(id),
              isModified: dirtyTabs[id],
              renderTab: isDraftId(id) ? undefined : (isActive) => (
                <div className="flex items-center gap-1.5 min-w-0">
                  {activeTabs.has(id) && (
                    <span
                      className="w-1.5 h-1.5 rounded-full flex-shrink-0"
                      style={{ background: "var(--c-signal)", boxShadow: "0 0 4px var(--c-signal)" }}
                    />
                  )}
                  <span className={`truncate text-xs font-medium ${isActive ? "text-foreground" : "text-muted-foreground"}`}>
                    {tabLabel(id)}
                  </span>
                </div>
              ),
            }))}
            activeTab={activeTab}
            onTabClick={openTab}
            onTabClose={closeTab}
            onNewTab={openNewTab}
            onReorderTabs={handleReorderTabs}
            newTabTitle={strings.webhooks.newTab}
            onCloseOthers={(id) => {
              openTabs.filter((t) => t !== id).forEach(closeTab);
            }}
            onCloseAll={() => {
              [...openTabs].forEach(closeTab);
            }}
            onTabDuplicate={handleDuplicate}
          />
          <div className="flex-1 overflow-hidden">
            {openTabs.map((tabId) => {
              const isTabActive = tabId === activeTab;
              const isDraft = isDraftId(tabId);
              const hook = isDraft ? null : (loadedEntities[tabId] ?? webhooks.find((h) => h.id === tabId) ?? null);
              const isActivated = !isDraft && activeTabs.has(tabId);
              const isAtLimit = activeTabs.size >= MAX_ACTIVE_WEBHOOKS;
              const tabPayloads = isDraft ? [] : (payloadMap[tabId] ?? []);
              const relPath = hook ? entityRelPath("webhooks", hook, folders) : "";
              const syncStatus = relPath ? entitySyncStatus?.[relPath] : undefined;

              return (
                <div key={tabId} style={{ display: isTabActive ? "flex" : "none", flexDirection: "column", height: "100%" }}>
                  <WebhookEditor
                    ref={(el) => { tabRefs.current[tabId] = el; }}
                    tabId={tabId}
                    webhookId={isDraft ? null : tabId}
                    initial={hook}
                    isNew={isDraft}
                    webhookPort={webhookPort}
                    onSave={(data) => isDraft ? handleNewSave(tabId, data) : handleTabSave(tabId, data)}
                    onClose={() => closeTab(tabId)}
                    folders={folders}
                    payloads={tabPayloads}
                    isActive={isActivated}
                    isAtLimit={isAtLimit && !isActivated}
                    onDirtyChange={(dirty) => setDirtyTabs((prev) => ({ ...prev, [tabId]: dirty }))}
                    onSync={onPublishItem ? async (savedId?: string) => {
                      const targetId = savedId ?? tabId;
                      await onPublishItem(targetId);
                    } : undefined}
                    onRevert={onRestoreItem ? async () => {
                      await onRestoreItem(tabId);
                      const res = await window.api.loadEntity(config.activeWorkspaceId, "webhooks", tabId);
                      if (res.ok && res.entity) {
                        const entity = res.entity as SavedWebhook;
                        setLoadedEntities((prev) => ({ ...prev, [tabId]: entity }));
                        tabRefs.current[tabId]?.refresh?.(entity);
                      } else if (!res.ok) {
                        closeTab(tabId);
                      }
                      setDirtyTabs((prev) => ({ ...prev, [tabId]: false }));
                    } : undefined}
                    onHistory={onHistoryOpen && relPath && !isDraft ? () => onHistoryOpen(relPath) : undefined}
                    syncStatus={syncStatus}
                  />
                </div>
              );
            })}
          </div>
        </>
      )}
    </div>
  );

  return (
    <>
      <SidebarLayout
        sidebarOpen={sidebarOpen}
        onToggle={() => setSidebarOpen((v) => !v)}
        sidebar={sidebarContent}
        storageKey="webhooks-panel-sidebar"
        collapsedBadge={
          <span className="text-[10px] text-muted-foreground rotate-90 whitespace-nowrap" style={{ writingMode: "vertical-rl" }}>
            {strings.webhooks.title}
          </span>
        }
      >
        {mainContent}
      </SidebarLayout>
    </>
  );
}
