import React, { useState, useEffect } from "react";
import { AppConfig, SyncState, SyncStatus } from "@/types";
import { strings } from "@/lib/strings";
import PanelLayout from "@/components/ui/PanelLayout";
import ImportExportModal from "@/components/modals/ImportExportModal";
import AuditLogPanel from "@/panels/AuditLogPanel";
import { WorkspaceCard } from "@/panels/WorkspaceCard";

interface Props {
  config: AppConfig;
  onConfigChange: (cfg: AppConfig) => void;
  onWorkspaceDelete: (id: string) => Promise<void>;
  onWorkspaceRename: (id: string, name: string) => Promise<void>;
}

export default function WorkspacePanel({ config, onConfigChange, onWorkspaceDelete, onWorkspaceRename }: Props) {
  const wsId = config.activeWorkspaceId;
  const workspace = (config.workspaces ?? []).find((w) => w.id === wsId) ?? null;

  const [syncState, setSyncState] = useState<SyncState>({ status: "idle", error: null, lastPushedAt: null, lastPulledAt: null, progressMessage: null });
  const [ieModalMode, setIeModalMode] = useState<"import" | "export" | null>(null);

  useEffect(() => {
    if (!wsId) return;
    window.api.syncGetState(wsId).then(setSyncState).catch(() => { });
    const unsub = window.api.onSyncStatus((evt) => {
      if (evt.wsId === wsId) {
        setSyncState((prev) => ({
          ...prev,
          status: evt.status as SyncStatus,
          error: evt.error ?? null,
          progressMessage: (evt as any).progressMessage ?? null,
        }));
        // Refresh config to pick up updated syncMeta timestamps
        if (evt.status === "idle" || evt.status === "error") {
          window.api.getConfig().then((fresh) => onConfigChange(fresh)).catch(() => { });
        }
      }
    });
    return unsub;
  }, [wsId]);

  const handleImportDone = async () => {
    const fresh = await window.api.getConfig();
    onConfigChange(fresh);
  };

  if (!workspace) return null;

  return (
    <>
      <PanelLayout title={strings.workspace.title} subtitle={strings.workspace.subtitle} noPadding>
        <div className="flex flex-col lg:flex-row flex-1 h-full min-h-0 divide-y lg:divide-y-0 lg:divide-x divide-border overflow-hidden">
          {/* Left Column: Workspace Settings */}
          <WorkspaceCard
            config={config}
            workspace={workspace}
            syncState={syncState}
            onConfigChange={onConfigChange}
            onWorkspaceDelete={onWorkspaceDelete}
            onWorkspaceRename={onWorkspaceRename}
            onIeModalOpen={setIeModalMode}
            setSyncState={setSyncState}
          />

          {/* Right Column: Workspace Audit Log */}
          <div className="flex-1 min-h-0 flex flex-col overflow-hidden min-w-0">
            <AuditLogPanel activeWorkspaceId={wsId} embedded />
          </div>
        </div>

        <ImportExportModal
          open={ieModalMode !== null}
          mode={ieModalMode ?? "export"}
          wsId={wsId}
          onClose={() => setIeModalMode(null)}
          onImportDone={handleImportDone}
        />
      </PanelLayout>
    </>
  );
}

