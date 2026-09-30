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
    // Present, and explicitly `null`. `App` renders its failure screen on `loadError !== null`, so a mock
    // that *omits* the field leaves it `undefined` — which is not `null`, so the error screen renders and
    // every case below fails looking for a title bar. Found by exactly that failure; the mock was
    // incomplete, not the guard.
    loadError: null,
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
    handleSettingsChange: vi.fn(),
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
    mappings: [],
    proxyRules: [],
    wsConnections: [],
    webhooks: [],
    activeEnvironmentId: null,
  }),
}));

vi.mock("@/hooks/useEntityGitActions", () => ({
  useEntityGitActions: () => ({
    publish: vi.fn(async () => ({ ok: true })),
    revert: vi.fn(async () => ({ ok: true })),
    publishPanel: vi.fn(async () => ({ ok: true })),
    isBusy: () => false,
    lastError: null,
    clearError: vi.fn(),
    keyFor: () => "key",
  }),
}));

/**
 * **This mock is deleted, and its absence is the assertion.** Until 2026-09-30 there was a
 * `vi.mock("@/lib/resolveVars", …)` here supplying `mergeEnvVars`, because `App` merged `__global__` with the
 * active environment on every render. `plan/19` WI 6 replaced that with `env.getMerged`, and
 * `lib/resolveVars.ts` no longer exists — so a mock of it would be a stub for a module nothing imports, which
 * reads as "`App` still has a resolver" to anyone scanning this file.
 *
 * What replaced it is not a mock: the `window.api` stub below has no `envGetMerged`, which is the *partial
 * bridge* case `App`'s guarded read exists for. If that guard is ever removed, this file fails with
 * `envGetMerged is not a function` — which is how the guard was found in the first place.
 */

vi.mock("@/lib/panelRegistry", () => ({
  enabledPanels: [{ id: "services", label: "Services" }],
  PANEL_HELP: { services: "Help" },
}));

vi.mock("@/lib/panelFactory", () => ({
  renderPanel: () => <div data-testid="app-panel">App Panel</div>,
}));

vi.mock("@/components/sidebar/RightSidebar", () => ({
  default: () => <div data-testid="right-sidebar" />,
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

    expect(screen.getByText("Review and accept the terms")).toBeInTheDocument();
    expect(screen.queryByTestId("title-bar")).not.toBeInTheDocument();
  });

  it("persists acceptance and unlocks the app shell", () => {
    render(<App />);

    fireEvent.click(screen.getByRole("button", { name: /accept and continue/i }));

    expect(screen.getByTestId("title-bar")).toBeInTheDocument();
    expect(JSON.parse(localStorage.getItem("app:tos-accepted") ?? "false")).toBe(true);
  });
});
