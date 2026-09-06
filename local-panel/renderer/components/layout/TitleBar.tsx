import React, { useRef, useState } from "react";
import { AppConfig } from "@/types";
import ServerToggle from "@/components/layout/ServerToggle";
import EnvSelector from "@/components/sidebar/EnvSelector";
import { strings } from "@/lib/strings";
import { Menu, ChevronRight } from "@/lib/icons";
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
}: Props) {
  const [envDropdownOpen, setEnvDropdownOpen] = useState(false);
  const envDropdownRef = useRef<HTMLDivElement>(null);

  const activeWorkspaceId = config.activeWorkspaceId ?? "";

  const wsEnvironments = (config.environments ?? []).filter(
    (e) => e.workspaceId === activeWorkspaceId
  );

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

      <div className="flex-1" />

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

      {/* Server play/stop */}
      <ServerToggle
        running={serverRunning}
        error={serverError}
        onStart={onServerStart}
        onStop={onServerStop}
      />

      {/* Server status */}
      {serverError ? (
        <div
          className="flex items-center gap-1.5 text-destructive text-xs font-medium max-w-[220px]"
          style={{ WebkitAppRegion: "no-drag" } as React.CSSProperties}
          title={serverError}
        >
          <span className="rounded-full flex-shrink-0" style={{ width: 6, height: 6, background: "var(--c-destructive)" }} />
          <span className="truncate">{strings.titleBar.portInUse.replace("{port}", String(config.port))}</span>
        </div>
      ) : serverRunning ? (
        <div
          className="flex items-center gap-1.5 text-signal text-xs font-medium"
          style={{ WebkitAppRegion: "no-drag" } as React.CSSProperties}
        >
          <span
            className="rounded-full flex-shrink-0 animate-pulse-dot"
            style={{
              width: 6,
              height: 6,
              background: "var(--c-signal)",
              boxShadow: "0 0 6px var(--c-signal)",
            }}
          />
          {strings.titleBar.active.replace("{port}", String(config.port))}
        </div>
      ) : (
        <div
          className="flex items-center gap-1.5 text-muted-foreground text-xs font-medium"
          style={{ WebkitAppRegion: "no-drag" } as React.CSSProperties}
        >
          <span
            className="rounded-full flex-shrink-0"
            style={{
              width: 6,
              height: 6,
              background: "var(--c-muted-foreground)",
            }}
          />
          {strings.titleBar.stopped.replace("{port}", String(config.port))}
        </div>
      )}
    </div>
  );
}
