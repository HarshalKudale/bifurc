import React from "react";
import RestTab from "@/components/rest/RestTab";
import GraphQLTab from "@/components/graphql/GraphQLTab";
import GrpcTab from "@/components/grpc/GrpcTab";
import SoapTab from "@/components/soap/SoapTab";
import ProtocolSelectorTab from "@/components/editor/ProtocolSelectorTab";
import { AppConfig, Folder, Environment, ApiProtocol } from "@/types";

interface Props {
  tabId: string;
  activeTab: string | null;
  isUnsaved: boolean;
  currentProto: ApiProtocol | null;
  initialData: any;
  itemMeta: any;
  relPath: string;
  syncStatus: any;
  folders: Folder[];
  activeEnv: Environment | null;
  config: AppConfig;
  tabRefs: React.MutableRefObject<Record<string, any>>;
  setDraftProtocols: React.Dispatch<React.SetStateAction<Record<string, ApiProtocol | null>>>;
  handleSaveEntity: (tabId: string, protocol: ApiProtocol, data: any) => Promise<any>;
  closeTab: (id: string) => void;
  setDirtyTabs: React.Dispatch<React.SetStateAction<Record<string, boolean>>>;
  handleToggle: (id: string) => Promise<void>;
  onPublishItem?: (id: string) => Promise<void> | void;
  onRestoreItem?: (id: string) => Promise<void> | void;
  setLoadedEntities: React.Dispatch<React.SetStateAction<Record<string, any>>>;
  onHistoryOpen?: (filePath: string) => void;
}

export default function MockTabContent({
  tabId,
  activeTab,
  isUnsaved,
  currentProto,
  initialData,
  itemMeta,
  relPath,
  syncStatus,
  folders,
  activeEnv,
  config,
  tabRefs,
  setDraftProtocols,
  handleSaveEntity,
  closeTab,
  setDirtyTabs,
  handleToggle,
  onPublishItem,
  onRestoreItem,
  setLoadedEntities,
  onHistoryOpen,
}: Props) {
  if (isUnsaved && !currentProto) {
    return (
      <div
        className="absolute inset-0 flex flex-col overflow-hidden"
        style={{ display: activeTab === tabId ? "flex" : "none" }}
      >
        <ProtocolSelectorTab
          mode="mock"
          onSelect={(proto) => {
            setDraftProtocols((prev) => ({ ...prev, [tabId]: proto }));
          }}
        />
      </div>
    );
  }

  const activeProtocol: ApiProtocol = currentProto ?? "rest";

  return (
    <div
      className="absolute inset-0 flex flex-col overflow-hidden"
      style={{ display: activeTab === tabId ? "flex" : "none" }}
    >
      {activeProtocol === "rest" && (
        <RestTab
          ref={(el) => { tabRefs.current[tabId] = el; }}
          tabType="mock"
          tabId={tabId}
          draftTabId={isUnsaved ? tabId : null}
          initial={initialData}
          folders={folders}
          activeEnv={activeEnv}
          onSave={(data) => handleSaveEntity(tabId, "rest", data)}
          onClose={() => closeTab(tabId)}
          onDirtyChange={(dirty) => setDirtyTabs((prev) => ({ ...prev, [tabId]: dirty }))}
          showCurlImport={isUnsaved}
          enabled={isUnsaved ? undefined : itemMeta?.enabled}
          onToggleEnabled={isUnsaved ? undefined : () => handleToggle(tabId)}
          onSync={onPublishItem ? async (savedId?: string) => {
            await onPublishItem(savedId ?? tabId);
          } : undefined}
          onRevert={onRestoreItem ? async () => {
            await onRestoreItem(tabId);
            const res = await window.api.loadEntity(config.activeWorkspaceId, "mocks", tabId);
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
          tabType="mock"
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
            const res = await window.api.loadEntity(config.activeWorkspaceId, "graphqlMocks", tabId);
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
          tabType="mock"
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
            const res = await window.api.loadEntity(config.activeWorkspaceId, "grpcMocks", tabId);
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
          tabType="mock"
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
            const res = await window.api.loadEntity(config.activeWorkspaceId, "soapMocks", tabId);
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
