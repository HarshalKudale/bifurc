import React, {
    forwardRef, useImperativeHandle, useReducer, useCallback, useEffect, useState,
} from "react";
import { SavedSoapRequest, Folder, Environment } from "@/types";
import { BottomBar, TabStrip, UrlBar, UrlBarVerb } from "@/components/editor/RequestTab";
import {
    TabPage, TabTitleBar, TabToolbar, TabToolbarRow, SplitPane, useSplitOrientation,
    ResponseStripSuffix, PROTOCOL_TONE,
} from "@/components/editor/TabPage";
import CodeEditor from "@/components/common/CodeEditor";
import HeaderTable from "@/components/common/HeaderTable";
import {
    soapTabReducer, initSoapState, soapStateToSavePayload, soapStateToDraft, isSoapDraftEmpty,
    SoapTabType, SoapTabState, SoapTabAction, SoapRequestDraft,
} from "@/components/soap/soapTabReducer";
import { useDraftPersist, loadDraft } from "@/hooks/useDraftPersist";
import { KVRow, mkRowId, headersToRows, rowsToHeaders } from "@/lib/utils";
import { resolveVars, resolveHeaders } from "@/lib/resolveVars";
import WsdlExplorer from "@/components/soap/WsdlExplorer";
import { strings } from "@/lib/strings";
import { useProtocolEditor } from "@/hooks/useProtocolEditor";

// -- Public handle for imperative refresh -----------------------------------

export interface SoapTabHandle {
    refresh(entity: SavedSoapRequest): void;
    save(): void;
    /** Rename the tab's entity (used by double-click tab rename). Marks the tab dirty. */
    rename(name: string): void;
}

// -- Props ------------------------------------------------------------------

export interface SoapTabProps {
    tabType: SoapTabType;
    tabId: string;
    draftTabId?: string | null;
    initial?: SavedSoapRequest | Partial<SavedSoapRequest> | null;
    folders?: Folder[];
    activeEnv?: Environment | null;
    onSave(data: Omit<SavedSoapRequest, "id" | "createdAt" | "workspaceId">): Promise<void>;
    onClose(): void;
    label?: string;
    onDirtyChange?(dirty: boolean): void;
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

const SoapTab = forwardRef<SoapTabHandle, SoapTabProps>(function SoapTab(
    { tabType, tabId, draftTabId, initial, folders = [], activeEnv = null, onSave, onClose, label, onDirtyChange, onSync, onRevert, syncStatus, onHistory },
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
        hasLocalChanges,
        autoSaving,
        autoSavedAt,
        autoSaveError,
        saveNow,
    /*
     * All six type arguments are spelled out, and that is deliberate rather than laziness.
     *
     * TypeScript will not infer `TabType` here. Removing the default and letting it try produced **more**
     * errors, not fewer (123 against 50), because a failed inference falls back to the constraint — so the
     * only options are "all six" or "none", and "none" is what produced the `as any` casts this replaces.
     * An explicit argument list is checked; a cast is not. The five that *could* be inferred are listed for
     * the sixth's sake.
     */
    } = useProtocolEditor<
        SoapTabState,
        SoapTabAction,
        SoapRequestDraft,
        Omit<SavedSoapRequest, "id" | "createdAt" | "workspaceId">,
        SavedSoapRequest | Partial<SavedSoapRequest>,
        SoapTabType
    >({
        tabType,
        tabId,
        draftTabId,
        initial,
        reducer: soapTabReducer,
        initState: initSoapState,
        stateToDraft: soapStateToDraft,
        isDraftEmpty: isSoapDraftEmpty,
        stateToSavePayload: soapStateToSavePayload,
        onSave,
        onSync,
        onRevert,
        syncStatus,
        onDirtyChange,
    });

    // The split is shared with every other protocol tab, and persisted under one global key.
    const [splitOrientation, toggleSplitOrientation] = useSplitOrientation();

    // -- Header rows helper -----------------------------------------------

    const headerRows: KVRow[] = headersToRows(state.headers);
    const setHeaderRows = useCallback((rows: KVRow[]) => {
        dispatch({ type: "SET_FIELD", field: "headers", value: rowsToHeaders(rows) });
    }, [dispatch]);

    // -- Send request -----------------------------------------------------

    const handleSend = useCallback(async () => {
        if (!state.endpointUrl.trim()) return;
        dispatch({ type: "SEND_START" });
        try {
            const url = resolveVars(state.endpointUrl, activeEnv);
            const hdrs = resolveHeaders(state.headers, activeEnv);
            const body = resolveVars(state.body, activeEnv);
            const result = await window.api.soapExecute(url, state.soapAction, hdrs, body);
            dispatch({
                type: "SEND_SUCCESS",
                status: result.status,
                headers: result.headers,
                body: result.body,
                durationMs: result.durationMs,
            });
        } catch (err: any) {
            dispatch({ type: "SEND_ERROR", error: err.message ?? String(err) });
        }
    }, [state.endpointUrl, state.soapAction, state.headers, state.body, activeEnv, dispatch]);

    // Expose imperative refresh
    useImperativeHandle(ref, () => ({
        refresh(entity: SavedSoapRequest) {
            handleRefresh(entity);
        },
        save() {
            // Ctrl+S: a draft is created, an existing entity flushes its pending autosave.
            return saveNow();
        },
        rename(name: string) {
            dispatch({ type: "SET_FIELD", field: "name", value: name });
        },
    }), [handleRefresh, saveNow]);

    const canSave = !(!state.name && !state.endpointUrl);
    const syncDisabled = !hasLocalChanges || (!canSave && isDirty) || syncing;
    const revertDisabled = !hasLocalChanges || reverting;
    const syncTitle = !hasLocalChanges ? strings.common.noChangesToSync : strings.common.syncTooltip;
    const revertTitle = !hasLocalChanges ? strings.common.noChangesToRevert : strings.common.revertTooltip;

    // -- Request mode: left pane tabs -------------------------------------

    const reqTabs = [
        { id: "body" as const, label: strings.soap.tabBody },
        { id: "headers" as const, label: strings.soap.tabHeaders },
        { id: "pre-script" as const, label: strings.soap.tabPreScript },
        { id: "post-script" as const, label: strings.soap.tabPostScript },
        { id: "wsdl" as const, label: strings.soap.tabWsdl },
    ];

    // The response strip uses the shared labels, so "the response body" and "the response headers"
    // are called the same thing on every protocol rather than "Body"/"Headers" here and
    // "Response"/"Res Headers" on REST.
    const resTabs = [
        { id: "body" as const, label: strings.editor.response },
        { id: "headers" as const, label: strings.editor.resHeaders },
    ];

    // -- Render -----------------------------------------------------------

    return (
        <TabPage>
            {/* Band 1 — title bar. Chip text matches the tab strip's own SOAP badge. */}
            <TabTitleBar
                chip={label ?? "SOAP"}
                tone={PROTOCOL_TONE.soap}
                name={state.name}
                namePlaceholder={strings.soap.requestNamePlaceholder}
                onNameChange={(v) => dispatch({ type: "SET_FIELD", field: "name", value: v })}
            />

            {/*
                Band 2 — address band.
                This used to be two stacked hand-rolled rows with their own padding and their own
                bottom border, which is what made the SOAP tab read as taller than the others. Row 1
                is now the shared `UrlBar` (fixed POST verb) and row 2 the SOAPAction field — the
                same two-row shape gRPC uses for its service/method pair.
            */}
            <TabToolbar>
                <TabToolbarRow>
                    <UrlBar
                        bare
                        leading={<UrlBarVerb verb={strings.soap.methodPost} />}
                        url={state.endpointUrl}
                        onUrlChange={(v) => dispatch({ type: "SET_FIELD", field: "endpointUrl", value: v })}
                        urlPlaceholder="https://example.com/ws/service"
                        actionLabel={strings.server.send}
                        actionLoadingLabel={strings.soap.sending}
                        actionLoading={state.sending}
                        actionDisabled={!state.endpointUrl}
                        onAction={handleSend}
                        onEnter={handleSend}
                    />
                </TabToolbarRow>
                <TabToolbarRow>
                    <label className="text-xs text-muted-foreground flex-shrink-0">{strings.soap.soapAction}</label>
                    <input
                        className="flex-1 bg-card border border-border rounded px-2.5 py-1.5 text-xs font-mono text-foreground outline-none focus:border-signal placeholder:text-muted-foreground"
                        placeholder='"http://example.com/Action"'
                        value={state.soapAction}
                        onChange={(e) => dispatch({ type: "SET_FIELD", field: "soapAction", value: e.target.value })}
                    />
                </TabToolbarRow>
            </TabToolbar>

            {/* Band 3 — the split. Orientation was hard-coded `horizontal`. */}
            <SplitPane
                orientation={splitOrientation}
                minSize={25}
                left={
                    <div className="flex flex-col h-full overflow-hidden">
                        <TabStrip
                            tabs={reqTabs}
                            active={state.reqTab}
                            onChange={(t) => dispatch({ type: "SET_FIELD", field: "reqTab", value: t })}
                        />
                        <div className="flex-1 overflow-hidden">
                            {state.reqTab === "body" && (
                                <CodeEditor
                                    value={state.body}
                                    onChange={(v) => dispatch({ type: "SET_FIELD", field: "body", value: v })}
                                    language="xml"
                                    className="h-full"
                                />
                            )}
                            {state.reqTab === "headers" && (
                                <HeaderTable rows={headerRows} onChange={setHeaderRows} />
                            )}
                            {state.reqTab === "pre-script" && (
                                <CodeEditor
                                    value={state.preScript}
                                    onChange={(v) => dispatch({ type: "SET_FIELD", field: "preScript", value: v })}
                                    language="javascript"
                                    className="h-full"
                                />
                            )}
                            {state.reqTab === "post-script" && (
                                <CodeEditor
                                    value={state.postScript}
                                    onChange={(v) => dispatch({ type: "SET_FIELD", field: "postScript", value: v })}
                                    language="javascript"
                                    className="h-full"
                                />
                            )}
                            {state.reqTab === "wsdl" && (
                                <WsdlExplorer
                                    wsdlId={state.wsdlId ?? null}
                                    onInsertEnvelope={(body, soapAction) => {
                                        dispatch({ type: "SET_FIELD", field: "body", value: body });
                                        if (soapAction) dispatch({ type: "SET_FIELD", field: "soapAction", value: soapAction });
                                    }}
                                />
                            )}
                        </div>
                    </div>
                }
                right={
                    <div className="flex flex-col h-full overflow-hidden">
                        {/* The status used to be a TabStrip `prefix` — a third position for the same
                            number. It is now the suffix, like every other protocol. */}
                        <TabStrip
                            tabs={resTabs}
                            active={state.resTab}
                            onChange={(t) => dispatch({ type: "SET_FIELD", field: "resTab", value: t })}
                            suffix={
                                <ResponseStripSuffix
                                    orientation={splitOrientation}
                                    onToggleOrientation={toggleSplitOrientation}
                                    status={state.resStatus}
                                    durationMs={state.resDuration}
                                />
                            }
                        />
                        <div className="flex-1 overflow-hidden">
                            {state.resError ? (
                                <div className="p-4 text-destructive text-xs font-mono">{state.resError}</div>
                            ) : state.resTab === "body" ? (
                                <CodeEditor
                                    value={state.resBody}
                                    onChange={() => { }}
                                    language="xml"
                                    readOnly
                                    className="h-full"
                                />
                            ) : (
                                <HeaderTable
                                    rows={headersToRows(state.resHeaders)}
                                    onChange={() => { }}
                                    readOnly
                                    emptyMessage={strings.soap.noResponseYet}
                                />
                            )}
                        </div>
                    </div>
                }
            />

            {/* Band 4 — Bottom bar. Cancel and Create exist only for a draft: an existing entity
                autosaves every edit and has Revert for undo. */}
            <BottomBar
                folders={folders}
                folderId={state.folderId}
                onFolderChange={(id) => dispatch({ type: "SET_FIELD", field: "folderId", value: id })}
                onCancel={draftTabId ? onClose : undefined}
                onSave={draftTabId ? handleSave : undefined}
                saveLabel={draftTabId ? strings.common.create : undefined}
                saveDisabled={!canSave}
                saving={state.saving}
                savingLabel={strings.server.saving}
                autoSaving={autoSaving}
                autoSavedAt={autoSavedAt}
                autoSaveError={autoSaveError}
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
            />
        </TabPage>
    );
});

export default SoapTab;
