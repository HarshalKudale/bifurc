import { SavedRequest, MockRule } from "@/types";
import {
  KVRow, mkRowId, headersToRows, rowsToHeaders,
  b64ToText, textToB64, tryFormat,
} from "@/lib/utils";
import { contentTypeToMode } from "@/lib/bodyUtils";
import { SKIP_CURL_HEADERS } from "@/lib/curlParser";
import { TabState, RequestDraft, MockDraft, TabType } from "./restTabReducer";

// -- Helper for case-insensitive header lookup -----------------------------
function getHeaderCaseInsensitive(headers: Record<string, string> | undefined, key: string): string | undefined {
  if (!headers) return undefined;
  const lowerKey = key.toLowerCase();
  for (const [k, v] of Object.entries(headers)) {
    if (k.toLowerCase() === lowerKey) return v;
  }
  return undefined;
}

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

// -- Entity -> state helpers -------------------------------------------------

export function entityFieldsFromRequest(req: Partial<SavedRequest>): Partial<TabState> {
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

export function entityFieldsFromMock(mock: Partial<MockRule>): Partial<TabState> {
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
