import React from "react";
import { AppConfig, MockRule, SavedRequest, ServiceInfo, Folder, SyncStatus } from "@/types";
import { Panel, isPanelEnabled } from "@/lib/panelRegistry";
import { CaptureStats } from "@/panels/CapturePanel";
import { ColorMode } from "@/lib/useTheme";

import ServicesPanel from "@/panels/ServicesPanel";
import MappingsPanel from "@/panels/MappingsPanel";
import ProxyRulesPanel from "@/panels/ProxyRulesPanel";
import SettingsPanel from "@/panels/SettingsPanel";
import CapturePanel from "@/panels/CapturePanel";
import MocksPanel from "@/panels/MocksPanel";
import RequestsPanel from "@/panels/RequestsPanel";
import WebSocketsPanel from "@/panels/WebSocketsPanel";
import WebhooksPanel from "@/panels/WebhooksPanel";
import EnvironmentsPanel from "@/panels/EnvironmentsPanel";
import AuditLogPanel from "@/panels/AuditLogPanel";
import WorkspacePanel from "@/panels/WorkspacePanel";
import HealthBarPanel from "@/panels/HealthBarPanel";
import PlaceholderPanel from "@/panels/PlaceholderPanel";
import GraphQLRequestsPanel from "@/panels/GraphQLRequestsPanel";
import GraphQLMocksPanel from "@/panels/GraphQLMocksPanel";
import GrpcRequestsPanel from "@/panels/GrpcRequestsPanel";
import GrpcMocksPanel from "@/panels/GrpcMocksPanel";
import SoapRequestsPanel from "@/panels/SoapRequestsPanel";
import SoapMocksPanel from "@/panels/SoapMocksPanel";
import { AlertCircle } from "@/lib/icons";

// -- Render context ----------------------------------------------------------
// A single bag containing everything panels might need. App.tsx constructs this
// once per render and passes it to `renderPanel`.

export interface PanelRenderContext {
    // Workspace-scoped config
    wsConfig: AppConfig;
    // Full (cross-workspace) config
    config: AppConfig;
    // Active workspace ID
    wsId: string;
    // Services list
    services: ServiceInfo[];
    // Server state
    serverRunning: boolean;
    serverError: string | null;
    // Color mode
    colorMode: ColorMode;
    setColorMode: (m: ColorMode) => void;
    // Active environment
    activeEnv: any;
    // History sidebar
    openHistory: (filePath: string) => void;
    handleEntityPathChange: (filePath: string) => void;
    historyOpen: boolean;
    bumpHistoryReload: () => void;
    // Entity sync
    entitySyncStatus: Record<string, "clean" | "modified" | "new" | "deleted">;
    refreshEntitySyncStatus: (wsId: string) => void;
    // Config change handlers
    handleConfigChange: (next: AppConfig) => Promise<void>;
    handleWsConfigChange: (next: AppConfig) => Promise<void>;
    setConfig: (cfg: AppConfig) => void;
    // Services
    refreshServices: () => Promise<void>;
    // Mapping prefill
    mappingPrefill: string | undefined;
    onPrefillConsumed: () => void;
    setPanel: (p: Panel) => void;
    setMappingPrefill: (v: string | undefined) => void;
    // Pending open request / mock initial / rule select
    pendingOpenRequest: Omit<SavedRequest, "id" | "createdAt" | "workspaceId"> | null;
    onPendingRequestConsumed: () => void;
    pendingMockInitial: Partial<MockRule> | null;
    onPendingMockConsumed: () => void;
    pendingRuleId: string | null;
    onPendingRuleConsumed: () => void;
    handleOpenMockEditor: (initial: Partial<MockRule>) => void;
    handleOpenInRequests: (req: Omit<SavedRequest, "id" | "createdAt" | "workspaceId">) => void;
    // Capture
    onStatsChange: (stats: CaptureStats | null) => void;
    // Publish helpers
    makePublishItem: (kind: string, folders: Folder[]) => (id: string) => Promise<void>;
    makePublishFolder: (kind: "requests" | "mocks" | "sockets" | "webhooks" | "rules" | string, folders: Folder[]) => (folderId: string | null) => Promise<void>;
    makeFlatPublish: (kind: "mappings") => (id: string) => Promise<void>;
    makeFlatRevert: (kind: "mappings") => (id: string) => Promise<void>;
    makeRestoreItem: (kind: string, folders: Folder[]) => (id: string) => Promise<void>;
    handlePublishHealthBar: () => Promise<void>;
    // Workspace panel handlers
    onWorkspaceRename: (id: string, name: string) => Promise<void>;
    onWorkspaceDelete: (id: string) => Promise<void>;
    // Server restart (settings panel)
    onServerRestart: () => Promise<void>;
    // Sidebar visibility (appearance settings)
    sidebarVisibility: Record<string, boolean>;
    setSidebarPanelVisible: (id: string, visible: boolean) => void;
}

// -- Disabled panel placeholder ----------------------------------------------

const DisabledPanel = () => (
    <PlaceholderPanel
        icon={<AlertCircle size={32} />}
        title="Panel Disabled"
        description="This panel is currently disabled in the panel registry."
    />
);

// -- Panel renderer map ------------------------------------------------------

const PANEL_RENDERERS: Record<Panel, (ctx: PanelRenderContext) => React.ReactNode> = {
    services: (ctx) => (
        <ServicesPanel
            services={ctx.services}
            config={ctx.wsConfig}
            onRefresh={ctx.refreshServices}
            onQuickMap={(target) => { ctx.setMappingPrefill(target); ctx.setPanel("mappings"); }}
            onOpenMappings={() => ctx.setPanel("mappings")}
            onOpenRequests={() => ctx.setPanel("req-rest")}
            onOpenCapture={() => ctx.setPanel("capture")}
            onOpenSettings={() => ctx.setPanel("settings")}
        />
    ),
    mappings: (ctx) => (
        <MappingsPanel
            config={ctx.wsConfig}
            onConfigChange={ctx.handleWsConfigChange}
            onRefreshServices={ctx.refreshServices}
            prefillTarget={ctx.mappingPrefill}
            onPrefillConsumed={ctx.onPrefillConsumed}
            onHistoryOpen={ctx.openHistory}
            entitySyncStatus={ctx.entitySyncStatus}
            onPublish={ctx.makeFlatPublish("mappings")}
            onRevert={ctx.makeFlatRevert("mappings")}
        />
    ),
    rules: (ctx) => (
        <ProxyRulesPanel
            config={ctx.wsConfig}
            onConfigChange={ctx.handleWsConfigChange}
            onHistoryOpen={ctx.openHistory}
            entitySyncStatus={ctx.entitySyncStatus}
            onPublishItem={ctx.makePublishItem("rules", ctx.wsConfig.ruleFolders ?? [])}
            onPublishFolder={ctx.makePublishFolder("rules", ctx.wsConfig.ruleFolders ?? [])}
            onRestoreItem={ctx.makeRestoreItem("rules", ctx.wsConfig.ruleFolders ?? [])}
            pendingRuleId={ctx.pendingRuleId}
            onPendingRuleConsumed={ctx.onPendingRuleConsumed}
        />
    ),
    capture: (ctx) => (
        <CapturePanel
            activeWorkspaceId={ctx.wsId}
            wsConfig={ctx.wsConfig}
            onConfigChange={ctx.handleWsConfigChange}
            onOpenInMocks={ctx.handleOpenMockEditor}
            onOpenInRequests={ctx.handleOpenInRequests}
            onStatsChange={ctx.onStatsChange}
        />
    ),
    requests: (ctx) => (
        <RequestsPanel
            config={ctx.wsConfig}
            onConfigChange={ctx.handleWsConfigChange}
            pendingOpenRequest={ctx.pendingOpenRequest}
            onPendingConsumed={ctx.onPendingRequestConsumed}
            onOpenMockEditor={ctx.handleOpenMockEditor}
            activeEnv={ctx.activeEnv}
            onHistoryOpen={ctx.openHistory}
            onEntityPathChange={ctx.handleEntityPathChange}
            historyOpen={ctx.historyOpen}
            onAfterSave={ctx.bumpHistoryReload}
            entitySyncStatus={ctx.entitySyncStatus}
            onPublishItem={ctx.makePublishItem("requests", ctx.wsConfig.requestFolders ?? [])}
            onPublishFolder={ctx.makePublishFolder("requests", ctx.wsConfig.requestFolders ?? [])}
            onRestoreItem={ctx.makeRestoreItem("requests", ctx.wsConfig.requestFolders ?? [])}
        />
    ),
    mocks: (ctx) => (
        <MocksPanel
            config={ctx.wsConfig}
            onConfigChange={ctx.handleWsConfigChange}
            pendingMockInitial={ctx.pendingMockInitial}
            onPendingConsumed={ctx.onPendingMockConsumed}
            activeEnv={ctx.activeEnv}
            onHistoryOpen={ctx.openHistory}
            onEntityPathChange={ctx.handleEntityPathChange}
            historyOpen={ctx.historyOpen}
            onAfterSave={ctx.bumpHistoryReload}
            entitySyncStatus={ctx.entitySyncStatus}
            onPublishItem={ctx.makePublishItem("mocks", ctx.wsConfig.mockFolders ?? [])}
            onPublishFolder={ctx.makePublishFolder("mocks", ctx.wsConfig.mockFolders ?? [])}
            onRestoreItem={ctx.makeRestoreItem("mocks", ctx.wsConfig.mockFolders ?? [])}
        />
    ),
    "mock-rest": (ctx) => PANEL_RENDERERS.mocks(ctx),
    "mock-graphql": (ctx) => PANEL_RENDERERS.mocks(ctx),
    "mock-soap": (ctx) => PANEL_RENDERERS.mocks(ctx),
    "mock-grpc": (ctx) => PANEL_RENDERERS.mocks(ctx),
    "req-rest": (ctx) => PANEL_RENDERERS.requests(ctx),
    "req-graphql": (ctx) => PANEL_RENDERERS.requests(ctx),
    "req-soap": (ctx) => PANEL_RENDERERS.requests(ctx),
    "req-grpc": (ctx) => PANEL_RENDERERS.requests(ctx),
    sockets: (ctx) => (
        <WebSocketsPanel
            config={ctx.wsConfig}
            onConfigChange={ctx.handleWsConfigChange}
            activeEnv={ctx.activeEnv}
            onHistoryOpen={ctx.openHistory}
            onEntityPathChange={ctx.handleEntityPathChange}
            historyOpen={ctx.historyOpen}
            onAfterSave={ctx.bumpHistoryReload}
            entitySyncStatus={ctx.entitySyncStatus}
            onPublishItem={ctx.makePublishItem("sockets", ctx.wsConfig.wsFolders ?? [])}
            onPublishFolder={ctx.makePublishFolder("sockets", ctx.wsConfig.wsFolders ?? [])}
            onRestoreItem={ctx.makeRestoreItem("sockets", ctx.wsConfig.wsFolders ?? [])}
        />
    ),
    webhooks: (ctx) => (
        <WebhooksPanel
            config={ctx.wsConfig}
            onConfigChange={ctx.handleWsConfigChange}
            onHistoryOpen={ctx.openHistory}
            onEntityPathChange={ctx.handleEntityPathChange}
            historyOpen={ctx.historyOpen}
            onAfterSave={ctx.bumpHistoryReload}
            entitySyncStatus={ctx.entitySyncStatus}
            onPublishItem={ctx.makePublishItem("webhooks", ctx.wsConfig.webhookFolders ?? [])}
            onPublishFolder={ctx.makePublishFolder("webhooks", ctx.wsConfig.webhookFolders ?? [])}
            onRestoreItem={ctx.makeRestoreItem("webhooks", ctx.wsConfig.webhookFolders ?? [])}
        />
    ),
    environments: (ctx) => (
        <EnvironmentsPanel
            config={ctx.wsConfig}
            onConfigChange={ctx.handleWsConfigChange}
            onHistoryOpen={ctx.openHistory}
            onAfterSave={ctx.bumpHistoryReload}
        />
    ),
    settings: (ctx) => (
        <SettingsPanel
            config={ctx.config}
            serverRunning={ctx.serverRunning}
            serverError={ctx.serverError}
            onConfigChange={ctx.handleConfigChange}
            colorMode={ctx.colorMode}
            onColorModeChange={ctx.setColorMode}
            onServerRestart={ctx.onServerRestart}
            sidebarVisibility={ctx.sidebarVisibility}
            onSidebarVisibilityChange={ctx.setSidebarPanelVisible}
        />
    ),
    audit: (ctx) => (
        <AuditLogPanel activeWorkspaceId={ctx.wsId} />
    ),
    workspace: (ctx) => (
        <WorkspacePanel
            config={ctx.config}
            onConfigChange={(fresh) => ctx.setConfig(fresh)}
            onWorkspaceRename={ctx.onWorkspaceRename}
            onWorkspaceDelete={ctx.onWorkspaceDelete}
        />
    ),
    healthbar: (ctx) => (
        <HealthBarPanel
            config={ctx.wsConfig}
            entitySyncStatus={ctx.entitySyncStatus}
            onPublish={ctx.handlePublishHealthBar}
            onAfterSave={() => ctx.refreshEntitySyncStatus(ctx.wsId)}
        />
    ),
    applications: () => <DisabledPanel />,
};

// -- Public API --------------------------------------------------------------

/**
 * Render the active panel. If the panel is disabled in the registry, renders a
 * placeholder instead. This allows compile-time toggling of in-progress panels.
 */
export function renderPanel(panelId: Panel, ctx: PanelRenderContext): React.ReactNode {
    let normalizedId = panelId;
    if (panelId === "req-rest" || panelId === "req-graphql" || panelId === "req-soap" || panelId === "req-grpc") {
        normalizedId = "requests";
    } else if (panelId === "mock-rest" || panelId === "mock-graphql" || panelId === "mock-soap" || panelId === "mock-grpc") {
        normalizedId = "mocks";
    }

    if (!isPanelEnabled(normalizedId)) {
        return <DisabledPanel />;
    }
    const renderer = PANEL_RENDERERS[normalizedId];
    return renderer ? renderer(ctx) : <DisabledPanel />;
}
