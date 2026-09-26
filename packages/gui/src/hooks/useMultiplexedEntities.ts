import { useMemo, useCallback } from "react";
import { AppConfig, ApiProtocol, Folder } from "@/types";
import { entityRelPath } from "@/lib/utils";

interface MultiplexedOptions {
  mode: "request" | "mock";
  config: AppConfig;
  reload: () => Promise<void>;
}

export function useMultiplexedEntities({ mode, config, reload }: MultiplexedOptions) {
  const isRequest = mode === "request";

  const restItems = isRequest ? (config.requests ?? []) : (config.mocks ?? []);
  // GraphQL / gRPC / SOAP entities exist only as requests — their mock counterparts were removed.
  const graphqlItems = isRequest ? (config.graphqlRequests ?? []) : [];
  const grpcItems = isRequest ? (config.grpcRequests ?? []) : [];
  const soapItems = isRequest ? (config.soapRequests ?? []) : [];

  // Unified folders deduplicated by ID
  const folders = useMemo(() => {
    const map = new Map<string, Folder>();
    const restFolders = isRequest ? (config.requestFolders ?? []) : (config.mockFolders ?? []);
    const gqlFolders = isRequest ? (config.graphqlRequestFolders ?? []) : [];
    const grpcF = isRequest ? (config.grpcRequestFolders ?? []) : [];
    const soapF = isRequest ? (config.soapRequestFolders ?? []) : [];

    restFolders.forEach((f) => map.set(f.id, f));
    gqlFolders.forEach((f) => { if (!map.has(f.id)) map.set(f.id, f); });
    grpcF.forEach((f) => { if (!map.has(f.id)) map.set(f.id, f); });
    soapF.forEach((f) => { if (!map.has(f.id)) map.set(f.id, f); });
    return Array.from(map.values());
  }, [isRequest, config]);

  // Map of entity id -> protocol
  const itemProtocolMap = useMemo(() => {
    const map = new Map<string, ApiProtocol>();
    restItems.forEach((m) => map.set(m.id, "rest"));
    graphqlItems.forEach((m) => map.set(m.id, "graphql"));
    grpcItems.forEach((m) => map.set(m.id, "grpc"));
    soapItems.forEach((m) => map.set(m.id, "soap"));
    return map;
  }, [restItems, graphqlItems, grpcItems, soapItems]);

  // Unified item metadata map for fast lookups
  const allItemsMap = useMemo(() => {
    const map = new Map<string, { id: string; name: string; folderId?: string | null; protocol: ApiProtocol; summary: string; methodBadge: string; enabled: boolean; relPath: string }>();

    restItems.forEach((r: any) => {
      map.set(r.id, {
        id: r.id,
        name: r.name || "",
        folderId: r.folderId ?? null,
        protocol: "rest",
        summary: isRequest ? (r.url || "REST Request") : (r.urlPattern || "REST Mock"),
        methodBadge: r.method || "GET",
        enabled: r.enabled ?? true,
        relPath: entityRelPath(isRequest ? "requests" : "mocks", r, folders),
      });
    });

    graphqlItems.forEach((m: any) => {
      map.set(m.id, {
        id: m.id,
        name: m.name || "",
        folderId: m.folderId ?? null,
        protocol: "graphql",
        summary: m.endpointUrl || "GraphQL Operation",
        methodBadge: "GQL",
        enabled: m.enabled ?? true,
        relPath: entityRelPath("graphqlRequests", m, folders),
      });
    });

    grpcItems.forEach((m: any) => {
      map.set(m.id, {
        id: m.id,
        name: m.name || "",
        folderId: m.folderId ?? null,
        protocol: "grpc",
        summary: m.serviceName && m.methodName ? `${m.serviceName}/${m.methodName}` : "gRPC Call",
        methodBadge: "gRPC",
        enabled: m.enabled ?? true,
        relPath: entityRelPath("grpcRequests", m, folders),
      });
    });

    soapItems.forEach((m: any) => {
      map.set(m.id, {
        id: m.id,
        name: m.name || "",
        folderId: m.folderId ?? null,
        protocol: "soap",
        summary: m.soapAction || m.endpointUrl || "SOAP Request",
        methodBadge: "SOAP",
        enabled: m.enabled ?? true,
        relPath: entityRelPath("soapRequests", m, folders),
      });
    });

    return map;
  }, [restItems, graphqlItems, grpcItems, soapItems, folders, isRequest]);

  // Unified entities array for useEntityTabs
  const allEntities = useMemo(() => {
    return [
      ...restItems,
      ...graphqlItems,
      ...grpcItems,
      ...soapItems,
    ];
  }, [restItems, graphqlItems, grpcItems, soapItems]);

  const resolveEntityKind = useCallback((id: string): string => {
    const proto = itemProtocolMap.get(id);
    if (proto === "graphql") return "graphqlRequests";
    if (proto === "grpc") return "grpcRequests";
    if (proto === "soap") return "soapRequests";
    return isRequest ? "requests" : "mocks";
  }, [itemProtocolMap, isRequest]);

  const handleSaveEntity = useCallback(async (tabId: string, protocol: ApiProtocol, data: any, isNew: boolean, currentLoadedEntities: Record<string, any>) => {
    let saved: any;
    if (protocol === "rest") {
      saved = isNew
        ? await (isRequest ? window.api.addRequest(data) : window.api.addMock(data))
        : await (isRequest ? window.api.updateRequest({ ...(currentLoadedEntities[tabId] ?? {}), ...data }) : window.api.updateMock({ ...(currentLoadedEntities[tabId] ?? {}), ...data }));
    } else if (protocol === "graphql") {
      saved = isNew
        ? await window.api.addGraphQLRequest(data)
        : await window.api.updateGraphQLRequest({ ...(currentLoadedEntities[tabId] ?? {}), ...data });
    } else if (protocol === "grpc") {
      saved = isNew
        ? await window.api.addGrpcRequest(data)
        : await window.api.updateGrpcRequest({ ...(currentLoadedEntities[tabId] ?? {}), ...data });
    } else if (protocol === "soap") {
      saved = isNew
        ? await window.api.addSoapRequest(data)
        : await window.api.updateSoapRequest({ ...(currentLoadedEntities[tabId] ?? {}), ...data });
    }
    await reload();
    return saved;
  }, [isRequest, reload]);

  const handleDelete = useCallback(async (id: string, closeTab: (id: string) => void) => {
    closeTab(id);
    const proto = itemProtocolMap.get(id);
    if (proto === "graphql") await window.api.deleteGraphQLRequest(id);
    else if (proto === "grpc") await window.api.deleteGrpcRequest(id);
    else if (proto === "soap") await window.api.deleteSoapRequest(id);
    else await (isRequest ? window.api.deleteRequest(id) : window.api.deleteMock(id));
    await reload();
  }, [itemProtocolMap, reload, isRequest]);

  const handleDeleteItems = useCallback(async (trackedIds: string[], untrackedIds: string[], closeTab: (id: string) => void) => {
    const allIds = [...trackedIds, ...untrackedIds];
    allIds.forEach((id) => closeTab(id));
    for (const id of allIds) {
      const proto = itemProtocolMap.get(id);
      if (proto === "graphql") await window.api.deleteGraphQLRequest(id);
      else if (proto === "grpc") await window.api.deleteGrpcRequest(id);
      else if (proto === "soap") await window.api.deleteSoapRequest(id);
      else await (isRequest ? window.api.deleteRequest(id) : window.api.deleteMock(id));
    }
    await reload();
  }, [itemProtocolMap, reload, isRequest]);

  const handleDuplicate = useCallback(async (id: string, currentLoadedEntities: Record<string, any>) => {
    const proto = itemProtocolMap.get(id) ?? "rest";
    const kind = resolveEntityKind(id);
    let item = currentLoadedEntities[id];
    if (!item) {
      const res = await window.api.loadEntity(config.activeWorkspaceId, kind, id);
      if (res.ok && res.entity) item = res.entity;
    }
    if (!item) return;
    const { id: _id, createdAt: _ca, workspaceId: _ws, ...rest } = item;
    const copyName = item.name ? `${item.name} (copy)` : "Copy";

    if (proto === "graphql") await window.api.addGraphQLRequest({ ...rest, name: copyName });
    else if (proto === "grpc") await window.api.addGrpcRequest({ ...rest, name: copyName });
    else if (proto === "soap") await window.api.addSoapRequest({ ...rest, name: copyName });
    else await (isRequest ? window.api.addRequest({ ...rest, name: copyName }) : window.api.addMock({ ...rest, name: copyName }));
    await reload();
  }, [itemProtocolMap, resolveEntityKind, config.activeWorkspaceId, reload, isRequest]);

  const handleMoveItems = useCallback(async (ids: string[], folderId: string | null, currentLoadedEntities: Record<string, any>) => {
    for (const id of ids) {
      const proto = itemProtocolMap.get(id) ?? "rest";
      const kind = resolveEntityKind(id);
      let item = currentLoadedEntities[id];
      if (!item) {
        const res = await window.api.loadEntity(config.activeWorkspaceId, kind, id);
        if (res.ok && res.entity) item = res.entity;
      }
      if (!item) continue;
      const updated = { ...item, folderId: folderId ?? undefined };
      if (proto === "graphql") await window.api.updateGraphQLRequest(updated);
      else if (proto === "grpc") await window.api.updateGrpcRequest(updated);
      else if (proto === "soap") await window.api.updateSoapRequest(updated);
      else await (isRequest ? window.api.updateRequest(updated) : window.api.updateMock(updated));
    }
    await reload();
  }, [itemProtocolMap, resolveEntityKind, config.activeWorkspaceId, reload, isRequest]);

  return {
    folders,
    itemProtocolMap,
    allItemsMap,
    allEntities,
    resolveEntityKind,
    handleSaveEntity,
    handleDelete,
    handleDeleteItems,
    handleDuplicate,
    handleMoveItems,
    restItems,
  };
}
