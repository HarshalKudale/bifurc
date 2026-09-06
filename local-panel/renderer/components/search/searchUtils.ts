import { AppConfig, Folder, ServiceInfo, SavedRequest, MockRule, ProxyRule, SavedWsConnection, SavedWebhook, Mapping } from "@/types";
import { Panel } from "@/lib/panelRegistry";
import { readStorage } from "@/lib/storage";
import { loadDraft } from "@/lib/useDraftPersist";

export interface SearchResultItem {
  id: string;
  title: string;
  subtitle?: string;
  method?: string;
  panel: Panel;
  isOpenTab: boolean;
  tabId?: string;
  isDraft?: boolean;
  folderName?: string;
  section: string;
  entityType: "tab" | "request" | "mock" | "rule" | "websocket" | "webhook" | "mapping" | "service" | "environment";
  original?: any;
}

export interface SearchSection {
  title: string;
  items: SearchResultItem[];
}

export interface SearchResult {
  sections: SearchSection[];
  flatItems: SearchResultItem[];
}

const RUNNER_PREFIX = "runner-";

function getFolderName(folderId: string | null | undefined, folders: Folder[] = []): string | undefined {
  if (!folderId) return undefined;
  return folders.find((f) => f.id === folderId)?.name;
}

/** Check if a panel is tab-based */
export function isTabSupportedPanel(panel: Panel): boolean {
  return (
    panel === "requests" ||
    panel === "req-rest" ||
    panel === "req-graphql" ||
    panel === "req-soap" ||
    panel === "req-grpc" ||
    panel === "mocks" ||
    panel === "mock-rest" ||
    panel === "mock-graphql" ||
    panel === "mock-soap" ||
    panel === "mock-grpc" ||
    panel === "sockets" ||
    panel === "webhooks"
  );
}

/** Normalize panel id to base panel */
export function normalizePanel(panel: Panel): Panel {
  if (panel === "req-rest" || panel === "req-graphql" || panel === "req-soap" || panel === "req-grpc") {
    return "requests";
  }
  if (panel === "mock-rest" || panel === "mock-graphql" || panel === "mock-soap" || panel === "mock-grpc") {
    return "mocks";
  }
  return panel;
}

/** Friendly display name for a panel */
export function getPanelDisplayName(panel: Panel): string {
  switch (panel) {
    case "requests":
    case "req-rest":
    case "req-graphql":
    case "req-soap":
    case "req-grpc":
      return "Requests";
    case "mocks":
    case "mock-rest":
    case "mock-graphql":
    case "mock-soap":
    case "mock-grpc":
      return "Mocks";
    case "rules":
      return "Proxy Rules";
    case "sockets":
      return "WebSockets";
    case "webhooks":
      return "Webhooks";
    case "mappings":
      return "Mappings";
    case "services":
      return "Services";
    case "capture":
      return "Capture";
    case "environments":
      return "Environments";
    case "healthbar":
      return "Health Bar";
    case "settings":
      return "Settings";
    case "audit":
      return "Audit Log";
    case "workspace":
      return "Workspace";
    default:
      return panel;
  }
}

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

/** Match text helper (case-insensitive substring) */
function matches(query: string, ...fields: (string | undefined | null)[]): boolean {
  if (!query) return true;
  const q = query.toLowerCase().trim();
  return fields.some((f) => f && f.toLowerCase().includes(q));
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

  // Maps to check which entities are currently opened in tabs
  const openTabIdsByPanel: Record<string, Set<string>> = {
    requests: new Set(getOpenTabs("requests")),
    mocks: new Set(getOpenTabs("mocks")),
    sockets: new Set(getOpenTabs("sockets")),
    webhooks: new Set(getOpenTabs("webhooks")),
  };

  const isEntityOpen = (panel: string, id: string): boolean => {
    return openTabIdsByPanel[panel]?.has(id) ?? false;
  };

  // --- Collect Open Tabs ---
  const openTabsList: SearchResultItem[] = [];

  // Helper for requests
  const reqFolders = [
    ...(config.requestFolders ?? []),
    ...(config.graphqlRequestFolders ?? []),
    ...(config.grpcRequestFolders ?? []),
    ...(config.soapRequestFolders ?? []),
  ];

  // Request entities
  const allReqs: SavedRequest[] = [
    ...(config.requests ?? []),
    ...(config.graphqlRequests ?? []).map((r: any) => ({ ...r, method: "GQL" })),
    ...(config.grpcRequests ?? []).map((r: any) => ({ ...r, method: "GRPC" })),
    ...(config.soapRequests ?? []).map((r: any) => ({ ...r, method: "SOAP" })),
  ];
  const reqMap = new Map<string, SavedRequest>(allReqs.map((r) => [r.id, r]));

  // Mock entities
  const mockFolders = [
    ...(config.mockFolders ?? []),
    ...(config.graphqlMockFolders ?? []),
    ...(config.grpcMockFolders ?? []),
    ...(config.soapMockFolders ?? []),
  ];
  const allMocks: MockRule[] = [
    ...(config.mocks ?? []),
    ...(config.graphqlMocks ?? []).map((m: any) => ({ ...m, method: "GQL" })),
    ...(config.grpcMocks ?? []).map((m: any) => ({ ...m, method: "GRPC" })),
    ...(config.soapMocks ?? []).map((m: any) => ({ ...m, method: "SOAP" })),
  ];
  const mockMap = new Map<string, MockRule>(allMocks.map((m) => [m.id, m]));

  // Rules
  const ruleFolders = config.ruleFolders ?? [];
  const allRules: ProxyRule[] = config.proxyRules ?? [];
  const ruleMap = new Map<string, ProxyRule>(allRules.map((r) => [r.id, r]));

  // Sockets
  const wsFolders = config.wsFolders ?? [];
  const allSockets: SavedWsConnection[] = config.wsConnections ?? [];
  const socketMap = new Map<string, SavedWsConnection>(allSockets.map((s) => [s.id, s]));

  // Webhooks
  const whFolders = config.webhookFolders ?? [];
  const allWebhooks: SavedWebhook[] = config.webhooks ?? [];
  const whMap = new Map<string, SavedWebhook>(allWebhooks.map((w) => [w.id, w]));

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

  // Populate open tabs based on mode
  if (mode === "current") {
    if (
      normalizedActive === "requests" ||
      normalizedActive === "mocks" ||
      normalizedActive === "rules" ||
      normalizedActive === "sockets" ||
      normalizedActive === "webhooks"
    ) {
      processOpenTabsFor(normalizedActive as any);
    }
  } else {
    // Global mode: include open tabs from all tabbed panels
    processOpenTabsFor("requests");
    processOpenTabsFor("mocks");
    processOpenTabsFor("sockets");
    processOpenTabsFor("webhooks");
  }

  // --- Collect Unopened Saved Entities ---
  const sections: SearchSection[] = [];

  // Add Open Tabs section first if there are any matches
  if (openTabsList.length > 0) {
    sections.push({
      title: "Open Tabs",
      items: openTabsList.slice(0, maxPerSection),
    });
  }

  // Helper to add entity section
  const addSection = (title: string, items: SearchResultItem[]) => {
    if (items.length > 0) {
      sections.push({
        title,
        items: items.slice(0, maxPerSection),
      });
    }
  };

  // Check requests
  if (mode === "global" || normalizedActive === "requests") {
    const unopenedReqs: SearchResultItem[] = [];
    allReqs.forEach((r) => {
      // Exclude if already open in a tab
      if (isEntityOpen("requests", r.id)) return;
      const folderName = getFolderName(r.folderId, reqFolders);
      const url = r.url || (r as any).path;
      if (matches(q, r.name, url, folderName, r.method)) {
        unopenedReqs.push({
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
    addSection("Requests", unopenedReqs);
  }

  // Check mocks
  if (mode === "global" || normalizedActive === "mocks") {
    const unopenedMocks: SearchResultItem[] = [];
    allMocks.forEach((m) => {
      if (isEntityOpen("mocks", m.id)) return;
      const folderName = getFolderName(m.folderId, mockFolders);
      const path = m.path || (m as any).urlPattern;
      if (matches(q, m.name, path, folderName, m.method)) {
        unopenedMocks.push({
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
    addSection("Mocks", unopenedMocks);
  }

  // Check rules
  if (mode === "global" || normalizedActive === "rules") {
    const matchingRules: SearchResultItem[] = [];
    allRules.forEach((r) => {
      const folderName = getFolderName(r.folderId, ruleFolders);
      const targetStr = r.targetType === "mapping"
        ? (config.mappings?.find((m) => m.id === r.targetMappingId)?.domain || r.targetMappingId)
        : r.targetExternal;
      if (matches(q, r.name, r.pattern, targetStr, folderName)) {
        matchingRules.push({
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
    addSection("Proxy Rules", matchingRules);
  }

  // Check sockets
  if (mode === "global" || normalizedActive === "sockets") {
    const unopenedSockets: SearchResultItem[] = [];
    allSockets.forEach((s) => {
      if (isEntityOpen("sockets", s.id)) return;
      const folderName = getFolderName(s.folderId, wsFolders);
      if (matches(q, s.name, s.url, folderName)) {
        unopenedSockets.push({
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
    addSection("WebSockets", unopenedSockets);
  }

  // Check webhooks
  if (mode === "global" || normalizedActive === "webhooks") {
    const unopenedWebhooks: SearchResultItem[] = [];
    allWebhooks.forEach((w) => {
      if (isEntityOpen("webhooks", w.id)) return;
      const folderName = getFolderName(w.folderId, whFolders);
      if (matches(q, w.name, w.urlSuffix, folderName)) {
        unopenedWebhooks.push({
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
    addSection("Webhooks", unopenedWebhooks);
  }

  // Check mappings
  if (mode === "global" || normalizedActive === "mappings") {
    const mappingItems: SearchResultItem[] = [];
    (config.mappings ?? []).forEach((m: Mapping) => {
      if (matches(q, m.subdomain, m.target, m.label)) {
        mappingItems.push({
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
    addSection("Domain Mappings", mappingItems);
  }

  // Check services
  if (mode === "global" || normalizedActive === "services") {
    const serviceItems: SearchResultItem[] = [];
    services.forEach((s) => {
      const portStr = String(s.port);
      if (matches(q, s.name, s.command, portStr)) {
        serviceItems.push({
          id: `svc-${s.port}-${s.name}`,
          title: s.name,
          subtitle: `Port ${s.port} · PID ${s.pid}`,
          method: "SVC",
          panel: "services",
          isOpenTab: false,
          section: "Services",
          entityType: "service",
          original: s,
        });
      }
    });
    addSection("Services", serviceItems);
  }

  // Check environments
  if (mode === "global" || normalizedActive === "environments") {
    const envItems: SearchResultItem[] = [];
    (config.environments ?? []).forEach((env) => {
      const varKeys = Object.keys(env.variables ?? {}).join(" ");
      if (matches(q, env.name, varKeys)) {
        envItems.push({
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
    addSection("Environments", envItems);
  }

  // Flatten items for arrow key navigation
  const flatItems: SearchResultItem[] = [];
  sections.forEach((sec) => {
    flatItems.push(...sec.items);
  });

  return { sections, flatItems };
}
