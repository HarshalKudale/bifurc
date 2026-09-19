import { ipcMain } from "electron";
import type {
  GrpcExecuteParams, GrpcReflectParams, GrpcMockServerStatusParams,
  GrpcStartMockServerParams, GrpcStopMockServerParams,
} from "@bifurc/protocol";
import { registerEntityCrudHandlers, registerSimpleEntityHandlers } from "@/ipc/handlers/entityCrudFactory";
import { commandRegistry } from "@bifurc/engine/commands/registry";
import { bus } from "@bifurc/engine/eventBus";

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

// ## Channels only — the registry half now lives in the engine
//
// `grpc.execute` / `reflect` / `mockServerStatus` / `startMockServer` / `stopMockServer` used to be
// registered at **module scope** in this file, which is the `plan/07` §5c defect: a containerised
// engine (P9) answered `UNKNOWN_COMMAND` for all five. Step 3e slice 4 (2026-09-19) moved them into
// `packages/engine/src/protocols/grpcCommands.ts` behind `registerGrpcCommands()`, called from
// `src/ipc/handlers.ts`.
//
// Every channel below **delegates** through `commandRegistry.invoke()` rather than duplicating the
// body — unlike `runnerHandlers.ts`, where three of the five are self-contained copies. That
// distinction decides what breaks: a delegating channel is only ever as alive as the registry behind
// it, so a suite that registers this handler group by hand **must** also call
// `registerGrpcCommands(commandRegistry)`, or the channel exists with nothing behind it. The failure
// is `No handler registered` at *invocation* time, so nothing catches it at build time.
//
// All five are deliberate stubs — `Cleanup_plan.md` §1.5 D6 disposition: **wontfix**, with the same
// note in `packages/protocol/src/commands/grpc.ts`. See the engine module for what each resolves.
//
// The three CRUD groups are a different thing and stay: `grpc:addRequest/updateRequest/deleteRequest`,
// `grpc:addMock/updateMock/deleteMock` and `grpc:addProto/deleteProto/listProtos` route through the
// CRUD collapse — see `entityCrudFactory.ts` and `plan/03-phase-2-engine-extraction.md` work item 7's
// tenth batch. The last group goes through the smaller `registerSimpleEntityHandlers()` path because
// protos don't track a `configKey` array the way every `CrudFactoryOpts` kind does (they are written
// straight to disk, nothing mirrors them into `AppConfig`) and have no "update" concept.
//
// P2 work item 7 — the five convert directly: their params match the frozen schema exactly, so no cast
// or translation is needed. That is not incidental — it is why a stub could be converted at all, since
// the schema is the contract and the bodies carry no state.
const ctx = { bus };

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

  registerSimpleEntityHandlers<SavedProtoFile>({
    kind: "protoFiles",
    ipcAdd: "grpc:addProto", ipcDelete: "grpc:deleteProto", ipcList: "grpc:listProtos",
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
