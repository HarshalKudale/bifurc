import { useReducer, useRef, useEffect, useCallback, useState } from "react";
import { useDraftPersist, loadDraft } from "@/hooks/useDraftPersist";
import { useDebouncedAutosave } from "@/hooks/useDebouncedAutosave";

export interface ProtocolEditorConfig<
    State,
    Action,
    Draft,
    SavePayload,
    Entity,
    TabType extends string = string,
> {
    /**
     * Which kind of tab this is — `"request"` or `"mock"` for SOAP, and so on.
     *
     * **Generic rather than `string`, and that is not decoration.** Every callback below receives it, and the
     * implementations narrow it (`SoapTabType`, `GraphQLTabType`). A function whose parameter is the *narrow*
     * union is **not** assignable to one taking `string`, because the hook could hand it a value it cannot
     * handle — so every one of these arguments was rejected and every one was silenced with `as any`. With
     * nothing left to infer `State` from, TypeScript fell back to `unknown`, which is the whole of the UI's
     * 105 `'state' is of type 'unknown'` errors: **one variance mismatch, 105 consequences.**
     *
     * Parameterising it lets the narrow type flow through instead, and the casts can go.
     */
    tabType: TabType;
    tabId: string;
    draftTabId?: string | null;
    /**
     * Optional, because every call site's own prop is optional and therefore `Entity | null | undefined`.
     * The hook already normalises it — `initState(initial ?? null, …)` — so accepting `undefined` here
     * states what the code does. Leaving it required was enough on its own to make TypeScript abandon
     * inference for the whole call and fall back to the parameter defaults, which is why `TabType` came out
     * as `string` and every `tabType` argument was rejected again.
     */
    initial?: Entity | null;
    reducer: React.Reducer<State, Action>;
    initState: (initial: Entity | null, draft: Draft | null, tabType: TabType) => State;
    stateToDraft: (state: State, tabType: TabType) => Draft;
    isDraftEmpty: (state: State, tabType: TabType) => boolean;
    stateToSavePayload: (state: State, tabType: TabType) => SavePayload;
    onSave: (payload: SavePayload) => Promise<any>;
    onSync?: (savedId?: string) => Promise<void>;
    onRevert?: () => Promise<void>;
    syncStatus?: "clean" | "modified" | "new" | "deleted";
    onDirtyChange?: (dirty: boolean) => void;
}

export function useProtocolEditor<
    State,
    Action,
    Draft,
    SavePayload,
    Entity,
    TabType extends string = string,
>({
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
}: ProtocolEditorConfig<State, Action, Draft, SavePayload, Entity, TabType>) {
    const draft = draftTabId ? loadDraft<Draft>(draftTabId) : null;

    const [state, dispatch] = useReducer(
        reducer,
        undefined,
        () => initState(initial ?? null, draft, tabType)
    );

    const initialSnapshot = useRef(JSON.stringify(stateToDraft(initState(initial ?? null, draft, tabType), tabType)));

    /**
     * The serialized draft projection, computed once and shared by the dirty check and the
     * autosave debounce — so "has the user edited this?" and "should we write it?" cannot drift
     * apart. Deriving it twice was the obvious way to introduce a save that never fires.
     */
    const draftSnapshot = JSON.stringify(stateToDraft(state, tabType));
    const isDirty = draftSnapshot !== initialSnapshot.current;

    useEffect(() => {
        onDirtyChange?.(isDirty);
    }, [isDirty, onDirtyChange]);

    const { markSaved } = useDraftPersist(
        draftTabId ?? null,
        () => stateToDraft(state, tabType),
        () => isDraftEmpty(state, tabType)
    );

    const handleSave = useCallback(async () => {
        // Nothing to write: an existing entity with no edits. Ctrl+S and Sync both route here,
        // and neither should cost a round-trip on a clean entity. Drafts are exempt — an
        // untouched new tab still has to be created.
        if (!draftTabId && !isDirty) return undefined;
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
    }, [state, tabType, onSave, markSaved, stateToSavePayload, stateToDraft, draftTabId, isDirty]);

    /**
     * **Autosave is now the only way an existing entity is written** — the Save button is gone.
     * A draft is excluded: it has no server id yet, so the user creates it explicitly and
     * autosave takes over from the next edit.
     */
    const {
        saving: autoSaving,
        savedAt: autoSavedAt,
        error: autoSaveError,
        flush: flushSave,
    } = useDebouncedAutosave({
        snapshot: draftSnapshot,
        enabled: !draftTabId,
        onSave: handleSave,
    });

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
                // A draft must go through `handleSave` (it creates); an existing entity goes
                // through `flushSave`, which also cancels the pending autosave so the same
                // edit is not written twice — once here and once by the debounce.
                const res: any = draftTabId ? await handleSave() : await flushSave();
                if (res && typeof res === "object" && res.id) {
                    savedId = res.id;
                }
            }
            await onSync(savedId);
        } finally {
            setSyncing(false);
        }
    }, [syncing, onSync, isDirty, draftTabId, handleSave, flushSave]);

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

    /**
     * Ctrl+S. A draft is created; an existing entity flushes its pending autosave — which also
     * cancels the debounce, so the same edit is not written twice (once by the shortcut, once by
     * the timer that was already scheduled).
     */
    const saveNow = useCallback(
        () => (draftTabId ? handleSave() : flushSave()),
        [draftTabId, handleSave, flushSave]
    );

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
        autoSaving,
        autoSavedAt,
        autoSaveError,
        flushSave,
        saveNow,
    };
}
