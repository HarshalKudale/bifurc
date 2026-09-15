import { ipcMain } from "electron";
import type {
  GrpcExecuteParams, GrpcReflectParams, GrpcMockServerStatusParams,
  GrpcStartMockServerParams, GrpcStopMockServerParams,
} from "@bifurc/protocol";
import { registerEntityCrudHandlers } from "@/ipc/handlers/entityCrudFactory";
import { loadConfig, generateId } from "@/store/config";
import { writeEntity, deleteEntityFile, readAllEntities } from "@/store/workspaceFs";
import { commandRegistry } from "@/commands/registry";
import { bus } from "@/eventBus";

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

// P2 work item 7 — grpc.execute/reflect/mockServerStatus/startMockServer/stopMockServer are pure
// stubs (pinned by tests/integration/protocolExecution.integration.test.ts, Cleanup_plan.md
// §1.5 D6 disposition: wontfix — see packages/protocol/src/commands/grpc.ts's own comment), and
// their params match the frozen schema exactly, so all five convert. The CRUD-factory channels
// and grpc:addProto/deleteProto/listProtos (no protocol command yet) wait for the CRUD collapse.
const ctx = { bus };

commandRegistry.register("grpc.execute", async (_params: GrpcExecuteParams) => {
  return { ok: false, error: "gRPC runtime not yet configured. Install @grpc/grpc-js and @grpc/proto-loader to enable gRPC calls." };
});

commandRegistry.register("grpc.reflect", async (_params: GrpcReflectParams) => {
  return { ok: false, error: "gRPC runtime not yet configured. Install @grpc/grpc-js to enable server reflection." };
});

commandRegistry.register("grpc.mockServerStatus", async (_params: GrpcMockServerStatusParams) => {
  return { running: false, port: 9102 };
});

commandRegistry.register("grpc.startMockServer", async (_params: GrpcStartMockServerParams) => {
  return { ok: false, error: "gRPC mock server not yet implemented. Install @grpc/grpc-js to enable." };
});

commandRegistry.register("grpc.stopMockServer", async (_params: GrpcStopMockServerParams) => {
  return { ok: true };
});

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

  ipcMain.handle("grpc:execute", (_e, params: GrpcExecuteParams) =>
    commandRegistry.invoke("grpc.execute", params, ctx));

  ipcMain.handle("grpc:reflect", (_e, params: GrpcReflectParams) =>
    commandRegistry.invoke("grpc.reflect", params, ctx));

  ipcMain.handle("grpc:mockServerStatus", () =>
    commandRegistry.invoke("grpc.mockServerStatus", {}, ctx));

  ipcMain.handle("grpc:startMockServer", () =>
    commandRegistry.invoke("grpc.startMockServer", {}, ctx));

  ipcMain.handle("grpc:stopMockServer", () =>
    commandRegistry.invoke("grpc.stopMockServer", {}, ctx));
}
