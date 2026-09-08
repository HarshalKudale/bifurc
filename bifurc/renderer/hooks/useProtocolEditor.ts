import { useReducer, useRef, useEffect, useCallback, useState } from "react";
import { useDraftPersist, loadDraft } from "@/hooks/useDraftPersist";

export interface ProtocolEditorConfig<State, Action, Draft, SavePayload, Entity> {
    tabType: string;
    tabId: string;
    draftTabId?: string | null;
    initial: Entity | null;
    reducer: React.Reducer<State, Action>;
    initState: (initial: Entity | null, draft: Draft | null, tabType: string) => State;
    stateToDraft: (state: State, tabType: string) => Draft;
    isDraftEmpty: (state: State, tabType: string) => boolean;
    stateToSavePayload: (state: State, tabType: string) => SavePayload;
    onSave: (payload: SavePayload) => Promise<any>;
    onSync?: (savedId?: string) => Promise<void>;
    onRevert?: () => Promise<void>;
    syncStatus?: "clean" | "modified" | "new" | "deleted";
    onDirtyChange?: (dirty: boolean) => void;
}

export function useProtocolEditor<State, Action, Draft, SavePayload, Entity>({
    tabType,
    tabId,
    draftTabId,
    initial,
    reducer,
    initState,
    stateToDraft,
    isDraftEmpty,
    stateToSavePayload,
    onSave,
    onSync,
    onRevert,
    syncStatus,
    onDirtyChange,
}: ProtocolEditorConfig<State, Action, Draft, SavePayload, Entity>) {
    const draft = draftTabId ? loadDraft<Draft>(draftTabId) : null;

    const [state, dispatch] = useReducer(
        reducer,
        undefined,
        () => initState(initial ?? null, draft, tabType)
    );

    const initialSnapshot = useRef(JSON.stringify(stateToDraft(initState(initial ?? null, draft, tabType), tabType)));
    const isDirty = JSON.stringify(stateToDraft(state, tabType)) !== initialSnapshot.current;

    useEffect(() => {
        onDirtyChange?.(isDirty);
    }, [isDirty, onDirtyChange]);

    const { markSaved } = useDraftPersist(
        draftTabId ?? null,
        () => stateToDraft(state, tabType),
        () => isDraftEmpty(state, tabType)
    );

    const handleSave = useCallback(async () => {
        dispatch({ type: "SAVE_START" } as unknown as Action);
        try {
            const payload = stateToSavePayload(state, tabType);
            const res = await onSave(payload);
            markSaved();
            initialSnapshot.current = JSON.stringify(stateToDraft(state, tabType));
            dispatch({ type: "SAVE_SUCCESS" } as unknown as Action);
            return res;
        } catch (e) {
            dispatch({ type: "SAVE_ERROR", error: e instanceof Error ? e.message : String(e) } as unknown as Action);
            throw e;
        }
    }, [state, tabType, onSave, markSaved, stateToSavePayload, stateToDraft]);

    const handleRefresh = useCallback((entity: Entity) => {
        dispatch({ type: "REFRESH", entity, tabType } as unknown as Action);
        initialSnapshot.current = JSON.stringify(stateToDraft(initState(entity, null, tabType), tabType));
    }, [tabType, initState, stateToDraft]);

    const [syncing, setSyncing] = useState(false);
    const [reverting, setReverting] = useState(false);

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

    const hasLocalChanges = !draftTabId && Boolean(isDirty || (syncStatus && syncStatus !== "clean"));

    return {
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
    };
}
