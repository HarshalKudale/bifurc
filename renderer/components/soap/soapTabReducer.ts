import { SavedSoapRequest, SavedSoapMock } from "@/types";

// -- Types ------------------------------------------------------------------

export type SoapTabType = "request" | "mock";

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
    // Mock-specific
    endpointPattern: string;
    useRegex: boolean;
    soapActionPattern: string;
    responseStatus: number;
    responseHeaders: Record<string, string>;
    responseBody: string;
    responseDelay: number;
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

export interface SoapMockDraft {
    name: string;
    endpointPattern: string;
    useRegex: boolean;
    soapActionPattern: string;
    responseStatus: number;
    responseHeaders: Record<string, string>;
    responseBody: string;
    responseDelay: number;
    folderId: string | null;
}

// -- Actions ----------------------------------------------------------------

import { createTabReducer, CommonTabAction } from "@/lib/createTabReducer";

export type SoapTabAction =
    | { type: "SEND_START" }
    | { type: "SEND_SUCCESS"; status: number; headers: Record<string, string>; body: string; durationMs: number }
    | CommonTabAction<SoapTabState, SavedSoapRequest | SavedSoapMock, SoapRequestDraft | SoapMockDraft>;

// -- Init -------------------------------------------------------------------

const DEFAULT_SOAP_BODY = `<?xml version="1.0" encoding="utf-8"?>
<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/">
  <soap:Header/>
  <soap:Body>
  </soap:Body>
</soap:Envelope>`;

export function initSoapState(
    initial: SavedSoapRequest | SavedSoapMock | Partial<SavedSoapRequest> | Partial<SavedSoapMock> | null,
    draft: SoapRequestDraft | SoapMockDraft | null,
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
        endpointPattern: "",
        useRegex: false,
        soapActionPattern: "",
        responseStatus: 200,
        responseHeaders: { "Content-Type": "text/xml; charset=utf-8" },
        responseBody: DEFAULT_SOAP_BODY,
        responseDelay: 0,
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
        if (tabType === "request") {
            const d = draft as SoapRequestDraft;
            base.name = d.name ?? "";
            base.endpointUrl = d.endpointUrl ?? "";
            base.soapAction = d.soapAction ?? "";
            base.headers = d.headers ?? {};
            base.body = d.body || DEFAULT_SOAP_BODY;
            base.folderId = d.folderId ?? null;
            base.preScript = d.preScript ?? "";
            base.postScript = d.postScript ?? "";
        } else {
            const d = draft as SoapMockDraft;
            base.name = d.name ?? "";
            base.endpointPattern = d.endpointPattern ?? "";
            base.useRegex = d.useRegex ?? false;
            base.soapActionPattern = d.soapActionPattern ?? "";
            base.responseStatus = d.responseStatus ?? 200;
            base.responseHeaders = d.responseHeaders ?? { "Content-Type": "text/xml; charset=utf-8" };
            base.responseBody = d.responseBody || DEFAULT_SOAP_BODY;
            base.responseDelay = d.responseDelay ?? 0;
            base.folderId = d.folderId ?? null;
        }
        return base;
    }

    if (initial) {
        if (tabType === "request") {
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
        } else {
            const m = initial as Partial<SavedSoapMock>;
            base.name = m.name ?? "";
            base.endpointPattern = m.endpointPattern ?? "";
            base.useRegex = m.useRegex ?? false;
            base.soapActionPattern = m.soapActionPattern ?? "";
            base.responseStatus = m.responseStatus ?? 200;
            base.responseHeaders = m.responseHeaders ?? { "Content-Type": "text/xml; charset=utf-8" };
            base.responseBody = m.responseBody || DEFAULT_SOAP_BODY;
            base.responseDelay = m.responseDelay ?? 0;
            base.wsdlId = m.wsdlId ?? null;
            base.operationName = m.operationName ?? "";
            base.folderId = m.folderId ?? null;
        }
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

export const soapTabReducer = createTabReducer<SoapTabState, SoapTabAction, SavedSoapRequest | SavedSoapMock, SoapRequestDraft | SoapMockDraft>({
    init: initSoapState
}, baseSoapTabReducer);

// -- Helpers ----------------------------------------------------------------

export function soapStateToSavePayload(
    state: SoapTabState,
    tabType: SoapTabType,
): Omit<SavedSoapRequest, "id" | "createdAt" | "workspaceId"> | Omit<SavedSoapMock, "id" | "createdAt" | "workspaceId"> {
    if (tabType === "request") {
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
    return {
        name: state.name,
        enabled: true,
        endpointPattern: state.endpointPattern,
        useRegex: state.useRegex,
        soapActionPattern: state.soapActionPattern,
        operationName: state.operationName,
        responseStatus: state.responseStatus,
        responseHeaders: state.responseHeaders,
        responseBody: state.responseBody,
        responseDelay: state.responseDelay,
        wsdlId: state.wsdlId,
        folderId: state.folderId,
    };
}

export function soapStateToDraft(
    state: SoapTabState,
    tabType: SoapTabType,
): SoapRequestDraft | SoapMockDraft {
    if (tabType === "request") {
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
    return {
        name: state.name,
        endpointPattern: state.endpointPattern,
        useRegex: state.useRegex,
        soapActionPattern: state.soapActionPattern,
        responseStatus: state.responseStatus,
        responseHeaders: state.responseHeaders,
        responseBody: state.responseBody,
        responseDelay: state.responseDelay,
        folderId: state.folderId,
    };
}

export function isSoapDraftEmpty(state: SoapTabState, tabType: SoapTabType): boolean {
    if (tabType === "request") {
        return !state.name && !state.endpointUrl && !state.soapAction && state.body === DEFAULT_SOAP_BODY;
    }
    return !state.name && !state.endpointPattern && !state.soapActionPattern && state.responseBody === DEFAULT_SOAP_BODY;
}
