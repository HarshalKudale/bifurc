import React from "react";
import { AppConfig, MockRule, SavedRequest, ServiceInfo, SyncStatus, WorkspaceGitState } from "@/types";
import { Panel, isPanelEnabled } from "@/lib/panelRegistry";
import { CaptureStats } from "@/panels/CapturePanel";
import { ColorMode } from "@/hooks/useTheme";
import type { GitTarget, GitWriteResult } from "@/hooks/useEntityGitActions";

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
    // Entity sync — **both axes**. `entitySyncStatus` is the per-file map; `workspaceGitState` is the
    // workspace's position relative to its remote. A surface needs both to decide what to offer an
    // entity — see `lib/entityGit.ts`. `workspaceGitState` is `null` until the first refresh lands.
    entitySyncStatus: Record<string, "clean" | "modified" | "new" | "deleted">;
    workspaceGitState: WorkspaceGitState | null;
    refreshEntitySyncStatus: (wsId: string) => void;
    // Git actions — one publisher, one reverter, for every entity kind. See
    // `hooks/useEntityGitActions.ts`. Panels receive these and bind them per kind; they do not call
    // `window.api` for git themselves.
    gitPublish: (target: GitTarget, message?: string) => Promise<GitWriteResult>;
    gitRevert: (target: GitTarget) => Promise<GitWriteResult>;
    gitPublishPanel: () => Promise<GitWriteResult>;
    gitIsBusy: (key: string) => boolean;
    gitKeyFor: (op: "publish" | "revert", target: GitTarget) => string;
    gitLastError: string | null;
    gitClearError: () => void;
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
    // Workspace panel handlers
    onWorkspaceRename: (id: string, name: string) => Promise<void>;
    onWorkspaceDelete: (id: string) => Promise<void>;
    // Server restart (settings panel)
    onServerRestart: () => Promise<void>;
    // Sidebar visibility (appearance settings)
    sidebarVisibility: Record<string, boolean>;
    /** `Panel`, not `string`: the appearance settings toggle only ever names a real panel, and typing
     *  it that way is what `useSidebarVisibility` already returns. */
    setSidebarPanelVisible: (id: Panel, visible: boolean) => void;
}

// -- Disabled panel placeholder ----------------------------------------------

const DisabledPanel = () => (
    <PlaceholderPanel
        icon={<AlertCircle size={32} />}
        title="Panel Disabled"
        description="This panel is currently disabled in the panel registry."
    />
);

// -- Binding helpers ---------------------------------------------------------
//
// Panels speak in entity **ids**; the git layer speaks in **targets**. These three binders are the
// only place that translation happens, so every panel gets the same behaviour from the same two
// writers. A panel that needs a kind the binder does not know is a missing entry here, not a second
// `window.api.publishEntity` call at the call site.

/**
 * Publish one entity by id.
 *
 * **Returns `void`, and the `void` is load-bearing.** Every panel prop these binders feed is typed
 * `(id: string) => void` — the panels fire them from a menu item and move on — so returning the
 * promise would be a floating rejection in a caller that cannot see it. `useEntityGitActions` never
 * rejects and records failures in `lastError` for the footer, which is where a fire-and-forget
 * failure has somewhere to go. The two surfaces that *do* await a result (`MappingsPanel`,
 * `HealthBarPanel`) get `pubItemResult` instead.
 */
const pubItem = (ctx: PanelRenderContext, kind: string) => (id: string): void => {
    void ctx.gitPublish({ type: "entity", kind, id });
};

/** Publish one entity and hand the caller the result, for a surface with its own error UI. */
const pubItemResult = (ctx: PanelRenderContext, kind: string) => (id: string) =>
    ctx.gitPublish({ type: "entity", kind, id });

/** Publish a whole folder (or the whole kind when `folderId` is null). */
const pubFolder = (ctx: PanelRenderContext, kind: string) => (folderId: string | null): void => {
    const name = folderId
        ? (ctx.wsConfig as any)[folderKeyFor(kind)]?.find((f: any) => f.id === folderId)?.name ?? null
        : null;
    void ctx.gitPublish({ type: "folder", kind, folderName: name });
};

/** Revert one entity by id — the same writer that backs "Recover". */
const revItem = (ctx: PanelRenderContext, kind: string) => (id: string): void => {
    void ctx.gitRevert({ type: "entity", kind, id });
};

/** Revert one entity and hand the caller the result. */
const revItemResult = (ctx: PanelRenderContext, kind: string) => (id: string) =>
    ctx.gitRevert({ type: "entity", kind, id });

function folderKeyFor(kind: string): string {
    switch (kind) {
        case "requests": return "requestFolders";
        case "mocks": return "mockFolders";
        case "rules": return "ruleFolders";
        case "sockets": return "wsFolders";
        case "webhooks": return "webhookFolders";
        case "graphqlRequests": return "graphqlRequestFolders";
        case "soapRequests": return "soapRequestFolders";
        case "grpcRequests": return "grpcRequestFolders";
        default: return "";
    }
}

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
            onAfterSave={ctx.bumpHistoryReload}
            entitySyncStatus={ctx.entitySyncStatus}
            workspaceGitState={ctx.workspaceGitState}
            onPublish={pubItemResult(ctx, "mappings")}
            onRevert={revItemResult(ctx, "mappings")}
            isPublishing={(id) => ctx.gitIsBusy(ctx.gitKeyFor("publish", { type: "entity", kind: "mappings", id }))}
        />
    ),
    rules: (ctx) => (
        <ProxyRulesPanel
            config={ctx.wsConfig}
            onConfigChange={ctx.handleWsConfigChange}
            onHistoryOpen={ctx.openHistory}
            onAfterSave={ctx.bumpHistoryReload}
            entitySyncStatus={ctx.entitySyncStatus}
            workspaceGitState={ctx.workspaceGitState}
            onPublishItem={pubItem(ctx, "rules")}
            onPublishFolder={pubFolder(ctx, "rules")}
            onRestoreItem={revItem(ctx, "rules")}
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
            workspaceGitState={ctx.workspaceGitState}
            onPublishItem={pubItem(ctx, "requests")}
            onPublishFolder={pubFolder(ctx, "requests")}
            onRestoreItem={revItem(ctx, "requests")}
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
            workspaceGitState={ctx.workspaceGitState}
            onPublishItem={pubItem(ctx, "mocks")}
            onPublishFolder={pubFolder(ctx, "mocks")}
            onRestoreItem={revItem(ctx, "mocks")}
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
            workspaceGitState={ctx.workspaceGitState}
            onPublishItem={pubItem(ctx, "sockets")}
            onPublishFolder={pubFolder(ctx, "sockets")}
            onRestoreItem={revItem(ctx, "sockets")}
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
            workspaceGitState={ctx.workspaceGitState}
            onPublishItem={pubItem(ctx, "webhooks")}
            onPublishFolder={pubFolder(ctx, "webhooks")}
            onRestoreItem={revItem(ctx, "webhooks")}
        />
    ),
    environments: (ctx) => (
        <EnvironmentsPanel
            config={ctx.wsConfig}
            onConfigChange={ctx.handleWsConfigChange}
            onHistoryOpen={ctx.openHistory}
            onAfterSave={ctx.bumpHistoryReload}
            entitySyncStatus={ctx.entitySyncStatus}
            workspaceGitState={ctx.workspaceGitState}
            onPublishItem={pubItem(ctx, "environments")}
            onRestoreItem={revItem(ctx, "environments")}
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
            workspaceGitState={ctx.workspaceGitState}
            onPublish={() => ctx.gitPublishPanel()}
            onRevert={() => ctx.gitRevert({ type: "path", path: "healthbar/services.json" })}
            onHistoryOpen={ctx.openHistory}
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
