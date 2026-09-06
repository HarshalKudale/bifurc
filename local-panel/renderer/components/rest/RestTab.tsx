import React, {
  forwardRef, useImperativeHandle, useReducer, useCallback, useEffect, useState, useRef,
} from "react";
import { SavedRequest, MockRule, Folder, Environment, ReplayResult } from "@/types";
import EditorTitleBar from "@/components/editor/EditorTitleBar";
import EditorTab from "@/components/editor/EditorTab";
import { UrlBar, BottomBar } from "@/components/editor/RequestTab";
import {
  tabReducer, initState, stateToSavePayload, stateToDraft, isDraftEmpty,
  TabType, TabState,
  RequestDraft, MockDraft,
} from "@/components/rest/restTabReducer";
import { useDraftPersist, loadDraft } from "@/hooks/useDraftPersist";
import { resolveVars, resolveHeaders } from "@/lib/resolveVars";
import { rowsToHeaders, b64ToText, textToB64, tryFormat, METHODS, MOCK_METHODS } from "@/lib/utils";
import { parseCurl, SKIP_CURL_HEADERS } from "@/lib/curlParser";
import { contentTypeToMode, isBinaryContentType } from "@/lib/bodyUtils";
import { strings } from "@/lib/strings";
import { runPreScript, runPostScript } from "@/lib/scriptRunner";
import { runTestScript } from "@/lib/testRunner";
import { ChevronDown } from "@/lib/icons";

import { useProtocolEditor } from "@/hooks/useProtocolEditor";
import { RestCurlImport } from "./RestCurlImport";
import { RestEditorPane } from "./RestEditorPane";
import { useRestActions } from "./useRestActions";

// -- Public handle for imperative refresh -----------------------------------

export interface RestTabHandle {
  refresh(entity: SavedRequest | MockRule): void;
  save(): void;
}

// -- Props ------------------------------------------------------------------

export interface RestTabProps {
  tabType: TabType;
  tabId: string;
  /** Present for unsaved/draft tabs to enable draft auto-save */
  draftTabId?: string | null;
  initial?: SavedRequest | MockRule | Partial<SavedRequest> | Partial<MockRule> | null;
  folders?: Folder[];
  activeEnv?: Environment | null;
  /** Called with the data to persist. Parent handles add vs update. */
  onSave(data: Omit<SavedRequest, "id" | "createdAt" | "workspaceId"> | Omit<MockRule, "id" | "createdAt" | "workspaceId">): Promise<void>;
  onClose(): void;
  /** Request mode only: open a new mock pre-filled from this request/response */
  onCreateMock?(initial: Partial<MockRule>): void;
  /** Called whenever the editor has unsaved changes (for tab dirty indicator) */
  onDirtyChange?(dirty: boolean): void;
  /** Show the cURL import section (for new/draft tabs) */
  showCurlImport?: boolean;
  /** Label shown in the title bar */
  label?: string;
  /** Enabled state for saved mock tabs */
  enabled?: boolean;
  /** Called when the enabled toggle is clicked */
  onToggleEnabled?: () => void;
  /** Commit and push current state of entity */
  onSync?: (savedId?: string) => Promise<void>;
  /** Revert local changes to last synced version */
  onRevert?: () => Promise<void>;
  /** Git sync status of this entity */
  syncStatus?: "clean" | "modified" | "new" | "deleted";
  /** View git history for this entity */
  onHistory?: () => void;
}

// -- Component --------------------------------------------------------------

const RestTab = forwardRef<RestTabHandle, RestTabProps>(function RestTab(
  {
    tabType, tabId, draftTabId, initial, folders = [], activeEnv = null,
    onSave, onClose, onCreateMock, onDirtyChange, showCurlImport = false, label,
    enabled, onToggleEnabled, onSync, onRevert, syncStatus, onHistory,
  },
  ref,
) {
  const {
    state,
    dispatch,
    isDirty,
    handleSave,
    handleRefresh,
    syncing,
    reverting,
    handleSyncClick,
    handleRevertClick,
    hasLocalChanges
  } = useProtocolEditor({
    tabType,
    tabId,
    draftTabId,
    initial: initial as any,
    reducer: tabReducer,
    initState: initState as any,
    stateToDraft: stateToDraft as any,
    isDraftEmpty: isDraftEmpty as any,
    stateToSavePayload: stateToSavePayload as any,
    onSave: onSave as any,
    onSync,
    onRevert,
    syncStatus,
    onDirtyChange,
  });

  const handleSaveRef = useRef(handleSave);
  useEffect(() => { handleSaveRef.current = handleSave; }, [handleSave]);

  // -- cURL parsing ------------------------------------------------------

  const handleCurlChange = useCallback((v: string) => {
    dispatch({ type: "SET_FIELD", field: "curlInput", value: v });
    if (!v.trim().startsWith("curl")) return;
    const p = parseCurl(v.trim());
    const filtered: Record<string, string> = {};
    for (const [k, hv] of Object.entries(p.headers)) {
      if (!SKIP_CURL_HEADERS.has(k)) filtered[k] = hv;
    }
    dispatch({ type: "APPLY_CURL", url: p.url ?? "", method: p.method ?? "", headers: filtered, body: p.body ?? "" });
  }, []);

  // -- Regex toggle (mock only) -------------------------------------------

  const handleRegexToggle = useCallback((checked: boolean) => {
    dispatch({ type: "SET_FIELD", field: "useRegex", value: checked });
    dispatch({ type: "SET_FIELD", field: "regexError", value: "" });
    if (checked && state.url) {
      try { new RegExp(state.url); } catch { dispatch({ type: "SET_FIELD", field: "regexError", value: strings.editor.invalidRegex }); }
    }
  }, [state.url]);

  const handleUrlChange = useCallback((v: string) => {
    dispatch({ type: "SET_URL", url: v });
    if (state.useRegex) {
      try { new RegExp(v); dispatch({ type: "SET_FIELD", field: "regexError", value: "" }); }
      catch { dispatch({ type: "SET_FIELD", field: "regexError", value: strings.editor.invalidRegex }); }
    }
  }, [state.useRegex]);

  // -- Send (request mode) ------------------------------------------------

  const { handleSend, handleTest, handleCreateMock } = useRestActions(state, dispatch, activeEnv, tabType, onCreateMock);

  // Expose imperative handle for save + refresh
  useImperativeHandle(ref, () => ({
    refresh(entity: SavedRequest | MockRule) {
      handleRefresh(entity);
    },
    save() {
      return handleSaveRef.current();
    },
  }), [handleRefresh]);

  // -- Derived ------------------------------------------------------------

  const resBodyText = state.result
    ? (state.resMode === "json" ? tryFormat(b64ToText(state.result.body)) : b64ToText(state.result.body))
    : "";

  const canSaveBase = tabType === "request"
    ? !!state.url.trim()
    : !!(state.url.trim() && !(state.useRegex && state.regexError) && (state.resMode === "none" || state.resBody.trim()));
  // For saved (non-draft) tabs, also require actual changes before enabling save
  const canSave = canSaveBase && (!!draftTabId || isDirty);

  const actionLabel = tabType === "request" ? strings.server.send : strings.server.test;
  const actionLoading = tabType === "request" ? state.loading : state.testLoading;
  const actionLoadLabel = tabType === "request" ? strings.server.sending : strings.server.testing;
  const isValidAbsoluteUrl = (() => { try { return !!new URL(state.url.trim()); } catch { return false; } })();
  const actionDisabled = tabType === "request" ? !state.url.trim() : (state.useRegex || !isValidAbsoluteUrl);
  const handleAction = tabType === "request" ? handleSend : handleTest;
  const methods = tabType === "request" ? METHODS : MOCK_METHODS;

  const titleLabel = label ?? (tabType === "request" ? "REST" : "REST MOCK");

  const namePlaceholder = tabType === "request" ? strings.requests.requestNamePlaceholder : strings.mocks.mockName;
  const urlPlaceholder = tabType === "request" ? strings.requests.urlPlaceholder : strings.mocks.urlPatternPlaceholder;

  const errorMsg = state.sendErr ?? state.saveErr ?? state.regexError ?? state.testError ?? null;

  const syncDisabled = !hasLocalChanges || (!canSaveBase && isDirty) || syncing;
  const revertDisabled = !hasLocalChanges || reverting;
  const syncTitle = !hasLocalChanges ? strings.common.noChangesToSync : strings.common.syncTooltip;
  const revertTitle = !hasLocalChanges ? strings.common.noChangesToRevert : strings.common.revertTooltip;

  // -- Render -------------------------------------------------------------

  return (
    <div className="flex flex-col h-full overflow-hidden bg-surface">
      {/* Title bar */}
      <EditorTitleBar
        label={titleLabel}
        namePlaceholder={namePlaceholder}
        name={state.name}
        onNameChange={(v) => dispatch({ type: "SET_FIELD", field: "name", value: v })}
        onClose={onClose}
        enabled={enabled}
        onToggleEnabled={onToggleEnabled}
      />

      {/* cURL import - collapsible for request, always open for mock */}
      {showCurlImport && (
        <RestCurlImport
          tabType={tabType}
          showCurl={state.showCurl}
          curlInput={state.curlInput}
          onToggleShowCurl={() => dispatch({ type: "SET_FIELD", field: "showCurl", value: !state.showCurl })}
          onCurlChange={handleCurlChange}
        />
      )}

      {/* URL bar */}
      <UrlBar
        method={state.method}
        onMethodChange={(v) => dispatch({ type: "SET_FIELD", field: "method", value: v })}
        url={state.url}
        onUrlChange={handleUrlChange}
        methods={methods}
        urlPlaceholder={urlPlaceholder}
        actionLabel={actionLabel}
        actionLoadingLabel={actionLoadLabel}
        actionLoading={actionLoading}
        actionDisabled={actionDisabled}
        onAction={handleAction}
        onEnter={tabType === "request" ? handleSend : undefined}
        activeEnv={activeEnv}
        showRandomizer={tabType !== "mock"}
        inputSuffix={
          tabType === "mock" ? (
            <label
              className="flex items-center gap-1.5 px-3 border-l border-border cursor-pointer select-none flex-shrink-0 hover:bg-card transition-colors"
              title={strings.editor.matchUrlAsRegex}
            >
              <input
                type="checkbox"
                checked={state.useRegex}
                onChange={(e) => handleRegexToggle(e.target.checked)}
                className="accent-signal"
              />
              <span className="font-mono text-[11px] text-muted-foreground">.*</span>
            </label>
          ) : undefined
        }
      />

      {/* Error banner */}
      {errorMsg && (
        <div className="px-4 py-2 border-b border-border bg-destructive/5 flex-shrink-0">
          <span className="text-xs text-destructive font-mono">{errorMsg}</span>
        </div>
      )}

      {/* Split-pane editor body */}
      <RestEditorPane
        tabType={tabType}
        state={state as any}
        dispatch={dispatch}
        activeEnv={activeEnv}
        resBodyText={resBodyText}
      />

      {/* Bottom bar */}
      <BottomBar
        folders={folders}
        folderId={state.folderId}
        onFolderChange={(v) => dispatch({ type: "SET_FIELD", field: "folderId", value: v })}
        onCancel={onClose}
        onSave={handleSave}
        saveLabel={draftTabId ? (tabType === "request" ? strings.editor.saveRequest : strings.mocks.saveMock) : (tabType === "request" ? strings.editor.updateRequest : strings.editor.updateMock)}
        saveDisabled={!canSave}
        saving={state.saving}
        savingLabel={strings.server.saving}
        onSync={onSync ? handleSyncClick : undefined}
        onRevert={onRevert ? handleRevertClick : undefined}
        onHistory={onHistory}
        historyDisabled={!onHistory || !!draftTabId}
        syncDisabled={syncDisabled}
        revertDisabled={revertDisabled}
        syncing={syncing}
        reverting={reverting}
        syncTitle={syncTitle}
        revertTitle={revertTitle}
        onCreateMock={tabType === "request" && onCreateMock ? handleCreateMock : undefined}
        createMockDisabled={!state.result || state.loading}
        extraLeft={
          tabType === "mock" ? (
            !state.resBody.trim()
              ? <span className="text-[10px] text-muted-foreground italic">{strings.mocks.addResponseBody}</span>
              : <span className="text-[10px] text-muted-foreground">{strings.mocks.mocksNote}</span>
          ) : undefined
        }
      />
    </div>
  );
});

export default RestTab;
