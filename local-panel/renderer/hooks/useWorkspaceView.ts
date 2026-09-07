import { useMemo } from "react";
import { AppConfig } from "@/types";

export function useWorkspaceView(config: AppConfig, wsId: string): AppConfig {
  return useMemo<AppConfig>(() => ({
    ...config,
    mappings: (config.mappings ?? []).filter((m) => m.workspaceId === wsId),
    proxyRules: (config.proxyRules ?? []).filter((r) => r.workspaceId === wsId),
    ruleFolders: (config.ruleFolders ?? []).filter((f) => f.workspaceId === wsId),
    mocks: (config.mocks ?? []).filter((m) => m.workspaceId === wsId),
    requests: (config.requests ?? []).filter((r) => r.workspaceId === wsId),
    mockFolders: (config.mockFolders ?? []).filter((f) => f.workspaceId === wsId),
    requestFolders: (config.requestFolders ?? []).filter((f) => f.workspaceId === wsId),
    wsConnections: (config.wsConnections ?? []).filter((c) => c.workspaceId === wsId),
    wsFolders: (config.wsFolders ?? []).filter((f) => f.workspaceId === wsId),
    webhooks: (config.webhooks ?? []).filter((h) => h.workspaceId === wsId),
    webhookFolders: (config.webhookFolders ?? []).filter((f) => f.workspaceId === wsId),
    graphqlRequests: (config.graphqlRequests ?? []).filter((r) => r.workspaceId === wsId),
    graphqlMocks: (config.graphqlMocks ?? []).filter((m) => m.workspaceId === wsId),
    graphqlSchemas: (config.graphqlSchemas ?? []),
    graphqlRequestFolders: (config.graphqlRequestFolders ?? []).filter((f) => f.workspaceId === wsId),
    graphqlMockFolders: (config.graphqlMockFolders ?? []).filter((f) => f.workspaceId === wsId),
    grpcRequests: (config.grpcRequests ?? []).filter((r) => r.workspaceId === wsId),
    grpcMocks: (config.grpcMocks ?? []).filter((m) => m.workspaceId === wsId),
    protoFiles: (config.protoFiles ?? []),
    grpcRequestFolders: (config.grpcRequestFolders ?? []).filter((f) => f.workspaceId === wsId),
    grpcMockFolders: (config.grpcMockFolders ?? []).filter((f) => f.workspaceId === wsId),
    soapRequests: (config.soapRequests ?? []).filter((r) => r.workspaceId === wsId),
    soapMocks: (config.soapMocks ?? []).filter((m) => m.workspaceId === wsId),
    savedWsdls: (config.savedWsdls ?? []),
    soapRequestFolders: (config.soapRequestFolders ?? []).filter((f) => f.workspaceId === wsId),
    soapMockFolders: (config.soapMockFolders ?? []).filter((f) => f.workspaceId === wsId),
    environments: (config.environments ?? []).filter((e) => e.workspaceId === wsId),
  }), [config, wsId]);
}
