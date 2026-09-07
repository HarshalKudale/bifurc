import { ipcMain } from "electron";
import { registerEntityCrudHandlers } from "@/ipc/handlers/entityCrudFactory";
import { loadConfig, generateId } from "@/store/config";
import { writeEntity, deleteEntityFile, readAllEntities } from "@/store/workspaceFs";

interface SavedGrpcRequest {
  id: string; name: string; serverAddress: string; serviceName: string; methodName: string;
  requestBody: string; metadata: Record<string, string>; protoFileId?: string | null;
  useReflection: boolean; streamingType: "unary" | "server" | "client" | "bidi";
  preScript?: string; postScript?: string;
  createdAt: number; folderId?: string | null; workspaceId: string;
}

interface SavedGrpcMock {
  id: string; name: string; enabled: boolean; serviceName: string; methodName: string;
  responseBody: string; responseMetadata: Record<string, string>; responseDelay?: number;
  streamingResponses?: string[]; errorCode?: number; errorMessage?: string;
  protoFileId: string; createdAt: number; folderId?: string | null; workspaceId: string;
}

interface SavedProtoFile {
  id: string; name: string; content: string;
  parsedServices?: { name: string; methods: { name: string; inputType: string; outputType: string; clientStreaming: boolean; serverStreaming: boolean }[] }[];
  createdAt: number; workspaceId: string;
}

export function registerGrpcHandlers() {
  registerEntityCrudHandlers<SavedGrpcRequest>({
    ipcAdd: "grpc:addRequest", ipcUpdate: "grpc:updateRequest", ipcDelete: "grpc:deleteRequest",
    kind: "grpcRequests",
    configKey: "grpcRequests" as any,
    folderConfigKey: "grpcRequestFolders" as any,
    validate: (req) => {
      if (!req.serviceName || !req.serviceName.trim()) throw new Error("serviceName is required for gRPC requests");
      if (!req.methodName || !req.methodName.trim()) throw new Error("methodName is required for gRPC requests");
    },
    getNameEntry: (req) => ({
      name: req.name,
      serverAddress: req.serverAddress,
      serviceName: req.serviceName,
      methodName: req.methodName,
    }),
  });

  registerEntityCrudHandlers<SavedGrpcMock>({
    ipcAdd: "grpc:addMock", ipcUpdate: "grpc:updateMock", ipcDelete: "grpc:deleteMock",
    kind: "grpcMocks",
    configKey: "grpcMocks" as any,
    folderConfigKey: "grpcMockFolders" as any,
    hasEnabledState: true,
    validate: (mock) => {
      if (!mock.serviceName || !mock.serviceName.trim()) throw new Error("serviceName is required for gRPC mocks");
      if (!mock.methodName || !mock.methodName.trim()) throw new Error("methodName is required for gRPC mocks");
    },
    getNameEntry: (mock) => ({
      name: mock.name,
      serviceName: mock.serviceName,
      methodName: mock.methodName,
    }),
  });

  ipcMain.handle("grpc:addProto", async (_e, proto: Omit<SavedProtoFile, "id" | "createdAt">) => {
    const cfg = loadConfig();
    const wsId = (proto as any).workspaceId ?? cfg.activeWorkspaceId;
    const newProto: SavedProtoFile = { ...proto, id: generateId(), createdAt: Date.now(), workspaceId: wsId };
    writeEntity(wsId, "protoFiles", newProto.id, newProto, null);
    return newProto;
  });

  ipcMain.handle("grpc:deleteProto", async (_e, id: string) => {
    const cfg = loadConfig();
    const wsId = cfg.activeWorkspaceId;
    deleteEntityFile(wsId, "protoFiles", id);
    return { ok: true };
  });

  ipcMain.handle("grpc:listProtos", async () => {
    const cfg = loadConfig();
    const wsId = cfg.activeWorkspaceId;
    return readAllEntities<SavedProtoFile>(wsId, "protoFiles");
  });

  ipcMain.handle("grpc:execute", async (_e, { serverAddress, serviceName, methodName, requestBody, metadata, protoFileId, useReflection }: {
    serverAddress: string; serviceName: string; methodName: string; requestBody: string;
    metadata: Record<string, string>; protoFileId: string | null; useReflection: boolean;
  }) => {
    return { ok: false, error: "gRPC runtime not yet configured. Install @grpc/grpc-js and @grpc/proto-loader to enable gRPC calls." };
  });

  ipcMain.handle("grpc:reflect", async (_e, { serverAddress }: { serverAddress: string }) => {
    return { ok: false, error: "gRPC runtime not yet configured. Install @grpc/grpc-js to enable server reflection." };
  });

  ipcMain.handle("grpc:mockServerStatus", async () => {
    return { running: false, port: 9102 };
  });

  ipcMain.handle("grpc:startMockServer", async () => {
    return { ok: false, error: "gRPC mock server not yet implemented. Install @grpc/grpc-js to enable." };
  });

  ipcMain.handle("grpc:stopMockServer", async () => {
    return { ok: true };
  });
}
