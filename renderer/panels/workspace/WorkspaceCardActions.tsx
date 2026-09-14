import React, { useState } from "react";
import { AppConfig, Workspace } from "@/types";
import { strings } from "@/lib/strings";
import { Button, SectionLabel, SectionCard, SettingsRow } from "@/components/ui";

interface WorkspaceCardActionsProps {
  config: AppConfig;
  workspace: Workspace;
  onWorkspaceDelete: (id: string) => Promise<void>;
  onIeModalOpen: (mode: "import" | "export") => void;
}

export function WorkspaceCardActions({
  config,
  workspace,
  onWorkspaceDelete,
  onIeModalOpen,
}: WorkspaceCardActionsProps) {
  const [deleteConfirm, setDeleteConfirm] = useState(false);

  const handleDeleteConfirm = async () => {
    setDeleteConfirm(false);
    await onWorkspaceDelete(workspace.id);
  };

  return (
    <>
      <section>
        <SectionLabel>{strings.workspace.sectionData}</SectionLabel>
        <SectionCard>
          <SettingsRow title={strings.workspace.exportBtn} desc={strings.workspace.exportDesc}>
            <Button variant="secondary" onClick={() => onIeModalOpen("export")}>{strings.common.export}</Button>
          </SettingsRow>
          <SettingsRow title={strings.workspace.importBtn} desc={strings.workspace.importDesc}>
            <Button variant="secondary" onClick={() => onIeModalOpen("import")}>{strings.common.import}</Button>
          </SettingsRow>
        </SectionCard>
      </section>

      <section>
        <SectionLabel>{strings.workspace.sectionDanger}</SectionLabel>
        <div className="bg-surface border border-destructive/20 rounded-lg overflow-hidden">
          <div className="flex items-center gap-4 px-5 py-4">
            <div className="flex-1 min-w-0">
              <div className="text-sm font-medium text-foreground">{strings.workspace.deleteBtn}</div>
              <div className="text-xs text-muted-foreground mt-0.5">{strings.workspace.deleteDesc}</div>
            </div>
            <div className="flex-shrink-0">
              {deleteConfirm ? (
                <div className="flex items-center gap-2">
                  <button onClick={() => setDeleteConfirm(false)} className="px-2.5 py-1 rounded border border-border bg-card hover:bg-surface-2 text-muted-foreground text-xs cursor-pointer">
                    {strings.common.cancel}
                  </button>
                  <button onClick={handleDeleteConfirm} className="px-2.5 py-1 rounded border border-destructive/40 bg-destructive/10 hover:bg-destructive/20 text-destructive text-xs font-medium cursor-pointer">
                    {strings.workspace.deleteConfirmBtn}
                  </button>
                </div>
              ) : (
                <button
                  onClick={() => setDeleteConfirm(true)}
                  disabled={config.workspaces.length <= 1}
                  className="px-3 py-1.5 rounded border border-destructive/40 bg-destructive/5 hover:bg-destructive/15 text-destructive text-xs font-medium transition-all cursor-pointer disabled:opacity-40 disabled:cursor-not-allowed"
                >
                  {strings.workspace.deleteBtn}
                </button>
              )}
            </div>
          </div>
        </div>
      </section>
    </>
  );
}
