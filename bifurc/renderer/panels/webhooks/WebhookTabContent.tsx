import React from "react";
import WebhookEditor from "./WebhookEditor";
import { SavedWebhook, Folder, WebhookPayload, AppConfig } from "@/types";

interface Props {
  tabId: string;
  activeTab: string | null;
  isDraft: boolean;
  hook: SavedWebhook | null;
  webhookPort: number;
  handleNewSave: (tabId: string, data: any) => Promise<any>;
  handleTabSave: (tabId: string, data: any) => Promise<any>;
  wrappedCloseTab: (tabId: string) => void;
  folders: Folder[];
  tabPayloads: WebhookPayload[];
  isActivated: boolean;
  isAtLimit: boolean;
  setDirtyTabs: React.Dispatch<React.SetStateAction<Record<string, boolean>>>;
  onPublishItem?: (id: string) => void;
  onRestoreItem?: (id: string) => void;
  config: AppConfig;
  setLoadedEntities: React.Dispatch<React.SetStateAction<Record<string, any>>>;
  tabRefs: React.MutableRefObject<Record<string, any>>;
  onHistoryOpen?: (filePath: string) => void;
  relPath: string;
  syncStatus?: any;
}

export default function WebhookTabContent({
  tabId,
  activeTab,
  isDraft,
  hook,
  webhookPort,
  handleNewSave,
  handleTabSave,
  wrappedCloseTab,
  folders,
  tabPayloads,
  isActivated,
  isAtLimit,
  setDirtyTabs,
  onPublishItem,
  onRestoreItem,
  config,
  setLoadedEntities,
  tabRefs,
  onHistoryOpen,
  relPath,
  syncStatus,
}: Props) {
  const isTabActive = tabId === activeTab;
  
  return (
    <div style={{ display: isTabActive ? "flex" : "none", flexDirection: "column", height: "100%" }}>
      <WebhookEditor
        ref={(el) => { tabRefs.current[tabId] = el; }}
        tabId={tabId}
        webhookId={isDraft ? null : tabId}
        initial={hook}
        isNew={isDraft}
        webhookPort={webhookPort}
        onSave={(data) => isDraft ? handleNewSave(tabId, data) : handleTabSave(tabId, data)}
        onClose={() => wrappedCloseTab(tabId)}
        folders={folders}
        payloads={tabPayloads}
        isActive={isActivated}
        isAtLimit={isAtLimit && !isActivated}
        onDirtyChange={(dirty) => setDirtyTabs((prev) => ({ ...prev, [tabId]: dirty }))}
        onSync={onPublishItem ? async (savedId?: string) => {
          const targetId = savedId ?? tabId;
          await onPublishItem(targetId);
        } : undefined}
        onRevert={onRestoreItem ? async () => {
          await onRestoreItem(tabId);
          const res = await window.api.loadEntity(config.activeWorkspaceId, "webhooks", tabId);
          if (res.ok && res.entity) {
            const entity = res.entity as SavedWebhook;
            setLoadedEntities((prev) => ({ ...prev, [tabId]: entity }));
            tabRefs.current[tabId]?.refresh?.(entity);
          } else if (!res.ok) {
            wrappedCloseTab(tabId);
          }
          setDirtyTabs((prev) => ({ ...prev, [tabId]: false }));
        } : undefined}
        onHistory={onHistoryOpen && relPath && !isDraft ? () => onHistoryOpen(relPath) : undefined}
        syncStatus={syncStatus}
      />
    </div>
  );
}
