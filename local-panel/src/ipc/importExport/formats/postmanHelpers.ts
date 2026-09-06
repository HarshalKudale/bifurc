import { Folder } from "@/store/config";

// ── Postman v2.1 type stubs ────────────────────────────────────────────────

export interface PMHeader  { key: string; value: string; type?: string; disabled?: boolean }
export interface PMUrl     { raw: string }
export interface PMBody {
  mode: string;
  raw?: string;
  urlencoded?: Array<{ key: string; value: string; disabled?: boolean }>;
  formdata?: Array<{ key: string; value: string; type?: string; disabled?: boolean }>;
  graphql?: { query?: string; variables?: string };
}
export interface PMEvent {
  listen: string;
  script?: { type?: string; exec?: string[] };
}

export interface PMRequest {
  method: string;
  header: PMHeader[] | string;
  body?: PMBody;
  url: PMUrl | string;
}

export interface PMResponse {
  name:            string;
  originalRequest?: PMRequest;
  status?:         string;
  code?:           number;
  header?:         PMHeader[] | string;
  body?:           string;
}

export interface PMItem {
  name:      string;
  item?:     PMItem[];
  request?:  PMRequest;
  response?: PMResponse[];
  event?:    PMEvent[];
  _localpanel?: {
    urlPattern: string;
    useRegex:   boolean;
    enabled:    boolean;
  };
}

export interface PMCollection {
  info: {
    name:         string;
    _postman_id?: string;
    schema:       string;
    description?: string;
  };
  item: PMItem[];
}

// ── Helpers ────────────────────────────────────────────────────────────────

export function mkId(): string {
  return Date.now().toString(36) + Math.random().toString(36).slice(2, 7);
}

export function urlRaw(u: PMUrl | string): string {
  return typeof u === "string" ? u : (u.raw ?? "");
}

export function hToRecord(hs: PMHeader[] | string | undefined): Record<string, string> {
  if (!hs || typeof hs === "string") return {};
  const out: Record<string, string> = {};
  for (const h of hs) if (!h.disabled && h.key) out[h.key] = h.value ?? "";
  return out;
}

export function bodyToText(body: PMBody | undefined): string {
  if (!body) return "";
  if (body.mode === "raw") return body.raw ?? "";
  if (body.mode === "urlencoded" && Array.isArray(body.urlencoded)) {
    return body.urlencoded
      .filter((p) => !p.disabled && p.key)
      .map((p) => `${encodeURIComponent(p.key)}=${encodeURIComponent(p.value ?? "")}`)
      .join("&");
  }
  if (body.mode === "formdata" && Array.isArray(body.formdata)) {
    return body.formdata
      .filter((p) => !p.disabled && p.key)
      .map((p) => `${p.key}: ${p.value ?? ""}`)
      .join("\n");
  }
  if (body.mode === "graphql" && body.graphql) {
    const parts: Record<string, string | undefined> = {};
    if (body.graphql.query) parts.query = body.graphql.query;
    if (body.graphql.variables) parts.variables = body.graphql.variables;
    return JSON.stringify(parts, null, 2);
  }
  return "";
}

export function eventsToScripts(events: PMEvent[] | undefined): { preScript: string; postScript: string } {
  let preScript = "";
  let postScript = "";
  if (!events) return { preScript, postScript };
  for (const ev of events) {
    const src = ev.script?.exec?.join("\n") ?? "";
    if (ev.listen === "prerequest") preScript = src;
    else if (ev.listen === "test") postScript = src;
  }
  return { preScript, postScript };
}

export function recordToH(r: Record<string, string>): PMHeader[] {
  return Object.entries(r).map(([key, value]) => ({ key, value, type: "text" }));
}

export function b64Decode(b64: string): string {
  if (!b64) return "";
  try {
    return Buffer.from(b64, "base64").toString("utf-8");
  } catch { return ""; }
}

export function b64Encode(text: string): string {
  if (!text.trim()) return "";
  try {
    return Buffer.from(text, "utf-8").toString("base64");
  } catch { return ""; }
}

export function statusText(code: number): string {
  const m: Record<number, string> = {
    200: "OK", 201: "Created", 204: "No Content",
    301: "Moved Permanently", 302: "Found", 304: "Not Modified",
    400: "Bad Request", 401: "Unauthorized", 403: "Forbidden",
    404: "Not Found", 405: "Method Not Allowed", 409: "Conflict",
    422: "Unprocessable Entity", 429: "Too Many Requests",
    500: "Internal Server Error", 502: "Bad Gateway", 503: "Service Unavailable",
  };
  return m[code] ?? "Unknown";
}

// ── Folder-tree builder ────────────────────────────────────────────────────

export interface FNode<T> {
  folder: Folder | null;
  children: FNode<T>[];
  items: T[];
}

export function buildFolderTree<T extends { folderId?: string | null }>(
  folders: Folder[],
  items: T[],
): FNode<T> {
  const nodeMap = new Map<string | null, FNode<T>>();
  nodeMap.set(null, { folder: null, children: [], items: [] });
  for (const f of folders) nodeMap.set(f.id, { folder: f, children: [], items: [] });
  for (const f of folders) {
    const parent = nodeMap.get(f.parentId ?? null) ?? nodeMap.get(null)!;
    parent.children.push(nodeMap.get(f.id)!);
  }
  for (const item of items) {
    const node = nodeMap.get(item.folderId ?? null) ?? nodeMap.get(null)!;
    node.items.push(item);
  }
  return nodeMap.get(null)!;
}
