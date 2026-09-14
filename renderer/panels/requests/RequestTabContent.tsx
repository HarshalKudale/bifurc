import React from "react";
import RestTab from "@/components/rest/RestTab";
import GraphQLTab from "@/components/graphql/GraphQLTab";
import GrpcTab from "@/components/grpc/GrpcTab";
import SoapTab from "@/components/soap/SoapTab";
import { AppConfig, Folder, Environment, ApiProtocol } from "@/types";

interface Props {
  tabId: string;
  activeTab: string | null;
  isUnsaved: boolean;
  activeProtocol: ApiProtocol;
  initialData: any;
  relPath: string;
  syncStatus: any;
  folders: Folder[];
  activeEnv: Environment | null;
  config: AppConfig;
  tabRefs: React.MutableRefObject<Record<string, any>>;
  handleSaveEntity: (tabId: string, protocol: ApiProtocol, data: any) => Promise<any>;
  closeTab: (id: string) => void;
  setDirtyTabs: React.Dispatch<React.SetStateAction<Record<string, boolean>>>;
  onOpenMockEditor?: (initial: any) => void;
  onPublishItem?: (id: string) => Promise<void> | void;
  onRestoreItem?: (id: string) => Promise<void> | void;
  setLoadedEntities: React.Dispatch<React.SetStateAction<Record<string, any>>>;
  onHistoryOpen?: (filePath: string) => void;
}

export default function RequestTabContent({
  tabId,
  activeTab,
  isUnsaved,
  activeProtocol,
  initialData,
  relPath,
  syncStatus,
  folders,
  activeEnv,
  config,
  tabRefs,
  handleSaveEntity,
  closeTab,
  setDirtyTabs,
  onOpenMockEditor,
  onPublishItem,
  onRestoreItem,
  setLoadedEntities,
  onHistoryOpen,
}: Props) {
  return (
    <div
      className="absolute inset-0 flex flex-col overflow-hidden"
      style={{ display: activeTab === tabId ? "flex" : "none" }}
    >
      {activeProtocol === "rest" && (
        <RestTab
          ref={(el) => { tabRefs.current[tabId] = el; }}
          tabType="request"
          tabId={tabId}
          draftTabId={isUnsaved ? tabId : null}
          initial={initialData}
          folders={folders}
          activeEnv={activeEnv}
          onSave={(data) => handleSaveEntity(tabId, "rest", data)}
          onCreateMock={(initial) => onOpenMockEditor?.(initial)}
          onClose={() => closeTab(tabId)}
          onDirtyChange={(dirty) => setDirtyTabs((prev) => ({ ...prev, [tabId]: dirty }))}
          showCurlImport={isUnsaved}
          onSync={onPublishItem ? async (savedId?: string) => {
            await onPublishItem(savedId ?? tabId);
          } : undefined}
          onRevert={onRestoreItem ? async () => {
            await onRestoreItem(tabId);
            const res = await window.api.loadEntity(config.activeWorkspaceId, "requests", tabId);
            if (res.ok && res.entity) {
              setLoadedEntities((prev) => ({ ...prev, [tabId]: res.entity }));
              tabRefs.current[tabId]?.refresh?.(res.entity);
            } else if (!res.ok) {
              closeTab(tabId);
            }
            setDirtyTabs((prev) => ({ ...prev, [tabId]: false }));
          } : undefined}
          onHistory={onHistoryOpen && relPath && !isUnsaved ? () => onHistoryOpen(relPath) : undefined}
          syncStatus={syncStatus}
        />
      )}

      {activeProtocol === "graphql" && (
        <GraphQLTab
          ref={(el) => { tabRefs.current[tabId] = el; }}
          tabType="request"
          tabId={tabId}
          draftTabId={isUnsaved ? tabId : null}
          initial={initialData}
          folders={folders}
          activeEnv={activeEnv}
          onSave={(data) => handleSaveEntity(tabId, "graphql", data)}
          onClose={() => closeTab(tabId)}
          onDirtyChange={(dirty) => setDirtyTabs((prev) => ({ ...prev, [tabId]: dirty }))}
          onSync={onPublishItem ? async (savedId?: string) => {
            await onPublishItem(savedId ?? tabId);
          } : undefined}
          onRevert={onRestoreItem ? async () => {
            await onRestoreItem(tabId);
            const res = await window.api.loadEntity(config.activeWorkspaceId, "graphqlRequests", tabId);
            if (res.ok && res.entity) {
              setLoadedEntities((prev) => ({ ...prev, [tabId]: res.entity }));
              tabRefs.current[tabId]?.refresh?.(res.entity);
            } else if (!res.ok) {
              closeTab(tabId);
            }
            setDirtyTabs((prev) => ({ ...prev, [tabId]: false }));
          } : undefined}
          onHistory={onHistoryOpen && relPath && !isUnsaved ? () => onHistoryOpen(relPath) : undefined}
          syncStatus={syncStatus}
        />
      )}

      {activeProtocol === "grpc" && (
        <GrpcTab
          ref={(el) => { tabRefs.current[tabId] = el; }}
          tabType="request"
          tabId={tabId}
          draftTabId={isUnsaved ? tabId : null}
          initial={initialData}
          folders={folders}
          activeEnv={activeEnv}
          onSave={(data) => handleSaveEntity(tabId, "grpc", data)}
          onClose={() => closeTab(tabId)}
          onSync={onPublishItem ? async (savedId?: string) => {
            await onPublishItem(savedId ?? tabId);
          } : undefined}
          onRevert={onRestoreItem ? async () => {
            await onRestoreItem(tabId);
            const res = await window.api.loadEntity(config.activeWorkspaceId, "grpcRequests", tabId);
            if (res.ok && res.entity) {
              setLoadedEntities((prev) => ({ ...prev, [tabId]: res.entity }));
              tabRefs.current[tabId]?.refresh?.(res.entity);
            } else if (!res.ok) {
              closeTab(tabId);
            }
            setDirtyTabs((prev) => ({ ...prev, [tabId]: false }));
          } : undefined}
          onHistory={onHistoryOpen && relPath && !isUnsaved ? () => onHistoryOpen(relPath) : undefined}
          syncStatus={syncStatus}
        />
      )}

      {activeProtocol === "soap" && (
        <SoapTab
          ref={(el) => { tabRefs.current[tabId] = el; }}
          tabType="request"
          tabId={tabId}
          draftTabId={isUnsaved ? tabId : null}
          initial={initialData}
          folders={folders}
          activeEnv={activeEnv}
          onSave={(data) => handleSaveEntity(tabId, "soap", data)}
          onClose={() => closeTab(tabId)}
          onDirtyChange={(dirty) => setDirtyTabs((prev) => ({ ...prev, [tabId]: dirty }))}
          onSync={onPublishItem ? async (savedId?: string) => {
            await onPublishItem(savedId ?? tabId);
          } : undefined}
          onRevert={onRestoreItem ? async () => {
            await onRestoreItem(tabId);
            const res = await window.api.loadEntity(config.activeWorkspaceId, "soapRequests", tabId);
            if (res.ok && res.entity) {
              setLoadedEntities((prev) => ({ ...prev, [tabId]: res.entity }));
              tabRefs.current[tabId]?.refresh?.(res.entity);
            } else if (!res.ok) {
              closeTab(tabId);
            }
            setDirtyTabs((prev) => ({ ...prev, [tabId]: false }));
          } : undefined}
          onHistory={onHistoryOpen && relPath && !isUnsaved ? () => onHistoryOpen(relPath) : undefined}
          syncStatus={syncStatus}
        />
      )}
    </div>
  );
}
