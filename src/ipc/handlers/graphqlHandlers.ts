import { ipcMain } from "electron";
import type { GraphqlIntrospectParams, GraphqlExecuteParams } from "@bifurc/protocol";
import { registerEntityCrudHandlers, registerSimpleEntityHandlers } from "@/ipc/handlers/entityCrudFactory";
import { commandRegistry } from "@bifurc/engine/commands/registry";
import { bus } from "@bifurc/engine/eventBus";

interface SavedGraphQLRequest {
  id: string; name: string; endpointUrl: string; headers: Record<string, string>;
  query: string; variables: string; operationName: string;
  preScript?: string; postScript?: string; schemaId?: string | null;
  createdAt: number; folderId?: string | null; workspaceId: string;
}

interface SavedGraphQLMock {
  id: string; name: string; enabled: boolean; endpointPattern: string; useRegex: boolean;
  operationType: "query" | "mutation" | "subscription" | "any"; operationName: string;
  responseStatus: number; responseHeaders: Record<string, string>; responseBody: string;
  responseDelay?: number; schemaId?: string | null;
  createdAt: number; folderId?: string | null; workspaceId: string;
}

interface SavedGraphQLSchema {
  id: string; name: string; content: string; endpointUrl?: string;
  introspectedAt?: number; createdAt: number; workspaceId: string;
}

// ## Channels only — the registry half now lives in the engine
//
// `graphql.introspect` / `graphql.execute` used to be registered at **module scope** in this file,
// which is the `plan/07` §5c defect: a containerised engine (P9) answered `UNKNOWN_COMMAND` for both.
// Step 3e slice 4 (2026-09-19) moved them into `packages/engine/src/protocols/graphqlCommands.ts`
// behind `registerGraphqlCommands()`, called from `src/ipc/handlers.ts`.
//
// Every channel below **delegates** through `commandRegistry.invoke()` rather than duplicating the
// body — unlike `runnerHandlers.ts`, where three of the five are self-contained copies. That
// distinction decides what breaks: a delegating channel is only ever as alive as the registry behind
// it, so a suite that registers this handler group by hand **must** also call
// `registerGraphqlCommands(commandRegistry)`, or the channel exists with nothing behind it. The
// failure is `No handler registered` at *invocation* time, so nothing catches it at build time.
//
// The three CRUD groups are a different thing and stay: `graphql:addRequest/updateRequest/
// deleteRequest`, `graphql:addMock/updateMock/deleteMock` and `graphql:addSchema/deleteSchema/
// listSchemas` route through the CRUD collapse — see `entityCrudFactory.ts` and
// `plan/03-phase-2-engine-extraction.md` work item 7's tenth batch. The last group goes through the
// smaller `registerSimpleEntityHandlers()` path because schemas don't track a `configKey` array the
// way every `CrudFactoryOpts` kind does (they are written straight to disk, nothing mirrors them into
// `AppConfig`) and have no "update" concept.
//
// P2 work item 7 — `graphql.introspect`/`execute` convert directly: the params match the frozen
// schema exactly, so no cast or translation is needed.
const ctx = { bus };

export function registerGraphqlHandlers() {
  registerEntityCrudHandlers<SavedGraphQLRequest>({
    ipcAdd: "graphql:addRequest", ipcUpdate: "graphql:updateRequest", ipcDelete: "graphql:deleteRequest",
    kind: "graphqlRequests",
    configKey: "graphqlRequests" as any,
    folderConfigKey: "graphqlRequestFolders" as any,
    getNameEntry: (req) => ({ name: req.name, endpointUrl: req.endpointUrl }),
  });

  registerEntityCrudHandlers<SavedGraphQLMock>({
    ipcAdd: "graphql:addMock", ipcUpdate: "graphql:updateMock", ipcDelete: "graphql:deleteMock",
    kind: "graphqlMocks",
    configKey: "graphqlMocks" as any,
    folderConfigKey: "graphqlMockFolders" as any,
    hasEnabledState: true,
    validate: (mock) => {
      if (!mock.endpointPattern || !mock.endpointPattern.trim()) throw new Error("endpointPattern is required for GraphQL mocks");
    },
    getNameEntry: (mock) => ({ name: mock.name, operationName: mock.operationName }),
  });

  registerSimpleEntityHandlers<SavedGraphQLSchema>({
    kind: "graphqlSchemas",
    ipcAdd: "graphql:addSchema", ipcDelete: "graphql:deleteSchema", ipcList: "graphql:listSchemas",
  });

  ipcMain.handle("graphql:introspect", (_e, params: GraphqlIntrospectParams) =>
    commandRegistry.invoke("graphql.introspect", params, ctx));

  ipcMain.handle("graphql:execute", (_e, params: GraphqlExecuteParams) =>
    commandRegistry.invoke("graphql.execute", params, ctx));
}
