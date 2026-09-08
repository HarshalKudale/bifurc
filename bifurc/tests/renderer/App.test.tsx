// @vitest-environment jsdom
import React from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import "@testing-library/jest-dom";
import App from "@/App";

vi.mock("@/hooks/useTheme", () => ({
  useColorMode: () => ["dark", vi.fn()],
}));

vi.mock("@/hooks/useSidebarVisibility", () => ({
  useSidebarVisibility: () => ({
    visibility: {},
    setPanelVisible: vi.fn(),
    isPanelVisible: () => true,
  }),
}));

vi.mock("@/hooks/useAppConfigSync", () => ({
  useAppConfigSync: () => ({
    config: {
      activeWorkspaceId: "ws-1",
      workspaces: [{ id: "ws-1", name: "Default" }],
    },
    setConfig: vi.fn(),
    wsLoading: null,
    setWsLoading: vi.fn(),
    services: [],
    refreshServices: vi.fn(),
    serverRunning: false,
    setServerRunning: vi.fn(),
    serverError: null,
    setServerError: vi.fn(),
    entitySyncStatus: {},
    refreshEntitySyncStatus: vi.fn(),
    syncStatus: "idle",
    handleConfigChange: vi.fn(),
    handleWsConfigChange: vi.fn(),
    refreshConfig: vi.fn(),
    clearWorkspaceContext: vi.fn(),
  }),
}));

vi.mock("@/hooks/useWorkspaceView", () => ({
  useWorkspaceView: () => ({
    environments: [],
    requests: [],
    graphqlRequests: [],
    grpcRequests: [],
    soapRequests: [],
    mocks: [],
    graphqlMocks: [],
    grpcMocks: [],
    soapMocks: [],
    mappings: [],
    proxyRules: [],
    wsConnections: [],
    webhooks: [],
    activeEnvironmentId: null,
  }),
}));

vi.mock("@/hooks/usePublishHandlers", () => ({
  usePublishHandlers: () => ({
    handlePublishPanel: vi.fn(),
  }),
}));

vi.mock("@/lib/resolveVars", () => ({
  mergeEnvVars: () => null,
}));

vi.mock("@/lib/panelRegistry", () => ({
  enabledPanels: [{ id: "services", label: "Services" }],
  PANEL_HELP: { services: "Help" },
}));

vi.mock("@/lib/panelFactory", () => ({
  renderPanel: () => <div data-testid="app-panel">App Panel</div>,
}));

vi.mock("@/components/sidebar/HistorySidebar", () => ({
  default: () => <div data-testid="history-sidebar" />,
}));

vi.mock("@/components/sidebar/AppSidebar", () => ({
  default: () => <div data-testid="app-sidebar" />,
}));

vi.mock("@/components/layout/TitleBar", () => ({
  default: () => <div data-testid="title-bar">Title Bar</div>,
}));

vi.mock("@/components/layout/GlobalFooter", () => ({
  default: () => <div data-testid="global-footer" />,
}));

vi.mock("@/components/search/SearchModal", () => ({
  default: () => null,
}));

describe("App TOS gate", () => {
  beforeEach(() => {
    localStorage.clear();
    (window as any).api = {
      openExternal: vi.fn(),
    };
  });

  it("blocks the app until the terms are accepted", () => {
    render(<App />);

    expect(screen.getByText("Accept Terms to continue")).toBeInTheDocument();
    expect(screen.queryByTestId("title-bar")).not.toBeInTheDocument();
  });

  it("persists acceptance and unlocks the app shell", () => {
    render(<App />);

    fireEvent.click(screen.getByRole("button", { name: "I accept" }));

    expect(screen.getByTestId("title-bar")).toBeInTheDocument();
    expect(JSON.parse(localStorage.getItem("app:tos-accepted") ?? "false")).toBe(true);
  });
});
