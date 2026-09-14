import React, { useState } from "react";
import { AppConfig, Workspace, SyncState, SyncStatus } from "@/types";
import { strings } from "@/lib/strings";
import { Cloud, CloudOff, ArrowUp, ArrowDown, Link, Unlink, GitBranch } from "@/lib/icons";
import { Input, SectionLabel, SectionCard, SettingsRow, Switch } from "@/components/ui";

function SyncStatusBadge({ status, progressMessage }: { status: SyncStatus; progressMessage: string | null }) {
  const map: Record<SyncStatus, { label: string; color: string }> = {
    idle: { label: strings.workspace.statusIdle, color: "text-signal" },
    pushing: { label: strings.workspace.statusPushing, color: "text-signal" },
    pulling: { label: strings.workspace.statusPulling, color: "text-signal" },
    cloning: { label: strings.workspace.statusCloning, color: "text-signal" },
    error: { label: strings.workspace.statusError, color: "text-destructive" },
  };
  const { label, color } = map[status] ?? map.idle;
  const spinning = status !== "idle" && status !== "error";
  const displayLabel = spinning && progressMessage ? progressMessage : label;
  return (
    <span className={`flex items-center gap-1.5 text-xs ${color}`}>
      {spinning ? (
        <span className="inline-block w-3 h-3 border border-current border-t-transparent rounded-full animate-spin flex-shrink-0" />
      ) : (
        <Cloud size={12} />
      )}
      {displayLabel}
    </span>
  );
}

function formatRelTime(ts: number): string {
  const diff = Date.now() - ts;
  const mins = Math.floor(diff / 60_000);
  if (mins < 1) return "just now";
  if (mins < 60) return `${mins}m ago`;
  const hrs = Math.floor(mins / 60);
  if (hrs < 24) return `${hrs}h ago`;
  const days = Math.floor(hrs / 24);
  return `${days}d ago`;
}

interface WorkspaceSyncSectionProps {
  config: AppConfig;
  workspace: Workspace;
  syncState: SyncState;
  onConfigChange: (cfg: AppConfig) => void;
  setSyncState: React.Dispatch<React.SetStateAction<SyncState>>;
}

export function WorkspaceSyncSection({
  config,
  workspace,
  syncState,
  onConfigChange,
  setSyncState,
}: WorkspaceSyncSectionProps) {
  const wsId = workspace.id;
  const [remoteInput, setRemoteInput] = useState("");
  const [branchInput, setBranchInput] = useState("main");
  const [connectConfirm, setConnectConfirm] = useState<"empty" | "non-empty" | null>(null);
  const [connecting, setConnecting] = useState(false);
  const [connectError, setConnectError] = useState<string | null>(null);
  const [disconnectConfirm, setDisconnectConfirm] = useState(false);

  const syncConfig = workspace.syncConfig;
  const isConnected = !!syncConfig;

  const handleConnectClick = () => {
    setConnectError(null);
    const hasData = (
      (config.mocks ?? []).filter((m) => m.workspaceId === wsId).length > 0 ||
      (config.mappings ?? []).filter((m) => m.workspaceId === wsId).length > 0 ||
      (config.proxyRules ?? []).filter((r) => r.workspaceId === wsId).length > 0 ||
      (config.requests ?? []).filter((r) => r.workspaceId === wsId).length > 0 ||
      (config.wsConnections ?? []).filter((c) => c.workspaceId === wsId).length > 0 ||
      (config.environments ?? []).filter((e) => e.workspaceId === wsId).length > 0
    );
    setConnectConfirm(hasData ? "non-empty" : "empty");
  };

  const handleConnectConfirm = async () => {
    if (!remoteInput.trim()) return;
    setConnecting(true);
    setConnectConfirm(null);
    setConnectError(null);
    try {
      const result = await window.api.syncSetRemote(wsId, remoteInput.trim(), branchInput.trim() || "main");
      if (result.ok) {
        const effectiveId = result.adoptedId ?? wsId;
        if (effectiveId !== wsId) {
          const switchResult = await window.api.setActiveWorkspace(effectiveId);
          if (switchResult.ok) {
            onConfigChange(switchResult.config);
          } else {
            const fresh = await window.api.getConfig();
            onConfigChange(fresh);
          }
        } else {
          const fresh = await window.api.getConfig();
          onConfigChange(fresh);
        }
        const state = await window.api.syncGetState(effectiveId);
        setSyncState(state);
      } else {
        setConnectError(result.error ?? "Connection failed");
      }
    } finally {
      setConnecting(false);
    }
  };

  const handleDisconnect = async () => {
    setDisconnectConfirm(false);
    await window.api.syncDisconnect(wsId);
    const fresh = await window.api.getConfig();
    onConfigChange(fresh);
    setSyncState({ status: "idle", error: null, lastPushedAt: null, lastPulledAt: null, progressMessage: null });
    setRemoteInput("");
  };

  const handlePush = async () => {
    const result = await window.api.syncPush(wsId);
    if (!result.ok) setSyncState((s) => ({ ...s, error: result.error ?? null }));
  };

  const handlePull = async () => {
    const result = await window.api.syncPull(wsId);
    if (!result.ok) setSyncState((s) => ({ ...s, error: result.error ?? null }));
  };

  const handleAutoSyncToggle = async (enabled: boolean) => {
    await window.api.syncSetAutoSync(wsId, enabled);
    const fresh = await window.api.getConfig();
    onConfigChange(fresh);
  };

  const lastPushed = workspace.syncMeta?.lastPushedAt ?? syncState.lastPushedAt;
  const lastPulled = workspace.syncMeta?.lastPulledAt ?? syncState.lastPulledAt;

  return (
    <section>
      <SectionLabel>{strings.workspace.sectionSync}</SectionLabel>
      {syncState.error && (
        <div className="mb-3 px-3 py-2 rounded border border-destructive/30 bg-destructive/5 text-xs text-destructive flex items-start gap-2">
          <span className="flex-shrink-0 mt-0.5"><Cloud size={12} /></span>
          <span>{strings.workspace.syncError}: {syncState.error}</span>
        </div>
      )}
      <SectionCard>
        {!isConnected ? (
          <>
            <SettingsRow title={strings.workspace.remoteUrl} desc={strings.workspace.remoteUrlDesc}>
              <Input
                aria-label={strings.workspace.remoteUrl}
                className="w-72 font-mono text-sm"
                placeholder={strings.workspace.remoteUrlPlaceholder}
                value={remoteInput}
                onChange={(e) => setRemoteInput(e.target.value)}
              />
            </SettingsRow>
            <SettingsRow title={strings.workspace.branch} desc="">
              <Input
                aria-label={strings.workspace.branch}
                className="w-40 font-mono text-sm"
                placeholder={strings.workspace.branchPlaceholder}
                value={branchInput}
                onChange={(e) => setBranchInput(e.target.value)}
              />
            </SettingsRow>
            <div className="px-5 py-3 border-t border-border flex items-center justify-between gap-3">
              <span className="text-xs text-muted-foreground flex items-center gap-1.5">
                <CloudOff size={12} />
                Not connected
              </span>
              <button
                disabled={!remoteInput.trim() || connecting}
                onClick={handleConnectClick}
                className="px-3 py-1.5 rounded border border-signal/40 bg-signal/10 hover:bg-signal/20 text-signal text-xs font-medium transition-all cursor-pointer disabled:opacity-40 disabled:cursor-not-allowed flex items-center gap-1.5"
              >
                <Link size={12} />
                {connecting ? strings.workspace.connectingBtn : strings.workspace.connectBtn}
              </button>
            </div>
          </>
        ) : (
          <>
            <SettingsRow title={strings.workspace.remoteUrl} desc="">
              <span className="text-xs font-mono text-muted-foreground truncate max-w-72 flex items-center gap-1.5">
                <GitBranch size={11} className="flex-shrink-0" />
                {syncConfig?.remote}
              </span>
            </SettingsRow>
            <SettingsRow title={strings.workspace.branch} desc="">
              <span className="text-xs font-mono text-muted-foreground">{syncConfig?.branch ?? "main"}</span>
            </SettingsRow>
            <SettingsRow title={strings.workspace.autoSync} desc={strings.workspace.autoSyncDesc}>
              <Switch
                checked={syncConfig?.autoSync ?? false}
                ariaLabel={strings.workspace.autoSync}
                onChange={handleAutoSyncToggle}
              />
            </SettingsRow>
            <div className="px-5 py-3 border-t border-border flex flex-col gap-2">
              <div className="flex items-center justify-between">
                <SyncStatusBadge status={syncState.status} progressMessage={syncState.progressMessage} />
                <div className="flex items-center gap-2">
                  <button
                    onClick={handlePull}
                    disabled={syncState.status !== "idle"}
                    title={strings.workspace.pullBtn}
                    className="px-2.5 py-1 rounded border border-border bg-card hover:bg-surface-2 text-muted-foreground hover:text-foreground text-xs transition-all cursor-pointer disabled:opacity-40 disabled:cursor-not-allowed flex items-center gap-1"
                  >
                    <ArrowDown size={11} />
                    {syncState.status === "pulling" ? strings.workspace.pullingBtn : strings.workspace.pullBtn}
                  </button>
                  <button
                    onClick={handlePush}
                    disabled={syncState.status !== "idle"}
                    title={strings.workspace.pushBtn}
                    className="px-2.5 py-1 rounded border border-border bg-card hover:bg-surface-2 text-muted-foreground hover:text-foreground text-xs transition-all cursor-pointer disabled:opacity-40 disabled:cursor-not-allowed flex items-center gap-1"
                  >
                    <ArrowUp size={11} />
                    {syncState.status === "pushing" ? strings.workspace.pushingBtn : strings.workspace.pushBtn}
                  </button>
                </div>
              </div>
              <div className="flex items-center gap-4 text-[11px] text-muted-foreground">
                <span>
                  {strings.workspace.lastPushed}: <span className="text-foreground">{lastPushed ? formatRelTime(lastPushed) : strings.workspace.never}</span>
                </span>
                <span>
                  {strings.workspace.lastPulled}: <span className="text-foreground">{lastPulled ? formatRelTime(lastPulled) : strings.workspace.never}</span>
                </span>
              </div>
            </div>
            <div className="px-5 py-3 border-t border-border">
              {disconnectConfirm ? (
                <div className="flex items-center gap-2">
                  <span className="text-xs text-muted-foreground flex-1">{strings.workspace.disconnectWarning}</span>
                  <button onClick={() => setDisconnectConfirm(false)} className="px-2.5 py-1 rounded border border-border bg-card hover:bg-surface-2 text-muted-foreground text-xs cursor-pointer">
                    {strings.common.cancel}
                  </button>
                  <button onClick={handleDisconnect} className="px-2.5 py-1 rounded border border-destructive/40 bg-destructive/10 hover:bg-destructive/20 text-destructive text-xs cursor-pointer flex items-center gap-1">
                    <Unlink size={11} />
                    {strings.workspace.disconnectConfirm}
                  </button>
                </div>
              ) : (
                <button
                  onClick={() => setDisconnectConfirm(true)}
                  className="px-2.5 py-1 rounded border border-border bg-card hover:bg-surface-2 text-muted-foreground text-xs cursor-pointer flex items-center gap-1.5"
                >
                  <Unlink size={11} />
                  {strings.workspace.disconnectBtn}
                </button>
              )}
            </div>
          </>
        )}
        {connectError && (
          <div className="px-5 py-3 border-t border-destructive/20 bg-destructive/5 text-xs text-destructive">{connectError}</div>
        )}
      </SectionCard>
      {connectConfirm && (
        <div className="mt-3 p-4 rounded border border-signal/20 bg-signal/5 text-xs text-muted-foreground flex flex-col gap-3">
          <p className="text-foreground">
            {connectConfirm === "empty" ? strings.workspace.connectWarningEmpty : strings.workspace.connectWarningNonEmpty}
          </p>
          <div className="flex items-center gap-2">
            <button
              onClick={() => setConnectConfirm(null)}
              className="px-3 py-1.5 rounded border border-border bg-card hover:bg-surface-2 text-muted-foreground text-xs cursor-pointer"
            >
              {strings.workspace.cancelConnect}
            </button>
            <button
              onClick={handleConnectConfirm}
              className="px-3 py-1.5 rounded border border-signal/40 bg-signal/10 hover:bg-signal/20 text-signal text-xs font-medium cursor-pointer flex items-center gap-1.5"
            >
              <Link size={11} />
              {strings.workspace.confirmConnect}
            </button>
          </div>
        </div>
      )}
      {connecting && (
        <div className="mt-3 p-4 rounded border border-signal/20 bg-signal/5 flex items-center gap-3">
          <span className="inline-block w-4 h-4 border-2 border-signal border-t-transparent rounded-full animate-spin flex-shrink-0" />
          <span className="text-xs text-signal font-medium">
            {syncState.progressMessage ?? strings.workspace.connectingBtn}
          </span>
        </div>
      )}
    </section>
  );
}
