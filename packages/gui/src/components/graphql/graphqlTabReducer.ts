import { SavedGraphQLRequest } from "@/types";

// -- Types ------------------------------------------------------------------

export type GraphQLTabType = "request";

export interface GraphQLTabState {
    tabType: GraphQLTabType;
    name: string;
    folderId: string | null;
    // Request fields
    endpointUrl: string;
    headers: Record<string, string>;
    query: string;
    variables: string;
    operationName: string;
    preScript: string;
    postScript: string;
    schemaId: string | null;
    // Response (after send)
    sending: boolean;
    resStatus: number | null;
    resHeaders: Record<string, string>;
    resBody: string;
    resDuration: number | null;
    resError: string | null;
    // Save state
    saving: boolean;
    dirty: boolean;
}

// -- Draft shapes -----------------------------------------------------------

export interface GraphQLRequestDraft {
    name: string;
    endpointUrl: string;
    headers: Record<string, string>;
    query: string;
    variables: string;
    operationName: string;
    preScript: string;
    postScript: string;
    folderId: string | null;
}

// -- Actions ----------------------------------------------------------------

import { createTabReducer, CommonTabAction } from "@/lib/createTabReducer";

export type GraphQLTabAction =
    | { type: "SEND_START" }
    | { type: "SEND_SUCCESS"; status: number; headers: Record<string, string>; body: string; durationMs: number }
    | CommonTabAction<GraphQLTabState, SavedGraphQLRequest, GraphQLRequestDraft>;

// -- Initial state ----------------------------------------------------------

export function initGraphQLState(
    entity: SavedGraphQLRequest | Partial<SavedGraphQLRequest> | null,
    draft: GraphQLRequestDraft | null,
    tabType: GraphQLTabType,
): GraphQLTabState {
    const base: GraphQLTabState = {
        tabType,
        name: "",
        folderId: null,
        endpointUrl: "",
        headers: {},
        query: "",
        variables: "",
        operationName: "",
        preScript: "",
        postScript: "",
        schemaId: null,
        sending: false,
        resStatus: null,
        resHeaders: {},
        resBody: "",
        resDuration: null,
        resError: null,
        saving: false,
        dirty: false,
    };

    if (draft) {
        return { ...base, name: draft.name, endpointUrl: draft.endpointUrl, headers: draft.headers, query: draft.query, variables: draft.variables, operationName: draft.operationName, preScript: draft.preScript, postScript: draft.postScript, folderId: draft.folderId };
    }

    if (entity) {
        const e = entity as SavedGraphQLRequest;
        return { ...base, name: e.name ?? "", endpointUrl: e.endpointUrl ?? "", headers: e.headers ?? {}, query: e.query ?? "", variables: e.variables ?? "", operationName: e.operationName ?? "", preScript: e.preScript ?? "", postScript: e.postScript ?? "", schemaId: e.schemaId ?? null, folderId: e.folderId ?? null };
    }

    return base;
}

// -- Reducer ----------------------------------------------------------------

const baseGraphqlTabReducer = (state: GraphQLTabState, action: GraphQLTabAction): GraphQLTabState => {
    switch (action.type) {
        case "SEND_START":
            return { ...state, sending: true, resError: null, resStatus: null, resBody: "", resHeaders: {}, resDuration: null };
        case "SEND_SUCCESS":
            return { ...state, sending: false, resStatus: action.status, resHeaders: action.headers, resBody: action.body, resDuration: action.durationMs };
        default:
            return state;
    }
};

export const graphqlTabReducer = createTabReducer<GraphQLTabState, GraphQLTabAction, SavedGraphQLRequest, GraphQLRequestDraft>({
    init: initGraphQLState
}, baseGraphqlTabReducer);

// -- Helpers ----------------------------------------------------------------

export function stateToRequestPayload(state: GraphQLTabState): Omit<SavedGraphQLRequest, "id" | "createdAt" | "workspaceId"> {
    return {
        name: state.name,
        endpointUrl: state.endpointUrl,
        headers: state.headers,
        query: state.query,
        variables: state.variables,
        operationName: state.operationName,
        preScript: state.preScript || undefined,
        postScript: state.postScript || undefined,
        schemaId: state.schemaId,
        folderId: state.folderId,
    };
}

export function stateToDraft(state: GraphQLTabState): GraphQLRequestDraft {
    return {
        name: state.name,
        endpointUrl: state.endpointUrl,
        headers: state.headers,
        query: state.query,
        variables: state.variables,
        operationName: state.operationName,
        preScript: state.preScript,
        postScript: state.postScript,
        folderId: state.folderId,
    };
}

export function isDraftEmpty(state: GraphQLTabState): boolean {
    return !state.name && !state.endpointUrl && !state.query;
}
