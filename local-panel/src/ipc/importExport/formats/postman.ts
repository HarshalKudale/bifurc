/**
 * Postman Collection v2.1 format helpers — pure conversion logic, no I/O.
 * Used by both exporters and importers.
 */

import { Folder, MockRule, SavedRequest } from "@/store/config";
import {
  PMHeader, PMUrl, PMBody, PMEvent, PMRequest, PMResponse, PMItem, PMCollection,
  mkId, urlRaw, hToRecord, bodyToText, eventsToScripts, recordToH,
  b64Decode, b64Encode, statusText, FNode, buildFolderTree
} from "./postmanHelpers";

// ── REQUESTS: export ───────────────────────────────────────────────────────

function scriptsToEvents(preScript?: string, postScript?: string): PMEvent[] {
  const events: PMEvent[] = [];
  if (preScript?.trim()) events.push({ listen: "prerequest", script: { type: "text/javascript", exec: preScript.split("\n") } });
  if (postScript?.trim()) events.push({ listen: "test", script: { type: "text/javascript", exec: postScript.split("\n") } });
  return events;
}

function reqToItem(r: SavedRequest): PMItem {
  const item: PMItem = {
    name: r.name || `${r.method} ${r.url}`,
    request: {
      method: r.method,
      header: recordToH(r.headers ?? {}),
      url: { raw: r.url },
    },
    response: [],
  };
  if (r.body?.trim()) item.request!.body = { mode: "raw", raw: r.body };
  const events = scriptsToEvents(r.preScript, r.postScript);
  if (events.length) item.event = events;
  return item;
}

function folderNodeToItems(node: FNode<SavedRequest>): PMItem[] {
  return [
    ...node.items.map(reqToItem),
    ...node.children.map((child): PMItem => ({
      name: child.folder!.name,
      item: folderNodeToItems(child),
    })),
  ];
}

export function exportRequestsToPostman(
  requests: SavedRequest[],
  folders: Folder[],
  name = "Local Panel Requests",
): string {
  const root = buildFolderTree(folders, requests);
  const col: PMCollection = {
    info: {
      name,
      _postman_id: mkId(),
      schema: "https://schema.getpostman.com/json/collection/v2.1.0/collection.json",
    },
    item: folderNodeToItems(root),
  };
  return JSON.stringify(col, null, 2);
}

// ── MOCKS: export ──────────────────────────────────────────────────────────

function mockToItem(m: MockRule): PMItem {
  const pmReq: PMRequest = {
    method: m.method === "*" ? "GET" : m.method,
    header: [],
    url: { raw: m.urlPattern },
  };
  const capturedBodyText = b64Decode(m.capturedBody ?? "");
  if (capturedBodyText.trim()) pmReq.body = { mode: "raw", raw: capturedBodyText };

  const pmRes: PMResponse = {
    name: m.name || `${m.method} ${m.urlPattern}`,
    originalRequest: pmReq,
    status: statusText(m.responseStatus),
    code: m.responseStatus,
    header: recordToH(m.responseHeaders ?? {}),
    body: m.responseBody,
  };

  return {
    name: m.name || `${m.method === "*" ? "ANY" : m.method} ${m.urlPattern}`,
    request: pmReq,
    response: [pmRes],
    _localpanel: {
      urlPattern: m.urlPattern,
      useRegex:   m.useRegex,
      enabled:    m.enabled,
    },
  };
}

function mockFolderNodeToItems(node: FNode<MockRule>): PMItem[] {
  return [
    ...node.items.map(mockToItem),
    ...node.children.map((child): PMItem => ({
      name: child.folder!.name,
      item: mockFolderNodeToItems(child),
    })),
  ];
}

export function exportMocksToPostman(
  mocks: MockRule[],
  folders: Folder[],
  name = "Local Panel Mocks",
): string {
  const root = buildFolderTree(folders, mocks);
  const col: PMCollection = {
    info: {
      name,
      _postman_id: mkId(),
      schema: "https://schema.getpostman.com/json/collection/v2.1.0/collection.json",
      description: "Local Panel mock rules. Each item's response[0] contains the mock response. _localpanel extension preserves regex/enabled state.",
    },
    item: mockFolderNodeToItems(root),
  };
  return JSON.stringify(col, null, 2);
}

// ── REQUESTS: import helpers (shared with importer) ───────────────────────

export interface ImportRequestItem {
  name: string;
  method: string;
  url: string;
  headers: Record<string, string>;
  body: string;
  preScript?: string;
  postScript?: string;
  folderId: string | null;
}

export interface ImportFolderItem {
  name: string;
  parentId: string | null;
}

export interface ParsedPostmanRequests {
  folders: ImportFolderItem[];
  requests: ImportRequestItem[];
}

export function parsePostmanRequests(jsonText: string): ParsedPostmanRequests {
  const col = JSON.parse(jsonText) as PMCollection;
  if (!col?.info?.schema?.includes("collection")) throw new Error("Not a Postman collection");

  const folders: ImportFolderItem[] = [];
  const requests: ImportRequestItem[] = [];

  function walk(items: PMItem[], parentName: string | null) {
    for (const item of items) {
      if (Array.isArray(item.item)) {
        folders.push({ name: item.name, parentId: parentName });
        walk(item.item, item.name);
      } else if (item.request) {
        const req = item.request;
        const { preScript, postScript } = eventsToScripts(item.event);
        requests.push({
          name: item.name,
          method: req.method?.toUpperCase() ?? "GET",
          url: urlRaw(req.url),
          headers: hToRecord(req.header),
          body: bodyToText(req.body),
          ...(preScript ? { preScript } : {}),
          ...(postScript ? { postScript } : {}),
          folderId: parentName,
        });
      }
    }
  }

  walk(col.item ?? [], null);
  return { folders, requests };
}

// ── MOCKS: import helpers ─────────────────────────────────────────────────

export interface ImportMockItem {
  name: string;
  method: string;
  urlPattern: string;
  useRegex: boolean;
  enabled: boolean;
  capturedHeaders: Record<string, string>;
  capturedBody: string;
  responseStatus: number;
  responseHeaders: Record<string, string>;
  responseBody: string;
  folderId: string | null;
}

export interface ParsedPostmanMocks {
  folders: ImportFolderItem[];
  mocks: ImportMockItem[];
}

export function parsePostmanMocks(jsonText: string): ParsedPostmanMocks {
  const col = JSON.parse(jsonText) as PMCollection;
  if (!col?.info?.schema?.includes("collection")) throw new Error("Not a Postman collection");

  const folders: ImportFolderItem[] = [];
  const mocks: ImportMockItem[] = [];

  function walk(items: PMItem[], parentName: string | null) {
    for (const item of items) {
      if (Array.isArray(item.item)) {
        folders.push({ name: item.name, parentId: parentName });
        walk(item.item, item.name);
      } else if (item.request) {
        const req = item.request;
        const lp = item._localpanel;
        const res = item.response?.[0];
        const urlPattern = lp?.urlPattern ?? urlRaw(req.url);
        const method = lp ? (req.method?.toUpperCase() ?? "*") : (req.method?.toUpperCase() ?? "GET");
        mocks.push({
          name: item.name,
          method: method || "*",
          urlPattern,
          useRegex: lp?.useRegex ?? false,
          enabled: lp?.enabled ?? false,
          capturedHeaders: hToRecord(req.header),
          capturedBody: b64Encode(req.body?.mode === "raw" ? (req.body.raw ?? "") : ""),
          responseStatus: res?.code ?? 200,
          responseHeaders: hToRecord(res?.header),
          responseBody: res?.body ?? "{}",
          folderId: parentName,
        });
      }
    }
  }

  walk(col.item ?? [], null);
  return { folders, mocks };
}
