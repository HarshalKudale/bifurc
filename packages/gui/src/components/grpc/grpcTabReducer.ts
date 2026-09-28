import { SavedGrpcRequest } from "@/types";

// -- State ------------------------------------------------------------------

export type GrpcTabType = "request";

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

// -- Actions ----------------------------------------------------------------

import { createTabReducer, CommonTabAction } from "@/lib/createTabReducer";

export type GrpcAction =
    | { type: "SET_METADATA"; metadata: Record<string, string> }
    | { type: "SEND_START" }
    | { type: "SEND_SUCCESS"; responses: string[]; metadata: Record<string, string>; status: number; statusMessage: string; durationMs: number }
    | { type: "LOAD"; state: Partial<GrpcTabState> }
    | CommonTabAction<GrpcTabState, SavedGrpcRequest, GrpcRequestDraft>;

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

// -- Reducer ----------------------------------------------------------------

const baseGrpcTabReducer = (state: GrpcTabState, action: GrpcAction): GrpcTabState => {
    switch (action.type) {
        case "SET_METADATA":
            return { ...state, metadata: action.metadata, dirty: true };
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

export const grpcTabReducer = createTabReducer<GrpcTabState, GrpcAction, SavedGrpcRequest, GrpcRequestDraft>(
    {
        // The gRPC tab's state factory is spelled out here so the shared LOAD_ENTITY / LOAD_DRAFT /
        // REFRESH handling in createTabReducer knows how to rebuild the editor. Mirrors `initState`
        // in GrpcTab.tsx.
        init: (entity: SavedGrpcRequest | null, draft: GrpcRequestDraft | null) => {
            if (draft) return { ...initGrpcRequestState(), ...draft };
            return initGrpcRequestState(entity);
        },
    },
    baseGrpcTabReducer,
);

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
