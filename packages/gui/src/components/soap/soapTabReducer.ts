import { SavedSoapRequest } from "@/types";

// -- Types ------------------------------------------------------------------

export type SoapTabType = "request";

export interface SoapTabState {
    tabType: SoapTabType;
    name: string;
    folderId: string | null;
    // Request fields
    endpointUrl: string;
    soapAction: string;
    headers: Record<string, string>;
    body: string;
    wsdlId: string | null;
    operationName: string;
    preScript: string;
    postScript: string;
    // Response state (request mode)
    sending: boolean;
    resStatus: number | null;
    resHeaders: Record<string, string>;
    resBody: string;
    resDuration: number | null;
    resError: string | null;
    // Save state
    saving: boolean;
    dirty: boolean;
    // UI tabs
    reqTab: "body" | "headers" | "pre-script" | "post-script" | "wsdl";
    resTab: "body" | "headers";
}

// -- Draft shapes -----------------------------------------------------------

export interface SoapRequestDraft {
    name: string;
    endpointUrl: string;
    soapAction: string;
    headers: Record<string, string>;
    body: string;
    folderId: string | null;
    preScript: string;
    postScript: string;
}

// -- Actions ----------------------------------------------------------------

import { createTabReducer, CommonTabAction } from "@/lib/createTabReducer";

export type SoapTabAction =
    | { type: "SEND_START" }
    | { type: "SEND_SUCCESS"; status: number; headers: Record<string, string>; body: string; durationMs: number }
    | CommonTabAction<SoapTabState, SavedSoapRequest, SoapRequestDraft>;

// -- Init -------------------------------------------------------------------

const DEFAULT_SOAP_BODY = `<?xml version="1.0" encoding="utf-8"?>
<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/">
  <soap:Header/>
  <soap:Body>
  </soap:Body>
</soap:Envelope>`;

export function initSoapState(
    initial: SavedSoapRequest | Partial<SavedSoapRequest> | null,
    draft: SoapRequestDraft | null,
    tabType: SoapTabType,
): SoapTabState {
    const base: SoapTabState = {
        tabType,
        name: "",
        folderId: null,
        endpointUrl: "",
        soapAction: "",
        headers: {},
        body: DEFAULT_SOAP_BODY,
        wsdlId: null,
        operationName: "",
        preScript: "",
        postScript: "",
        sending: false,
        resStatus: null,
        resHeaders: {},
        resBody: "",
        resDuration: null,
        resError: null,
        saving: false,
        dirty: false,
        reqTab: "body",
        resTab: "body",
    };

    if (draft) {
        base.name = draft.name ?? "";
        base.endpointUrl = draft.endpointUrl ?? "";
        base.soapAction = draft.soapAction ?? "";
        base.headers = draft.headers ?? {};
        base.body = draft.body || DEFAULT_SOAP_BODY;
        base.folderId = draft.folderId ?? null;
        base.preScript = draft.preScript ?? "";
        base.postScript = draft.postScript ?? "";
        return base;
    }

    if (initial) {
        const r = initial as Partial<SavedSoapRequest>;
        base.name = r.name ?? "";
        base.endpointUrl = r.endpointUrl ?? "";
        base.soapAction = r.soapAction ?? "";
        base.headers = r.headers ?? {};
        base.body = r.body || DEFAULT_SOAP_BODY;
        base.wsdlId = r.wsdlId ?? null;
        base.operationName = r.operationName ?? "";
        base.preScript = r.preScript ?? "";
        base.postScript = r.postScript ?? "";
        base.folderId = r.folderId ?? null;
    }

    return base;
}

// -- Reducer ----------------------------------------------------------------

const baseSoapTabReducer = (state: SoapTabState, action: SoapTabAction): SoapTabState => {
    switch (action.type) {
        case "SEND_START":
            return { ...state, sending: true, resStatus: null, resHeaders: {}, resBody: "", resDuration: null, resError: null };
        case "SEND_SUCCESS":
            return { ...state, sending: false, resStatus: action.status, resHeaders: action.headers, resBody: action.body, resDuration: action.durationMs };
        default:
            return state;
    }
};

export const soapTabReducer = createTabReducer<SoapTabState, SoapTabAction, SavedSoapRequest, SoapRequestDraft>({
    init: initSoapState
}, baseSoapTabReducer);

// -- Helpers ----------------------------------------------------------------

export function soapStateToSavePayload(
    state: SoapTabState,
): Omit<SavedSoapRequest, "id" | "createdAt" | "workspaceId"> {
    return {
        name: state.name,
        endpointUrl: state.endpointUrl,
        soapAction: state.soapAction,
        headers: state.headers,
        body: state.body,
        wsdlId: state.wsdlId,
        operationName: state.operationName,
        preScript: state.preScript,
        postScript: state.postScript,
        folderId: state.folderId,
    };
}

export function soapStateToDraft(
    state: SoapTabState,
): SoapRequestDraft {
    return {
        name: state.name,
        endpointUrl: state.endpointUrl,
        soapAction: state.soapAction,
        headers: state.headers,
        body: state.body,
        folderId: state.folderId,
        preScript: state.preScript,
        postScript: state.postScript,
    };
}

export function isSoapDraftEmpty(state: SoapTabState): boolean {
    return !state.name && !state.endpointUrl && !state.soapAction && state.body === DEFAULT_SOAP_BODY;
}
