export type CommonTabAction<S, Entity, Draft> =
    | { type: "SET_FIELD"; field: keyof S; value: S[keyof S] }
    | { type: "LOAD_ENTITY"; entity: Entity; tabType: any }
    | { type: "LOAD_DRAFT"; draft: Draft; tabType: any }
    | { type: "REFRESH"; entity: Entity; tabType: any }
    | { type: "SAVE_START" }
    | { type: "SAVE_SUCCESS" }
    | { type: "SAVE_DONE" }
    | { type: "SAVE_ERROR"; error?: string }
    | { type: "SEND_ERROR"; error: string };

export interface TabReducerOptions<S, Entity, Draft> {
    init?: (entity: Entity | null, draft: Draft | null, tabType: any) => S;
}

/**
 * Fields that hold *runtime* state — in-flight flags and the response pane — rather than
 * entity data. A `REFRESH` must re-derive the entity-derived fields but leave these alone,
 * mirroring `restTabReducer`'s `REFRESH`, which the REST tab has always relied on.
 */
const RUNTIME_FIELDS = new Set<string>([
    // send / response pane
    "loading", "result", "sendErr", "durationMs",
    "sending", "resStatus", "resHeaders", "resBody", "resDuration", "resError",
    "responses", "resMetadata", "resStatusMessage",
    // mock "test" pane
    "testLoading", "testError",
    // save bookkeeping
    "saving", "dirty",
]);

export function createTabReducer<S extends object, A extends { type: string }, Entity = any, Draft = any>(
    options: TabReducerOptions<S, Entity, Draft>,
    protocolReducer: (state: S, action: A) => S
) {
    return function tabReducer(state: S, action: A | CommonTabAction<S, Entity, Draft>): S {
        switch (action.type) {
            /**
             * These three were silently dropped for the protocol tabs. This layer only
             * handled the save/send cases, so they fell through to the protocol reducer's
             * `default:` branch and returned the state unchanged. The result was
             * user-visible: discarding changes on a GraphQL/SOAP/gRPC tab dispatches
             * `REFRESH` after reloading the entity, so the editor kept showing the edits
             * the user had just thrown away.
             *
             * REST never hit this — it implements all three itself and passes no `init`,
             * so it still falls through to its own reducer below.
             */
            case "LOAD_ENTITY":
            case "LOAD_DRAFT":
            case "REFRESH": {
                if (!options.init) return protocolReducer(state, action as A);
                const { type, entity, draft, tabType } = action as any;
                if (type === "LOAD_ENTITY") return options.init(entity ?? null, null, tabType) as S;
                if (type === "LOAD_DRAFT") return options.init(null, draft ?? null, tabType) as S;

                const fresh = options.init(entity ?? null, null, tabType) as S;
                const next: Record<string, unknown> = { ...(fresh as Record<string, unknown>) };
                const current = state as Record<string, unknown>;
                for (const key of Object.keys(current)) {
                    if (RUNTIME_FIELDS.has(key)) next[key] = current[key];
                }
                return next as S;
            }
            case "SET_FIELD":
                if ("dirty" in state) {
                    return { ...state, [(action as any).field]: (action as any).value, dirty: true };
                }
                return { ...state, [(action as any).field]: (action as any).value };
            case "SAVE_START":
                return { ...state, saving: true, saveErr: null };
            case "SAVE_SUCCESS":
            case "SAVE_DONE":
                if ("dirty" in state) {
                    return { ...state, saving: false, dirty: false };
                }
                return { ...state, saving: false };
            case "SAVE_ERROR":
                return { ...state, saving: false, saveErr: (action as any).error };
            case "SEND_ERROR":
                if ("sendErr" in state) {
                    return { ...state, loading: false, sendErr: (action as any).error };
                }
                return { ...state, sending: false, resError: (action as any).error };
        }
        
        return protocolReducer(state, action as A);
    };
}
