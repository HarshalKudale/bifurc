import React, {
    forwardRef, useImperativeHandle, useReducer, useCallback, useEffect, useState,
} from "react";
import { SavedGraphQLRequest, Folder, Environment } from "@/types";
import EditorTitleBar from "@/components/editor/EditorTitleBar";
import { BottomBar, TabStrip } from "@/components/editor/RequestTab";
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
import { Group as PanelGroup, Panel, Separator as PanelResizeHandle } from "react-resizable-panels";
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

    // -- Request pane sub-tabs ----------------------------------------------

    type ReqSubTab = "query" | "variables" | "headers" | "pre-script" | "post-script" | "schema";
    const [reqSubTab, setReqSubTab] = useState<ReqSubTab>("query");
    const [showSchemaExplorer, setShowSchemaExplorer] = useState(false);

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

    // -- Format response body -----------------------------------------------

    const formattedResBody = (() => {
        try { return JSON.stringify(JSON.parse(state.resBody), null, 2); } catch { return state.resBody; }
    })();

    // -- Render -------------------------------------------------------------

    return (
        <div className="flex flex-col h-full overflow-hidden">
            <EditorTitleBar
                label={label ?? "GRAPHQL"}
                namePlaceholder={strings.graphql.requestNamePlaceholder}
                name={state.name}
                onNameChange={(v) => dispatch({ type: "SET_FIELD", field: "name", value: v })}
                onClose={onClose}
            />

            {/* URL bar */}
            <div className="px-4 py-2.5 border-b border-border flex-shrink-0 flex items-center gap-2">
                <div
                    className="flex items-stretch rounded border border-border focus-within:border-signal transition-colors overflow-hidden flex-1"
                    style={{ background: "var(--c-card)" }}
                >
                    <span className="bg-surface-2 border-r border-border text-xs font-bold font-mono px-3 py-2.5 flex-shrink-0 text-signal">
                        {strings.graphql.methodPost}
                    </span>
                    <input
                        className="flex-1 bg-transparent px-3 py-2.5 text-sm font-mono text-foreground outline-none placeholder:text-muted-foreground min-w-0"
                        placeholder="https://api.example.com/graphql"
                        value={state.endpointUrl}
                        onChange={(e) => dispatch({ type: "SET_FIELD", field: "endpointUrl", value: e.target.value })}
                        onKeyDown={(e) => { if (e.key === "Enter") handleSend(); }}
                    />
                </div>
                <button
                    onClick={handleSend}
                    disabled={state.sending || !state.endpointUrl.trim()}
                    className="px-4 py-2.5 rounded bg-signal hover:bg-signal/80 disabled:opacity-40 disabled:cursor-not-allowed text-background text-xs font-semibold transition-all cursor-pointer flex-shrink-0 flex items-center gap-1.5"
                >
                    {state.sending
                        ? <><span className="inline-block w-3 h-3 border-2 border-white/30 border-t-white rounded-full animate-spin" />{strings.graphql.sending}</>
                        : strings.server.send}
                </button>
            </div>

            {/* Split panes */}
            <PanelGroup orientation="horizontal" className="flex-1 overflow-hidden">
                {/* Left pane */}
                <Panel defaultSize={50} minSize={25}>
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
                </Panel>

                <PanelResizeHandle className="w-px bg-border hover:bg-signal transition-colors cursor-col-resize" />

                {/* Right pane */}
                <Panel defaultSize={50} minSize={25}>
                    <div className="flex flex-col h-full overflow-hidden">
                        {/* Response header bar */}
                        <div className="flex items-center gap-3 px-4 py-2 border-b border-border flex-shrink-0 bg-background/40">
                            <span className="text-[10px] font-semibold text-muted-foreground uppercase tracking-wider">{strings.graphql.response}</span>
                            {state.resStatus !== null && (
                                <>
                                    <span className={`text-xs font-mono font-bold ${state.resStatus < 300 ? "text-success" : state.resStatus < 400 ? "text-amber" : "text-destructive"}`}>
                                        {state.resStatus}
                                    </span>
                                    <span className="text-xs text-muted-foreground">{state.resDuration}ms</span>
                                </>
                            )}
                            {state.resError && <span className="text-xs text-destructive">{state.resError}</span>}
                        </div>
                        <div className="flex-1 overflow-hidden">
                            <CodeEditor
                                value={formattedResBody}
                                language="json"
                                readOnly
                                placeholder={strings.graphql.responsePlaceholder}
                                className="h-full"
                            />
                        </div>
                    </div>
                </Panel>
            </PanelGroup>

            {/* Bottom bar — Cancel and Create exist only for a draft: an existing entity
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
        </div>
    );
});

export default GraphQLTab;
