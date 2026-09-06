import React, { forwardRef, useImperativeHandle, useReducer, useCallback, useEffect, useState } from "react";
import { Group as PanelGroup, Panel, Separator as PanelResizeHandle } from "react-resizable-panels";
import { SavedGrpcRequest, SavedGrpcMock, Folder, Environment } from "@/types";
import CodeEditor from "@/components/common/CodeEditor";
import HeaderTable from "@/components/editor/HeaderTable";
import EditorTitleBar from "@/components/editor/EditorTitleBar";
import { TabStrip, BottomBar } from "@/components/editor/RequestTab";
import { useDraftPersist, loadDraft } from "@/lib/useDraftPersist";
import { KVRow, mkRowId } from "@/lib/utils";
import { resolveVars } from "@/lib/resolveVars";
import { cn } from "@/components/ui/cn";
import { strings } from "@/lib/strings";
import {
    GrpcTabState, GrpcTabType, GrpcAction,
    grpcTabReducer, initGrpcRequestState, initGrpcMockState,
    stateToRequestDraft, stateToMockDraft, requestToSaveData, mockToSaveData,
    GrpcRequestDraft, GrpcMockDraft,
} from "@/components/grpc/grpcTabReducer";
import ProtoExplorer from "@/components/grpc/ProtoExplorer";

// -- Props ------------------------------------------------------------------

export interface GrpcTabHandle {
    save(): Promise<any> | void;
    refresh?(entity: SavedGrpcRequest | SavedGrpcMock): void;
}

interface Props {
    tabType: GrpcTabType;
    tabId: string;
    draftTabId: string | null;
    initial: SavedGrpcRequest | SavedGrpcMock | null;
    folders: Folder[];
    activeEnv?: Environment | null;
    onSave: (data: Omit<SavedGrpcRequest, "id" | "createdAt" | "workspaceId"> | Omit<SavedGrpcMock, "id" | "createdAt" | "workspaceId">) => Promise<any>;
    onClose: () => void;
    onSync?: (savedId?: string) => Promise<void>;
    onRevert?: () => Promise<void>;
    syncStatus?: "clean" | "modified" | "new" | "deleted";
    onHistory?: () => void;
}

// -- Helpers ----------------------------------------------------------------

function metadataToRows(meta: Record<string, string>): KVRow[] {
    const entries = Object.entries(meta);
    if (entries.length === 0) return [{ id: mkRowId(), enabled: true, key: "", value: "" }];
    return entries.map(([key, value]) => ({ id: mkRowId(), enabled: true, key, value }));
}

function rowsToMetadata(rows: KVRow[]): Record<string, string> {
    const result: Record<string, string> = {};
    for (const r of rows) {
        if (r.enabled && r.key.trim()) result[r.key] = r.value;
    }
    return result;
}

const STREAMING_BADGES: Record<string, string> = {
    unary: strings.grpc.streamUnary,
    server: strings.grpc.streamServer,
    client: strings.grpc.streamClient,
    bidi: strings.grpc.streamBidirectional,
};

// -- Component --------------------------------------------------------------

const GrpcTab = forwardRef<GrpcTabHandle, Props>(function GrpcTab(
    { tabType, tabId, draftTabId, initial, folders, activeEnv = null, onSave, onClose, onSync, onRevert, syncStatus, onHistory }: Props,
    ref,
) {
    const isNew = !!draftTabId;

    // Initialize state from draft or saved entity
    const initState = useCallback((): GrpcTabState => {
        if (isNew && draftTabId) {
            const draft = loadDraft<GrpcRequestDraft | GrpcMockDraft>(draftTabId);
            if (draft) {
                if (tabType === "request") {
                    const d = draft as GrpcRequestDraft;
                    return { ...initGrpcRequestState(), ...d };
                } else {
                    const d = draft as GrpcMockDraft;
                    return { ...initGrpcMockState(), ...d };
                }
            }
        }
        if (tabType === "request") return initGrpcRequestState(initial as SavedGrpcRequest | null);
        return initGrpcMockState(initial as SavedGrpcMock | null);
    }, []);

    const [state, dispatch] = useReducer(grpcTabReducer, undefined, initState);

    // Sub-tab state
    type ReqSubTab = "message" | "metadata" | "pre-script" | "post-script" | "proto";
    type ResSubTab = "response" | "res-metadata";
    type MockSubTab = "response" | "metadata" | "settings" | "proto";
    const [reqTab, setReqTab] = useState<ReqSubTab>("message");
    const [resTab, setResTab] = useState<ResSubTab>("response");
    const [mockTab, setMockTab] = useState<MockSubTab>("response");

    // KV rows for metadata editing
    const [metaRows, setMetaRows] = useState<KVRow[]>(() => metadataToRows(state.metadata));
    const [resMetaRows, setResMetaRows] = useState<KVRow[]>(() => metadataToRows(state.responseMetadata));

    // Sync metadata changes
    useEffect(() => {
        dispatch({ type: "SET_METADATA", metadata: rowsToMetadata(metaRows) });
    }, [metaRows]);

    useEffect(() => {
        dispatch({ type: "SET_RESPONSE_METADATA", metadata: rowsToMetadata(resMetaRows) });
    }, [resMetaRows]);

    // Draft persistence
    const draftData = useCallback(() => {
        if (tabType === "request") return stateToRequestDraft(state);
        return stateToMockDraft(state);
    }, [state, tabType]);

    const isEmptyDraft = useCallback(() => {
        if (tabType === "request") return !state.serverAddress && !state.serviceName && !state.methodName;
        return !state.serviceName && !state.methodName;
    }, [state, tabType]);

    useDraftPersist(draftTabId, draftData, isEmptyDraft);

    // Send gRPC request
    const handleSend = useCallback(async () => {
        dispatch({ type: "SEND_START" });
        try {
            const addr = resolveVars(state.serverAddress, activeEnv);
            const body = resolveVars(state.requestBody, activeEnv);
            const meta: Record<string, string> = {};
            for (const [k, v] of Object.entries(state.metadata)) { meta[k] = resolveVars(v, activeEnv); }
            const result = await window.api.grpcExecute(
                addr,
                state.serviceName,
                state.methodName,
                body,
                meta,
                state.protoFileId,
                state.useReflection,
            );
            if (result.ok) {
                dispatch({
                    type: "SEND_SUCCESS",
                    responses: result.responses ?? [],
                    metadata: result.metadata ?? {},
                    status: result.status ?? 0,
                    statusMessage: result.statusMessage ?? "OK",
                    durationMs: result.durationMs ?? 0,
                });
            } else {
                dispatch({ type: "SEND_ERROR", error: result.error ?? strings.grpc.unknownError });
            }
        } catch (err: any) {
            dispatch({ type: "SEND_ERROR", error: err?.message ?? strings.grpc.sendFailed });
        }
    }, [state.serverAddress, state.serviceName, state.methodName, state.requestBody, state.metadata, state.protoFileId, state.useReflection, activeEnv]);

    // Save
    const handleSave = useCallback(async () => {
        dispatch({ type: "SAVE_START" });
        try {
            let res: any;
            if (tabType === "request") {
                res = await onSave(requestToSaveData(state));
            } else {
                res = await onSave(mockToSaveData(state));
            }
            dispatch({ type: "SAVE_DONE" });
            return res;
        } catch (err) {
            dispatch({ type: "SAVE_DONE" });
            throw err;
        }
    }, [state, tabType, onSave]);

    useImperativeHandle(ref, () => ({
        save() {
            return handleSave();
        },
        refresh(entity: SavedGrpcRequest | SavedGrpcMock) {
            if (tabType === "request") {
                const freshState = initGrpcRequestState(entity as SavedGrpcRequest);
                dispatch({ type: "LOAD", state: freshState });
                setMetaRows(metadataToRows(freshState.metadata));
                setResMetaRows(metadataToRows(freshState.responseMetadata));
            } else {
                const freshState = initGrpcMockState(entity as SavedGrpcMock);
                dispatch({ type: "LOAD", state: freshState });
                setMetaRows(metadataToRows(freshState.metadata));
                setResMetaRows(metadataToRows(freshState.responseMetadata));
            }
        },
    }), [handleSave, tabType]);

    // Sync and Revert handling
    const [syncing, setSyncing] = useState(false);
    const [reverting, setReverting] = useState(false);

    const handleSyncClick = useCallback(async () => {
        if (!onSync || syncing) return;
        setSyncing(true);
        try {
            let targetId = tabId;
            if (state.dirty || isNew) {
                const saved: any = await handleSave();
                if (saved && typeof saved === "object" && saved.id) {
                    targetId = saved.id;
                }
            }
            await onSync(targetId);
        } finally {
            setSyncing(false);
        }
    }, [onSync, syncing, state.dirty, isNew, handleSave, tabId]);

    const hasChanges = !isNew && Boolean(state.dirty || (syncStatus && syncStatus !== "clean"));

    const handleRevertClick = useCallback(async () => {
        if (!onRevert || reverting) return;
        setReverting(true);
        try {
            await onRevert();
        } finally {
            setReverting(false);
        }
    }, [onRevert, reverting]);

    const canSave = Boolean(state.serviceName && state.methodName);
    const syncDisabled = !hasChanges || (!canSave && state.dirty) || syncing;
    const revertDisabled = !hasChanges || reverting;
    const syncTitle = !hasChanges ? strings.common.noChangesToSync : strings.common.syncTooltip;
    const revertTitle = !hasChanges ? strings.common.noChangesToRevert : strings.common.revertTooltip;

    const set = (field: keyof GrpcTabState) => (value: unknown) => dispatch({ type: "SET_FIELD", field, value });

    // -- Render -------------------------------------------------------------

    const reqSubTabs: { id: ReqSubTab; label: string }[] = tabType === "request"
        ? [{ id: "message", label: strings.grpc.tabMessage }, { id: "metadata", label: strings.grpc.tabMetadata }, { id: "pre-script", label: strings.grpc.tabPreScript }, { id: "post-script", label: strings.grpc.tabPostScript }, { id: "proto", label: strings.grpc.tabProto }]
        : [{ id: "message", label: strings.grpc.tabMessage }, { id: "metadata", label: strings.grpc.tabMetadata }, { id: "proto", label: strings.grpc.tabProto }];

    const resSubTabs: { id: ResSubTab; label: string }[] = [
        { id: "response", label: strings.grpc.tabResponse },
        { id: "res-metadata", label: strings.grpc.tabTrailingMetadata },
    ];

    const mockSubTabs: { id: MockSubTab; label: string }[] = [
        { id: "response", label: strings.grpc.tabResponseBody },
        { id: "metadata", label: strings.grpc.tabResponseMetadata },
        { id: "settings", label: strings.grpc.tabSettings },
        { id: "proto", label: strings.grpc.tabProto },
    ];

    return (
        <div className="flex flex-col h-full overflow-hidden">
            {/* Title bar */}
            <EditorTitleBar
                label={tabType === "request" ? "GRPC" : "GRPC MOCK"}
                namePlaceholder={tabType === "request" ? strings.grpc.requestNamePlaceholder : strings.grpc.mockNamePlaceholder}
                name={state.name}
                onNameChange={(v) => set("name")(v)}
                onClose={onClose}
                autoFocus={isNew}
            />

            {/* Connection bar */}
            <div className="flex items-center gap-2 px-4 py-2 border-b border-border flex-shrink-0 bg-background/30">
                {tabType === "request" && (
                    <input
                        className="flex-1 bg-card border border-border focus:border-signal rounded px-3 py-1.5 text-sm text-foreground outline-none font-mono placeholder:text-muted-foreground"
                        placeholder="localhost:50051"
                        value={state.serverAddress}
                        onChange={(e) => set("serverAddress")(e.target.value)}
                    />
                )}
                <input
                    className={cn(
                        "bg-card border border-border focus:border-signal rounded px-3 py-1.5 text-sm text-foreground outline-none font-mono placeholder:text-muted-foreground",
                        tabType === "request" ? "w-48" : "flex-1"
                    )}
                    placeholder="ServiceName"
                    value={state.serviceName}
                    onChange={(e) => set("serviceName")(e.target.value)}
                />
                <input
                    className={cn(
                        "bg-card border border-border focus:border-signal rounded px-3 py-1.5 text-sm text-foreground outline-none font-mono placeholder:text-muted-foreground",
                        tabType === "request" ? "w-48" : "flex-1"
                    )}
                    placeholder="MethodName"
                    value={state.methodName}
                    onChange={(e) => set("methodName")(e.target.value)}
                />
                <span className="text-[10px] font-semibold px-2 py-1 rounded bg-surface-2 text-muted-foreground whitespace-nowrap">
                    {STREAMING_BADGES[state.streamingType] ?? strings.grpc.streamUnary}
                </span>
                {tabType === "request" && (
                    <button
                        onClick={handleSend}
                        disabled={state.sending || !state.serverAddress || !state.serviceName || !state.methodName}
                        className="px-4 py-1.5 rounded bg-signal hover:bg-signal/80 disabled:opacity-40 disabled:cursor-not-allowed text-background text-xs font-semibold transition-all cursor-pointer flex items-center gap-1.5"
                    >
                        {state.sending ? (
                            <span className="inline-block w-3 h-3 border-2 border-white/30 border-t-white rounded-full animate-spin" />
                        ) : strings.server.send}
                    </button>
                )}
            </div>

            {/* Streaming type + reflection toggle */}
            <div className="flex items-center gap-3 px-4 py-1.5 border-b border-border flex-shrink-0">
                <label className="flex items-center gap-2 text-xs text-muted-foreground">
                    <span>{strings.grpc.type}</span>
                    <select
                        className="bg-card border border-border rounded px-2 py-1 text-xs text-foreground outline-none"
                        value={state.streamingType}
                        onChange={(e) => set("streamingType")(e.target.value)}
                    >
                        <option value="unary">{strings.grpc.streamUnary}</option>
                        <option value="server">{strings.grpc.streamServerStreaming}</option>
                        <option value="client">{strings.grpc.streamClientStreaming}</option>
                        <option value="bidi">{strings.grpc.streamBidirectional}</option>
                    </select>
                </label>
                {tabType === "request" && (
                    <label className="flex items-center gap-1.5 text-xs text-muted-foreground cursor-pointer">
                        <input
                            type="checkbox"
                            checked={state.useReflection}
                            onChange={(e) => set("useReflection")(e.target.checked)}
                            className="accent-signal"
                        />
                        {strings.grpc.useReflection}
                    </label>
                )}
                {tabType === "mock" && (
                    <label className="flex items-center gap-1.5 text-xs text-muted-foreground cursor-pointer">
                        <input
                            type="checkbox"
                            checked={state.enabled}
                            onChange={(e) => set("enabled")(e.target.checked)}
                            className="accent-signal"
                        />
                        {strings.grpc.enabled}
                    </label>
                )}
            </div>

            {/* Split pane */}
            <div className="flex-1 min-h-0 overflow-hidden">
                <PanelGroup orientation="horizontal" className="h-full">
                    {/* Left panel: request body / metadata / scripts */}
                    <Panel defaultSize={tabType === "request" ? 50 : 100} minSize={30}>
                        <div className="flex flex-col h-full overflow-hidden">
                            {tabType === "request" ? (
                                <>
                                    <TabStrip tabs={reqSubTabs} active={reqTab} onChange={(t) => setReqTab(t as ReqSubTab)} />
                                    <div className="flex-1 overflow-hidden">
                                        {reqTab === "message" && (
                                            <CodeEditor value={state.requestBody} onChange={(v) => set("requestBody")(v)} language="json" placeholder='{"key": "value"}' className="h-full" />
                                        )}
                                        {reqTab === "metadata" && (
                                            <HeaderTable rows={metaRows} onChange={setMetaRows} emptyMessage={strings.grpc.noMetadata} />
                                        )}
                                        {reqTab === "pre-script" && (
                                            <CodeEditor value={state.preScript} onChange={(v) => set("preScript")(v)} language="javascript" placeholder="// Pre-request script" className="h-full" />
                                        )}
                                        {reqTab === "post-script" && (
                                            <CodeEditor value={state.postScript} onChange={(v) => set("postScript")(v)} language="javascript" placeholder="// Post-response script" className="h-full" />
                                        )}
                                        {reqTab === "proto" && (
                                            <ProtoExplorer
                                                protoFileId={state.protoFileId}
                                                onSelectMethod={(serviceName, methodName, streamingType, skeleton) => {
                                                    dispatch({ type: "SET_FIELD", field: "serviceName", value: serviceName });
                                                    dispatch({ type: "SET_FIELD", field: "methodName", value: methodName });
                                                    dispatch({ type: "SET_FIELD", field: "streamingType", value: streamingType });
                                                    if (skeleton && skeleton !== "{}") dispatch({ type: "SET_FIELD", field: "requestBody", value: skeleton });
                                                }}
                                                onProtoChange={(id) => dispatch({ type: "SET_FIELD", field: "protoFileId", value: id })}
                                            />
                                        )}
                                    </div>
                                </>
                            ) : (
                                <>
                                    <TabStrip tabs={mockSubTabs} active={mockTab} onChange={(t) => setMockTab(t as MockSubTab)} />
                                    <div className="flex-1 overflow-hidden">
                                        {mockTab === "response" && (
                                            <CodeEditor value={state.responseBody} onChange={(v) => set("responseBody")(v)} language="json" placeholder='{"result": "mocked"}' className="h-full" />
                                        )}
                                        {mockTab === "metadata" && (
                                            <HeaderTable rows={resMetaRows} onChange={setResMetaRows} emptyMessage={strings.grpc.noResponseMetadata} />
                                        )}
                                        {mockTab === "settings" && (
                                            <div className="p-4 space-y-4 overflow-y-auto">
                                                <div className="space-y-1">
                                                    <label className="text-xs font-medium text-muted-foreground">{strings.grpc.responseDelay}</label>
                                                    <input
                                                        type="number"
                                                        min={0}
                                                        className="w-32 bg-card border border-border focus:border-signal rounded px-3 py-1.5 text-sm text-foreground outline-none"
                                                        value={state.responseDelay}
                                                        onChange={(e) => set("responseDelay")(Number(e.target.value))}
                                                    />
                                                </div>
                                                <div className="space-y-1">
                                                    <label className="text-xs font-medium text-muted-foreground">{strings.grpc.errorCode}</label>
                                                    <input
                                                        type="number"
                                                        min={0}
                                                        max={16}
                                                        className="w-32 bg-card border border-border focus:border-signal rounded px-3 py-1.5 text-sm text-foreground outline-none"
                                                        value={state.errorCode}
                                                        onChange={(e) => set("errorCode")(Number(e.target.value))}
                                                    />
                                                </div>
                                                <div className="space-y-1">
                                                    <label className="text-xs font-medium text-muted-foreground">{strings.grpc.errorMessage}</label>
                                                    <input
                                                        className="w-full bg-card border border-border focus:border-signal rounded px-3 py-1.5 text-sm text-foreground outline-none placeholder:text-muted-foreground"
                                                        placeholder={strings.grpc.errorMessagePlaceholder}
                                                        value={state.errorMessage}
                                                        onChange={(e) => set("errorMessage")(e.target.value)}
                                                    />
                                                </div>
                                            </div>
                                        )}
                                        {mockTab === "proto" && (
                                            <ProtoExplorer
                                                protoFileId={state.protoFileId}
                                                onSelectMethod={(serviceName, methodName, streamingType, skeleton) => {
                                                    dispatch({ type: "SET_FIELD", field: "serviceName", value: serviceName });
                                                    dispatch({ type: "SET_FIELD", field: "methodName", value: methodName });
                                                    dispatch({ type: "SET_FIELD", field: "streamingType", value: streamingType });
                                                    if (skeleton && skeleton !== "{}") dispatch({ type: "SET_FIELD", field: "responseBody", value: skeleton });
                                                }}
                                                onProtoChange={(id) => dispatch({ type: "SET_FIELD", field: "protoFileId", value: id })}
                                            />
                                        )}
                                    </div>
                                </>
                            )}
                        </div>
                    </Panel>

                    {/* Only show right panel for request mode */}
                    {tabType === "request" && (
                        <>
                            <PanelResizeHandle className="w-px bg-border hover:bg-signal/50 transition-colors cursor-col-resize" />
                            <Panel defaultSize={50} minSize={25}>
                                <div className="flex flex-col h-full overflow-hidden">
                                    {/* Status bar */}
                                    {state.resStatus !== null && (
                                        <div className="flex items-center gap-3 px-4 py-2 border-b border-border bg-background/30 flex-shrink-0">
                                            <span className={cn(
                                                "text-xs font-semibold",
                                                state.resStatus === 0 ? "text-signal" : "text-destructive"
                                            )}>
                                                {strings.grpc.status} {state.resStatus}
                                            </span>
                                            {state.resStatusMessage && (
                                                <span className="text-xs text-muted-foreground">{state.resStatusMessage}</span>
                                            )}
                                            {state.resDuration !== null && (
                                                <span className="text-xs text-muted-foreground ml-auto">{state.resDuration}ms</span>
                                            )}
                                        </div>
                                    )}
                                    {state.resError && (
                                        <div className="px-4 py-2 border-b border-border bg-destructive/5 flex-shrink-0">
                                            <p className="text-xs text-destructive">{state.resError}</p>
                                        </div>
                                    )}

                                    <TabStrip tabs={resSubTabs} active={resTab} onChange={(t) => setResTab(t as ResSubTab)} />
                                    <div className="flex-1 overflow-hidden">
                                        {resTab === "response" && (
                                            state.responses.length === 0 ? (
                                                <div className="flex items-center justify-center h-full text-xs text-muted-foreground">
                                                    {state.sending ? strings.grpc.sending : strings.grpc.noResponseYet}
                                                </div>
                                            ) : state.responses.length === 1 ? (
                                                <CodeEditor value={state.responses[0]} language="json" readOnly className="h-full" />
                                            ) : (
                                                <div className="flex flex-col h-full overflow-y-auto p-2 gap-1">
                                                    {state.responses.map((r, i) => (
                                                        <div key={i} className="border border-border rounded p-2">
                                                            <div className="text-[10px] text-muted-foreground font-semibold mb-1">{strings.grpc.responseNumber.replace("{n}", String(i + 1))}</div>
                                                            <pre className="text-xs text-foreground font-mono whitespace-pre-wrap break-all">{r}</pre>
                                                        </div>
                                                    ))}
                                                </div>
                                            )
                                        )}
                                        {resTab === "res-metadata" && (
                                            <div className="p-4 overflow-y-auto">
                                                {Object.keys(state.resMetadata).length === 0 ? (
                                                    <p className="text-xs text-muted-foreground italic">{strings.grpc.noTrailingMetadata}</p>
                                                ) : (
                                                    <table className="w-full text-xs">
                                                        <tbody>
                                                            {Object.entries(state.resMetadata).map(([k, v]) => (
                                                                <tr key={k} className="border-b border-border/30">
                                                                    <td className="py-1.5 pr-4 text-signal font-mono">{k}</td>
                                                                    <td className="py-1.5 text-foreground font-mono">{v}</td>
                                                                </tr>
                                                            ))}
                                                        </tbody>
                                                    </table>
                                                )}
                                            </div>
                                        )}
                                    </div>
                                </div>
                            </Panel>
                        </>
                    )}
                </PanelGroup>
            </div>

            {/* Bottom bar */}
            <BottomBar
                folders={folders}
                folderId={state.folderId}
                onFolderChange={(id) => set("folderId")(id)}
                onCancel={onClose}
                onSave={handleSave}
                saveLabel={isNew ? strings.common.save : strings.grpc.update}
                saveDisabled={tabType === "request" ? (!state.serviceName || !state.methodName) : (!state.serviceName || !state.methodName)}
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

export default GrpcTab;
