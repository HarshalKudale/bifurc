import { SavedGrpcRequest, SavedGrpcMock } from "@/types";

// -- State ------------------------------------------------------------------

export type GrpcTabType = "request" | "mock";

export interface GrpcTabState {
    tabType: GrpcTabType;
    serverAddress: string;
    serviceName: string;
    methodName: string;
    requestBody: string;
    metadata: Record<string, string>;
    protoFileId: string | null;
    useReflection: boolean;
    streamingType: "unary" | "server" | "client" | "bidi";
    preScript: string;
    postScript: string;
    // Mock-specific
    responseBody: string;
    responseMetadata: Record<string, string>;
    responseDelay: number;
    streamingResponses: string[];
    errorCode: number;
    errorMessage: string;
    enabled: boolean;
    // Response state
    sending: boolean;
    responses: string[];
    resMetadata: Record<string, string>;
    resStatus: number | null;
    resStatusMessage: string;
    resDuration: number | null;
    resError: string | null;
    // Save
    saving: boolean;
    dirty: boolean;
    name: string;
    folderId: string | null;
}

// -- Draft shapes -----------------------------------------------------------

export interface GrpcRequestDraft {
    name: string;
    serverAddress: string;
    serviceName: string;
    methodName: string;
    requestBody: string;
    metadata: Record<string, string>;
    protoFileId: string | null;
    useReflection: boolean;
    streamingType: "unary" | "server" | "client" | "bidi";
    preScript: string;
    postScript: string;
    folderId: string | null;
}

export interface GrpcMockDraft {
    name: string;
    serviceName: string;
    methodName: string;
    responseBody: string;
    responseMetadata: Record<string, string>;
    responseDelay: number;
    streamingResponses: string[];
    errorCode: number;
    errorMessage: string;
    protoFileId: string | null;
    enabled: boolean;
    folderId: string | null;
}

// -- Actions ----------------------------------------------------------------

import { createTabReducer, CommonTabAction } from "@/lib/createTabReducer";

export type GrpcAction =
    | { type: "SET_METADATA"; metadata: Record<string, string> }
    | { type: "SET_RESPONSE_METADATA"; metadata: Record<string, string> }
    | { type: "SEND_START" }
    | { type: "SEND_SUCCESS"; responses: string[]; metadata: Record<string, string>; status: number; statusMessage: string; durationMs: number }
    | { type: "LOAD"; state: Partial<GrpcTabState> }
    | CommonTabAction<GrpcTabState, any, any>;

// -- Initial state factory --------------------------------------------------

export function initGrpcRequestState(req?: SavedGrpcRequest | null): GrpcTabState {
    return {
        tabType: "request",
        name: req?.name ?? "",
        folderId: req?.folderId ?? null,
        serverAddress: req?.serverAddress ?? "localhost:50051",
        serviceName: req?.serviceName ?? "",
        methodName: req?.methodName ?? "",
        requestBody: req?.requestBody ?? "{}",
        metadata: req?.metadata ?? {},
        protoFileId: req?.protoFileId ?? null,
        useReflection: req?.useReflection ?? false,
        streamingType: req?.streamingType ?? "unary",
        preScript: req?.preScript ?? "",
        postScript: req?.postScript ?? "",
        // Mock-specific (unused in request mode)
        responseBody: "",
        responseMetadata: {},
        responseDelay: 0,
        streamingResponses: [],
        errorCode: 0,
        errorMessage: "",
        enabled: true,
        // Response state
        sending: false,
        responses: [],
        resMetadata: {},
        resStatus: null,
        resStatusMessage: "",
        resDuration: null,
        resError: null,
        // Save
        saving: false,
        dirty: false,
    };
}

export function initGrpcMockState(mock?: SavedGrpcMock | null): GrpcTabState {
    return {
        tabType: "mock",
        name: mock?.name ?? "",
        folderId: mock?.folderId ?? null,
        serverAddress: "",
        serviceName: mock?.serviceName ?? "",
        methodName: mock?.methodName ?? "",
        requestBody: "",
        metadata: {},
        protoFileId: mock?.protoFileId ?? null,
        useReflection: false,
        streamingType: "unary",
        preScript: "",
        postScript: "",
        // Mock-specific
        responseBody: mock?.responseBody ?? "{}",
        responseMetadata: mock?.responseMetadata ?? {},
        responseDelay: mock?.responseDelay ?? 0,
        streamingResponses: mock?.streamingResponses ?? [],
        errorCode: mock?.errorCode ?? 0,
        errorMessage: mock?.errorMessage ?? "",
        enabled: mock?.enabled ?? true,
        // Response state (unused in mock mode)
        sending: false,
        responses: [],
        resMetadata: {},
        resStatus: null,
        resStatusMessage: "",
        resDuration: null,
        resError: null,
        // Save
        saving: false,
        dirty: false,
    };
}

// -- Reducer ----------------------------------------------------------------

const baseGrpcTabReducer = (state: GrpcTabState, action: GrpcAction): GrpcTabState => {
    switch (action.type) {
        case "SET_METADATA":
            return { ...state, metadata: action.metadata, dirty: true };
        case "SET_RESPONSE_METADATA":
            return { ...state, responseMetadata: action.metadata, dirty: true };
        case "SEND_START":
            return { ...state, sending: true, resError: null, responses: [], resMetadata: {}, resStatus: null, resStatusMessage: "", resDuration: null };
        case "SEND_SUCCESS":
            return { ...state, sending: false, responses: action.responses, resMetadata: action.metadata, resStatus: action.status, resStatusMessage: action.statusMessage, resDuration: action.durationMs };
        case "LOAD":
            return { ...state, ...action.state, dirty: false };
        default:
            return state;
    }
};

export const grpcTabReducer = createTabReducer<GrpcTabState, GrpcAction, any, any>({}, baseGrpcTabReducer);

// -- Serialization helpers --------------------------------------------------

export function stateToRequestDraft(s: GrpcTabState): GrpcRequestDraft {
    return {
        name: s.name,
        serverAddress: s.serverAddress,
        serviceName: s.serviceName,
        methodName: s.methodName,
        requestBody: s.requestBody,
        metadata: s.metadata,
        protoFileId: s.protoFileId,
        useReflection: s.useReflection,
        streamingType: s.streamingType,
        preScript: s.preScript,
        postScript: s.postScript,
        folderId: s.folderId,
    };
}

export function stateToMockDraft(s: GrpcTabState): GrpcMockDraft {
    return {
        name: s.name,
        serviceName: s.serviceName,
        methodName: s.methodName,
        responseBody: s.responseBody,
        responseMetadata: s.responseMetadata,
        responseDelay: s.responseDelay,
        streamingResponses: s.streamingResponses,
        errorCode: s.errorCode,
        errorMessage: s.errorMessage,
        protoFileId: s.protoFileId,
        enabled: s.enabled,
        folderId: s.folderId,
    };
}

export function requestToSaveData(s: GrpcTabState): Omit<SavedGrpcRequest, "id" | "createdAt" | "workspaceId"> {
    return {
        name: s.name,
        serverAddress: s.serverAddress,
        serviceName: s.serviceName,
        methodName: s.methodName,
        requestBody: s.requestBody,
        metadata: s.metadata,
        protoFileId: s.protoFileId,
        useReflection: s.useReflection,
        streamingType: s.streamingType,
        preScript: s.preScript,
        postScript: s.postScript,
        folderId: s.folderId,
    };
}

export function mockToSaveData(s: GrpcTabState): Omit<SavedGrpcMock, "id" | "createdAt" | "workspaceId"> {
    return {
        name: s.name,
        enabled: s.enabled,
        serviceName: s.serviceName,
        methodName: s.methodName,
        responseBody: s.responseBody,
        responseMetadata: s.responseMetadata,
        responseDelay: s.responseDelay,
        streamingResponses: s.streamingResponses,
        errorCode: s.errorCode,
        errorMessage: s.errorMessage,
        protoFileId: s.protoFileId ?? "",
        folderId: s.folderId,
    };
}
