import React, { forwardRef, useImperativeHandle, useReducer, useCallback, useEffect, useState } from "react";
import { SavedGrpcRequest, Folder, Environment } from "@/types";
import { BottomBar, TabStrip, UrlBar, UrlBarVerb } from "@/components/editor/RequestTab";
import { TabPage, TabTitleBar, TabToolbar, TabToolbarRow, SplitPane, useSplitOrientation, PROTOCOL_TONE } from "@/components/editor/TabPage";
import { useDraftPersist, loadDraft } from "@/hooks/useDraftPersist";
import { KVRow, mkRowId } from "@/lib/utils";
import { resolveVars } from "@/lib/resolveVars";
import { cn } from "@/components/ui/cn";
import { strings } from "@/lib/strings";
import {
    GrpcTabState, GrpcTabType, GrpcAction,
    grpcTabReducer, initGrpcRequestState,
    stateToRequestDraft, requestToSaveData,
    GrpcRequestDraft,
} from "@/components/grpc/grpcTabReducer";
import { useProtocolEditor } from "@/hooks/useProtocolEditor";
import { GrpcLeftPane, ReqSubTab } from "./GrpcLeftPane";
import { GrpcResponsePane, ResSubTab } from "./GrpcResponsePane";

// -- Props ------------------------------------------------------------------

export interface GrpcTabHandle {
    save(): Promise<any> | void;
    refresh?(entity: SavedGrpcRequest): void;
    /** Rename the tab's entity (used by double-click tab rename). Marks the tab dirty. */
    rename?(name: string): void;
}

interface Props {
    tabType: GrpcTabType;
    tabId: string;
    draftTabId: string | null;
    initial: SavedGrpcRequest | null;
    folders: Folder[];
    activeEnv?: Environment | null;
    onSave: (data: Omit<SavedGrpcRequest, "id" | "createdAt" | "workspaceId">) => Promise<any>;
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
        hasLocalChanges,
        autoSaving,
        autoSavedAt,
        autoSaveError,
        saveNow,
    } = useProtocolEditor({
        tabType,
        tabId,
        draftTabId,
        initial,
        reducer: grpcTabReducer as any,
        initState: (init, draft) => {
            if (draft) return { ...initGrpcRequestState(), ...(draft as GrpcRequestDraft) };
            return initGrpcRequestState(init as SavedGrpcRequest);
        },
        stateToDraft: stateToRequestDraft,
        isDraftEmpty: (s) => !s.serverAddress && !s.serviceName && !s.methodName,
        stateToSavePayload: requestToSaveData,
        onSave: onSave as any,
        onSync,
        onRevert,
        syncStatus,
    });

    // The split is shared with every other protocol tab, and persisted under one global key.
    const [splitOrientation, toggleSplitOrientation] = useSplitOrientation();

    const [reqTab, setReqTab] = useState<ReqSubTab>("message");
    const [resTab, setResTab] = useState<ResSubTab>("response");

    // KV rows for metadata editing
    const [metaRows, setMetaRows] = useState<KVRow[]>(() => metadataToRows(state.metadata));

    // Sync metadata changes
    useEffect(() => {
        dispatch({ type: "SET_METADATA", metadata: rowsToMetadata(metaRows) });
    }, [metaRows]);

    /** Field setter. Declared before the imperative handle, which reaches for `set("name")`. */
    const set = (field: keyof GrpcTabState) => (value: unknown) => dispatch({ type: "SET_FIELD", field, value });

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
            // Ctrl+S: a draft is created, an existing entity flushes its pending autosave.
            return saveNow();
        },
        refresh(entity: SavedGrpcRequest) {
            handleRefresh(entity);
            const freshState = initGrpcRequestState(entity);
            setMetaRows(metadataToRows(freshState.metadata));
        },
        rename(name: string) {
            set("name")(name);
        },
    }), [saveNow, handleRefresh]);

    const canSave = Boolean(state.serviceName && state.methodName);
    const syncDisabled = !hasLocalChanges || (!canSave && state.dirty) || syncing;
    const revertDisabled = !hasLocalChanges || reverting;
    const syncTitle = !hasLocalChanges ? strings.common.noChangesToSync : strings.common.syncTooltip;
    const revertTitle = !hasLocalChanges ? strings.common.noChangesToRevert : strings.common.revertTooltip;

    // -- Render -------------------------------------------------------------

    return (
        <TabPage>
            {/* Band 1 — title bar. */}
            <TabTitleBar
                chip="gRPC"
                tone={PROTOCOL_TONE.grpc}
                name={state.name}
                namePlaceholder={strings.grpc.requestNamePlaceholder}
                onNameChange={(v) => set("name")(v)}
                autoFocus={isNew}
            />

            {/*
                Band 2 — address band.
                The address, the service and the method are **one line**, because together they are
                "what am I calling". An earlier revision of this refactor split the service/method
                pair onto a second row in the name of consistency; that was wrong — it made the
                target harder to read and pushed the tab taller for no gain. They ride the address
                row through `UrlBar`'s `beforeAction` slot, which lands them before Send rather than
                after it.

                The streaming type and the reflection toggle stay on row 2, where this tab always had
                them. The streaming *badge* that used to sit on row 1 is gone: it repeated the value
                the select was already showing.
            */}
            <TabToolbar>
                <TabToolbarRow>
                    <UrlBar
                        bare
                        leading={<UrlBarVerb verb="gRPC" />}
                        url={state.serverAddress}
                        onUrlChange={set("serverAddress")}
                        urlPlaceholder="localhost:50051"
                        beforeAction={
                            <>
                                <input
                                    className={cn(
                                        "bg-card border border-border focus:border-signal rounded px-3 py-1.5 text-sm text-foreground outline-none font-mono placeholder:text-muted-foreground",
                                        "w-48"
                                    )}
                                    placeholder="ServiceName"
                                    value={state.serviceName}
                                    onChange={(e) => set("serviceName")(e.target.value)}
                                />
                                <input
                                    className={cn(
                                        "bg-card border border-border focus:border-signal rounded px-3 py-1.5 text-sm text-foreground outline-none font-mono placeholder:text-muted-foreground",
                                        "w-48"
                                    )}
                                    placeholder="MethodName"
                                    value={state.methodName}
                                    onChange={(e) => set("methodName")(e.target.value)}
                                />
                            </>
                        }
                        actionLabel={strings.server.send}
                        actionLoadingLabel={strings.grpc.sending}
                        actionLoading={state.sending}
                        actionDisabled={!state.serverAddress || !state.serviceName || !state.methodName}
                        onAction={handleSend}
                        onEnter={handleSend}
                        activeEnv={activeEnv}
                    />
                </TabToolbarRow>
                <TabToolbarRow>
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
                    <label className="flex items-center gap-1.5 text-xs text-muted-foreground cursor-pointer">
                        <input
                            type="checkbox"
                            checked={state.useReflection}
                            onChange={(e) => set("useReflection")(e.target.checked)}
                            className="accent-signal"
                        />
                        {strings.grpc.useReflection}
                    </label>
                </TabToolbarRow>
            </TabToolbar>

            {/* Band 3 — the split. Orientation was hard-coded `horizontal`. */}
            <SplitPane
                orientation={splitOrientation}
                minSize={25}
                left={
                    <GrpcLeftPane
                        state={state}
                        dispatch={dispatch}
                        set={set}
                        reqTab={reqTab}
                        setReqTab={setReqTab}
                        metaRows={metaRows}
                        setMetaRows={setMetaRows}
                    />
                }
                right={
                    <GrpcResponsePane
                        state={state}
                        resTab={resTab}
                        onResTabChange={setResTab}
                        splitOrientation={splitOrientation}
                        onToggleSplitOrientation={toggleSplitOrientation}
                    />
                }
            />

            {/* Band 4 — Bottom bar. Cancel and Create exist only for a draft: an existing entity
                autosaves every edit and has Revert for undo. */}
            <BottomBar
                folders={folders}
                folderId={state.folderId}
                onFolderChange={(id) => set("folderId")(id)}
                onCancel={isNew ? onClose : undefined}
                onSave={isNew ? handleSave : undefined}
                saveLabel={isNew ? strings.common.create : undefined}
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

export default GrpcTab;
