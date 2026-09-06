import { beforeEach, describe, expect, it } from "vitest";
import { searchEntities, getOpenTabs } from "@/components/search/searchUtils";
import { AppConfig, SavedRequest, MockRule, ProxyRule, WebSocketConnection, WebhookEndpoint } from "@/types";
import { writeStorage } from "@/lib/storage";

class MockLocalStorage {
  private store: Record<string, string> = {};

  getItem(key: string): string | null {
    return this.store[key] ?? null;
  }

  setItem(key: string, value: string): void {
    this.store[key] = value;
  }

  removeItem(key: string): void {
    delete this.store[key];
  }

  clear(): void {
    this.store = {};
  }
}

const mockStorage = new MockLocalStorage();
(globalThis as unknown as { localStorage: MockLocalStorage }).localStorage = mockStorage;

const mockConfig: AppConfig = {
  port: 80,
  companionPort: 9271,
  minimizeToTray: true,
  tlsEnabled: false,
  tlsCaCertPath: null,
  tlsCaKeyPath: null,
  workspaces: [],
  activeWorkspaceId: "ws-1",
  mappings: [
    { id: "map-1", workspaceId: "ws-1", pattern: "api.example.com", target: "http://localhost:3000", enabled: true },
  ],
  proxyRules: [
    { id: "rule-1", workspaceId: "ws-1", name: "Inject Auth Header", enabled: true, priority: 1, action: "modify-headers" } as ProxyRule,
  ],
  ruleFolders: [],
  mocks: [
    { id: "mock-1", workspaceId: "ws-1", name: "Users List Mock", method: "GET", path: "/api/users", enabled: true } as MockRule,
    { id: "mock-2", workspaceId: "ws-1", name: "Login Mock", method: "POST", path: "/api/login", enabled: true } as MockRule,
  ],
  requests: [
    { id: "req-1", workspaceId: "ws-1", name: "Get User Profile", method: "GET", url: "https://api.example.com/user/1", createdAt: Date.now() } as SavedRequest,
    { id: "req-2", workspaceId: "ws-1", name: "Update User", method: "PUT", url: "https://api.example.com/user/1", createdAt: Date.now() } as SavedRequest,
    { id: "req-3", workspaceId: "ws-1", name: "Delete User", method: "DELETE", url: "https://api.example.com/user/1", createdAt: Date.now() } as SavedRequest,
  ],
  mockFolders: [],
  requestFolders: [],
  wsConnections: [
    { id: "ws-1", workspaceId: "ws-1", name: "Chat Socket", url: "wss://chat.example.com" } as WebSocketConnection,
  ],
  wsFolders: [],
  webhooks: [
    { id: "wh-1", workspaceId: "ws-1", name: "Stripe Webhook", path: "/webhooks/stripe" } as WebhookEndpoint,
  ],
  webhookFolders: [],
  graphqlRequests: [],
  graphqlMocks: [],
  graphqlSchemas: [],
  graphqlRequestFolders: [],
  graphqlMockFolders: [],
  grpcRequests: [],
  grpcMocks: [],
  protoFiles: [],
  grpcRequestFolders: [],
  grpcMockFolders: [],
  grpcMockServerPort: 9102,
  soapRequests: [],
  soapMocks: [],
  savedWsdls: [],
  soapRequestFolders: [],
  soapMockFolders: [],
  webhookPort: 9101,
  environments: [],
  activeEnvironmentId: null,
};

describe("searchUtils", () => {
  beforeEach(() => {
    mockStorage.clear();
  });

  it("returns open tabs correctly from localStorage", () => {
    writeStorage("requests:openTabs", ["req-1", "draft-123"]);
    expect(getOpenTabs("requests")).toEqual(["req-1", "draft-123"]);
    expect(getOpenTabs("mocks")).toEqual([]);
  });

  it("prioritizes open tabs and excludes opened entities from unopened list", () => {
    // Open req-1
    writeStorage("requests:openTabs", ["req-1"]);

    const res = searchEntities({
      query: "user",
      mode: "current",
      activePanel: "requests",
      config: mockConfig,
    });

    // Should have 2 sections: Open Tabs (with req-1) and Requests (with req-2, req-3, but NOT req-1)
    expect(res.sections.length).toBe(2);
    expect(res.sections[0].title).toBe("Open Tabs");
    expect(res.sections[0].items.length).toBe(1);
    expect(res.sections[0].items[0].id).toBe("req-1");
    expect(res.sections[0].items[0].isOpenTab).toBe(true);

    // Unopened section should not include req-1
    expect(res.sections[1].title).toBe("Requests");
    const unopenedIds = res.sections[1].items.map((i) => i.id);
    expect(unopenedIds).toContain("req-2");
    expect(unopenedIds).toContain("req-3");
    expect(unopenedIds).not.toContain("req-1");
  });

  it("searches only current screen entities when mode is 'current'", () => {
    const res = searchEntities({
      query: "user",
      mode: "current",
      activePanel: "requests",
      config: mockConfig,
    });

    // Only requests should appear, no mocks
    for (const section of res.sections) {
      for (const item of section.items) {
        expect(item.panel).toBe("requests");
      }
    }
  });

  it("searches across all entity types when mode is 'global'", () => {
    const res = searchEntities({
      query: "user",
      mode: "global",
      activePanel: "services",
      config: mockConfig,
    });

    // Should include both Requests and Mocks
    const titles = res.sections.map((s) => s.title);
    expect(titles).toContain("Requests");
    expect(titles).toContain("Mocks");
  });

  it("limits items per section to maxPerSection", () => {
    const manyRequests: SavedRequest[] = Array.from({ length: 20 }, (_, i) => ({
      id: `req-gen-${i}`,
      workspaceId: "ws-1",
      name: `Generated Request ${i}`,
      method: "GET",
      url: `https://example.com/${i}`,
      createdAt: Date.now(),
    }));

    const cfg: AppConfig = {
      ...mockConfig,
      requests: manyRequests,
    };

    const res = searchEntities({
      query: "generated",
      mode: "current",
      activePanel: "requests",
      config: cfg,
      maxPerSection: 5,
    });

    expect(res.sections[0].items.length).toBe(5);
  });
});
