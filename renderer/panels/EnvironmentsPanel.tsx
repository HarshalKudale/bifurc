import React, { useCallback } from "react";
import { AppConfig, Environment } from "@/types";
import { Globe, Plus } from "@/lib/icons";
import { Button, EmptyState } from "@/components/ui";
import { strings } from "@/lib/strings";
import SidebarLayout, { SidebarHeader } from "@/components/ui/SidebarLayout";
import ActiveDot from "@/components/ui/ActiveDot";
import { useConfirmDialog } from "@/hooks/useConfirmDialog";
import { usePersistedState } from "@/hooks/usePersistedState";
import { EnvVariableTable } from "./EnvVariableTable";

const GLOBAL_ENV_ID = "__global__";

interface Props {
  config: AppConfig;
  onConfigChange: (cfg: AppConfig) => Promise<void>;
  onHistoryOpen?: (filePath: string) => void;
  onAfterSave?: () => void;
}


// -- Sidebar env item -------------------------------------------------------

function EnvItem({
  env,
  isActive,
  isSelected,
  isGlobal,
  onClick,
}: {
  env: Environment;
  isActive: boolean;
  isSelected: boolean;
  isGlobal: boolean;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      className={`w-full flex items-center gap-2 px-3 py-2 text-left transition-colors cursor-pointer ${
        isSelected ? "bg-signal/15 text-foreground" : "hover:bg-card/50 text-foreground"
      }`}
    >
      {isGlobal ? (
        <Globe size={13} className={isSelected ? "text-signal" : "text-muted-foreground"} />
      ) : (
        <ActiveDot active={isActive} color="accent" size="sm" />
      )}
      <span className="flex-1 text-xs truncate">{env.name}</span>
      {isGlobal && (
        <span className="text-[9px] font-semibold uppercase tracking-wide text-signal opacity-70 flex-shrink-0">
          {strings.environments.alwaysActive}
        </span>
      )}
    </button>
  );
}

// -- EnvironmentsPanel ------------------------------------------------------

export default function EnvironmentsPanel({ config, onConfigChange, onHistoryOpen, onAfterSave }: Props) {
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

  const sidebar = (
    <div className="flex flex-col flex-1 overflow-hidden">
      <SidebarHeader onCollapse={() => setSidebarOpen(false)}>
        <span className="text-xs font-semibold text-muted-foreground uppercase tracking-wider px-1">{strings.environments.title}</span>
      </SidebarHeader>

      <div className="flex flex-col flex-1 overflow-y-auto">
        {/* Global always at top */}
        {globalEnv && (
          <EnvItem
            env={globalEnv}
            isActive={false}
            isSelected={selectedEnvId === GLOBAL_ENV_ID}
            isGlobal={true}
            onClick={() => setSelectedEnvId(GLOBAL_ENV_ID)}
          />
        )}

        {userEnvs.length > 0 && (
          <div className="border-t border-border/40 my-1" />
        )}

        {userEnvs.map((env) => (
          <EnvItem
            key={env.id}
            env={env}
            isActive={env.id === activeId}
            isSelected={selectedEnvId === env.id}
            isGlobal={false}
            onClick={() => setSelectedEnvId(env.id)}
          />
        ))}
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
