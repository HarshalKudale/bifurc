import { useCallback } from "react";
import { AppConfig, Folder } from "@/types";
import { entityRelPath, flatEntityRelPath } from "@/lib/utils";

function getItemFromConfig(cfg: any, kind: string, id: string) {
  if (kind === "rules") return (cfg.proxyRules ?? []).find((r: any) => r.id === id);
  if (kind === "requests") return (cfg.requests ?? []).find((r: any) => r.id === id);
  if (kind === "mocks") return (cfg.mocks ?? []).find((m: any) => m.id === id);
  if (kind === "webhooks") return (cfg.webhooks ?? []).find((h: any) => h.id === id);
  if (kind === "sockets") return (cfg.wsConnections ?? []).find((c: any) => c.id === id);
  if (kind === "graphqlRequests") return (cfg.graphqlRequests ?? []).find((r: any) => r.id === id);
  if (kind === "graphqlMocks") return (cfg.graphqlMocks ?? []).find((m: any) => m.id === id);
  if (kind === "soapRequests") return (cfg.soapRequests ?? []).find((r: any) => r.id === id);
  if (kind === "soapMocks") return (cfg.soapMocks ?? []).find((m: any) => m.id === id);
  if (kind === "grpcRequests") return (cfg.grpcRequests ?? []).find((r: any) => r.id === id);
  if (kind === "grpcMocks") return (cfg.grpcMocks ?? []).find((m: any) => m.id === id);
  return null;
}

export function usePublishHandlers({
  wsConfig,
  wsId,
  refreshConfig,
  refreshEntitySyncStatus,
  panel
}: {
  wsConfig: AppConfig;
  wsId: string;
  refreshConfig: () => Promise<void>;
  refreshEntitySyncStatus: (id: string) => void;
  panel: string;
}) {
  const makePublishItem = useCallback((kind: string, folders: Folder[]) =>
    async (id: string) => {
      let item = getItemFromConfig(wsConfig, kind, id);
      if (!item) {
        const fresh = await window.api.getConfig();
        const ws = (fresh.workspaces ?? []).find((w: any) => w.id === wsId) ?? fresh;
        item = getItemFromConfig(ws, kind, id);
      }
      if (!item) return;
      const relPath = entityRelPath(kind, item as any, folders);
      await window.api.publishEntity(wsId, [relPath]);
      await refreshConfig();
      refreshEntitySyncStatus(wsId);
    },
    [wsConfig, wsId, refreshEntitySyncStatus, refreshConfig]
  );

  const makePublishFolder = useCallback((kind: "requests" | "mocks" | "sockets" | "webhooks" | "rules", folders: Folder[]) =>
    async (folderId: string | null) => {
      await window.api.publishFolder(wsId, kind, folderId ? folders.find((f) => f.id === folderId)?.name ?? null : null);
      await refreshConfig();
      refreshEntitySyncStatus(wsId);
    },
    [wsId, refreshEntitySyncStatus, refreshConfig]
  );

  const makeFlatPublish = useCallback((kind: "mappings") =>
    async (id: string) => {
      const relPath = flatEntityRelPath(kind, id);
      await window.api.publishEntity(wsId, [relPath]);
      refreshEntitySyncStatus(wsId);
    },
    [wsId, refreshEntitySyncStatus]
  );

  const makeFlatRevert = useCallback((kind: "mappings") =>
    async (id: string) => {
      const relPath = flatEntityRelPath(kind, id);
      await window.api.gitDiscard(wsId, relPath);
      await refreshConfig();
      refreshEntitySyncStatus(wsId);
    },
    [wsId, refreshEntitySyncStatus, refreshConfig]
  );

  const handlePublishHealthBar = useCallback(async () => {
    await window.api.publishEntity(wsId, ["healthbar/services.json"]);
    refreshEntitySyncStatus(wsId);
  }, [wsId, refreshEntitySyncStatus]);

  const makeRestoreItem = useCallback((kind: string, folders: Folder[]) =>
    async (id: string) => {
      let item = getItemFromConfig(wsConfig, kind, id);
      if (!item) {
        const fresh = await window.api.getConfig();
        const ws = (fresh.workspaces ?? []).find((w: any) => w.id === wsId) ?? fresh;
        item = getItemFromConfig(ws, kind, id);
      }
      if (!item) return;
      const relPath = entityRelPath(kind, item as any, folders);
      await window.api.gitDiscard(wsId, relPath);
      await refreshConfig();
      refreshEntitySyncStatus(wsId);
    },
    [wsConfig, wsId, refreshEntitySyncStatus, refreshConfig]
  );

  const PANEL_PUBLISH_KIND: Partial<Record<string, string>> = {
    requests: "requests",
    mocks: "mocks",
    "mock-rest": "mocks",
    "req-rest": "requests",
    sockets: "sockets",
    webhooks: "webhooks",
    mappings: "mappings",
    rules: "rules",
    environments: "environments",
    healthbar: "healthbar",
  };

  const handlePublishPanel = useCallback(async () => {
    const kind = PANEL_PUBLISH_KIND[panel];
    if (!kind) return;
    if (kind === "healthbar") {
      await window.api.publishEntity(wsId, ["healthbar/services.json"]);
    } else {
      await window.api.publishFolder(wsId, kind, null);
    }
    await refreshConfig();
    refreshEntitySyncStatus(wsId);
  }, [panel, wsId, refreshConfig, refreshEntitySyncStatus]);

  return {
    makePublishItem,
    makePublishFolder,
    makeFlatPublish,
    makeFlatRevert,
    makeRestoreItem,
    handlePublishHealthBar,
    handlePublishPanel,
  };
}
