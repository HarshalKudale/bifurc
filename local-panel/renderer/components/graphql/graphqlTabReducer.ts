import { SavedGraphQLRequest, SavedGraphQLMock } from "@/types";

// -- Types ------------------------------------------------------------------

export type GraphQLTabType = "request" | "mock";

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
    // Mock-specific
    endpointPattern: string;
    useRegex: boolean;
    operationType: "query" | "mutation" | "subscription" | "any";
    operationNameMatch: string;
    responseStatus: number;
    responseHeaders: Record<string, string>;
    responseBody: string;
    responseDelay: number;
    enabled: boolean;
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

export interface GraphQLMockDraft {
    name: string;
    endpointPattern: string;
    useRegex: boolean;
    operationType: "query" | "mutation" | "subscription" | "any";
    operationNameMatch: string;
    responseStatus: number;
    responseHeaders: Record<string, string>;
    responseBody: string;
    responseDelay: number;
    folderId: string | null;
}

// -- Actions ----------------------------------------------------------------

import { createTabReducer, CommonTabAction } from "@/lib/createTabReducer";

export type GraphQLTabAction =
    | { type: "SEND_START" }
    | { type: "SEND_SUCCESS"; status: number; headers: Record<string, string>; body: string; durationMs: number }
    | CommonTabAction<GraphQLTabState, SavedGraphQLRequest | SavedGraphQLMock, GraphQLRequestDraft | GraphQLMockDraft>;

// -- Initial state ----------------------------------------------------------

export function initGraphQLState(
    entity: SavedGraphQLRequest | SavedGraphQLMock | Partial<SavedGraphQLRequest> | Partial<SavedGraphQLMock> | null,
    draft: GraphQLRequestDraft | GraphQLMockDraft | null,
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
        endpointPattern: "",
        useRegex: false,
        operationType: "any",
        operationNameMatch: "",
        responseStatus: 200,
        responseHeaders: { "content-type": "application/json" },
        responseBody: "{\n  \"data\": {}\n}",
        responseDelay: 0,
        enabled: true,
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
        if (tabType === "request") {
            const d = draft as GraphQLRequestDraft;
            return { ...base, name: d.name, endpointUrl: d.endpointUrl, headers: d.headers, query: d.query, variables: d.variables, operationName: d.operationName, preScript: d.preScript, postScript: d.postScript, folderId: d.folderId };
        } else {
            const d = draft as GraphQLMockDraft;
            return { ...base, name: d.name, endpointPattern: d.endpointPattern, useRegex: d.useRegex, operationType: d.operationType, operationNameMatch: d.operationNameMatch, responseStatus: d.responseStatus, responseHeaders: d.responseHeaders, responseBody: d.responseBody, responseDelay: d.responseDelay, folderId: d.folderId };
        }
    }

    if (entity) {
        if (tabType === "request") {
            const e = entity as SavedGraphQLRequest;
            return { ...base, name: e.name ?? "", endpointUrl: e.endpointUrl ?? "", headers: e.headers ?? {}, query: e.query ?? "", variables: e.variables ?? "", operationName: e.operationName ?? "", preScript: e.preScript ?? "", postScript: e.postScript ?? "", schemaId: e.schemaId ?? null, folderId: e.folderId ?? null };
        } else {
            const e = entity as SavedGraphQLMock;
            return { ...base, name: e.name ?? "", endpointPattern: e.endpointPattern ?? "", useRegex: e.useRegex ?? false, operationType: e.operationType ?? "any", operationNameMatch: e.operationName ?? "", responseStatus: e.responseStatus ?? 200, responseHeaders: e.responseHeaders ?? { "content-type": "application/json" }, responseBody: e.responseBody ?? "", responseDelay: e.responseDelay ?? 0, enabled: e.enabled ?? true, folderId: e.folderId ?? null };
        }
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

export const graphqlTabReducer = createTabReducer<GraphQLTabState, GraphQLTabAction, SavedGraphQLRequest | SavedGraphQLMock, GraphQLRequestDraft | GraphQLMockDraft>({
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

export function stateToMockPayload(state: GraphQLTabState): Omit<SavedGraphQLMock, "id" | "createdAt" | "workspaceId"> {
    return {
        name: state.name,
        enabled: state.enabled,
        endpointPattern: state.endpointPattern,
        useRegex: state.useRegex,
        operationType: state.operationType,
        operationName: state.operationNameMatch,
        responseStatus: state.responseStatus,
        responseHeaders: state.responseHeaders,
        responseBody: state.responseBody,
        responseDelay: state.responseDelay || undefined,
        schemaId: state.schemaId,
        folderId: state.folderId,
    };
}

export function stateToDraft(state: GraphQLTabState, tabType: GraphQLTabType): GraphQLRequestDraft | GraphQLMockDraft {
    if (tabType === "request") {
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
    return {
        name: state.name,
        endpointPattern: state.endpointPattern,
        useRegex: state.useRegex,
        operationType: state.operationType,
        operationNameMatch: state.operationNameMatch,
        responseStatus: state.responseStatus,
        responseHeaders: state.responseHeaders,
        responseBody: state.responseBody,
        responseDelay: state.responseDelay,
        folderId: state.folderId,
    };
}

export function isDraftEmpty(state: GraphQLTabState, tabType: GraphQLTabType): boolean {
    if (tabType === "request") {
        return !state.name && !state.endpointUrl && !state.query;
    }
    return !state.name && !state.operationNameMatch && !state.responseBody;
}
