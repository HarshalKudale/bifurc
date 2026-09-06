import React, {
    forwardRef, useImperativeHandle, useReducer, useCallback, useEffect, useState,
} from "react";
import { SavedSoapRequest, SavedSoapMock, Folder, Environment } from "@/types";
import EditorTitleBar from "@/components/editor/EditorTitleBar";
import { BottomBar, TabStrip } from "@/components/editor/RequestTab";
import CodeEditor from "@/components/common/CodeEditor";
import HeaderTable from "@/components/editor/HeaderTable";
import {
    soapTabReducer, initSoapState, soapStateToSavePayload, soapStateToDraft, isSoapDraftEmpty,
    SoapTabType, SoapTabState, SoapRequestDraft, SoapMockDraft,
} from "@/components/soap/soapTabReducer";
import { useDraftPersist, loadDraft } from "@/lib/useDraftPersist";
import { Group as PanelGroup, Panel, Separator as PanelResizeHandle } from "react-resizable-panels";
import { KVRow, mkRowId, headersToRows, rowsToHeaders, statusColor } from "@/lib/utils";
import { resolveVars, resolveHeaders } from "@/lib/resolveVars";
import { cn } from "@/components/ui/cn";
import WsdlExplorer from "@/components/soap/WsdlExplorer";
import { strings } from "@/lib/strings";

// -- Public handle for imperative refresh -----------------------------------

export interface SoapTabHandle {
    refresh(entity: SavedSoapRequest | SavedSoapMock): void;
    save(): void;
}

// -- Props ------------------------------------------------------------------

export interface SoapTabProps {
    tabType: SoapTabType;
    tabId: string;
    draftTabId?: string | null;
    initial?: SavedSoapRequest | SavedSoapMock | Partial<SavedSoapRequest> | Partial<SavedSoapMock> | null;
    folders?: Folder[];
    activeEnv?: Environment | null;
    onSave(data: Omit<SavedSoapRequest, "id" | "createdAt" | "workspaceId"> | Omit<SavedSoapMock, "id" | "createdAt" | "workspaceId">): Promise<void>;
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
    const draft = draftTabId
        ? (tabType === "request"
            ? loadDraft<SoapRequestDraft>(draftTabId)
            : loadDraft<SoapMockDraft>(draftTabId))
        : null;

    const [state, dispatch] = useReducer(
        soapTabReducer,
        undefined,
        () => initSoapState(initial ?? null, draft, tabType),
    );

    const [initialSnapshot, setInitialSnapshot] = useState(() => JSON.stringify(soapStateToDraft(initSoapState(initial ?? null, draft, tabType), tabType)));
    const isDirty = JSON.stringify(soapStateToDraft(state, tabType)) !== initialSnapshot;
    useEffect(() => { onDirtyChange?.(isDirty); }, [isDirty, onDirtyChange]);

    // Draft auto-save
    const { markSaved } = useDraftPersist(
        draftTabId ?? null,
        () => soapStateToDraft(state, tabType),
        () => isSoapDraftEmpty(state, tabType),
    );

    // -- Header rows helper -----------------------------------------------

    const headerRows: KVRow[] = headersToRows(
        tabType === "request" ? state.headers : state.responseHeaders,
    );
    const setHeaderRows = useCallback((rows: KVRow[]) => {
        const field = tabType === "request" ? "headers" : "responseHeaders";
        dispatch({ type: "SET_FIELD", field, value: rowsToHeaders(rows) });
    }, [tabType]);

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
    }, [state.endpointUrl, state.soapAction, state.headers, state.body, activeEnv]);

    // -- Save -------------------------------------------------------------

    const handleSave = useCallback(async () => {
        dispatch({ type: "SAVE_START" });
        try {
            const payload = soapStateToSavePayload(state, tabType);
            const res = await onSave(payload);
            dispatch({ type: "SAVE_SUCCESS" });
            markSaved();
            setInitialSnapshot(JSON.stringify(soapStateToDraft(state, tabType)));
            return res;
        } catch {
            dispatch({ type: "SAVE_ERROR" });
        }
    }, [state, tabType, onSave, markSaved]);

    // Expose imperative refresh
    useImperativeHandle(ref, () => ({
        refresh(entity: SavedSoapRequest | SavedSoapMock) {
            dispatch({ type: "REFRESH", entity, tabType });
            setInitialSnapshot(JSON.stringify(soapStateToDraft(initSoapState(entity, null, tabType), tabType)));
        },
        save() {
            return handleSave();
        },
    }), [tabType, handleSave]);

    const [syncing, setSyncing] = useState(false);
    const [reverting, setReverting] = useState(false);

    const canSave = tabType === "request" ? !(!state.name && !state.endpointUrl) : !(!state.name && !state.endpointPattern);
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

    // -- Request mode: left pane tabs -------------------------------------

    const reqTabs = tabType === "request"
        ? [
            { id: "body" as const, label: strings.soap.tabBody },
            { id: "headers" as const, label: strings.soap.tabHeaders },
            { id: "pre-script" as const, label: strings.soap.tabPreScript },
            { id: "post-script" as const, label: strings.soap.tabPostScript },
            { id: "wsdl" as const, label: strings.soap.tabWsdl },
        ]
        : [
            { id: "body" as const, label: strings.soap.tabResponseBody },
            { id: "headers" as const, label: strings.soap.tabResponseHeaders },
        ];

    const resTabs = [
        { id: "body" as const, label: strings.soap.tabBody },
        { id: "headers" as const, label: strings.soap.tabHeaders },
    ];

    // -- Render -----------------------------------------------------------

    return (
        <div className="flex flex-col h-full overflow-hidden">
            {/* Title bar */}
            <EditorTitleBar
                label={label ?? (tabType === "request" ? "SOAP" : "SOAP MOCK")}
                namePlaceholder={tabType === "request" ? strings.soap.requestNamePlaceholder : strings.soap.mockNamePlaceholder}
                name={state.name}
                onNameChange={(v) => dispatch({ type: "SET_FIELD", field: "name", value: v })}
                onClose={onClose}
            />

            {/* URL / Endpoint bar (request mode) */}
            {tabType === "request" && (
                <div className="px-4 py-2.5 border-b border-border flex-shrink-0 flex flex-col gap-2">
                    <div className="flex items-center gap-2">
                        <div
                            className="flex items-stretch rounded border border-border focus-within:border-signal transition-colors overflow-hidden flex-1"
                            style={{ background: "var(--c-card)" }}
                        >
                            <span className="bg-surface-2 border-r border-border text-xs font-bold font-mono px-3 py-2.5 text-signal flex-shrink-0">{strings.soap.methodPost}</span>
                            <input
                                className="flex-1 bg-transparent px-3 py-2.5 text-sm font-mono text-foreground outline-none placeholder:text-muted-foreground min-w-0"
                                placeholder="https://example.com/ws/service"
                                value={state.endpointUrl}
                                onChange={(e) => dispatch({ type: "SET_FIELD", field: "endpointUrl", value: e.target.value })}
                                onKeyDown={(e) => { if (e.key === "Enter") handleSend(); }}
                            />
                        </div>
                        <button
                            onClick={handleSend}
                            disabled={state.sending || !state.endpointUrl}
                            className="px-4 py-2.5 rounded bg-signal hover:bg-signal/80 disabled:opacity-40 disabled:cursor-not-allowed text-background text-xs font-semibold transition-all cursor-pointer flex-shrink-0 flex items-center gap-1.5"
                        >
                            {state.sending
                                ? <><span className="inline-block w-3 h-3 border-2 border-white/30 border-t-white rounded-full animate-spin" />{strings.soap.sending}</>
                                : strings.server.send}
                        </button>
                    </div>
                    <div className="flex items-center gap-2">
                        <label className="text-xs text-muted-foreground flex-shrink-0">{strings.soap.soapAction}</label>
                        <input
                            className="flex-1 bg-card border border-border rounded px-2.5 py-1.5 text-xs font-mono text-foreground outline-none focus:border-signal placeholder:text-muted-foreground"
                            placeholder='"http://example.com/Action"'
                            value={state.soapAction}
                            onChange={(e) => dispatch({ type: "SET_FIELD", field: "soapAction", value: e.target.value })}
                        />
                    </div>
                </div>
            )}

            {/* Mock matching config */}
            {tabType === "mock" && (
                <div className="px-4 py-2.5 border-b border-border flex-shrink-0 flex flex-col gap-2">
                    <div className="flex items-center gap-2">
                        <label className="text-xs text-muted-foreground flex-shrink-0 w-28">{strings.soap.endpointPattern}</label>
                        <input
                            className="flex-1 bg-card border border-border rounded px-2.5 py-1.5 text-xs font-mono text-foreground outline-none focus:border-signal placeholder:text-muted-foreground"
                            placeholder="/ws/service"
                            value={state.endpointPattern}
                            onChange={(e) => dispatch({ type: "SET_FIELD", field: "endpointPattern", value: e.target.value })}
                        />
                        <label className="flex items-center gap-1.5 text-xs text-muted-foreground cursor-pointer">
                            <input
                                type="checkbox"
                                checked={state.useRegex}
                                onChange={(e) => dispatch({ type: "SET_FIELD", field: "useRegex", value: e.target.checked })}
                                className="accent-signal"
                            />
                            {strings.soap.regex}
                        </label>
                    </div>
                    <div className="flex items-center gap-2">
                        <label className="text-xs text-muted-foreground flex-shrink-0 w-28">{strings.soap.soapActionMatch}</label>
                        <input
                            className="flex-1 bg-card border border-border rounded px-2.5 py-1.5 text-xs font-mono text-foreground outline-none focus:border-signal placeholder:text-muted-foreground"
                            placeholder="*"
                            value={state.soapActionPattern}
                            onChange={(e) => dispatch({ type: "SET_FIELD", field: "soapActionPattern", value: e.target.value })}
                        />
                    </div>
                    <div className="flex items-center gap-2">
                        <label className="text-xs text-muted-foreground flex-shrink-0 w-28">{strings.soap.responseStatus}</label>
                        <input
                            type="number"
                            className="w-20 bg-card border border-border rounded px-2.5 py-1.5 text-xs font-mono text-foreground outline-none focus:border-signal"
                            value={state.responseStatus}
                            onChange={(e) => dispatch({ type: "SET_FIELD", field: "responseStatus", value: parseInt(e.target.value) || 200 })}
                        />
                        <label className="text-xs text-muted-foreground flex-shrink-0 ml-4">{strings.soap.delay}</label>
                        <input
                            type="number"
                            className="w-20 bg-card border border-border rounded px-2.5 py-1.5 text-xs font-mono text-foreground outline-none focus:border-signal"
                            value={state.responseDelay}
                            onChange={(e) => dispatch({ type: "SET_FIELD", field: "responseDelay", value: parseInt(e.target.value) || 0 })}
                        />
                    </div>
                </div>
            )}

            {/* Main split pane */}
            <div className="flex-1 overflow-hidden">
                {tabType === "request" ? (
                    <PanelGroup orientation="horizontal">
                        {/* Left: Request editor */}
                        <Panel defaultSize={50} minSize={25}>
                            <div className="flex flex-col h-full">
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
                        </Panel>

                        <PanelResizeHandle className="w-1.5 bg-border hover:bg-signal transition-colors cursor-col-resize" />

                        {/* Right: Response viewer */}
                        <Panel defaultSize={50} minSize={25}>
                            <div className="flex flex-col h-full">
                                <TabStrip
                                    tabs={resTabs}
                                    active={state.resTab}
                                    onChange={(t) => dispatch({ type: "SET_FIELD", field: "resTab", value: t })}
                                    prefix={
                                        state.resStatus != null ? (
                                            <div className="flex items-center gap-2 px-3 text-xs">
                                                <span className={cn("font-bold", statusColor(state.resStatus))}>{state.resStatus}</span>
                                                {state.resDuration != null && <span className="text-muted-foreground">{state.resDuration}ms</span>}
                                            </div>
                                        ) : undefined
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
                                        <div className="p-3 overflow-auto h-full">
                                            {Object.entries(state.resHeaders).map(([k, v]) => (
                                                <div key={k} className="flex gap-2 text-xs py-0.5 font-mono">
                                                    <span className="text-signal">{k}:</span>
                                                    <span className="text-foreground">{v}</span>
                                                </div>
                                            ))}
                                            {Object.keys(state.resHeaders).length === 0 && (
                                                <div className="text-muted-foreground text-xs">{strings.soap.noResponseYet}</div>
                                            )}
                                        </div>
                                    )}
                                </div>
                            </div>
                        </Panel>
                    </PanelGroup>
                ) : (
                    /* Mock mode: response editor */
                    <div className="flex flex-col h-full">
                        <TabStrip
                            tabs={reqTabs}
                            active={state.reqTab}
                            onChange={(t) => dispatch({ type: "SET_FIELD", field: "reqTab", value: t })}
                        />
                        <div className="flex-1 overflow-hidden">
                            {state.reqTab === "body" && (
                                <CodeEditor
                                    value={state.responseBody}
                                    onChange={(v) => dispatch({ type: "SET_FIELD", field: "responseBody", value: v })}
                                    language="xml"
                                    className="h-full"
                                />
                            )}
                            {state.reqTab === "headers" && (
                                <HeaderTable rows={headerRows} onChange={setHeaderRows} />
                            )}
                        </div>
                    </div>
                )}
            </div>

            {/* Bottom bar */}
            <BottomBar
                folders={folders}
                folderId={state.folderId}
                onFolderChange={(id) => dispatch({ type: "SET_FIELD", field: "folderId", value: id })}
                onCancel={onClose}
                onSave={handleSave}
                saveLabel={draftTabId ? strings.common.save : strings.soap.update}
                saveDisabled={tabType === "request" ? !state.name && !state.endpointUrl : !state.name && !state.endpointPattern}
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

export default SoapTab;
