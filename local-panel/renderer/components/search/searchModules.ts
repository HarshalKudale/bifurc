import { AppConfig, ServiceInfo, SavedRequest, MockRule, ProxyRule, SavedWsConnection, SavedWebhook, Mapping } from "@/types";
import { SearchResultItem } from "./searchTypes";
import { getFolderName, matches } from "./searchHelpers";

export function searchRequests(q: string, allReqs: SavedRequest[], reqFolders: any[], isEntityOpen: (p: string, id: string) => boolean): SearchResultItem[] {
  const items: SearchResultItem[] = [];
  allReqs.forEach((r) => {
    if (isEntityOpen("requests", r.id)) return;
    const folderName = getFolderName(r.folderId, reqFolders);
    const url = r.url || (r as any).path;
    if (matches(q, r.name, url, folderName, r.method)) {
      items.push({
        id: r.id,
        tabId: r.id,
        title: r.name || "Untitled Request",
        subtitle: url,
        method: r.method || "GET",
        folderName,
        panel: "requests",
        isOpenTab: false,
        section: "Requests",
        entityType: "request",
        original: r,
      });
    }
  });
  return items;
}

export function searchMocks(q: string, allMocks: MockRule[], mockFolders: any[], isEntityOpen: (p: string, id: string) => boolean): SearchResultItem[] {
  const items: SearchResultItem[] = [];
  allMocks.forEach((m) => {
    if (isEntityOpen("mocks", m.id)) return;
    const folderName = getFolderName(m.folderId, mockFolders);
    const path = m.path || (m as any).urlPattern;
    if (matches(q, m.name, path, folderName, m.method)) {
      items.push({
        id: m.id,
        tabId: m.id,
        title: m.name || "Untitled Mock",
        subtitle: path,
        method: m.method || "*",
        folderName,
        panel: "mocks",
        isOpenTab: false,
        section: "Mocks",
        entityType: "mock",
        original: m,
      });
    }
  });
  return items;
}

export function searchRules(q: string, allRules: ProxyRule[], ruleFolders: any[], config: AppConfig): SearchResultItem[] {
  const items: SearchResultItem[] = [];
  allRules.forEach((r) => {
    const folderName = getFolderName(r.folderId, ruleFolders);
    const targetStr = r.targetType === "mapping"
      ? (config.mappings?.find((m) => m.id === r.targetMappingId)?.domain || r.targetMappingId)
      : r.targetExternal;
    if (matches(q, r.name, r.pattern, targetStr, folderName)) {
      items.push({
        id: r.id,
        tabId: r.id,
        title: r.name || "Untitled Rule",
        subtitle: r.pattern,
        method: "RULE",
        folderName,
        panel: "rules",
        isOpenTab: false,
        section: "Proxy Rules",
        entityType: "rule",
        original: r,
      });
    }
  });
  return items;
}

export function searchSockets(q: string, allSockets: SavedWsConnection[], wsFolders: any[], isEntityOpen: (p: string, id: string) => boolean): SearchResultItem[] {
  const items: SearchResultItem[] = [];
  allSockets.forEach((s) => {
    if (isEntityOpen("sockets", s.id)) return;
    const folderName = getFolderName(s.folderId, wsFolders);
    if (matches(q, s.name, s.url, folderName)) {
      items.push({
        id: s.id,
        tabId: s.id,
        title: s.name || "Untitled WebSocket",
        subtitle: s.url,
        method: "WS",
        folderName,
        panel: "sockets",
        isOpenTab: false,
        section: "WebSockets",
        entityType: "websocket",
        original: s,
      });
    }
  });
  return items;
}

export function searchWebhooks(q: string, allWebhooks: SavedWebhook[], whFolders: any[], isEntityOpen: (p: string, id: string) => boolean): SearchResultItem[] {
  const items: SearchResultItem[] = [];
  allWebhooks.forEach((w) => {
    if (isEntityOpen("webhooks", w.id)) return;
    const folderName = getFolderName(w.folderId, whFolders);
    if (matches(q, w.name, w.urlSuffix, folderName)) {
      items.push({
        id: w.id,
        tabId: w.id,
        title: w.name || "Untitled Webhook",
        subtitle: `/localpanel/webhooks/${w.urlSuffix}`,
        method: "HOOK",
        folderName,
        panel: "webhooks",
        isOpenTab: false,
        section: "Webhooks",
        entityType: "webhook",
        original: w,
      });
    }
  });
  return items;
}

export function searchMappings(q: string, mappings: Mapping[]): SearchResultItem[] {
  const items: SearchResultItem[] = [];
  mappings.forEach((m) => {
    if (matches(q, m.subdomain, m.target, m.label)) {
      items.push({
        id: m.id,
        title: m.subdomain,
        subtitle: `→ ${m.target}${m.label ? ` (${m.label})` : ""}`,
        method: "MAP",
        panel: "mappings",
        isOpenTab: false,
        section: "Domain Mappings",
        entityType: "mapping",
        original: m,
      });
    }
  });
  return items;
}

export function searchServices(q: string, services: ServiceInfo[], mappings: Mapping[]): SearchResultItem[] {
  const items: SearchResultItem[] = [];
  const portToMapping = new Map(
    mappings.map((m: Mapping) => {
      const parts = m.target.split(":");
      const port = parseInt(parts[parts.length - 1], 10);
      return [port, m];
    })
  );

  services.forEach((s) => {
    const portStr = String(s.port);
    const pidStr = String(s.pid);
    const mapping = portToMapping.get(s.port);
    const isMapped = !!mapping;
    const mappingDisplay = mapping ? (mapping.label || mapping.subdomain || mapping.target) : "";

    if (matches(q, s.processName, portStr, pidStr, s.address, mappingDisplay)) {
      items.push({
        id: `svc-${s.port}-${s.pid}`,
        title: s.processName || `Service (Port ${s.port})`,
        subtitle: `${s.address}:${s.port} · PID ${s.pid}${isMapped ? ` · Mapped: ${mappingDisplay}` : " · Unmapped"}`,
        method: "SVC",
        panel: "services",
        isOpenTab: false,
        section: "Services",
        entityType: "service",
        original: s,
        isMapped,
        mappingLabel: mappingDisplay,
      });
    }
  });
  return items;
}

export function searchEnvironments(q: string, environments: any[]): SearchResultItem[] {
  const items: SearchResultItem[] = [];
  environments.forEach((env) => {
    const varKeys = Object.keys(env.variables ?? {}).join(" ");
    if (matches(q, env.name, varKeys)) {
      items.push({
        id: env.id,
        title: env.name,
        subtitle: `${Object.keys(env.variables ?? {}).length} variables`,
        method: "ENV",
        panel: "environments",
        isOpenTab: false,
        section: "Environments",
        entityType: "environment",
        original: env,
      });
    }
  });
  return items;
}
