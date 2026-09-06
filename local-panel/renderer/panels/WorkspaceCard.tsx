import React, { useState, useCallback } from "react";
import { AppConfig, Workspace, SyncState } from "@/types";
import { strings } from "@/lib/strings";
import { Input, SectionLabel, SectionCard, SettingsRow } from "@/components/ui";
import { WorkspaceSyncSection } from "./workspace/WorkspaceSyncSection";
import { WorkspaceCardActions } from "./workspace/WorkspaceCardActions";

interface WorkspaceCardProps {
  config: AppConfig;
  workspace: Workspace;
  syncState: SyncState;
  onConfigChange: (cfg: AppConfig) => void;
  onWorkspaceDelete: (id: string) => Promise<void>;
  onWorkspaceRename: (id: string, name: string) => Promise<void>;
  onIeModalOpen: (mode: "import" | "export") => void;
  setSyncState: React.Dispatch<React.SetStateAction<SyncState>>;
}

export function WorkspaceCard({
  config,
  workspace,
  syncState,
  onConfigChange,
  onWorkspaceDelete,
  onWorkspaceRename,
  onIeModalOpen,
  setSyncState,
}: WorkspaceCardProps) {
  const wsId = workspace.id;
  const [nameInput, setNameInput] = useState(workspace.name);

  const handleNameBlur = useCallback(async () => {
    const trimmed = nameInput.trim();
    if (trimmed && trimmed !== workspace.name) {
      await onWorkspaceRename(wsId, trimmed);
    }
  }, [nameInput, workspace.name, wsId, onWorkspaceRename]);

  return (
    <div className="flex-1 overflow-y-auto px-4 py-5 md:px-6 md:py-6 flex flex-col gap-6 min-w-0">
      <section>
        <SectionLabel>{strings.workspace.sectionDetails}</SectionLabel>
        <SectionCard>
          <SettingsRow title={strings.workspace.workspaceName} desc="">
            <Input
              aria-label={strings.workspace.workspaceName}
              className="w-56 font-mono"
              value={nameInput}
              onChange={(e) => setNameInput(e.target.value)}
              onBlur={handleNameBlur}
              onKeyDown={(e) => { if (e.key === "Enter") e.currentTarget.blur(); }}
            />
          </SettingsRow>
        </SectionCard>
      </section>

      <WorkspaceSyncSection
        config={config}
        workspace={workspace}
        syncState={syncState}
        onConfigChange={onConfigChange}
        setSyncState={setSyncState}
      />

      <WorkspaceCardActions
        config={config}
        workspace={workspace}
        onWorkspaceDelete={onWorkspaceDelete}
        onIeModalOpen={onIeModalOpen}
      />
    </div>
  );
}
