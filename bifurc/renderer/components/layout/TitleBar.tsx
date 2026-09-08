import React, { useRef, useState } from "react";
import { AppConfig, Workspace } from "@/types";
import { Panel } from "@/lib/panelRegistry";
import WorkspaceSelector from "@/components/layout/WorkspaceSelector";
import EnvSelector from "@/components/sidebar/EnvSelector";
import { strings } from "@/lib/strings";
import { Settings, Search } from "@/lib/icons";
import iconUrl from "@/icon.png";
import HelpTooltip from "@/components/common/HelpTooltip";

interface Props {
  sidebarOpen?: boolean;
  onSidebarToggle?: () => void;
  config: AppConfig;
  serverRunning: boolean;
  serverError: string | null;
  helpText: string;
  onServerStart: () => Promise<void>;
  onServerStop: () => Promise<void>;
  onEnvChange: (id: string | null) => Promise<void>;
  onManageEnvs: () => void;
  workspaces: Workspace[];
  activeWorkspaceId: string;
  onWorkspaceChange: (id: string) => void;
  onWorkspaceCreate: () => void;
  onWorkspaceRename: (id: string, name: string) => void;
  onWorkspaceDelete: (id: string) => void;
  activePanel: Panel;
  onOpenWorkspaceSettings: () => void;
  onOpenSearch: (mode?: "current" | "global") => void;
}

export default function TitleBar({
  config,
  serverRunning,
  serverError,
  helpText,
  onServerStart,
  onServerStop,
  onEnvChange,
  onManageEnvs,
  workspaces,
  activeWorkspaceId,
  onWorkspaceChange,
  onWorkspaceCreate,
  onWorkspaceRename,
  onWorkspaceDelete,
  activePanel,
  onOpenWorkspaceSettings,
  onOpenSearch,
}: Props) {
  const [envDropdownOpen, setEnvDropdownOpen] = useState(false);
  const [serverBusy, setServerBusy] = useState(false);
  const envDropdownRef = useRef<HTMLDivElement>(null);

  const wsEnvironments = (config.environments ?? []).filter(
    (e) => e.workspaceId === activeWorkspaceId
  );
  const serverLabel = serverError
    ? strings.titleBar.portInUse.replace("{port}", String(config.port))
    : serverRunning
      ? strings.titleBar.active.replace("{port}", String(config.port))
      : strings.titleBar.stopped.replace("{port}", String(config.port));

  const handleServerBadgeClick = async () => {
    if (serverBusy) return;
    setServerBusy(true);
    try {
      if (serverRunning) await onServerStop();
      else await onServerStart();
    } finally {
      setServerBusy(false);
    }
  };

  return (
    <div
      className="h-12 bg-background flex items-center px-3 gap-2 flex-shrink-0 relative"
      style={{
        WebkitAppRegion: "drag",
        paddingRight: "calc(100vw - env(titlebar-area-width, 100vw) + 5px)",
      } as React.CSSProperties}
    >
      <div className="absolute inset-x-0 bottom-0 h-px bg-border" />

      {/* App identity */}
      <img
        src={iconUrl}
        className="w-5 h-5 rounded flex-shrink-0"
        alt=""
        draggable={false}
      />
      <span className="text-sm font-semibold text-foreground tracking-wide select-none">
        {strings.titleBar.appName}
      </span>

      {/* Divider between App identity and Workspace */}
      <div className="h-4 w-px bg-border/80 mx-1 flex-shrink-0" />

      {/* Workspace Selector */}
      <WorkspaceSelector
        workspaces={workspaces}
        activeId={activeWorkspaceId}
        onSelect={onWorkspaceChange}
        onCreate={onWorkspaceCreate}
        onRename={onWorkspaceRename}
        onDelete={onWorkspaceDelete}
      />

      {/* Workspace Settings / Config button */}
      <button
        type="button"
        onClick={onOpenWorkspaceSettings}
        title={strings.panels.sectionWorkspace}
        aria-label={strings.panels.sectionWorkspace}
        style={{ WebkitAppRegion: "no-drag" } as React.CSSProperties}
        className={`flex h-8 w-8 items-center justify-center rounded-md border transition-colors cursor-pointer select-none flex-shrink-0 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-signal/35 ${
          activePanel === "workspace"
            ? "border-signal/40 bg-signal/15 text-signal"
            : "border-border/80 bg-card/60 text-muted-foreground hover:bg-surface-2 hover:text-foreground"
        }`}
      >
        <Settings size={13} />
      </button>

      {/* Search trigger button */}
      <div className="flex-1 flex justify-center px-2 min-w-0">
        <button
          type="button"
          onClick={() => onOpenSearch("current")}
          title="Search (Ctrl+K or Ctrl+Shift+K)"
          aria-label="Search"
          style={{ WebkitAppRegion: "no-drag" } as React.CSSProperties}
          className="w-full max-w-[320px] flex items-center justify-between h-8 px-3 rounded-md border border-border/80 bg-card/60 hover:bg-surface-2 hover:border-signal/40 text-muted-foreground hover:text-foreground transition-all cursor-pointer select-none text-xs group"
        >
          <div className="flex items-center gap-2 min-w-0">
            <Search size={13} className="text-muted-foreground group-hover:text-signal transition-colors flex-shrink-0" />
            <span className="truncate text-muted-foreground/80 group-hover:text-foreground transition-colors">Search...</span>
          </div>
          <kbd className="text-[10px] bg-surface border border-border px-1.5 py-0.5 rounded text-muted-foreground font-mono group-hover:text-foreground group-hover:border-signal/30 transition-colors">
            Ctrl K
          </kbd>
        </button>
      </div>

      {/* Help tooltip */}
      <div style={{ WebkitAppRegion: "no-drag" } as React.CSSProperties}>
        <HelpTooltip text={helpText} />
      </div>

      {/* Environment selector */}
      <EnvSelector
        environments={wsEnvironments}
        activeId={config.activeEnvironmentId ?? null}
        open={envDropdownOpen}
        dropdownRef={envDropdownRef}
        onToggle={() => setEnvDropdownOpen((v) => !v)}
        onClose={() => setEnvDropdownOpen(false)}
        onSelect={async (id) => {
          await onEnvChange(id);
          setEnvDropdownOpen(false);
        }}
        onManage={() => {
          onManageEnvs();
          setEnvDropdownOpen(false);
        }}
      />

      <button
        type="button"
        onClick={handleServerBadgeClick}
        disabled={serverBusy}
        title={serverError ?? (serverRunning ? strings.titleBar.stopServer : strings.titleBar.startServer)}
        aria-label={serverRunning ? strings.titleBar.stopServer : strings.titleBar.startServer}
        style={{ WebkitAppRegion: "no-drag" } as React.CSSProperties}
        className={`group flex h-9 max-w-[220px] items-center gap-2 rounded-xl border px-4 text-sm font-medium transition-all duration-150 cursor-pointer select-none disabled:cursor-not-allowed disabled:opacity-70 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-signal/35 ${
          serverError
            ? "border-destructive/20 bg-destructive/10 text-destructive hover:border-destructive/35 hover:bg-destructive/14"
            : serverRunning
              ? "border-signal/20 bg-card/70 text-signal hover:border-signal/30 hover:bg-card/90"
              : "border-border/80 bg-card/60 text-muted-foreground hover:border-signal/25 hover:bg-surface-2 hover:text-foreground"
        }`}
      >
        <span
          className={`h-3 w-3 rounded-full flex-shrink-0 ${serverRunning && !serverError && !serverBusy ? "animate-pulse-dot" : ""}`}
          style={{
            background: serverError
              ? "var(--c-destructive)"
              : serverRunning
                ? "var(--c-signal)"
                : "var(--c-muted-foreground)",
            boxShadow: serverError || !serverRunning
              ? "none"
              : "0 0 10px color-mix(in oklab, var(--c-signal) 55%, transparent)",
          }}
        />
        <span className="truncate">{serverBusy ? strings.footer.syncing : serverLabel}</span>
      </button>
    </div>
  );
}
