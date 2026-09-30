import React, { useCallback } from "react";
import { AppConfig, Environment } from "@/types";
import { Globe, Plus } from "@/lib/icons";
import { Button, EmptyState, StatusDot } from "@/components/ui";
import { strings } from "@/lib/strings";
import SidebarLayout, { SidebarHeader } from "@/components/ui/SidebarLayout";
import ActiveDot from "@/components/ui/ActiveDot";
import { useConfirmDialog } from "@/hooks/useConfirmDialog";
import { usePersistedState } from "@/hooks/usePersistedState";
import { EnvVariableTable } from "./EnvVariableTable";
import { entityGitView, entityGitDotColor } from "@/lib/entityGit";
import { useEntityPaths } from "@/lib/entityPaths";
import { History, RotateCcw, ArrowUp } from "@/lib/icons";

const GLOBAL_ENV_ID = "__global__";

interface Props {
  config: AppConfig;
  onConfigChange: (cfg: AppConfig) => Promise<void>;
  onHistoryOpen?: (filePath: string) => void;
  onAfterSave?: () => void;
  /** Per-environment git status, keyed by workspace-relative path. */
  entitySyncStatus?: Record<string, "clean" | "modified" | "new" | "deleted">;
  /** The workspace's position relative to its remote — needed for "committed but not pushed". */
  workspaceGitState?: import("@/types").WorkspaceGitState | null;
  /** Publish one environment by id. */
  onPublishItem?: (id: string) => void;
  /** Revert one environment by id — also backs "Recover" for a deleted one. */
  onRestoreItem?: (id: string) => void;
}


// -- Sidebar env item -------------------------------------------------------

/**
 * One row in the environments sidebar.
 *
 * ## Why the git actions are siblings of the button, not inside it
 *
 * The row is a `<button>` for selection, and a nested `<button>` is invalid HTML — the browser
 * hoists it out and the click handler stops working. So the row is a flex container with the
 * select button and the git actions beside it.
 *
 * ## Why the environments panel needed them at all
 *
 * Environments are publishable entities like every other kind, and this panel had **no** git
 * affordance whatsoever — no publish, no revert, no history. A published environment could only be
 * updated from the global footer, and could never be rolled back from the UI.
 */
function EnvItem({
  env,
  isActive,
  isSelected,
  isGlobal,
  view,
  onClick,
  onPublish,
  onRevert,
  onHistoryOpen,
  busy,
}: {
  env: Environment;
  isActive: boolean;
  isSelected: boolean;
  isGlobal: boolean;
  view?: import("@/lib/entityGit").EntityGitView;
  onClick: () => void;
  onPublish?: (id: string) => void;
  onRevert?: (id: string) => void;
  onHistoryOpen?: (filePath: string) => void;
  busy?: boolean;
}) {
  /** The engine's path for this environment (`plan/19` WI 5 item 3) — see `useEntityPaths`. */
  const { relPathOf } = useEntityPaths();

  return (
    <div className={`flex items-center gap-1 transition-colors group ${isSelected ? "bg-signal/15" : "hover:bg-card/50"}`}>
      <button
        onClick={onClick}
        className="flex flex-1 min-w-0 items-center gap-2 px-3 py-2 text-left transition-colors cursor-pointer"
      >
        {isGlobal ? (
          <Globe size={13} className={isSelected ? "text-signal" : "text-muted-foreground"} />
        ) : (
          <ActiveDot active={isActive} color="accent" size="sm" />
        )}
        <span className="flex-1 text-xs truncate">{env.name}</span>
        {view && view.status !== "clean" && (
          <span title={view.stateLabel} className="flex-shrink-0 flex items-center gap-1">
            <StatusDot color={entityGitDotColor(view.status)} />
            <span className="text-[9px] font-mono text-muted-foreground">{view.primaryLabel || view.stateLabel}</span>
          </span>
        )}
        {isGlobal && (
          <span className="text-[9px] font-semibold uppercase tracking-wide text-signal opacity-70 flex-shrink-0">
            {strings.environments.alwaysActive}
          </span>
        )}
      </button>
      <div className="flex items-center gap-1 pr-2 flex-shrink-0 opacity-0 group-hover:opacity-100 focus-within:opacity-100 transition-opacity">
        {view?.primary && onPublish && (
          <button
            onClick={() => onPublish(env.id)}
            disabled={busy}
            title={view.primaryTitle}
            className="px-1.5 py-0.5 rounded border border-border bg-card hover:bg-surface-2 disabled:opacity-40 text-[9px] font-medium text-foreground cursor-pointer flex items-center gap-1"
          >
            <ArrowUp size={9} />
            {view.primaryLabel}
          </button>
        )}
        {view?.canRevert && onRevert && (
          <button
            onClick={() => onRevert(env.id)}
            title={strings.entityGit.revertTooltip}
            className="px-1.5 py-0.5 rounded border border-border bg-card hover:bg-surface-2 text-[9px] font-medium text-amber cursor-pointer flex items-center gap-1"
          >
            <RotateCcw size={9} />
            {strings.entityGit.revert}
          </button>
        )}
        {view?.canViewHistory && onHistoryOpen && (
          <button
            onClick={() => onHistoryOpen(relPathOf(env.id))}
            title={strings.entityGit.historyTooltip}
            className="px-1.5 py-0.5 rounded border border-border bg-card hover:bg-surface-2 text-[9px] font-medium text-muted-foreground hover:text-signal cursor-pointer"
          >
            <History size={9} />
          </button>
        )}
      </div>
    </div>
  );
}

// -- EnvironmentsPanel ------------------------------------------------------

export default function EnvironmentsPanel({ config, onConfigChange, onHistoryOpen, onAfterSave, entitySyncStatus, workspaceGitState, onPublishItem, onRestoreItem }: Props) {
  const allEnvs = config.environments ?? [];
  const activeId = config.activeEnvironmentId ?? null;

  const globalEnv = allEnvs.find((e) => e.id === GLOBAL_ENV_ID) ?? null;
  const userEnvs = allEnvs.filter((e) => e.id !== GLOBAL_ENV_ID);

  const [selectedEnvId, setSelectedEnvId] = usePersistedState<string>(`environments:${config.activeWorkspaceId}:selected`, GLOBAL_ENV_ID);
  const [sidebarOpen, setSidebarOpen] = usePersistedState(`environments:${config.activeWorkspaceId}:sidebar-open`, true);
  const { confirm, ConfirmDialogElement } = useConfirmDialog();

  const selectedEnv = allEnvs.find((e) => e.id === selectedEnvId) ?? globalEnv;

  const reloadConfig = useCallback(async () => {
    const fresh = await window.api.getConfig();
    await onConfigChange(fresh);
  }, [onConfigChange]);

  const handleAdd = async () => {
    const newEnv = await window.api.addEnvironment({ name: "New Environment", variables: [] });
    await reloadConfig();
    if (newEnv?.id) setSelectedEnvId(newEnv.id);
  };

  const handleSave = useCallback(async (updated: Environment) => {
    await window.api.updateEnvironment(updated);
    await reloadConfig();
    onAfterSave?.();
  }, [reloadConfig, onAfterSave]);

  const handleDelete = useCallback(async (id: string) => {
    const ok = await confirm("Delete this environment? All variables will be lost.");
    if (!ok) return;
    await window.api.deleteEnvironment(id);
    setSelectedEnvId(GLOBAL_ENV_ID);
    await reloadConfig();
  }, [reloadConfig, confirm]);

  const handleActivate = useCallback(async (id: string) => {
    await window.api.setActiveEnvironment(id);
    await reloadConfig();
  }, [reloadConfig]);

  /** The engine's `environments/<id>.json` (`plan/19` WI 5 item 3) — a flat kind, so no folder level. */
  const { relPathOf } = useEntityPaths();

  /**
   * One environment's git view, from the shared status map, keyed by the engine's path — the same string
   * `statusTracker` produces.
   */
  const envView = (id: string) =>
    entityGitView(entitySyncStatus?.[relPathOf(id)], workspaceGitState);

  const renderEnvItem = (env: Environment, isGlobal: boolean) => (
    <EnvItem
      key={env.id}
      env={env}
      isActive={env.id === activeId}
      isSelected={selectedEnvId === env.id}
      isGlobal={isGlobal}
      view={envView(env.id)}
      onClick={() => setSelectedEnvId(env.id)}
      onPublish={onPublishItem}
      onRevert={onRestoreItem}
      onHistoryOpen={onHistoryOpen}
    />
  );

  const sidebar = (
    <div className="flex flex-col flex-1 overflow-hidden">
      <SidebarHeader onCollapse={() => setSidebarOpen(false)}>
        <span className="text-xs font-semibold text-muted-foreground uppercase tracking-wider px-1">{strings.environments.title}</span>
      </SidebarHeader>

      <div className="flex flex-col flex-1 overflow-y-auto">
        {/* Global always at top */}
        {globalEnv && renderEnvItem(globalEnv, true)}

        {userEnvs.length > 0 && (
          <div className="border-t border-border/40 my-1" />
        )}

        {userEnvs.map((env) => renderEnvItem(env, false))}
      </div>

      {/* New env button at bottom */}
      <div className="flex-shrink-0 border-t border-border/40 p-2">
        <Button variant="secondary" icon={<Plus size={12} />} onClick={handleAdd} className="w-full justify-center">
          {strings.environments.newEnvironment}
        </Button>
      </div>
    </div>
  );

  const content = selectedEnv ? (
    <EnvVariableTable
      key={selectedEnv.id}
      env={selectedEnv}
      isActive={selectedEnv.id === activeId}
      isGlobal={selectedEnv.id === GLOBAL_ENV_ID}
      onSave={handleSave}
      onDelete={() => handleDelete(selectedEnv.id)}
      onActivate={() => handleActivate(selectedEnv.id)}
      onHistory={onHistoryOpen ? () => onHistoryOpen(`environments/${selectedEnv.id}.json`) : undefined}
    />
  ) : (
    <div className="flex flex-col flex-1 items-center justify-center">
      <EmptyState
        fill
        icon={<Globe size={36} />}
        title={strings.environments.noEnvironments}
        description={
          <>
            {strings.environments.noEnvironmentsDesc}{" "}
            {strings.environments.useVarPrefix} <code className="font-mono bg-surface-2 px-1 rounded">{"{{VAR}}"}</code> {strings.environments.useVarSuffix}
          </>
        }
      />
    </div>
  );

  return (
    <>
      <SidebarLayout
        sidebarOpen={sidebarOpen}
        onToggle={() => setSidebarOpen(true)}
        sidebar={sidebar}
        storageKey="environments-panel-sidebar"
      >
        {content}
      </SidebarLayout>
      {ConfirmDialogElement}
    </>
  );
}
