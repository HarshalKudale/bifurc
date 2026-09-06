import React, {
    forwardRef, useImperativeHandle, useReducer, useCallback, useEffect, useState,
} from "react";
import { SavedGraphQLRequest, SavedGraphQLMock, Folder, Environment } from "@/types";
import EditorTitleBar from "@/components/editor/EditorTitleBar";
import { BottomBar, TabStrip } from "@/components/editor/RequestTab";
import {
    graphqlTabReducer, initGraphQLState, stateToRequestPayload, stateToMockPayload,
    stateToDraft, isDraftEmpty,
    GraphQLTabType, GraphQLTabState,
    GraphQLRequestDraft, GraphQLMockDraft,
} from "@/components/graphql/graphqlTabReducer";
import { useDraftPersist, loadDraft } from "@/lib/useDraftPersist";
import CodeEditor from "@/components/common/CodeEditor";
import HeaderTable from "@/components/editor/HeaderTable";
import { KVRow, mkRowId, headersToRows, rowsToHeaders } from "@/lib/utils";
import { Group as PanelGroup, Panel, Separator as PanelResizeHandle } from "react-resizable-panels";
import { strings } from "@/lib/strings";
import { resolveVars, resolveHeaders } from "@/lib/resolveVars";
import SchemaExplorer from "@/components/graphql/SchemaExplorer";

// -- Public handle for imperative refresh -----------------------------------

export interface GraphQLTabHandle {
    refresh(entity: SavedGraphQLRequest | SavedGraphQLMock): void;
    save(): void;
}

// -- Props ------------------------------------------------------------------

export interface GraphQLTabProps {
    tabType: GraphQLTabType;
    tabId: string;
    draftTabId?: string | null;
    initial?: SavedGraphQLRequest | SavedGraphQLMock | Partial<SavedGraphQLRequest> | Partial<SavedGraphQLMock> | null;
    folders?: Folder[];
    activeEnv?: Environment | null;
    onSave(data: Omit<SavedGraphQLRequest, "id" | "createdAt" | "workspaceId"> | Omit<SavedGraphQLMock, "id" | "createdAt" | "workspaceId">): Promise<void>;
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
    const draft = draftTabId
        ? (tabType === "request"
            ? loadDraft<GraphQLRequestDraft>(draftTabId)
            : loadDraft<GraphQLMockDraft>(draftTabId))
        : null;

    const [state, dispatch] = useReducer(
        graphqlTabReducer,
        undefined,
        () => initGraphQLState(initial ?? null, draft, tabType),
    );

    const [initialSnapshot, setInitialSnapshot] = useState(() => JSON.stringify(stateToDraft(initGraphQLState(initial ?? null, draft, tabType), tabType)));
    useEffect(() => {
        const current = JSON.stringify(stateToDraft(state, tabType));
        onDirtyChange?.(current !== initialSnapshot);
    }, [state, tabType, initialSnapshot, onDirtyChange]);

    const { markSaved } = useDraftPersist(
        draftTabId ?? null,
        () => stateToDraft(state, tabType),
        () => isDraftEmpty(state, tabType),
    );

    // -- Request pane sub-tabs ----------------------------------------------

    type ReqSubTab = "query" | "variables" | "headers" | "pre-script" | "post-script" | "schema";
    const [reqSubTab, setReqSubTab] = useState<ReqSubTab>("query");
    const [showSchemaExplorer, setShowSchemaExplorer] = useState(false);

    const headerRows: KVRow[] = headersToRows(state.headers);

    const handleHeadersChange = useCallback((rows: KVRow[]) => {
        dispatch({ type: "SET_FIELD", field: "headers", value: rowsToHeaders(rows) });
    }, []);

    // Mock response headers
    const resHeaderRows: KVRow[] = headersToRows(state.responseHeaders);
    const handleResHeadersChange = useCallback((rows: KVRow[]) => {
        dispatch({ type: "SET_FIELD", field: "responseHeaders", value: rowsToHeaders(rows) });
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

    // -- Save ---------------------------------------------------------------

    const handleSave = useCallback(async () => {
        dispatch({ type: "SAVE_START" });
        try {
            const data = tabType === "request" ? stateToRequestPayload(state) : stateToMockPayload(state);
            const res = await onSave(data);
            dispatch({ type: "SAVE_SUCCESS" });
            markSaved();
            setInitialSnapshot(JSON.stringify(stateToDraft(state, tabType)));
            return res;
        } catch {
            dispatch({ type: "SAVE_ERROR" });
        }
    }, [state, tabType, onSave, markSaved]);

    useImperativeHandle(ref, () => ({
        refresh(entity: SavedGraphQLRequest | SavedGraphQLMock) {
            dispatch({ type: "REFRESH", entity, tabType });
            setInitialSnapshot(JSON.stringify(stateToDraft(initGraphQLState(entity, null, tabType), tabType)));
        },
        save() {
            return handleSave();
        },
    }), [tabType, handleSave]);

    const [syncing, setSyncing] = useState(false);
    const [reverting, setReverting] = useState(false);

    const isDirty = JSON.stringify(stateToDraft(state, tabType)) !== initialSnapshot;
    const canSave = tabType === "request" ? !!(state.name || state.endpointUrl) : !!state.name;
    const hasLocalChanges = !draftTabId && Boolean(isDirty || (syncStatus && syncStatus !== "clean"));
    const syncDisabled = !hasLocalChanges || (!canSave && isDirty) || syncing;
    const revertDisabled = !hasLocalChanges || reverting;
    const syncTitle = !hasLocalChanges ? strings.common.noChangesToSync : strings.common.syncTooltip;
    const revertTitle = !hasLocalChanges ? strings.common.noChangesToRevert : strings.common.revertTooltip;

    const handleSyncClick = useCallback(async () => {
        if (syncing || !onSync) return;
        setSyncing(true);
        try {
            let savedId: string | undefined = undefined;
            if (isDirty || draftTabId) {
                const res: any = await handleSave();
                if (res && typeof res === "object" && res.id) {
                    savedId = res.id;
                }
            }
            await onSync(savedId);
        } finally {
            setSyncing(false);
        }
    }, [syncing, onSync, isDirty, draftTabId, handleSave]);

    const handleRevertClick = useCallback(async () => {
        if (reverting || !onRevert) return;
        setReverting(true);
        try {
            await onRevert();
        } finally {
            setReverting(false);
        }
    }, [reverting, onRevert]);

    // -- Render: Request mode -----------------------------------------------

    const reqTabs: { id: ReqSubTab; label: string }[] = [
        { id: "query", label: strings.graphql.tabQuery },
        { id: "variables", label: strings.graphql.tabVariables },
        { id: "headers", label: strings.graphql.tabHeaders },
        { id: "pre-script", label: strings.graphql.tabPreScript },
        { id: "post-script", label: strings.graphql.tabPostScript },
        { id: "schema", label: strings.graphql.tabSchema },
    ];

    const mockReqTabs: { id: ReqSubTab; label: string }[] = [
        { id: "query", label: strings.graphql.tabMatch },
        { id: "headers", label: strings.graphql.tabResponseHeaders },
    ];

    // -- Format response body -----------------------------------------------

    const formattedResBody = (() => {
        try { return JSON.stringify(JSON.parse(state.resBody), null, 2); } catch { return state.resBody; }
    })();

    // -- Render -------------------------------------------------------------

    return (
        <div className="flex flex-col h-full overflow-hidden">
            <EditorTitleBar
                label={label ?? (tabType === "request" ? "GRAPHQL" : "GRAPHQL MOCK")}
                namePlaceholder={tabType === "request" ? strings.graphql.requestNamePlaceholder : strings.graphql.mockNamePlaceholder}
                name={state.name}
                onNameChange={(v) => dispatch({ type: "SET_FIELD", field: "name", value: v })}
                onClose={onClose}
            />

            {/* URL bar - only for request mode */}
            {tabType === "request" && (
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
            )}

            {/* Mock endpoint pattern */}
            {tabType === "mock" && (
                <div className="px-4 py-2.5 border-b border-border flex-shrink-0 flex items-center gap-2">
                    <div
                        className="flex items-stretch rounded border border-border focus-within:border-signal transition-colors overflow-hidden flex-1"
                        style={{ background: "var(--c-card)" }}
                    >
                        <span className="bg-surface-2 border-r border-border text-xs font-bold font-mono px-3 py-2.5 flex-shrink-0 text-muted-foreground">
                            {strings.graphql.endpoint}
                        </span>
                        <input
                            className="flex-1 bg-transparent px-3 py-2.5 text-sm font-mono text-foreground outline-none placeholder:text-muted-foreground min-w-0"
                            placeholder="/graphql or regex pattern…"
                            value={state.endpointPattern}
                            onChange={(e) => dispatch({ type: "SET_FIELD", field: "endpointPattern", value: e.target.value })}
                        />
                    </div>
                    <label className="flex items-center gap-1.5 text-xs text-muted-foreground cursor-pointer select-none">
                        <input
                            type="checkbox"
                            checked={state.useRegex}
                            onChange={(e) => dispatch({ type: "SET_FIELD", field: "useRegex", value: e.target.checked })}
                            className="accent-signal"
                        />
                        {strings.graphql.regex}
                    </label>
                </div>
            )}

            {/* Split panes */}
            <PanelGroup orientation="horizontal" className="flex-1 overflow-hidden">
                {/* Left pane */}
                <Panel defaultSize={50} minSize={25}>
                    <div className="flex flex-col h-full overflow-hidden">
                        <TabStrip
                            tabs={tabType === "request" ? reqTabs : mockReqTabs}
                            active={reqSubTab}
                            onChange={(t) => setReqSubTab(t as ReqSubTab)}
                        />
                        <div className="flex-1 overflow-hidden">
                            {reqSubTab === "query" && tabType === "request" && (
                                <CodeEditor
                                    value={state.query}
                                    onChange={(v) => dispatch({ type: "SET_FIELD", field: "query", value: v })}
                                    language="text"
                                    placeholder="query { ... }"
                                    className="h-full"
                                />
                            )}
                            {reqSubTab === "query" && tabType === "mock" && (
                                <div className="flex flex-col gap-3 p-4 overflow-y-auto h-full">
                                    <div className="flex flex-col gap-1.5">
                                        <label className="text-[10px] font-semibold text-muted-foreground uppercase tracking-wider">{strings.graphql.operationType}</label>
                                        <select
                                            value={state.operationType}
                                            onChange={(e) => dispatch({ type: "SET_FIELD", field: "operationType", value: e.target.value })}
                                            className="bg-card border border-border rounded px-3 py-2 text-sm text-foreground outline-none focus:border-signal"
                                        >
                                            <option value="any">{strings.graphql.opAny}</option>
                                            <option value="query">{strings.graphql.opQuery}</option>
                                            <option value="mutation">{strings.graphql.opMutation}</option>
                                            <option value="subscription">{strings.graphql.opSubscription}</option>
                                        </select>
                                    </div>
                                    <div className="flex flex-col gap-1.5">
                                        <label className="text-[10px] font-semibold text-muted-foreground uppercase tracking-wider">{strings.graphql.operationName}</label>
                                        <input
                                            className="bg-card border border-border rounded px-3 py-2 text-sm text-foreground outline-none focus:border-signal placeholder:text-muted-foreground"
                                            placeholder={strings.graphql.operationNamePlaceholder}
                                            value={state.operationNameMatch}
                                            onChange={(e) => dispatch({ type: "SET_FIELD", field: "operationNameMatch", value: e.target.value })}
                                        />
                                    </div>
                                    <div className="flex flex-col gap-1.5">
                                        <label className="text-[10px] font-semibold text-muted-foreground uppercase tracking-wider">{strings.graphql.responseStatus}</label>
                                        <input
                                            type="number"
                                            className="bg-card border border-border rounded px-3 py-2 text-sm text-foreground outline-none focus:border-signal w-24"
                                            value={state.responseStatus}
                                            onChange={(e) => dispatch({ type: "SET_FIELD", field: "responseStatus", value: Number(e.target.value) })}
                                        />
                                    </div>
                                    <div className="flex flex-col gap-1.5">
                                        <label className="text-[10px] font-semibold text-muted-foreground uppercase tracking-wider">{strings.graphql.responseDelay}</label>
                                        <input
                                            type="number"
                                            className="bg-card border border-border rounded px-3 py-2 text-sm text-foreground outline-none focus:border-signal w-24"
                                            value={state.responseDelay}
                                            onChange={(e) => dispatch({ type: "SET_FIELD", field: "responseDelay", value: Number(e.target.value) })}
                                        />
                                    </div>
                                </div>
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
                            {reqSubTab === "headers" && tabType === "request" && (
                                <HeaderTable rows={headerRows} onChange={handleHeadersChange} />
                            )}
                            {reqSubTab === "headers" && tabType === "mock" && (
                                <HeaderTable rows={resHeaderRows} onChange={handleResHeadersChange} />
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
                        {tabType === "request" && (
                            <>
                                {/* Response header bar */}
                                <div className="flex items-center gap-3 px-4 py-2 border-b border-border flex-shrink-0 bg-background/40">
                                    <span className="text-[10px] font-semibold text-muted-foreground uppercase tracking-wider">{strings.graphql.response}</span>
                                    {state.resStatus !== null && (
                                        <>
                                            <span className={`text-xs font-mono font-bold ${state.resStatus < 300 ? "text-signal" : state.resStatus < 400 ? "text-amber" : "text-destructive"}`}>
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
                            </>
                        )}
                        {tabType === "mock" && (
                            <>
                                <div className="flex items-center gap-3 px-4 py-2 border-b border-border flex-shrink-0 bg-background/40">
                                    <span className="text-[10px] font-semibold text-muted-foreground uppercase tracking-wider">{strings.graphql.responseBody}</span>
                                </div>
                                <div className="flex-1 overflow-hidden">
                                    <CodeEditor
                                        value={state.responseBody}
                                        onChange={(v) => dispatch({ type: "SET_FIELD", field: "responseBody", value: v })}
                                        language="json"
                                        placeholder='{"data": { ... }}'
                                        className="h-full"
                                    />
                                </div>
                            </>
                        )}
                    </div>
                </Panel>
            </PanelGroup>

            {/* Bottom bar */}
            <BottomBar
                folders={folders}
                folderId={state.folderId}
                onFolderChange={(id) => dispatch({ type: "SET_FIELD", field: "folderId", value: id })}
                onCancel={onClose}
                onSave={handleSave}
                saveLabel={draftTabId ? strings.common.save : strings.graphql.update}
                saveDisabled={tabType === "request" ? !state.name && !state.endpointUrl : !state.name}
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
            />
        </div>
    );
});

export default GraphQLTab;
