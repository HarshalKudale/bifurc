import { SavedRequest, MockRule, ReplayResult } from "@/types";
import { KVRow, mkRowId, headersToRows, rowsToHeaders, tryFormat } from "@/lib/utils";
import { BodyMode, modeToContentType, contentTypeToMode } from "@/lib/bodyUtils";
import { urlToParams, paramsToUrl, entityFieldsFromRequest, entityFieldsFromMock } from "./restTabHelpers";

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

import { createTabReducer, CommonTabAction } from "@/lib/createTabReducer";

export type TabAction =
  | { type: "SET_URL"; url: string }
  | { type: "SET_PARAMS"; params: KVRow[] }
  | { type: "SET_HEADERS"; target: "req" | "res"; rows: KVRow[] }
  | { type: "SET_REQ_MODE"; mode: BodyMode }
  | { type: "SET_RES_MODE"; mode: BodyMode }
  | { type: "SET_ALL_RES_HEADERS_MOCKED"; mocked: boolean }
  | { type: "APPLY_CURL"; url: string; method: string; headers: Record<string, string>; body: string }
  | { type: "SEND_START" }
  | { type: "SEND_SUCCESS"; result: ReplayResult; resMode: BodyMode; durationMs?: number }
  | { type: "TEST_START" }
  | { type: "TEST_SUCCESS"; resStatus: number; resHeaders: KVRow[]; resBody: string; resMode: BodyMode; resBodyEncoding?: "utf8" | "base64" }
  | { type: "TEST_ERROR"; error: string }
  | { type: "RUN_TESTS_START" }
  | { type: "RUN_TESTS_DONE"; results: { name: string; passed: boolean; error?: string; durationMs: number }[]; logs: string[] }
  | { type: "RESET"; tabType: TabType }
  | CommonTabAction<TabState, SavedRequest | MockRule | null, RequestDraft | MockDraft | null>;

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

// -- Reducer ----------------------------------------------------------------

const baseTabReducer = (state: TabState, action: TabAction): TabState => {
  switch (action.type) {

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

    case "RESET":
      return { ...defaultState(), method: action.tabType === "mock" ? "*" : "GET" };

    default:
      return state;
  }
};

export const tabReducer = createTabReducer<TabState, TabAction, SavedRequest | MockRule | null, RequestDraft | MockDraft | null>({}, baseTabReducer);

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

export { stateToSavePayload, stateToDraft } from "./restTabHelpers";

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
