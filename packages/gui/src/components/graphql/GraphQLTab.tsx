import React, {
    forwardRef, useImperativeHandle, useReducer, useCallback, useEffect, useState,
} from "react";
import { SavedGraphQLRequest, Folder, Environment } from "@/types";
import { BottomBar, TabStrip, UrlBar, UrlBarVerb } from "@/components/editor/RequestTab";
import {
    TabPage, TabTitleBar, TabToolbar, TabToolbarRow, SplitPane, useSplitOrientation,
    ResponseStripSuffix, PROTOCOL_TONE,
} from "@/components/editor/TabPage";
import {
    graphqlTabReducer, initGraphQLState, stateToRequestPayload,
    stateToDraft, isDraftEmpty,
    GraphQLTabType, GraphQLTabState, GraphQLTabAction,
    GraphQLRequestDraft,
} from "@/components/graphql/graphqlTabReducer";
import { useDraftPersist, loadDraft } from "@/hooks/useDraftPersist";
import CodeEditor from "@/components/common/CodeEditor";
import HeaderTable from "@/components/common/HeaderTable";
import { KVRow, mkRowId, headersToRows, rowsToHeaders } from "@/lib/utils";
import { strings } from "@/lib/strings";
import { resolveVars, resolveHeaders } from "@/lib/resolveVars";
import SchemaExplorer from "@/components/graphql/SchemaExplorer";
import { useProtocolEditor } from "@/hooks/useProtocolEditor";

// -- Public handle for imperative refresh -----------------------------------

export interface GraphQLTabHandle {
    refresh(entity: SavedGraphQLRequest): void;
    save(): void;
    /** Rename the tab's entity (used by double-click tab rename). Marks the tab dirty. */
    rename(name: string): void;
}

// -- Props ------------------------------------------------------------------

export interface GraphQLTabProps {
    tabType: GraphQLTabType;
    tabId: string;
    draftTabId?: string | null;
    initial?: SavedGraphQLRequest | Partial<SavedGraphQLRequest> | null;
    folders?: Folder[];
    activeEnv?: Environment | null;
    onSave(data: Omit<SavedGraphQLRequest, "id" | "createdAt" | "workspaceId">): Promise<void>;
    onClose(): void;
    onDirtyChange?(dirty: boolean): void;
    label?: string;
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

const GraphQLTab = forwardRef<GraphQLTabHandle, GraphQLTabProps>(function GraphQLTab(
    { tabType, tabId, draftTabId, initial, folders = [], activeEnv = null, onSave, onClose, onDirtyChange, label, onSync, onRevert, syncStatus, onHistory },
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
    /* All six spelled out — see the note in `SoapTab.tsx`; TypeScript cannot infer `TabType`, and a partial
       list is rejected outright ("Expected 5-6 type arguments"), so it is all six or the casts this replaced. */
    } = useProtocolEditor<
        GraphQLTabState,
        GraphQLTabAction,
        GraphQLRequestDraft,
        Omit<SavedGraphQLRequest, "id" | "createdAt" | "workspaceId">,
        SavedGraphQLRequest | Partial<SavedGraphQLRequest>,
        GraphQLTabType
    >({
        tabType,
        tabId,
        draftTabId,
        initial,
        reducer: graphqlTabReducer,
        initState: initGraphQLState,
        stateToDraft,
        isDraftEmpty,
        stateToSavePayload: stateToRequestPayload,
        onSave,
        onSync,
        onRevert,
        syncStatus,
        onDirtyChange,
    });

    // The split is shared with every other protocol tab, and persisted under one global key.
    const [splitOrientation, toggleSplitOrientation] = useSplitOrientation();

    // -- Request pane sub-tabs ----------------------------------------------

    type ReqSubTab = "query" | "variables" | "headers" | "pre-script" | "post-script" | "schema";
    const [reqSubTab, setReqSubTab] = useState<ReqSubTab>("query");

    // -- Response pane sub-tabs ---------------------------------------------
    //
    // New, and layout-only: `resHeaders` has been in the reducer since the beginning — `handleSend`
    // dispatches `headers: result.headers` on every successful call — it simply had nowhere to be
    // shown. REST, gRPC and SOAP all expose the response headers, so a GraphQL tab that could not
    // was the odd one out rather than the simple one.
    type ResSubTab = "body" | "headers";
    const [resSubTab, setResSubTab] = useState<ResSubTab>("body");

    const headerRows: KVRow[] = headersToRows(state.headers);

    const handleHeadersChange = useCallback((rows: KVRow[]) => {
        dispatch({ type: "SET_FIELD", field: "headers", value: rowsToHeaders(rows) });
    }, []);

    // -- Send request -------------------------------------------------------

    const handleSend = useCallback(async () => {
        if (!state.endpointUrl.trim()) return;
        dispatch({ type: "SEND_START" });
        try {
            const url = resolveVars(state.endpointUrl, activeEnv);
            const hdrs = resolveHeaders(state.headers, activeEnv);
            const vars = resolveVars(state.variables, activeEnv);
            const result = await window.api.graphqlExecute(
                url,
                hdrs,
                state.query,
                vars,
                state.operationName,
            );
            dispatch({ type: "SEND_SUCCESS", status: result.status, headers: result.headers, body: result.body, durationMs: result.durationMs });
        } catch (err: any) {
            dispatch({ type: "SEND_ERROR", error: err.message ?? String(err) });
        }
    }, [state.endpointUrl, state.headers, state.query, state.variables, state.operationName, activeEnv]);

    useImperativeHandle(ref, () => ({
        refresh(entity: SavedGraphQLRequest) {
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

    const canSave = !!(state.name || state.endpointUrl);
    const syncDisabled = !hasLocalChanges || (!canSave && isDirty) || syncing;
    const revertDisabled = !hasLocalChanges || reverting;
    const syncTitle = !hasLocalChanges ? strings.common.noChangesToSync : strings.common.syncTooltip;
    const revertTitle = !hasLocalChanges ? strings.common.noChangesToRevert : strings.common.revertTooltip;

    // -- Render: Request mode -----------------------------------------------

    const reqTabs: { id: ReqSubTab; label: string }[] = [
        { id: "query", label: strings.graphql.tabQuery },
        { id: "variables", label: strings.graphql.tabVariables },
        { id: "headers", label: strings.graphql.tabHeaders },
        { id: "pre-script", label: strings.graphql.tabPreScript },
        { id: "post-script", label: strings.graphql.tabPostScript },
        { id: "schema", label: strings.graphql.tabSchema },
    ];

    const resTabs: { id: ResSubTab; label: string }[] = [
        { id: "body", label: strings.editor.response },
        { id: "headers", label: strings.editor.resHeaders },
    ];

    // -- Format response body -----------------------------------------------

    const formattedResBody = (() => {
        try { return JSON.stringify(JSON.parse(state.resBody), null, 2); } catch { return state.resBody; }
    })();

    // -- Render -------------------------------------------------------------

    return (
        <TabPage>
            {/* Band 1 — title bar. Was `EditorTitleBar`; the chip now matches the tab strip's own
                badge, so the tab and its page agree about which protocol this is. */}
            <TabTitleBar
                chip={label ?? "GQL"}
                tone={PROTOCOL_TONE.graphql}
                name={state.name}
                namePlaceholder={strings.graphql.requestNamePlaceholder}
                onNameChange={(v) => dispatch({ type: "SET_FIELD", field: "name", value: v })}
            />

            {/* Band 2 — address row. Was a hand-rolled copy of `UrlBar` with a hard-coded POST badge;
                it is now literally `UrlBar` with that badge passed as `leading`, so the input, the
                helper buttons and the Send button are the same elements REST uses. */}
            <TabToolbar>
                <TabToolbarRow>
                    <UrlBar
                        bare
                        leading={<UrlBarVerb verb={strings.graphql.methodPost} />}
                        url={state.endpointUrl}
                        onUrlChange={(v) => dispatch({ type: "SET_FIELD", field: "endpointUrl", value: v })}
                        urlPlaceholder="https://api.example.com/graphql"
                        actionLabel={strings.server.send}
                        actionLoadingLabel={strings.graphql.sending}
                        actionLoading={state.sending}
                        actionDisabled={!state.endpointUrl.trim()}
                        onAction={handleSend}
                        onEnter={handleSend}
                    />
                </TabToolbarRow>
            </TabToolbar>

            {/* Band 3 — the split. The orientation was hard-coded `horizontal` here, so this tab was
                one of the three you could not stack. */}
            <SplitPane
                orientation={splitOrientation}
                minSize={25}
                left={
                    <div className="flex flex-col h-full overflow-hidden">
                        <TabStrip
                            tabs={reqTabs}
                            active={reqSubTab}
                            onChange={(t) => setReqSubTab(t as ReqSubTab)}
                        />
                        <div className="flex-1 overflow-hidden">
                            {reqSubTab === "query" && (
                                <CodeEditor
                                    value={state.query}
                                    onChange={(v) => dispatch({ type: "SET_FIELD", field: "query", value: v })}
                                    language="text"
                                    placeholder="query { ... }"
                                    className="h-full"
                                />
                            )}
                            {reqSubTab === "variables" && (
                                <CodeEditor
                                    value={state.variables}
                                    onChange={(v) => dispatch({ type: "SET_FIELD", field: "variables", value: v })}
                                    language="json"
                                    placeholder='{ "key": "value" }'
                                    className="h-full"
                                />
                            )}
                            {reqSubTab === "headers" && (
                                <HeaderTable rows={headerRows} onChange={handleHeadersChange} />
                            )}
                            {reqSubTab === "pre-script" && (
                                <CodeEditor
                                    value={state.preScript}
                                    onChange={(v) => dispatch({ type: "SET_FIELD", field: "preScript", value: v })}
                                    language="javascript"
                                    placeholder="// Pre-request script"
                                    className="h-full"
                                />
                            )}
                            {reqSubTab === "post-script" && (
                                <CodeEditor
                                    value={state.postScript}
                                    onChange={(v) => dispatch({ type: "SET_FIELD", field: "postScript", value: v })}
                                    language="javascript"
                                    placeholder="// Post-response script"
                                    className="h-full"
                                />
                            )}
                            {reqSubTab === "schema" && (
                                <SchemaExplorer
                                    schemaId={state.schemaId}
                                    onInsertQuery={(q) => dispatch({ type: "SET_FIELD", field: "query", value: q })}
                                    onInsertVariables={(v) => dispatch({ type: "SET_FIELD", field: "variables", value: v })}
                                />
                            )}
                        </div>
                    </div>
                }
                right={
                    <div className="flex flex-col h-full overflow-hidden">
                        {/* The response had its own bespoke header bar with the status code inside it.
                            It is now the same TabStrip every other protocol uses, with the status and
                            the layout toggle in the suffix — see `ResponseStripSuffix`. */}
                        <TabStrip
                            tabs={resTabs}
                            active={resSubTab}
                            onChange={(t) => setResSubTab(t as ResSubTab)}
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
                            {state.resError && (
                                <div className="p-4">
                                    <div className="px-3 py-2 rounded border border-destructive/30 bg-destructive/5 text-xs text-destructive font-mono">
                                        {state.resError}
                                    </div>
                                </div>
                            )}
                            {!state.resError && resSubTab === "body" && (
                                <CodeEditor
                                    value={formattedResBody}
                                    language="json"
                                    readOnly
                                    placeholder={strings.graphql.responsePlaceholder}
                                    className="h-full"
                                />
                            )}
                            {!state.resError && resSubTab === "headers" && (
                                <HeaderTable rows={headersToRows(state.resHeaders)} onChange={() => { }} readOnly />
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

export default GraphQLTab;
