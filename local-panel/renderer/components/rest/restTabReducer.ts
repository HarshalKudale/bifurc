import { SavedRequest, MockRule, ReplayResult } from "@/types";
import {
  KVRow, mkRowId, headersToRows, rowsToHeaders,
  b64ToText, textToB64, tryFormat,
} from "@/lib/utils";
import { BodyMode, contentTypeToMode, modeToContentType } from "@/lib/bodyUtils";
import { SKIP_CURL_HEADERS } from "@/lib/curlParser";

// -- Helper for case-insensitive header lookup -----------------------------

function getHeaderCaseInsensitive(headers: Record<string, string> | undefined, key: string): string | undefined {
  if (!headers) return undefined;
  const lowerKey = key.toLowerCase();
  for (const [k, v] of Object.entries(headers)) {
    if (k.toLowerCase() === lowerKey) return v;
  }
  return undefined;
}

// -- Types ------------------------------------------------------------------

export type TabType = "request" | "mock";

export interface TabState {
  // Identity / meta
  name: string;
  folderId: string | null;

  // URL bar
  method: string;
  url: string;          // request URL for requests; urlPattern for mocks

  // Mock-only URL settings
  useRegex: boolean;
  regexError: string;

  // Request (left) pane
  reqTab: "params" | "headers" | "body" | "pre-script";
  reqParams: KVRow[];   // ephemeral - derived from url, never saved
  reqHeaders: KVRow[];
  reqBody: string;
  reqMode: BodyMode;
  reqBodyStash: Partial<Record<BodyMode, string>>;  // ephemeral - per-mode body content

  // Response (right) pane
  resTab: "body" | "headers" | "post-script" | "tests";
  resMode: BodyMode;

  // Mock response fields (editable)
  resStatus: number;
  resStatusMocked: boolean;
  resHeaders: KVRow[];
  resBody: string;
  resBodyMocked: boolean;
  resDelay: number;
  resDelayMocked: boolean;
  resBodyEncoding: "utf8" | "base64";
  streamingMode: "none" | "sse" | "chunked";
  streamingChunkDelay: number;
  streamingChunkSeparator: string;

  // Scripts (request only)
  preScript: string;
  postScript: string;
  testScript: string;

  // cURL import
  curlInput: string;
  showCurl: boolean;

  // Runtime: request send state
  loading: boolean;
  result: ReplayResult | null;
  durationMs: number | null;
  sendErr: string | null;
  scriptErr: string | null;

  // Runtime: test results
  testResults: { name: string; passed: boolean; error?: string; durationMs: number }[];
  testLogs: string[];
  testRunning: boolean;

  // Runtime: save state
  saving: boolean;
  saveErr: string | null;

  // Runtime: mock test state
  testLoading: boolean;
  testError: string | null;
}

// -- Draft shapes -----------------------------------------------------------

export interface RequestDraft {
  name: string; method: string; url: string; folderId: string | null;
  headers: Record<string, string>; body: string; reqMode: BodyMode;
  preScript: string; postScript: string; testScript: string;
}

export interface MockDraft {
  name: string; method: string; urlPattern: string; useRegex: boolean;
  folderId: string | null;
  reqHeaders: Record<string, string>; reqBody: string; reqMode: BodyMode;
  resStatus: number; resStatusMocked: boolean; resHeaders: Record<string, string>; mockedResponseHeaders?: string[]; resBody: string; resBodyMocked: boolean; resMode: BodyMode;
  resDelay: number; resBodyEncoding: "utf8" | "base64";
  resDelayMocked: boolean;
  streamingMode: "none" | "sse" | "chunked";
  streamingChunkDelay: number;
  streamingChunkSeparator: string;
}

// -- Actions ----------------------------------------------------------------

// Parse the query string from a URL into KVRows (preserves order, handles duplicates)
export function urlToParams(url: string): KVRow[] {
  try {
    const qIdx = url.indexOf("?");
    if (qIdx === -1) return [];
    const search = url.slice(qIdx + 1);
    if (!search) return [];
    return search.split("&").filter(Boolean).map((part) => {
      const eq = part.indexOf("=");
      return {
        id: mkRowId(),
        enabled: true,
        key: eq === -1 ? decodeURIComponent(part) : decodeURIComponent(part.slice(0, eq)),
        value: eq === -1 ? "" : decodeURIComponent(part.slice(eq + 1)),
      };
    });
  } catch {
    return [];
  }
}

// Rebuild a URL by replacing its query string from the param rows
export function paramsToUrl(url: string, params: KVRow[]): string {
  const qIdx = url.indexOf("?");
  const base = qIdx === -1 ? url : url.slice(0, qIdx);
  const active = params.filter((p) => p.enabled && p.key.trim());
  if (active.length === 0) return base;
  const qs = active
    .map((p) => `${encodeURIComponent(p.key.trim())}=${encodeURIComponent(p.value)}`)
    .join("&");
  return `${base}?${qs}`;
}

export type TabAction =
  | { type: "SET_FIELD"; field: keyof TabState; value: TabState[keyof TabState] }
  | { type: "SET_URL"; url: string }
  | { type: "SET_PARAMS"; params: KVRow[] }
  | { type: "SET_HEADERS"; target: "req" | "res"; rows: KVRow[] }
  | { type: "SET_REQ_MODE"; mode: BodyMode }
  | { type: "SET_RES_MODE"; mode: BodyMode }
  | { type: "SET_ALL_RES_HEADERS_MOCKED"; mocked: boolean }
  | { type: "LOAD_ENTITY"; entity: SavedRequest | MockRule | null; tabType: TabType }
  | { type: "LOAD_DRAFT"; draft: RequestDraft | MockDraft | null; tabType: TabType }
  | { type: "REFRESH"; entity: SavedRequest | MockRule; tabType: TabType }
  | { type: "APPLY_CURL"; url: string; method: string; headers: Record<string, string>; body: string }
  | { type: "SEND_START" }
  | { type: "SEND_SUCCESS"; result: ReplayResult; resMode: BodyMode; durationMs?: number }
  | { type: "SEND_ERROR"; error: string }
  | { type: "TEST_START" }
  | { type: "TEST_SUCCESS"; resStatus: number; resHeaders: KVRow[]; resBody: string; resMode: BodyMode; resBodyEncoding?: "utf8" | "base64" }
  | { type: "TEST_ERROR"; error: string }
  | { type: "RUN_TESTS_START" }
  | { type: "RUN_TESTS_DONE"; results: { name: string; passed: boolean; error?: string; durationMs: number }[]; logs: string[] }
  | { type: "SAVE_START" }
  | { type: "SAVE_SUCCESS" }
  | { type: "SAVE_ERROR"; error: string }
  | { type: "RESET"; tabType: TabType };

// -- Default state ----------------------------------------------------------

function defaultState(): TabState {
  return {
    name: "", folderId: null,
    method: "GET", url: "",
    useRegex: false, regexError: "",
    reqTab: "params",
    reqParams: [], reqHeaders: [], reqBody: "", reqMode: "json", reqBodyStash: {},
    resTab: "body",
    resMode: "json",
    resStatus: 200, resStatusMocked: true, resHeaders: [], resBody: "", resBodyMocked: true,
    resDelay: 0, resDelayMocked: true,
    resBodyEncoding: "utf8",
    streamingMode: "none",
    streamingChunkDelay: 100,
    streamingChunkSeparator: "\n\n",
    preScript: "", postScript: "", testScript: "",
    curlInput: "", showCurl: false,
    loading: false, result: null, durationMs: null, sendErr: null, scriptErr: null,
    testResults: [], testLogs: [], testRunning: false,
    saving: false, saveErr: null,
    testLoading: false, testError: null,
  };
}

// -- Entity -> state helpers -------------------------------------------------

function entityFieldsFromRequest(req: Partial<SavedRequest>): Partial<TabState> {
  const url = req.url ?? "";
  const body = tryFormat(req.body ?? "");
  const mode = contentTypeToMode((req.headers ?? {})["content-type"]);
  return {
    name: req.name ?? "",
    method: req.method ?? "GET",
    url,
    folderId: req.folderId ?? null,
    reqParams: urlToParams(url),
    reqHeaders: headersToRows(req.headers ?? {}),
    reqBody: body,
    reqMode: mode,
    reqBodyStash: body ? { [mode]: body } : {},
    preScript: req.preScript ?? "",
    postScript: req.postScript ?? "",
    testScript: req.testScript ?? "",
  };
}

function buildMockReqHeaders(mock: Partial<MockRule>): KVRow[] {
  const filtered: Record<string, string> = {};
  for (const [k, v] of Object.entries(mock.capturedHeaders ?? {})) {
    if (!SKIP_CURL_HEADERS.has(k.toLowerCase())) filtered[k] = v;
  }
  return headersToRows(filtered);
}

function entityFieldsFromMock(mock: Partial<MockRule>): Partial<TabState> {
  const reqContentType = getHeaderCaseInsensitive(mock.capturedHeaders, "content-type");
  const resContentType = getHeaderCaseInsensitive(mock.responseHeaders, "content-type");
  const mockedHeaderKeys = new Set((mock.mockedResponseHeaders ?? []).map((key) => key.toLowerCase()));

  return {
    name: mock.name ?? "",
    method: mock.method ?? "*",
    url: mock.urlPattern ?? "",
    useRegex: mock.useRegex ?? false,
    folderId: mock.folderId ?? null,
    reqHeaders: buildMockReqHeaders(mock),
    reqBody: tryFormat(b64ToText(mock.capturedBody ?? "")),
    reqMode: contentTypeToMode(reqContentType),
    resStatus: mock.responseStatus ?? 200,
    resStatusMocked: mock.responseStatusMocked ?? true,
    resHeaders: headersToRows(mock.responseHeaders ?? { "content-type": "application/json" }, undefined, mockedHeaderKeys),
    resBody: mock.responseBodyEncoding === "base64" ? (mock.responseBody ?? "") : tryFormat(mock.responseBody ?? ""),
    resBodyMocked: mock.responseBodyMocked ?? true,
    resMode: contentTypeToMode(resContentType),
    resDelay: mock.responseDelay ?? 0,
    resDelayMocked: mock.responseDelayMocked ?? true,
    resBodyEncoding: mock.responseBodyEncoding ?? "utf8",
    streamingMode: mock.streamingMode ?? "none",
    streamingChunkDelay: mock.streamingChunkDelay ?? 100,
    streamingChunkSeparator: mock.streamingChunkSeparator ?? "\n\n",
  };
}

// -- Reducer ----------------------------------------------------------------

export function tabReducer(state: TabState, action: TabAction): TabState {
  switch (action.type) {

    case "SET_FIELD":
      return { ...state, [action.field]: action.value };

    // URL changed from the URL bar - re-derive params
    case "SET_URL":
      return { ...state, url: action.url, reqParams: urlToParams(action.url) };

    // Param rows edited - rebuild URL query string
    case "SET_PARAMS": {
      const newUrl = paramsToUrl(state.url, action.params);
      return { ...state, reqParams: action.params, url: newUrl };
    }

    case "SET_HEADERS":
      return action.target === "req"
        ? { ...state, reqHeaders: action.rows }
        : { ...state, resHeaders: action.rows };

    case "SET_ALL_RES_HEADERS_MOCKED":
      return {
        ...state,
        resHeaders: state.resHeaders.map((row) => ({ ...row, mocked: action.mocked })),
      };

    case "SET_REQ_MODE": {
      const ct = modeToContentType(action.mode);
      const withoutCT = state.reqHeaders.filter((r) => r.key.toLowerCase() !== "content-type");
      const reqHeaders = ct
        ? [{ id: mkRowId(), enabled: true, key: "content-type", value: ct }, ...withoutCT]
        : withoutCT;
      // Save current body to stash (skip stashing if switching away from none - it was empty)
      const stash = { ...state.reqBodyStash };
      if (state.reqMode !== "none") stash[state.reqMode] = state.reqBody;
      // Restore body for the incoming mode (none shows no body, so body = "")
      const reqBody = action.mode === "none" ? "" : (stash[action.mode] ?? "");
      return { ...state, reqMode: action.mode, reqHeaders, reqBody, reqBodyStash: stash };
    }

    case "SET_RES_MODE": {
      const ct = modeToContentType(action.mode);
      const withoutCT = state.resHeaders.filter((r) => r.key.toLowerCase() !== "content-type");
      const existingCt = state.resHeaders.find((r) => r.key.toLowerCase() === "content-type");
      const resHeaders = ct
        ? [{ id: mkRowId(), enabled: true, key: "content-type", value: ct, mocked: existingCt?.mocked ?? false }, ...withoutCT]
        : withoutCT;
      return { ...state, resMode: action.mode, resHeaders };
    }

    case "LOAD_ENTITY": {
      const base = defaultState();
      if (!action.entity) return base;
      const fields = action.tabType === "request"
        ? entityFieldsFromRequest(action.entity as SavedRequest)
        : entityFieldsFromMock(action.entity as MockRule);
      return { ...base, ...fields };
    }

    case "LOAD_DRAFT": {
      const base = defaultState();
      if (!action.draft) return base;
      if (action.tabType === "request") {
        const d = action.draft as RequestDraft;
        return {
          ...base,
          name: d.name, method: d.method, url: d.url, folderId: d.folderId,
          reqParams: urlToParams(d.url),
          reqHeaders: headersToRows(d.headers),
          reqBody: d.body, reqMode: d.reqMode,
          reqBodyStash: d.body && d.reqMode !== "none" ? { [d.reqMode]: d.body } : {},
          preScript: d.preScript, postScript: d.postScript, testScript: d.testScript,
        };
      } else {
        const d = action.draft as MockDraft;
        return {
          ...base,
          name: d.name, method: d.method, url: d.urlPattern,
          useRegex: d.useRegex, folderId: d.folderId,
          reqHeaders: headersToRows(d.reqHeaders),
          reqBody: d.reqBody, reqMode: d.reqMode,
          resStatus: d.resStatus, resStatusMocked: d.resStatusMocked ?? true,
          resHeaders: headersToRows(d.resHeaders, undefined, new Set((d.mockedResponseHeaders ?? []).map((key) => key.toLowerCase()))),
          resBody: d.resBody, resBodyMocked: d.resBodyMocked ?? true, resMode: d.resMode,
          resDelay: d.resDelay ?? 0,
          resDelayMocked: d.resDelayMocked ?? true,
          resBodyEncoding: d.resBodyEncoding ?? "utf8",
          streamingMode: d.streamingMode ?? "none",
          streamingChunkDelay: d.streamingChunkDelay ?? 100,
          streamingChunkSeparator: d.streamingChunkSeparator ?? "\n\n",
        };
      }
    }

    case "REFRESH": {
      // Update only entity-derived fields; preserve all runtime state
      const fields = action.tabType === "request"
        ? entityFieldsFromRequest(action.entity as SavedRequest)
        : entityFieldsFromMock(action.entity as MockRule);
      return { ...state, ...fields };
    }

    case "APPLY_CURL": {
      const headers = headersToRows(action.headers);
      const ct = action.headers["content-type"] ?? action.headers["Content-Type"];
      const reqMode = ct ? contentTypeToMode(ct) : state.reqMode;
      const newUrl = action.url || state.url;
      const hasBody = !!action.body;
      const hasHeaders = Object.keys(action.headers).length > 0;
      // Auto-switch to the most relevant tab after curl import
      const newTab = hasBody ? "body" : (hasHeaders ? "headers" : state.reqTab);
      return {
        ...state,
        url: newUrl,
        method: action.method || state.method,
        reqParams: urlToParams(newUrl),
        reqHeaders: hasHeaders ? headers : state.reqHeaders,
        reqBody: action.body ? tryFormat(action.body) : state.reqBody,
        reqMode,
        reqTab: newTab,
      };
    }

    case "SEND_START":
      return { ...state, loading: true, result: null, durationMs: null, sendErr: null, scriptErr: null };

    case "SEND_SUCCESS":
      return { ...state, loading: false, result: action.result, resMode: action.resMode, resTab: "body", durationMs: action.durationMs ?? null };

    case "SEND_ERROR":
      return { ...state, loading: false, sendErr: action.error };

    case "TEST_START":
      return { ...state, testLoading: true, testError: null };

    case "TEST_SUCCESS":
      return {
        ...state,
        testLoading: false,
        resStatus: action.resStatus,
        resHeaders: action.resHeaders,
        resBody: action.resBody,
        resStatusMocked: false,
        resBodyMocked: false,
        resDelayMocked: false,
        resMode: action.resMode,
        resBodyEncoding: action.resBodyEncoding ?? "utf8",
        resTab: "body",
      };

    case "TEST_ERROR":
      return { ...state, testLoading: false, testError: action.error };

    case "RUN_TESTS_START":
      return { ...state, testRunning: true, testResults: [], testLogs: [] };

    case "RUN_TESTS_DONE":
      return { ...state, testRunning: false, testResults: action.results, testLogs: action.logs };

    case "SAVE_START":
      return { ...state, saving: true, saveErr: null };

    case "SAVE_SUCCESS":
      return { ...state, saving: false };

    case "SAVE_ERROR":
      return { ...state, saving: false, saveErr: action.error };

    case "RESET":
      return { ...defaultState(), method: action.tabType === "mock" ? "*" : "GET" };

    default:
      return state;
  }
}

// -- initState --------------------------------------------------------------

export function initState(
  entity: SavedRequest | MockRule | Partial<SavedRequest> | Partial<MockRule> | null | undefined,
  draft: RequestDraft | MockDraft | null | undefined,
  tabType: TabType,
): TabState {
  const base = defaultState();
  if (tabType === "mock") base.method = "*";

  // Draft takes priority over entity
  if (draft) {
    const action: TabAction = { type: "LOAD_DRAFT", draft, tabType };
    return tabReducer(base, action);
  }
  if (entity) {
    const action: TabAction = { type: "LOAD_ENTITY", entity: entity as SavedRequest | MockRule, tabType };
    return tabReducer(base, action);
  }
  return base;
}

// -- stateToSavePayload -----------------------------------------------------

export function stateToSavePayload(
  state: TabState,
  tabType: TabType,
): Omit<SavedRequest, "id" | "createdAt" | "workspaceId"> | Omit<MockRule, "id" | "createdAt" | "workspaceId"> {
  if (tabType === "request") {
    return {
      name: state.name.trim(),
      method: state.method,
      url: state.url.trim(),
      headers: rowsToHeaders(state.reqHeaders),
      body: state.reqBody,
      preScript: state.preScript || undefined,
      postScript: state.postScript || undefined,
      testScript: state.testScript || undefined,
      folderId: state.folderId ?? null,
    };
  } else {
    return {
      name: state.name.trim(),
      method: state.method,
      urlPattern: state.url.trim(),
      useRegex: state.useRegex,
      enabled: true,
      capturedHeaders: rowsToHeaders(state.reqHeaders),
      capturedBody: textToB64(state.reqBody),
      responseStatus: state.resStatus,
      responseStatusMocked: state.resStatusMocked,
      responseHeaders: rowsToHeaders(state.resHeaders),
      mockedResponseHeaders: state.resHeaders.filter((row) => row.mocked && row.key.trim()).map((row) => row.key.trim()),
      responseBody: state.resBody,
      responseBodyMocked: state.resBodyMocked,
      responseBodyEncoding: state.resBodyEncoding !== "utf8" ? state.resBodyEncoding : undefined,
      responseDelay: state.resDelay > 0 ? state.resDelay : undefined,
      responseDelayMocked: state.resDelayMocked,
      streamingMode: state.streamingMode !== "none" ? state.streamingMode : undefined,
      streamingChunkDelay: state.streamingMode !== "none" ? state.streamingChunkDelay : undefined,
      streamingChunkSeparator: state.streamingMode === "chunked" ? state.streamingChunkSeparator : undefined,
      folderId: state.folderId ?? null,
    };
  }
}

// -- stateToDraft -----------------------------------------------------------

export function stateToDraft(state: TabState, tabType: TabType): RequestDraft | MockDraft {
  if (tabType === "request") {
    return {
      name: state.name, method: state.method, url: state.url, folderId: state.folderId,
      headers: rowsToHeaders(state.reqHeaders), body: state.reqBody, reqMode: state.reqMode,
      preScript: state.preScript, postScript: state.postScript, testScript: state.testScript,
    };
  } else {
    return {
      name: state.name, method: state.method, urlPattern: state.url,
      useRegex: state.useRegex, folderId: state.folderId,
      reqHeaders: rowsToHeaders(state.reqHeaders), reqBody: state.reqBody, reqMode: state.reqMode,
      resStatus: state.resStatus, resStatusMocked: state.resStatusMocked,
      resHeaders: rowsToHeaders(state.resHeaders), mockedResponseHeaders: state.resHeaders.filter((row) => row.mocked && row.key.trim()).map((row) => row.key.trim()), resBody: state.resBody, resBodyMocked: state.resBodyMocked, resMode: state.resMode,
      resDelay: state.resDelay, resDelayMocked: state.resDelayMocked, resBodyEncoding: state.resBodyEncoding,
      streamingMode: state.streamingMode,
      streamingChunkDelay: state.streamingChunkDelay,
      streamingChunkSeparator: state.streamingChunkSeparator,
    };
  }
}

// -- isDraftEmpty -----------------------------------------------------------

export function isDraftEmpty(state: TabState, tabType: TabType): boolean {
  if (tabType === "request") {
    return (
      !state.name.trim() && !state.url.trim() &&
      state.reqHeaders.filter((r) => r.enabled && r.key.trim()).length === 0 &&
      !state.reqBody.trim() && !state.preScript.trim() && !state.postScript.trim()
    );
  } else {
    return !state.name.trim() && !state.url.trim() && !state.resBody.trim();
  }
}
