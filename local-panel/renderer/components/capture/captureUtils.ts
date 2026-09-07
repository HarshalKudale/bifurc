import { RequestLogEntry, MockRule, SavedRequest } from "@/types";
import { EditorLanguage } from "@/components/common/CodeEditor";
import { isBinaryContentType, formatBytes } from "@/lib/bodyUtils";
import { strings } from "@/lib/strings";

export type { EditorLanguage };

export type CaptureType = "xhr" | "doc" | "css" | "js" | "font" | "img" | "media" | "other";

export type RequestPayload = Omit<SavedRequest, "id" | "createdAt" | "workspaceId">;

// Headers stripped when replaying / saving a captured request as an editable request.
const SKIP_HEADERS = new Set(["host", "proxy-connection", "connection", "content-length", "transfer-encoding"]);

export function b64ToText(b64: string): string {
  if (!b64) return "";
  try {
    const bytes = Uint8Array.from(atob(b64), (c) => c.charCodeAt(0));
    return new TextDecoder("utf-8", { fatal: false }).decode(bytes);
  } catch { return b64; }
}

export function tryFormat(text: string): string {
  try { return JSON.stringify(JSON.parse(text), null, 2); } catch { return text; }
}

export function ctToLang(ct: string): EditorLanguage {
  if (ct.includes("json")) return "json";
  if (ct.includes("html")) return "html";
  if (ct.includes("xml")) return "xml";
  if (ct.includes("javascript") || ct.includes("ecmascript")) return "javascript";
  return "text";
}

export function statusColor(s: number | null): string {
  if (s === null) return "text-muted-foreground";
  if (s < 300) return "text-signal";
  if (s < 400) return "text-amber";
  return "text-destructive";
}

export function fmtTime(ts: number): string {
  const d = new Date(ts);
  return d.toTimeString().slice(0, 8) + "." + String(d.getMilliseconds()).padStart(3, "0");
}

export function fmtDur(ms: number | null): string {
  if (ms === null) return "—";
  if (ms < 1000) return `${ms}ms`;
  return `${(ms / 1000).toFixed(1)}s`;
}

/** Case-insensitive header lookup. */
export function getHeader(headers: Record<string, string>, name: string): string {
  const lower = name.toLowerCase();
  for (const [k, v] of Object.entries(headers)) {
    if (k.toLowerCase() === lower) return v;
  }
  return "";
}

/** Decoded size of a base64 response body, formatted (e.g. "1.2 KB"). */
export function resBodySize(entry: RequestLogEntry): string {
  const b64 = entry.resBody;
  if (!b64) return "—";
  const padding = b64.endsWith("==") ? 2 : b64.endsWith("=") ? 1 : 0;
  const bytes = Math.max(0, Math.floor(b64.length * 3 / 4) - padding);
  return formatBytes(bytes);
}

/** URL path for the Name column, with a raw-string fallback for odd proxied URLs. */
export function urlName(url: string): string {
  try {
    const u = new URL(url);
    return (u.pathname || "/") + u.search;
  } catch {
    return url;
  }
}

function extOf(url: string): string {
  let path = url;
  try { path = new URL(url).pathname; } catch { /* raw fallback */ }
  const lastSeg = path.split("/").pop() ?? "";
  const dot = lastSeg.lastIndexOf(".");
  return dot === -1 ? "" : lastSeg.slice(dot + 1).toLowerCase();
}

const EXT_TYPE: Record<string, CaptureType> = {
  html: "doc", htm: "doc",
  css: "css",
  js: "js", mjs: "js", cjs: "js", ts: "js",
  woff: "font", woff2: "font", ttf: "font", otf: "font", eot: "font",
  png: "img", jpg: "img", jpeg: "img", gif: "img", webp: "img", svg: "img", ico: "img", bmp: "img", avif: "img",
  mp3: "media", wav: "media", ogg: "media", mp4: "media", webm: "media", mov: "media",
  json: "xhr", xml: "xhr",
};

/** DevTools-style resource type derived from content-type, then URL extension. */
export function deriveType(entry: RequestLogEntry): CaptureType {
  const ct = getHeader(entry.resHeaders, "content-type").toLowerCase().split(";")[0].trim();
  if (ct.includes("html")) return "doc";
  if (ct.includes("css")) return "css";
  if (ct.includes("javascript") || ct.includes("ecmascript")) return "js";
  if (ct.startsWith("font/") || ct.includes("font-woff") || ct.includes("application/font")) return "font";
  if (ct.startsWith("image/")) return "img";
  if (ct.startsWith("audio/") || ct.startsWith("video/")) return "media";
  if (ct.includes("json") || ct.includes("xml") || ct.includes("text/plain")
    || ct.includes("x-www-form-urlencoded") || ct.includes("grpc")) return "xhr";

  const ext = extOf(entry.url);
  if (ext && EXT_TYPE[ext]) return EXT_TYPE[ext];
  return "other";
}

export function fulfilledBy(via: RequestLogEntry["via"]): string {
  switch (via) {
    case "proxy": return strings.capture.fulfilledExternal;
    case "rule":
    case "rfc6761": return strings.capture.fulfilledProxy;
    case "mock": return strings.capture.fulfilledMock;
    case "error": return strings.capture.fulfilledError;
  }
}

export function fulfilledColor(via: RequestLogEntry["via"]): string {
  switch (via) {
    case "proxy": return "text-signal";
    case "rule":
    case "rfc6761": return "text-signal";
    case "mock": return "text-amber";
    case "error": return "text-destructive";
  }
}

/** Build a SavedRequest payload from a captured entry (used by open/save paths). */
export function reqToHeadersBody(entry: RequestLogEntry): RequestPayload {
  const headers: Record<string, string> = {};
  for (const [k, v] of Object.entries(entry.reqHeaders)) {
    if (!SKIP_HEADERS.has(k.toLowerCase())) headers[k] = v;
  }
  return { name: "", method: entry.method, url: entry.url, headers, body: b64ToText(entry.reqBody) };
}

/** Build a MockRule payload from a captured entry's request/response. */
export function buildMockInitial(entry: RequestLogEntry): Partial<MockRule> {
  const resCt = getHeader(entry.resHeaders, "content-type").toLowerCase();
  const isBinaryRes = isBinaryContentType(resCt);
  return {
    name: "",
    method: entry.method,
    urlPattern: entry.url,
    useRegex: false,
    capturedHeaders: entry.reqHeaders,
    capturedBody: entry.reqBody,
    responseStatus: entry.resStatus ?? 200,
    responseStatusMocked: true,
    responseHeaders: entry.resHeaders,
    mockedResponseHeaders: [],
    responseBody: isBinaryRes ? entry.resBody : (b64ToText(entry.resBody) || "{}"),
    responseBodyMocked: true,
    responseBodyEncoding: isBinaryRes ? "base64" : undefined,
    responseDelayMocked: true,
  };
}
