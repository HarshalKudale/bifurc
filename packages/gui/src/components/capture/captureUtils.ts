// The capture panel's **presentation** helpers.
//
// `plan/19` WI 5, item 2 split this file. The *constructing* half — `reqToHeadersBody` and
// `buildMockInitial`, which decided which headers to drop, how to decode a body and how a response is
// stored — moved into the engine (`packages/engine/src/utils/captureEntities.ts`) and is reached through
// `window.api.captureBuildRequest` / `captureBuildMock`. What is left is what the table and the detail pane
// actually render: base64/text plumbing, formatting, resource-type derivation and the fulfilled-by labels.
// Those are presentation, and the boundary ratchet's test — *would this differ if the UI were a terminal?* —
// answers no for every one of them.

import { RequestLogEntry } from "@/types";
import { EditorLanguage } from "@/components/common/CodeEditor";
import { formatBytes } from "@/lib/bodyUtils";
import { strings } from "@/lib/strings";

export type { EditorLanguage };

export type CaptureType = "xhr" | "doc" | "css" | "js" | "font" | "img" | "media" | "other";

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
    // One label for all three: a proxy rule, an RFC 6761 mapping and any other configured domain are
    // all "served by the proxy" from the user's side. They differ only in how the request was
    // addressed, which is what `ViaBadge`'s badge says — see `logEmitter`'s note on `via`.
    case "rule":
    case "rfc6761":
    case "mapping": return strings.capture.fulfilledProxy;
    case "mock": return strings.capture.fulfilledMock;
    case "error": return strings.capture.fulfilledError;
  }
}

export function fulfilledColor(via: RequestLogEntry["via"]): string {
  switch (via) {
    case "proxy": return "text-signal";
    case "rule":
    case "rfc6761":
    case "mapping": return "text-signal";
    case "mock": return "text-amber";
    case "error": return "text-destructive";
  }
}

// `reqToHeadersBody` and `buildMockInitial` lived here until 2026-09-30. They are `capturedToRequest` and
// `capturedToMock` in `packages/engine/src/utils/captureEntities.ts` now, reached through
// `window.api.captureBuildRequest` / `window.api.captureBuildMock` — see the header for why the split runs
// where it does.
