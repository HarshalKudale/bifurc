// Shared renderer utilities

export interface KVRow {
  id: string;
  enabled: boolean;
  key: string;
  value: string;
  mocked?: boolean;
}

let _rid = 0;
export const mkRowId = () => `r${++_rid}`;

export function headersToRows(h: Record<string, string>, skip?: Set<string>, mockedKeys?: Set<string>): KVRow[] {
  return Object.entries(h)
    .filter(([k]) => !skip || !skip.has(k.toLowerCase()))
    .map(([key, value]) => ({
      id: mkRowId(),
      enabled: true,
      key,
      value,
      mocked: mockedKeys ? mockedKeys.has(key.toLowerCase()) : undefined,
    }));
}

export function rowsToHeaders(rows: KVRow[]): Record<string, string> {
  const out: Record<string, string> = {};
  for (const r of rows) {
    if (r.enabled && r.key.trim()) out[r.key.trim()] = r.value;
  }
  return out;
}

export function b64ToText(b64: string): string {
  if (!b64) return "";
  try {
    const bytes = Uint8Array.from(atob(b64), (c) => c.charCodeAt(0));
    return new TextDecoder("utf-8", { fatal: false }).decode(bytes);
  } catch { return ""; }
}

export function textToB64(text: string): string {
  if (!text.trim()) return "";
  try {
    const bytes = new TextEncoder().encode(text);
    let bin = "";
    bytes.forEach((b) => (bin += String.fromCharCode(b)));
    return btoa(bin);
  } catch { return ""; }
}

export function tryFormat(text: string): string {
  if (!text.trim()) return text;
  try { return JSON.stringify(JSON.parse(text), null, 2); } catch { return text; }
}

export function statusColor(code: number): string {
  if (code < 300) return "var(--c-success)";
  if (code < 400) return "var(--c-amber)";
  return "var(--c-destructive)";
}

export const METHODS = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];

export const MOCK_METHODS = ["*", ...METHODS];

export const METHOD_HEX: Record<string, string> = {
  "*": "var(--c-muted-foreground)",
  GET: "var(--c-method-get)",
  POST: "var(--c-method-post)",
  PUT: "var(--c-method-put)",
  PATCH: "var(--c-method-patch)",
  DELETE: "var(--c-method-delete)",
  HEAD: "var(--c-violet)",
  OPTIONS: "var(--c-violet)",
  WS: "var(--c-method-ws)",
  GQL: "#d946ef",
  GRPC: "#06b6d4",
  SOAP: "#14b8a6",
};

// Semi-transparent backgrounds for method badges (12% washes per Signal spec)
const METHOD_BG: Record<string, string> = {
  "*": "oklch(var(--muted-foreground) / 0.13)",
  GET: "oklch(var(--method-get) / 0.13)",
  POST: "oklch(var(--method-post) / 0.13)",
  PUT: "oklch(var(--method-put) / 0.13)",
  PATCH: "oklch(var(--method-patch) / 0.13)",
  DELETE: "oklch(var(--method-delete) / 0.13)",
  HEAD: "oklch(var(--violet) / 0.13)",
  OPTIONS: "oklch(var(--violet) / 0.13)",
  WS: "oklch(var(--method-ws) / 0.13)",
  GQL: "rgba(217, 70, 239, 0.15)",
  GRPC: "rgba(6, 182, 212, 0.15)",
  SOAP: "rgba(20, 184, 166, 0.15)",
};

export const methodColor = (m: string) => METHOD_HEX[m.toUpperCase()] ?? "var(--c-muted-foreground)";
export const methodBg = (m: string) => METHOD_BG[m.toUpperCase()] ?? "oklch(var(--muted-foreground) / 0.13)";

/** CSS class for the tinted-wash HTTP method badge (Signal spec). */
export function methodBadgeClass(m: string): string {
  switch (m.toUpperCase()) {
    case "GET": return "method-badge method-get";
    case "POST": return "method-badge method-post";
    case "PUT": return "method-badge method-put";
    case "PATCH": return "method-badge method-patch";
    case "DELETE": return "method-badge method-delete";
    case "WS": return "method-badge method-ws";
    default: return "method-badge method-other";
  }
}

// -- Entity file path helpers (mirrors workspaceFs.ts logic, renderer-side) -----

/**
 * The directory-name sanitiser the **storage layer** uses — `strip`, not `replace`.
 *
 * Exported because anything that needs to name a folder's path on disk has to agree with it, and two
 * things now do: `entityRelPath()` (above) and the folder git-status rollup in
 * `components/sidebar/FolderTree.tsx`. The engine's `workspaceFs.sanitizeDirName` is the authority and
 * this is its mirror; the engine's **`folder.publish`** used to carry a *third*, different one
 * (`[<>:"/\\|?*]+` → `"-"`), which is why a folder whose name contains a colon could never be
 * published — the pathspec it built pointed at a directory that does not exist. That is fixed at the
 * source (`sync/syncCommands.ts` now uses the storage sanitiser); do not reintroduce a local variant.
 */
export function sanitizeDirName(name: string): string {
  return name.replace(/[<>:"/\\|?*\x00-\x1f]/g, "").trim() || "unnamed";
}

type HasFolderId = { id: string; folderId?: string | null };
type FolderLike = { id: string; name: string };

/** Compute the relative git path for a folder-based entity (mocks, requests, sockets, rules, graphql, soap, grpc, etc.). */
export function entityRelPath(
  kind: "mocks" | "requests" | "sockets" | "webhooks" | "rules" | "graphqlRequests" | "soapRequests" | "grpcRequests" | string,
  entity: HasFolderId,
  folders: FolderLike[],
): string {
  const folder = entity.folderId ? folders.find((f) => f.id === entity.folderId) : null;
  if (folder) return `${kind}/${sanitizeDirName(folder.name)}/${entity.id}.json`;
  return `${kind}/${entity.id}.json`;
}

/** Compute the relative git path for a flat entity (mappings, environments). */
export function flatEntityRelPath(kind: "mappings" | "environments", id: string): string {
  return `${kind}/${id}.json`;
}

const FIELD_LABELS: Record<string, string> = {
  name: "name", domain: "domain", target: "target", enabled: "enabled",
  pattern: "pattern", method: "method", urlPattern: "url pattern",
  responseStatus: "status", responseBody: "body", responseHeaders: "headers",
  capturedBody: "req body", capturedHeaders: "req headers",
  url: "url", headers: "headers", body: "body",
  variables: "variables", useRegex: "regex",
};

/** Turn a camelCase field key into a human-readable short label. */
export function formatFieldLabel(key: string): string {
  if (FIELD_LABELS[key]) return FIELD_LABELS[key];
  return key.replace(/([A-Z])/g, " $1").toLowerCase().trim();
}

/** 
 * Calculate folder status based on items with isEnabled or enabled property.
 * Returns "enabled" if all items are enabled, "disabled" if all are disabled, "mixed" otherwise.
 * Only returns status for folders that contain enableable items.
 */
export type FolderStatus = "enabled" | "mixed" | "disabled";

export function calculateFolderStatus<T extends { folderId?: string | null; isEnabled?: boolean; enabled?: boolean }>(
  items: T[],
  folders: { id: string; parentId?: string | null }[]
): Record<string, FolderStatus> {
  const result: Record<string, FolderStatus> = {};

  // Get all descendant folder IDs for a given folder (including itself)
  const getDescendantFolderIds = (folderId: string): string[] => {
    const descendants = [folderId];
    const children = folders.filter((f) => f.parentId === folderId);
    for (const child of children) {
      descendants.push(...getDescendantFolderIds(child.id));
    }
    return descendants;
  };

  // Calculate status for each folder
  for (const folder of folders) {
    const descendantFolderIds = new Set(getDescendantFolderIds(folder.id));

    // Get all items in this folder and its descendants
    const folderItems = items.filter((item) => {
      const itemFolderId = item.folderId ?? null;
      return itemFolderId === folder.id || (itemFolderId && descendantFolderIds.has(itemFolderId));
    });

    // Only calculate status if items have isEnabled or enabled property
    const enableableItems = folderItems.filter((item) =>
      item.isEnabled !== undefined || item.enabled !== undefined
    );
    if (enableableItems.length === 0) continue;

    // Check enabled status (support both isEnabled and enabled properties)
    const getEnabledStatus = (item: T): boolean => {
      if (item.isEnabled !== undefined) return item.isEnabled;
      if (item.enabled !== undefined) return item.enabled;
      return false;
    };

    const enabledCount = enableableItems.filter((item) => getEnabledStatus(item) === true).length;
    const disabledCount = enableableItems.filter((item) => getEnabledStatus(item) === false).length;

    if (enabledCount === enableableItems.length) {
      result[folder.id] = "enabled";
    } else if (disabledCount === enableableItems.length) {
      result[folder.id] = "disabled";
    } else {
      result[folder.id] = "mixed";
    }
  }

  return result;
}
