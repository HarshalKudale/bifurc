import { AppConfig, Folder, ServiceInfo, SavedRequest, MockRule, ProxyRule, SavedWsConnection, SavedWebhook, Mapping } from "@/types";
import { Panel } from "@/lib/panelRegistry";
import { readStorage } from "@/lib/storage";
import { loadDraft } from "@/hooks/useDraftPersist";
import { SearchResultItem, SearchSection, SearchResult } from "./searchTypes";
import { getFolderName, matches } from "./searchHelpers";
import {
  searchRequests,
  searchMocks,
  searchRules,
  searchSockets,
  searchWebhooks,
  searchMappings,
  searchServices,
  searchEnvironments,
} from "./searchModules";
import { normalizePanel, getPanelDisplayName } from "./searchPanelUtils";

export * from "./searchTypes";
export * from "./searchPanelUtils";

export const RUNNER_PREFIX = "runner-";

/** Read open tabs for a specific panel */
export function getOpenTabs(panel: Panel): string[] {
  const norm = normalizePanel(panel);
  switch (norm) {
    case "requests":
      return readStorage<string[]>("requests:openTabs", []);
    case "mocks":
      return readStorage<string[]>("mocks:openTabs", []);
    case "sockets":
      return readStorage<string[]>("ws:openTabs", []);
    case "webhooks":
      return readStorage<string[]>("webhooks:openTabs", []);
    default:
      return [];
  }
}

export function searchEntities({
  query,
  mode,
  activePanel,
  config,
  services = [],
  maxPerSection = 8,
}: {
  query: string;
  mode: "current" | "global";
  activePanel: Panel;
  config: AppConfig;
  services?: ServiceInfo[];
  maxPerSection?: number;
}): SearchResult {
  const q = query.trim().toLowerCase();
  const normalizedActive = normalizePanel(activePanel);

  const openTabIdsByPanel: Record<string, Set<string>> = {
    requests: new Set(getOpenTabs("requests")),
    mocks: new Set(getOpenTabs("mocks")),
    sockets: new Set(getOpenTabs("sockets")),
    webhooks: new Set(getOpenTabs("webhooks")),
  };

  const isEntityOpen = (panel: string, id: string): boolean => openTabIdsByPanel[panel]?.has(id) ?? false;

  const reqFolders = [...(config.requestFolders ?? []), ...(config.graphqlRequestFolders ?? []), ...(config.grpcRequestFolders ?? []), ...(config.soapRequestFolders ?? [])];
  const allReqs: SavedRequest[] = [
    ...(config.requests ?? []),
    ...(config.graphqlRequests ?? []).map((r: any) => ({ ...r, method: "GQL" })),
    ...(config.grpcRequests ?? []).map((r: any) => ({ ...r, method: "GRPC" })),
    ...(config.soapRequests ?? []).map((r: any) => ({ ...r, method: "SOAP" })),
  ];
  const reqMap = new Map<string, SavedRequest>(allReqs.map((r) => [r.id, r]));

  const mockFolders = [...(config.mockFolders ?? []), ...(config.graphqlMockFolders ?? []), ...(config.grpcMockFolders ?? []), ...(config.soapMockFolders ?? [])];
  const allMocks: MockRule[] = [
    ...(config.mocks ?? []),
    ...(config.graphqlMocks ?? []).map((m: any) => ({ ...m, method: "GQL" })),
    ...(config.grpcMocks ?? []).map((m: any) => ({ ...m, method: "GRPC" })),
    ...(config.soapMocks ?? []).map((m: any) => ({ ...m, method: "SOAP" })),
  ];
  const mockMap = new Map<string, MockRule>(allMocks.map((m) => [m.id, m]));

  const ruleFolders = config.ruleFolders ?? [];
  const allRules: ProxyRule[] = config.proxyRules ?? [];

  const wsFolders = config.wsFolders ?? [];
  const allSockets: SavedWsConnection[] = config.wsConnections ?? [];
  const socketMap = new Map<string, SavedWsConnection>(allSockets.map((s) => [s.id, s]));

  const whFolders = config.webhookFolders ?? [];
  const allWebhooks: SavedWebhook[] = config.webhooks ?? [];
  const whMap = new Map<string, SavedWebhook>(allWebhooks.map((w) => [w.id, w]));

  const openTabsList: SearchResultItem[] = [];

  const processOpenTabsFor = (panelKey: "requests" | "mocks" | "sockets" | "webhooks") => {
    const tabIds = openTabIdsByPanel[panelKey];
    if (!tabIds) return;

    tabIds.forEach((tabId) => {
      let title = "Untitled";
      let subtitle: string | undefined;
      let method: string | undefined;
      let folderName: string | undefined;
      let isDraft = false;

      if (panelKey === "requests") {
        if (tabId.startsWith(RUNNER_PREFIX)) {
          const fId = tabId.slice(RUNNER_PREFIX.length);
          const folder = reqFolders.find((f) => f.id === fId);
          title = `Runner: ${folder?.name ?? "Collection"}`;
          method = "RUN";
        } else if (tabId.startsWith("req-draft-") || tabId.startsWith("pending-") || tabId.startsWith("gql-req-draft-") || tabId.startsWith("grpc-req-draft-") || tabId.startsWith("soap-req-draft-")) {
          isDraft = true;
          const draft = loadDraft<any>(tabId);
          title = draft?.name || "New Request";
          subtitle = draft?.url || draft?.path;
          method = draft?.method || (tabId.includes("gql") ? "GQL" : tabId.includes("grpc") ? "GRPC" : tabId.includes("soap") ? "SOAP" : "GET");
        } else {
          const req = reqMap.get(tabId);
          if (req) {
            title = req.name || "Untitled Request";
            subtitle = req.url || (req as any).path;
            method = req.method || "GET";
            folderName = getFolderName(req.folderId, reqFolders);
          } else {
            title = "Request Tab";
          }
        }
      } else if (panelKey === "mocks") {
        if (tabId.startsWith("mock-draft-") || tabId.startsWith("pending-mock-") || tabId.startsWith("gql-mock-draft-") || tabId.startsWith("grpc-mock-draft-") || tabId.startsWith("soap-mock-draft-")) {
          isDraft = true;
          const draft = loadDraft<any>(tabId);
          title = draft?.name || "New Mock";
          subtitle = draft?.path || draft?.urlPattern;
          method = draft?.method || (tabId.includes("gql") ? "GQL" : tabId.includes("grpc") ? "GRPC" : tabId.includes("soap") ? "SOAP" : "*");
        } else {
          const mock = mockMap.get(tabId);
          if (mock) {
            title = mock.name || "Untitled Mock";
            subtitle = mock.path || (mock as any).urlPattern;
            method = mock.method || "*";
            folderName = getFolderName(mock.folderId, mockFolders);
          } else {
            title = "Mock Tab";
          }
        }
      } else if (panelKey === "sockets") {
        if (tabId.startsWith("ws-draft-")) {
          isDraft = true;
          const draft = loadDraft<any>(tabId);
          title = draft?.name || "New WebSocket";
          subtitle = draft?.url;
          method = "WS";
        } else {
          const ws = socketMap.get(tabId);
          if (ws) {
            title = ws.name || "Untitled WebSocket";
            subtitle = ws.url;
            method = "WS";
            folderName = getFolderName(ws.folderId, wsFolders);
          } else {
            title = "WebSocket Tab";
          }
        }
      } else if (panelKey === "webhooks") {
        if (tabId.startsWith("wh-draft-")) {
          isDraft = true;
          const draft = loadDraft<any>(tabId);
          title = draft?.name || "New Webhook";
          subtitle = draft?.urlSuffix;
          method = "HOOK";
        } else {
          const wh = whMap.get(tabId);
          if (wh) {
            title = wh.name || "Untitled Webhook";
            subtitle = wh.urlSuffix;
            method = "HOOK";
            folderName = getFolderName(wh.folderId, whFolders);
          } else {
            title = "Webhook Tab";
          }
        }
      }

      if (matches(q, title, subtitle, folderName, method)) {
        openTabsList.push({
          id: tabId,
          tabId,
          title,
          subtitle,
          method,
          folderName,
          panel: panelKey,
          isOpenTab: true,
          isDraft,
          section: "Open Tabs",
          entityType: "tab",
        });
      }
    });
  };

  if (mode === "current") {
    if (["requests", "mocks", "rules", "sockets", "webhooks"].includes(normalizedActive)) {
      processOpenTabsFor(normalizedActive as any);
    }
  } else {
    processOpenTabsFor("requests");
    processOpenTabsFor("mocks");
    processOpenTabsFor("sockets");
    processOpenTabsFor("webhooks");
  }

  const sections: SearchSection[] = [];
  if (openTabsList.length > 0) sections.push({ title: "Open Tabs", items: openTabsList.slice(0, maxPerSection) });

  const addSection = (title: string, items: SearchResultItem[], limit = maxPerSection) => {
    if (items.length > 0) sections.push({ title, items: limit > 0 ? items.slice(0, limit) : items });
  };

  if (mode === "global" || normalizedActive === "requests") addSection("Requests", searchRequests(q, allReqs, reqFolders, isEntityOpen));
  if (mode === "global" || normalizedActive === "mocks") addSection("Mocks", searchMocks(q, allMocks, mockFolders, isEntityOpen));
  if (mode === "global" || normalizedActive === "rules") addSection("Proxy Rules", searchRules(q, allRules, ruleFolders, config));
  if (mode === "global" || normalizedActive === "sockets") addSection("WebSockets", searchSockets(q, allSockets, wsFolders, isEntityOpen));
  if (mode === "global" || normalizedActive === "webhooks") addSection("Webhooks", searchWebhooks(q, allWebhooks, whFolders, isEntityOpen));
  if (mode === "global" || normalizedActive === "mappings") addSection("Domain Mappings", searchMappings(q, config.mappings ?? []));
  if (mode === "global" || normalizedActive === "services") {
    const serviceLimit = normalizedActive === "services" ? 0 : Math.max(maxPerSection, 12);
    addSection("Services", searchServices(q, services, config.mappings ?? []), serviceLimit);
  }
  if (mode === "global" || normalizedActive === "environments") addSection("Environments", searchEnvironments(q, config.environments ?? []));

  const flatItems: SearchResultItem[] = [];
  sections.forEach((sec) => flatItems.push(...sec.items));

  return { sections, flatItems };
}
