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

export function createTabReducer<S extends object, A extends { type: string }, Entity = any, Draft = any>(
    options: TabReducerOptions<S, Entity, Draft>,
    protocolReducer: (state: S, action: A) => S
) {
    return function tabReducer(state: S, action: A | CommonTabAction<S, Entity, Draft>): S {
        switch (action.type) {
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
