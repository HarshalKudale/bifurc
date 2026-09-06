import React from "react";
import { strings } from "@/lib/strings";
import {
    Zap,
    ArrowLeftRight,
    Settings,
    Clipboard,
    ArrowUpRight,
    Radio,
    Globe,
    ClipboardList,
    Layers,
    Activity,
    Webhook,
    Network,
    FileCode,
    Braces,
    Play,
} from "@/lib/icons";

// -- Panel ID union type -----------------------------------------------------

export type Panel =
    | "services"
    | "mappings"
    | "rules"
    | "capture"
    | "requests"
    | "mocks"
    | "mock-rest"
    | "mock-graphql"
    | "mock-soap"
    | "mock-grpc"
    | "req-rest"
    | "req-graphql"
    | "req-soap"
    | "req-grpc"
    | "sockets"
    | "environments"
    | "settings"
    | "audit"
    | "workspace"
    | "healthbar"
    | "webhooks"
    | "applications";

// -- Panel entry definition --------------------------------------------------

export interface PanelEntry {
    id: Panel;
    label: string;
    icon: React.ReactNode;
    section: string;
    sectionType: "flat" | "collapsible";
    enabled: boolean;
    /** When false, the panel is functional but hidden from the sidebar nav.
     *  Use the three-dot workspace menu to access workspace/audit panels. */
    showInSidebar?: boolean;
    /** When true, the panel cannot be hidden by the user in Appearance settings. */
    alwaysVisible?: boolean;
    helpText: string;
}

// -- Panel registry ----------------------------------------------------------
// Toggle `enabled` to false to hide a panel from the sidebar and show a
// placeholder when navigated to directly. Useful for in-progress panels.

export const PANEL_REGISTRY: PanelEntry[] = [
    // --- Applications (flat) --------------------------------------------------
    {
        id: "services",
        label: strings.nav.services,
        icon: <Zap size={20} />,
        section: strings.panels.sectionApplications,
        sectionType: "flat",
        enabled: true,
        helpText: strings.services.helpText,
    },
    {
        id: "healthbar",
        label: strings.panels.sectionHealthBar,
        icon: <Activity size={20} />,
        section: strings.panels.sectionApplications,
        sectionType: "flat",
        enabled: true,
        helpText: strings.panels.healthbarHelp,
    },

    // --- Routing (flat) -------------------------------------------------------
    {
        id: "mappings",
        label: strings.nav.mappings,
        icon: <ArrowLeftRight size={20} />,
        section: strings.nav.routing,
        sectionType: "flat",
        enabled: true,
        helpText: strings.mappings.helpText,
    },
    {
        id: "rules",
        label: strings.nav.proxyRules,
        icon: <Settings size={20} />,
        section: strings.nav.routing,
        sectionType: "flat",
        enabled: true,
        helpText: strings.proxyRules.helpText,
    },
    {
        id: "capture",
        label: strings.nav.capture,
        icon: <Clipboard size={20} />,
        section: strings.nav.routing,
        sectionType: "flat",
        enabled: true,
        helpText: strings.capture.helpText,
    },

    // --- API (flat) -----------------------------------------------------------
    {
        id: "requests",
        label: strings.nav.requests,
        icon: <ArrowUpRight size={20} />,
        section: strings.panels.sectionApi,
        sectionType: "flat",
        enabled: true,
        helpText: strings.panels.requestsHelp,
    },
    {
        id: "mocks",
        label: strings.nav.mocks,
        icon: <Layers size={20} />,
        section: strings.panels.sectionApi,
        sectionType: "flat",
        enabled: true,
        helpText: strings.panels.mocksHelp,
    },

    // --- Realtime (flat) ------------------------------------------------------
    {
        id: "sockets",
        label: strings.panels.sectionWebsocket,
        icon: <Radio size={20} />,
        section: strings.panels.sectionRealtime,
        sectionType: "flat",
        enabled: true,
        helpText: strings.sockets.helpText,
    },
    {
        id: "webhooks",
        label: strings.panels.sectionWebhooks,
        icon: <Webhook size={20} />,
        section: strings.panels.sectionRealtime,
        sectionType: "flat",
        enabled: true,
        helpText: strings.panels.webhooksHelp,
    },

    // --- Workspace (flat) ---------------------------------------------------------
    {
        id: "environments",
        label: strings.nav.environments,
        icon: <Globe size={20} />,
        section: strings.nav.tools,
        sectionType: "flat",
        enabled: true,
        alwaysVisible: true,
        helpText: strings.environments.helpText,
    },
    {
        id: "workspace",
        label: strings.panels.sectionWorkspace,
        icon: <Layers size={20} />,
        section: strings.nav.tools,
        sectionType: "flat",
        enabled: true,
        helpText: strings.workspace.helpText,
    },
    {
        id: "audit",
        label: strings.panels.sectionAuditLog,
        icon: <ClipboardList size={20} />,
        section: strings.nav.tools,
        sectionType: "flat",
        enabled: true,
        helpText: strings.panels.auditHelp,
    },

    // --- Config (flat) --------------------------------------------------------

    {
        id: "settings",
        label: strings.nav.settings,
        icon: <Settings size={20} />,
        section: strings.nav.config,
        sectionType: "flat",
        enabled: true,
        showInSidebar: false,
        helpText: strings.settings.helpText,
    },
];

// -- Derived helpers ---------------------------------------------------------

/** Panel entries that are both enabled and visible in the sidebar nav. */
export const enabledPanels = PANEL_REGISTRY.filter((e) => e.enabled && e.showInSidebar !== false);

/** Panels that cannot be hidden by the user (always shown in sidebar). */
export const ALWAYS_VISIBLE_PANELS: Panel[] = PANEL_REGISTRY
    .filter((e) => e.alwaysVisible)
    .map((e) => e.id);

/** Quick lookup: is a given panel enabled? */
export function isPanelEnabled(id: Panel): boolean {
    const entry = PANEL_REGISTRY.find((e) => e.id === id);
    return entry?.enabled ?? false;
}

/** Help text keyed by panel ID (only enabled panels) */
export const PANEL_HELP: Record<Panel, string> = Object.fromEntries(
    PANEL_REGISTRY.map((e) => [e.id, e.helpText])
) as Record<Panel, string>;
