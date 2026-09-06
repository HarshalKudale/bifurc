import React, { forwardRef, useImperativeHandle, useReducer, useCallback, useEffect, useState } from "react";
import { Group as PanelGroup, Panel, Separator as PanelResizeHandle } from "react-resizable-panels";
import { SavedGrpcRequest, SavedGrpcMock, Folder, Environment } from "@/types";
import CodeEditor from "@/components/common/CodeEditor";
import HeaderTable from "@/components/common/HeaderTable";
import EditorTitleBar from "@/components/editor/EditorTitleBar";
import { TabStrip, BottomBar } from "@/components/editor/RequestTab";
import { useDraftPersist, loadDraft } from "@/hooks/useDraftPersist";
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
import { useProtocolEditor } from "@/hooks/useProtocolEditor";
import { GrpcLeftPane, ReqSubTab, MockSubTab } from "./GrpcLeftPane";
import { GrpcResponsePane, ResSubTab } from "./GrpcResponsePane";

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
        initial,
        reducer: grpcTabReducer as any,
        initState: (init, draft, type) => {
            if (draft) {
                if (type === "request") return { ...initGrpcRequestState(), ...(draft as GrpcRequestDraft) };
                return { ...initGrpcMockState(), ...(draft as GrpcMockDraft) };
            }
            if (type === "request") return initGrpcRequestState(init as SavedGrpcRequest);
            return initGrpcMockState(init as SavedGrpcMock);
        },
        stateToDraft: (s, t) => t === "request" ? stateToRequestDraft(s) : stateToMockDraft(s),
        isDraftEmpty: (s, t) => t === "request" ? (!s.serverAddress && !s.serviceName && !s.methodName) : (!s.serviceName && !s.methodName),
        stateToSavePayload: (s, t) => t === "request" ? requestToSaveData(s) : mockToSaveData(s),
        onSave: onSave as any,
        onSync,
        onRevert,
        syncStatus,
    });

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

    useImperativeHandle(ref, () => ({
        save() {
            return handleSave();
        },
        refresh(entity: SavedGrpcRequest | SavedGrpcMock) {
            handleRefresh(entity);
            if (tabType === "request") {
                const freshState = initGrpcRequestState(entity as SavedGrpcRequest);
                setMetaRows(metadataToRows(freshState.metadata));
                setResMetaRows(metadataToRows(freshState.responseMetadata));
            } else {
                const freshState = initGrpcMockState(entity as SavedGrpcMock);
                setMetaRows(metadataToRows(freshState.metadata));
                setResMetaRows(metadataToRows(freshState.responseMetadata));
            }
        },
    }), [handleSave, handleRefresh, tabType]);

    const canSave = Boolean(state.serviceName && state.methodName);
    const syncDisabled = !hasLocalChanges || (!canSave && state.dirty) || syncing;
    const revertDisabled = !hasLocalChanges || reverting;
    const syncTitle = !hasLocalChanges ? strings.common.noChangesToSync : strings.common.syncTooltip;
    const revertTitle = !hasLocalChanges ? strings.common.noChangesToRevert : strings.common.revertTooltip;

    const set = (field: keyof GrpcTabState) => (value: unknown) => dispatch({ type: "SET_FIELD", field, value });

    // -- Render -------------------------------------------------------------

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
                        <GrpcLeftPane
                            tabType={tabType}
                            state={state}
                            dispatch={dispatch}
                            set={set}
                            reqTab={reqTab}
                            setReqTab={setReqTab}
                            mockTab={mockTab}
                            setMockTab={setMockTab}
                            metaRows={metaRows}
                            setMetaRows={setMetaRows}
                            resMetaRows={resMetaRows}
                            setResMetaRows={setResMetaRows}
                        />
                    </Panel>

                    {/* Only show right panel for request mode */}
                    {tabType === "request" && (
                        <>
                            <PanelResizeHandle className="w-px bg-border hover:bg-signal/50 transition-colors cursor-col-resize" />
                            <Panel defaultSize={50} minSize={25}>
                                <GrpcResponsePane
                                    state={state}
                                    resTab={resTab}
                                    onResTabChange={setResTab}
                                />
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
